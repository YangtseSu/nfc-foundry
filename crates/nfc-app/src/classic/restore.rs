//! Dump restore with preflight plans, selective writes, and a stop policy.

use nfc_classic::{AccessBits, Dump, KeyMask, Layout};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, Key, KeyType, NfcError, Result};

use crate::operation::Operation;

use super::backup::{BackupRecord, BackupStore};
use super::keys::KeyStore;
use super::plan::{
    ClassicWritePlan, ClassicWritePreflight, PlannedWriteBlock, RestoreSelection, WriteToken,
    unix_now, validate_source,
};
use super::read::{candidate_keys, layout_for_card, read_dump};
use super::workbench::dump_fingerprint;
use super::write::{AllowedTypes, backup_label, check_expected_card};

/// Request to restore a dump onto the selected card.
pub struct ClassicRestore<'a> {
    pub dump: Dump,
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    /// Restore mutates the card, so the caller must confirm it openly.
    pub confirmed: bool,
    /// Default stops at the first failure. The caller must opt in to continue.
    pub continue_on_failure: bool,
    /// Identity of the image the restore was composed from. The dump's own
    /// identity is always checked; this field guards an image without one.
    pub expected_card: Option<CardInfo>,
    /// Blocks the caller wants to write. Full-card restore is the default.
    pub selection: RestoreSelection,
    /// Baseline for `ChangesOnly`. When absent, the live card is the baseline.
    pub baseline: Option<Dump>,
    /// Compute the plan and report it without mutating the card.
    pub dry_run: bool,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> ClassicRestore<'a> {
    pub fn new(dump: Dump, keys: &'a KeyStore, backup: &'a BackupStore) -> Self {
        Self {
            dump,
            keys,
            include_common_keys: true,
            confirmed: false,
            continue_on_failure: false,
            expected_card: None,
            selection: RestoreSelection::All,
            baseline: None,
            dry_run: false,
            backup,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn confirmed(mut self, confirmed: bool) -> Self {
        self.confirmed = confirmed;
        self
    }

    pub fn continue_on_failure(mut self, continue_on_failure: bool) -> Self {
        self.continue_on_failure = continue_on_failure;
        self
    }

    pub fn expected_card(mut self, card: CardInfo) -> Self {
        self.expected_card = Some(card);
        self
    }

    pub fn selection(mut self, selection: RestoreSelection) -> Self {
        self.selection = selection;
        self
    }

    pub fn baseline(mut self, baseline: Dump) -> Self {
        self.baseline = Some(baseline);
        self
    }

    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Per-block restore outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreBlockState {
    Planned,
    Written,
    Unchanged,
    NotSelected,
    SkippedUnknown,
    Failed,
}

impl RestoreBlockState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Written => "written",
            Self::Unchanged => "unchanged",
            Self::NotSelected => "not selected",
            Self::SkippedUnknown => "unknown",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreBlockResult {
    pub block: usize,
    pub state: RestoreBlockState,
    pub detail: Option<String>,
}

/// Outcome of a restore. `failures` is non-empty only when the continue policy
/// was selected.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicRestoreResult {
    pub written_blocks: usize,
    pub unchanged_blocks: usize,
    pub not_selected_blocks: usize,
    pub skipped_unknown_blocks: usize,
    pub trailer_writes: usize,
    pub planned_blocks: usize,
    pub dry_run: bool,
    pub backup: Option<BackupRecord>,
    pub warnings: Vec<String>,
    pub failures: Vec<String>,
    /// One entry per block in the target layout, in block order.
    pub blocks: Vec<RestoreBlockResult>,
}

#[derive(Clone, Debug)]
struct PendingBlock {
    number: usize,
    data: [u8; BLOCK_SIZE],
    is_trailer: bool,
}

struct PreparedRestore {
    backup: Dump,
    warnings: Vec<String>,
    outcomes: Vec<Option<RestoreBlockResult>>,
    pending: Vec<Vec<PendingBlock>>,
    blocked: Vec<Option<String>>,
    allowed: Vec<AllowedTypes>,
    selected_blocks: Vec<usize>,
    unchanged_blocks: usize,
    not_selected_blocks: usize,
    skipped_unknown_blocks: usize,
}

struct PrepareRequest<'a> {
    dump: &'a Dump,
    keys: &'a KeyStore,
    include_common_keys: bool,
    expected_card: Option<&'a CardInfo>,
    selection: &'a RestoreSelection,
    baseline: Option<&'a Dump>,
    continue_on_failure: bool,
}

