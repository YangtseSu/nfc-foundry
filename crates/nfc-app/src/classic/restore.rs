//! Dump restore with pre-write backup, access checks, and stop policy.

use nfc_classic::Dump;
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, Key, KeyType, NfcError, Result};

use crate::operation::Operation;

use super::backup::{BackupRecord, BackupStore};
use super::keys::KeyStore;
use super::read::{candidate_keys, layout_for_card, read_dump};
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

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Per-block restore outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreBlockState {
    Written,
    Unchanged,
    SkippedUnknown,
    Failed,
}

impl RestoreBlockState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Written => "written",
            Self::Unchanged => "unchanged",
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
    pub skipped_unknown_blocks: usize,
    pub trailer_writes: usize,
    pub backup: Option<BackupRecord>,
    pub warnings: Vec<String>,
    pub failures: Vec<String>,
    /// One entry per block in the target layout, in block order.
    pub blocks: Vec<RestoreBlockResult>,
}

struct PendingBlock {
    number: usize,
    data: [u8; BLOCK_SIZE],
    is_trailer: bool,
}

/// Restores every known block that differs from the card, writing sector
/// trailers after ordinary data blocks. The source manufacturer block may
/// match but must never change.
pub fn restore(
    operation: &mut Operation<'_>,
    request: ClassicRestore<'_>,
) -> Result<ClassicRestoreResult> {
    request.cancel.check()?;
    if !request.confirmed {
        return Err(NfcError::InvalidArgument(
            "restore requires explicit confirmation".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let layout = layout_for_card(operation.card())?;
    request.dump.validate_geometry()?;
    if request.dump.layout != layout {
        return Err(NfcError::InvalidArgument(format!(
            "dump layout {} does not match the card layout {}",
            request.dump.layout.name(),
            layout.name()
        )));
    }
    if let Some(card) = &request.dump.card
        && !card.same_identity(operation.card())
    {
        return Err(NfcError::CardChanged);
    }

    let backup_dump = read_dump(operation, request.keys, request.include_common_keys)?.dump;
    let mut warnings = Vec::new();
    let mut skipped_unknown_blocks = 0;
    let mut outcomes: Vec<Option<RestoreBlockResult>> = vec![None; layout.block_count()];

    let source_block_zero = &request.dump.blocks[0];
    if source_block_zero.complete() {
        let current_block_zero = &backup_dump.blocks[0];
        if !current_block_zero.complete() {
            return Err(NfcError::VerificationFailed(
                "manufacturer block 0 could not be read before the restore".to_owned(),
            ));
        }
        if current_block_zero.data != source_block_zero.data {
            return Err(NfcError::VerificationFailed(
                "the restore would change manufacturer block 0; this path cannot write it"
                    .to_owned(),
            ));
        }
        outcomes[0] = Some(RestoreBlockResult {
            block: 0,
            state: RestoreBlockState::Unchanged,
            detail: Some("manufacturer block is never written".to_owned()),
        });
    } else {
        skipped_unknown_blocks += 1;
        outcomes[0] = Some(RestoreBlockResult {
            block: 0,
            state: RestoreBlockState::SkippedUnknown,
            detail: Some("manufacturer block is unknown in the dump".to_owned()),
        });
        warnings
            .push("manufacturer block 0 is unknown in the dump and will not be written".to_owned());
    }

    let mut pending: Vec<Vec<PendingBlock>> =
        (0..layout.sector_count()).map(|_| Vec::new()).collect();
    let mut unchanged_blocks = 0;
    for (number, outcome) in outcomes.iter_mut().enumerate().skip(1) {
        let source = &request.dump.blocks[number];
        if !source.complete() {
            skipped_unknown_blocks += 1;
            *outcome = Some(RestoreBlockResult {
                block: number,
                state: RestoreBlockState::SkippedUnknown,
                detail: Some("block is unknown in the dump".to_owned()),
            });
            continue;
        }
        let current = &backup_dump.blocks[number];
        if current.complete() && current.data == source.data {
            unchanged_blocks += 1;
            *outcome = Some(RestoreBlockResult {
                block: number,
                state: RestoreBlockState::Unchanged,
                detail: None,
            });
            continue;
        }
        if !current.complete() {
            return Err(NfcError::VerificationFailed(format!(
                "block {number} could not be backed up before the write"
            )));
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

    if pending.iter().all(Vec::is_empty) {
        return Ok(ClassicRestoreResult {
            written_blocks: 0,
            unchanged_blocks,
            skipped_unknown_blocks,
            trailer_writes: 0,
            backup: None,
            warnings,
            failures: Vec::new(),
            blocks: finish_outcomes(outcomes),
        });
    }

    let backup = request
        .backup
        .save(&backup_dump, &backup_label(&backup_dump, layout, "restore"))?;
    let mut written_blocks = 0;
    let mut trailer_writes = 0;
    let mut failures = Vec::new();

    for (sector, sector_pending) in pending.iter_mut().enumerate() {
        if sector_pending.is_empty() {
            continue;
        }
        sector_pending.sort_by_key(|block| block.is_trailer);
        let access = match backup_dump.access_bits(sector) {
            Ok(access) => access,
            Err(error) => {
                if request.continue_on_failure {
                    let detail = format!("sector {sector}: {error}");
                    failures.push(detail.clone());
                    mark_failed(&mut outcomes, sector_pending, &detail);
                    continue;
                }
                return Err(error);
            }
        };
        let mut allowed = AllowedTypes::all();
        for block in sector_pending.iter() {
            let mask = if block.is_trailer {
                access.groups[3].full_trailer_write_keys()
            } else {
                let group = layout
                    .access_group_for_block(block.number)
                    .expect("block is within the layout");
                access.groups[group].data_write_keys()
            };
            allowed.intersect(mask);
        }
        if !allowed.any() {
            let message = format!(
                "sector {sector}: the current access bits do not permit writing every pending block"
            );
            if request.continue_on_failure {
                failures.push(message.clone());
                mark_failed(&mut outcomes, sector_pending, &message);
                continue;
            }
            return Err(NfcError::Unsupported(
                "the current access bits do not permit writing every pending block",
            ));
        }
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
                mark_failed(&mut outcomes, sector_pending, &message);
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
                    outcomes[block.number] = Some(RestoreBlockResult {
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
                        outcomes[block.number] = Some(RestoreBlockResult {
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

    if !failures.is_empty() {
        warnings.push(format!(
            "restore continued after {} failure(s)",
            failures.len()
        ));
    }
    Ok(ClassicRestoreResult {
        written_blocks,
        unchanged_blocks,
        skipped_unknown_blocks,
        trailer_writes,
        backup: Some(backup),
        warnings,
        failures,
        blocks: finish_outcomes(outcomes),
    })
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