impl<'a> PrepareRequest<'a> {
    fn from_restore(request: &'a ClassicRestore<'_>) -> Self {
        Self {
            dump: &request.dump,
            keys: request.keys,
            include_common_keys: request.include_common_keys,
            expected_card: request.expected_card.as_ref(),
            selection: &request.selection,
            baseline: request.baseline.as_ref(),
            continue_on_failure: request.continue_on_failure,
        }
    }

    fn from_preflight(request: &'a ClassicWritePreflight<'_>) -> Self {
        Self {
            dump: &request.dump,
            keys: request.keys,
            include_common_keys: request.include_common_keys,
            expected_card: request.expected_card.as_ref(),
            selection: &request.selection,
            baseline: request.baseline.as_ref(),
            continue_on_failure: request.continue_on_failure,
        }
    }
}

/// Restores selected known blocks that differ from the card, writing sector
/// trailers after ordinary data blocks. The source manufacturer block may
/// match but must never change.
pub fn restore(
    operation: &mut Operation<'_>,
    request: ClassicRestore<'_>,
) -> Result<ClassicRestoreResult> {
    request.cancel.check()?;
    if !request.confirmed && !request.dry_run {
        return Err(NfcError::InvalidArgument(
            "restore requires explicit confirmation".to_owned(),
        ));
    }
    let prepared = prepare(operation, &PrepareRequest::from_restore(&request))?;
    execute(operation, &request, prepared)
}

/// Executes a plan produced by `Service::classic_write_preflight`. The plan's
/// source fingerprint, expiry, card, and current block values are rechecked
/// before the backup and the first write.
pub(crate) fn restore_with_plan(
    operation: &mut Operation<'_>,
    request: ClassicRestore<'_>,
    plan: &ClassicWritePlan,
) -> Result<ClassicRestoreResult> {
    request.cancel.check()?;
    if !request.confirmed {
        return Err(NfcError::InvalidArgument(
            "restore requires explicit confirmation".to_owned(),
        ));
    }
    if plan.expires_at <= unix_now() {
        return Err(NfcError::InvalidArgument(
            "the write plan expired; run the preflight again".to_owned(),
        ));
    }
    if dump_fingerprint(&request.dump) != plan.source_fingerprint {
        return Err(NfcError::CardChanged);
    }
    if !plan.card.same_identity(operation.card()) {
        return Err(NfcError::CardChanged);
    }
    let prepared = prepare(operation, &PrepareRequest::from_restore(&request))?;
    verify_plan(plan, &prepared)?;
    execute(operation, &request, prepared)
}

/// Reads the card and produces a write plan without mutating it.
pub(crate) fn preflight(
    operation: &mut Operation<'_>,
    request: &ClassicWritePreflight<'_>,
    token: WriteToken,
) -> Result<ClassicWritePlan> {
    request.cancel.check()?;
    let prepare_request = PrepareRequest::from_preflight(request);
    let prepared = prepare(operation, &prepare_request)?;
    plan_from_prepared(operation.card(), &prepare_request, &prepared, token)
}

fn prepare(operation: &mut Operation<'_>, request: &PrepareRequest<'_>) -> Result<PreparedRestore> {
    check_expected_card(operation.card(), request.expected_card)?;
    let layout = layout_for_card(operation.card())?;
    validate_source(request.dump, operation.card())?;

    let backup = read_dump(operation, request.keys, request.include_common_keys)?.dump;
    let selected = selected_blocks(
        request.selection,
        request.dump,
        &backup,
        request.baseline,
        layout,
    )?;
    let mut selected_mask = vec![false; layout.block_count()];
    for &block in &selected {
        selected_mask[block] = true;
    }

    let mut warnings = Vec::new();
    let mut skipped_unknown_blocks = 0;
    let mut unchanged_blocks = 0;
    let mut not_selected_blocks = 0;
    let mut outcomes: Vec<Option<RestoreBlockResult>> = vec![None; layout.block_count()];
    let mut pending: Vec<Vec<PendingBlock>> =
        (0..layout.sector_count()).map(|_| Vec::new()).collect();

    for (number, outcome) in outcomes.iter_mut().enumerate() {
        if !selected_mask[number] {
            not_selected_blocks += 1;
            *outcome = Some(RestoreBlockResult {
                block: number,
                state: RestoreBlockState::NotSelected,
                detail: None,
            });
            continue;
        }
        let source = &request.dump.blocks[number];
        if !source.complete() {
            skipped_unknown_blocks += 1;
            *outcome = Some(RestoreBlockResult {
                block: number,
                state: RestoreBlockState::SkippedUnknown,
                detail: Some("block is unknown in the dump".to_owned()),
            });
            if number == 0 {
                warnings.push(
                    "manufacturer block 0 is unknown in the dump and will not be written"
                        .to_owned(),
                );
            }
            continue;
        }
        let current = &backup.blocks[number];
        if number == 0 {
            if !current.complete() {
                return Err(NfcError::VerificationFailed(
                    "manufacturer block 0 could not be read before the restore".to_owned(),
                ));
            }
            if current.data != source.data {
                return Err(NfcError::VerificationFailed(
                    "the restore would change manufacturer block 0; this path cannot write it"
                        .to_owned(),
                ));
            }
            *outcome = Some(RestoreBlockResult {
                block: 0,
                state: RestoreBlockState::Unchanged,
                detail: Some("manufacturer block is never written".to_owned()),
            });
            continue;
        }
        if !current.complete() {
            return Err(NfcError::VerificationFailed(format!(
                "block {number} could not be backed up before the write"
            )));
        }
        if current.data == source.data {
            unchanged_blocks += 1;
            *outcome = Some(RestoreBlockResult {
                block: number,
                state: RestoreBlockState::Unchanged,
                detail: None,
            });
            continue;
        }
        let sector = layout
            .sector_for_block(number)
            .expect("block is within the layout");
        pending[sector].push(PendingBlock {
            number,
            data: source.data,
            is_trailer: layout
                .is_trailer(number)
                .expect("block is within the layout"),
        });
    }

    let mut blocked = vec![None; layout.sector_count()];
    let mut allowed = vec![AllowedTypes::all(); layout.sector_count()];
    for (sector, sector_pending) in pending.iter_mut().enumerate() {
        if sector_pending.is_empty() {
            continue;
        }
        sector_pending.sort_by_key(|block| block.is_trailer);
        let access = match backup.access_bits(sector) {
            Ok(access) => access,
            Err(error) => {
                let message = format!("sector {sector}: {error}");
                if request.continue_on_failure {
                    blocked[sector] = Some(message.clone());
                    warnings.push(message);
                    continue;
                }
                return Err(error);
            }
        };
        let mut sector_allowed = AllowedTypes::all();
        for block in sector_pending.iter() {
            let mask = write_key_mask(&access, layout, block);
            sector_allowed.intersect(mask);
        }
        if !sector_allowed.any() {
            let message = format!(
                "sector {sector}: the current access bits do not permit writing every pending block"
            );
            if request.continue_on_failure {
                blocked[sector] = Some(message.clone());
                warnings.push(message);
                continue;
            }
            return Err(NfcError::Unsupported(
                "the current access bits do not permit writing every pending block",
            ));
        }
        if !candidate_keys(
            request.keys,
            operation.card(),
            request.include_common_keys,
            sector,
        )
        .into_iter()
        .any(|(key_type, _, _)| sector_allowed.allows(key_type))
        {
            let message =
                format!("sector {sector}: no candidate key is allowed by the access bits");
            if request.continue_on_failure {
                blocked[sector] = Some(message.clone());
                warnings.push(message);
                continue;
            }
            return Err(NfcError::AuthenticationFailed);
        }
        allowed[sector] = sector_allowed;
    }

    Ok(PreparedRestore {
        backup,
        warnings,
        outcomes,
        pending,
        blocked,
        allowed,
        selected_blocks: selected,
        unchanged_blocks,
        not_selected_blocks,
        skipped_unknown_blocks,
    })
}

fn selected_blocks(
    selection: &RestoreSelection,
    source: &Dump,
    current: &Dump,
    baseline: Option<&Dump>,
    layout: Layout,
) -> Result<Vec<usize>> {
    let mut blocks = match selection {
        RestoreSelection::All => (0..layout.block_count()).collect(),
        RestoreSelection::Blocks(blocks) => {
            let mut blocks = blocks.clone();
            for &block in &blocks {
                if block >= layout.block_count() {
                    return Err(NfcError::InvalidArgument(format!(
                        "block {block} is outside the {} layout",
                        layout.name()
                    )));
                }
            }
            blocks.sort_unstable();
            blocks.dedup();
            blocks
        }
        RestoreSelection::ChangesOnly => {
            let mut blocks = Vec::new();
            for number in 0..layout.block_count() {
                let changed = match baseline {
                    Some(baseline) => {
                        !blocks_equal(&source.blocks[number], &baseline.blocks[number])
                    }
                    None => !blocks_equal(&source.blocks[number], &current.blocks[number]),
                };
                if changed {
                    blocks.push(number);
                }
            }
            if blocks.is_empty() {
                return Err(NfcError::InvalidArgument(
                    "changes-only write has no changed blocks".to_owned(),
                ));
            }
            blocks
        }
    };
    blocks.sort_unstable();
    blocks.dedup();
    Ok(blocks)
}

fn blocks_equal(left: &nfc_classic::Block, right: &nfc_classic::Block) -> bool {
    left.known_mask == right.known_mask && left.data == right.data
}

fn write_key_mask(access: &AccessBits, layout: Layout, block: &PendingBlock) -> KeyMask {
    if block.is_trailer {
        access.groups[3].full_trailer_write_keys()
    } else {
        let group = layout
            .access_group_for_block(block.number)
            .expect("block is within the layout");
        access.groups[group].data_write_keys()
    }
}

fn plan_from_prepared(
    card: &CardInfo,
    request: &PrepareRequest<'_>,
    prepared: &PreparedRestore,
    token: WriteToken,
) -> Result<ClassicWritePlan> {
    let mut blocks = Vec::new();
    for sector in 0..prepared.pending.len() {
        let allowed = prepared.allowed[sector];
        let write_keys = allowed.types();
        for block in &prepared.pending[sector] {
            blocks.push(PlannedWriteBlock {
                block: block.number,
                sector,
                trailer: block.is_trailer,
                old: prepared.backup.blocks[block.number].data,
                new: block.data,
                write_keys,
                access_note: format!("write with {}", write_keys.label()),
            });
        }
    }
    Ok(ClassicWritePlan {
        token,
        layout: prepared.backup.layout,
        card: card.clone(),
        selection: request.selection.clone(),
        selected_blocks: prepared.selected_blocks.clone(),
        blocks,
        unchanged_blocks: prepared.unchanged_blocks,
        skipped_unknown_blocks: prepared.skipped_unknown_blocks,
        warnings: prepared.warnings.clone(),
        expires_at: unix_now() + super::plan::WRITE_PLAN_TTL.as_secs(),
        source_fingerprint: dump_fingerprint(request.dump),
        card_fingerprint: dump_fingerprint(&prepared.backup),
        continue_on_failure: request.continue_on_failure,
    })
}

fn verify_plan(plan: &ClassicWritePlan, prepared: &PreparedRestore) -> Result<()> {
    if plan.selected_blocks != prepared.selected_blocks
        || plan.card_fingerprint != dump_fingerprint(&prepared.backup)
    {
        return Err(NfcError::CardChanged);
    }
    let mut planned: Vec<(usize, [u8; BLOCK_SIZE], [u8; BLOCK_SIZE])> = plan
        .blocks
        .iter()
        .map(|block| (block.block, block.old, block.new))
        .collect();
    let mut current: Vec<(usize, [u8; BLOCK_SIZE], [u8; BLOCK_SIZE])> = Vec::new();
    for sector_pending in &prepared.pending {
        for block in sector_pending {
            current.push((
                block.number,
                prepared.backup.blocks[block.number].data,
                block.data,
            ));
        }
    }
    planned.sort_unstable_by_key(|entry| entry.0);
    current.sort_unstable_by_key(|entry| entry.0);
    if planned != current {
        return Err(NfcError::CardChanged);
    }
    Ok(())
}

fn execute(
    operation: &mut Operation<'_>,
    request: &ClassicRestore<'_>,
    mut prepared: PreparedRestore,
) -> Result<ClassicRestoreResult> {
    let layout = prepared.backup.layout;
    let pending_blocks: usize = prepared.pending.iter().map(Vec::len).sum();
    if request.dry_run {
        for (sector, sector_pending) in prepared.pending.iter().enumerate() {
            if let Some(message) = &prepared.blocked[sector] {
                mark_failed(&mut prepared.outcomes, sector_pending, message);
                continue;
            }
            for block in sector_pending {
                prepared.outcomes[block.number] = Some(RestoreBlockResult {
                    block: block.number,
                    state: RestoreBlockState::Planned,
                    detail: None,
                });
            }
        }
        let mut warnings = prepared.warnings.clone();
        warnings.push("dry run: no card writes were performed".to_owned());
        return Ok(result_from(
            &mut prepared,
            warnings,
            Vec::new(),
            0,
            0,
            pending_blocks,
            true,
            None,
        ));
    }

    if pending_blocks == 0 {
        let warnings = prepared.warnings.clone();
        return Ok(result_from(
            &mut prepared,
            warnings,
            Vec::new(),
            0,
            0,
            0,
            false,
            None,
        ));
    }

    let backup = request.backup.save(
        &prepared.backup,
        &backup_label(&prepared.backup, layout, "restore"),
    )?;
    let mut written_blocks = 0;
    let mut trailer_writes = 0;
    let mut failures = Vec::new();

    for (sector, sector_pending) in prepared.pending.iter().enumerate() {
        if sector_pending.is_empty() {
            continue;
        }
        if let Some(message) = &prepared.blocked[sector] {
            failures.push(message.clone());
            mark_failed(&mut prepared.outcomes, sector_pending, message);
            continue;
        }
        let allowed = prepared.allowed[sector];
        let candidates: Vec<(KeyType, Key)> = candidate_keys(
            request.keys,
            operation.card(),
            request.include_common_keys,
            sector,
        )
        .into_iter()
        .filter(|(key_type, _, _)| allowed.allows(*key_type))
        .map(|(key_type, key, _)| (key_type, key))
        .collect();
        let first = layout
            .first_block(sector)
            .expect("sector is within the layout");
        let mut authenticated = false;
        for (key_type, key) in candidates {
            request.cancel.check()?;
            match operation.authenticate(first as u8, key_type, key) {
                Ok(()) => {
                    authenticated = true;
                    break;
                }
                Err(NfcError::AuthenticationFailed) => continue,
                Err(error) => return Err(error),
            }
        }
        if !authenticated {
            let message = format!("sector {sector}: no candidate key was accepted");
            if request.continue_on_failure {
                failures.push(message.clone());
                mark_failed(&mut prepared.outcomes, sector_pending, &message);
                continue;
            }
            return Err(NfcError::AuthenticationFailed);
        }

        for block in sector_pending.iter() {
            request.cancel.check()?;
            match operation.write_block_verified(block.number as u8, block.data) {
                Ok(()) => {
                    written_blocks += 1;
                    if block.is_trailer {
                        trailer_writes += 1;
                    }
                    prepared.outcomes[block.number] = Some(RestoreBlockResult {
                        block: block.number,
                        state: RestoreBlockState::Written,
                        detail: None,
                    });
                }
                Err(error) if is_fatal_write_error(&error) => return Err(error),
                Err(error) => {
                    let message = format!("block {}: {error}", block.number);
                    if request.continue_on_failure {
                        failures.push(message.clone());
                        prepared.outcomes[block.number] = Some(RestoreBlockResult {
                            block: block.number,
                            state: RestoreBlockState::Failed,
                            detail: Some(message),
                        });
                        continue;
                    }
                    return Err(NfcError::VerificationFailed(message));
                }
            }
        }
        operation.check_card()?;
    }

    let mut warnings = prepared.warnings.clone();
    if !failures.is_empty() {
        warnings.push(format!(
            "restore continued after {} failure(s)",
            failures.len()
        ));
    }
    Ok(result_from(
        &mut prepared,
        warnings,
        failures,
        written_blocks,
        trailer_writes,
        pending_blocks,
        false,
        Some(backup),
    ))
}

#[allow(clippy::too_many_arguments)]
fn result_from(
    prepared: &mut PreparedRestore,
    warnings: Vec<String>,
    failures: Vec<String>,
    written_blocks: usize,
    trailer_writes: usize,
    planned_blocks: usize,
    dry_run: bool,
    backup: Option<BackupRecord>,
) -> ClassicRestoreResult {
    let outcomes = std::mem::take(&mut prepared.outcomes);
    ClassicRestoreResult {
        written_blocks,
        unchanged_blocks: prepared.unchanged_blocks,
        not_selected_blocks: prepared.not_selected_blocks,
        skipped_unknown_blocks: prepared.skipped_unknown_blocks,
        trailer_writes,
        planned_blocks,
        dry_run,
        backup,
        warnings,
        failures,
        blocks: finish_outcomes(outcomes),
    }
}

fn mark_failed(outcomes: &mut [Option<RestoreBlockResult>], blocks: &[PendingBlock], detail: &str) {
    for block in blocks {
        outcomes[block.number] = Some(RestoreBlockResult {
            block: block.number,
            state: RestoreBlockState::Failed,
            detail: Some(detail.to_owned()),
        });
    }
}

fn finish_outcomes(outcomes: Vec<Option<RestoreBlockResult>>) -> Vec<RestoreBlockResult> {
    outcomes
        .into_iter()
        .enumerate()
        .map(|(block, outcome)| {
            outcome.unwrap_or(RestoreBlockResult {
                block,
                state: RestoreBlockState::Failed,
                detail: Some("block was not attempted".to_owned()),
            })
        })
        .collect()
}

fn is_fatal_write_error(error: &NfcError) -> bool {
    matches!(
        error,
        NfcError::Cancelled
            | NfcError::NoCard
            | NfcError::NoReader
            | NfcError::Timeout(_)
            | NfcError::CardChanged
            | NfcError::Unsupported(_)
    )
}
