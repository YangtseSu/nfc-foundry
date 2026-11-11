// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recovery strategy selection and execution.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use nfc_classic::Layout;
use nfc_core::{CancelToken, CardType, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::auto::{CardProfile, RecoveryStrategy};
use crate::classify;
use crate::darkside;
use crate::hardnested::{self, HardnestedEvent, HardnestedRequest};
use crate::nested::{self, NestedRequest};

/// Darkside probes attempted before the seed acquisition gives up. Each probe
/// reselects the card, so cancellation and the attempt cap stay responsive.
const DARKSIDE_ATTEMPTS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackKind {
    Dictionary,
    Ev1Signature,
    DarkSide,
    Nested,
    StaticNested,
    HardNested,
    Fm11rf08s,
}

/// Progress events from long-running recovery acquisition and brute force.
#[derive(Clone, Copy, Debug)]
pub enum RecoveryProgress {
    /// The all-sector candidate scan finished one sector.
    Dictionary {
        sector: usize,
        sector_count: usize,
        found: usize,
    },
    /// Darkside seed acquisition reported its attempt count.
    Darkside {
        attempts: usize,
    },
    /// A missing sector key is being attacked.
    Sector {
        sector: usize,
        sector_count: usize,
        hardnested: bool,
    },
    /// Read verification finished one sector.
    Verify {
        sector: usize,
        sector_count: usize,
    },
    Acquiring {
        received: usize,
        distinct_nonces: usize,
        first_bytes: usize,
        expected_states: f64,
    },
    Filtering {
        expected_states: f64,
    },
    BruteForce {
        tested_states: u64,
        total_states: u64,
        expected_states: f64,
    },
}

pub type ProgressSink = Arc<dyn Fn(RecoveryProgress) + Send + Sync>;

#[derive(Clone)]
pub struct RecoveryRequest {
    pub known_block: u8,
    pub known_key_type: KeyType,
    pub target_block: u8,
    pub target_key_type: KeyType,
    /// Candidate keys tried during the auto scan and against `known_block`
    /// before a nested attack.
    pub candidates: Vec<(KeyType, Key)>,
    pub cancel: CancelToken,
    /// Optional caller-visible progress. Events are throttled by the engine.
    pub progress: Option<ProgressSink>,
    /// Hardnested table directory; `None` uses the installed resource path.
    pub tables_dir: Option<PathBuf>,
    /// Hardnested acquisition cap; zero uses the engine default.
    pub max_nonces: usize,
}

impl fmt::Debug for RecoveryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryRequest")
            .field("known_block", &self.known_block)
            .field("known_key_type", &self.known_key_type)
            .field("target_block", &self.target_block)
            .field("target_key_type", &self.target_key_type)
            .field("candidates", &self.candidates.len())
            .field("cancel", &self.cancel)
            .field("progress", &self.progress.is_some())
            .field("tables_dir", &self.tables_dir)
            .field("max_nonces", &self.max_nonces)
            .finish()
    }
}

impl Default for RecoveryRequest {
    fn default() -> Self {
        Self {
            known_block: 4,
            known_key_type: KeyType::A,
            target_block: 0,
            target_key_type: KeyType::A,
            candidates: Vec::new(),
            cancel: CancelToken::new(),
            progress: None,
            tables_dir: None,
            max_nonces: 0,
        }
    }
}

/// One key that authenticated its sector and passed a trailer read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveredKey {
    pub sector: usize,
    pub key_type: KeyType,
    pub key: Key,
    pub method: AttackKind,
}

#[derive(Clone, Debug, Default)]
pub struct RecoveryResult {
    /// Card selection identity for key capture; set by the application layer.
    pub card: Option<nfc_core::CardInfo>,
    pub keys: Vec<(KeyType, Key)>,
    /// Per-sector attribution for auto recovery; empty for single-target
    /// strategies.
    pub sector_keys: Vec<RecoveredKey>,
    pub attempts: Vec<AttackKind>,
    pub profile: Option<CardProfile>,
    pub note: String,
}

/// Runs one recovery strategy with one reader-reopen retry for transport
/// failures.
pub fn recover(
    reader: &mut dyn Reader,
    strategy: RecoveryStrategy,
    request: &RecoveryRequest,
) -> Result<RecoveryResult> {
    match run_once(reader, strategy, request) {
        Ok(result) => Ok(result),
        Err(error) if retryable(&error) => {
            reopen(reader)?;
            run_once(reader, strategy, request)
        }
        Err(error) => Err(error),
    }
}

fn run_once(
    reader: &mut dyn Reader,
    strategy: RecoveryStrategy,
    request: &RecoveryRequest,
) -> Result<RecoveryResult> {
    request.cancel.check()?;
    match strategy {
        RecoveryStrategy::Auto => auto(reader, request),
        RecoveryStrategy::DarkSide => darkside_attack(reader, request),
        RecoveryStrategy::Nested => nested_attack(reader, request, false),
        RecoveryStrategy::StaticNested => nested_attack(reader, request, true),
        RecoveryStrategy::HardNested => hardnested_attack(reader, request),
        RecoveryStrategy::Fm11rf08s => fm11rf08s_attack(reader, request),
        RecoveryStrategy::Dictionary => {
            let found = find_known_key(reader, request)?;
            Ok(RecoveryResult {
                card: None,
                keys: found.into_iter().collect(),
                sector_keys: Vec::new(),
                attempts: vec![AttackKind::Dictionary],
                profile: None,
                note: "dictionary hit".to_owned(),
            })
        }
    }
}

/// PM3/MFCUK-style autopwn: scan every sector with the candidate dictionary,
/// take a darkside seed when no candidate authenticates, recover the remaining
/// sector keys with nested then hardnested, and verify the result with a full
/// read. The known and target fields are not used.
fn auto(reader: &mut dyn Reader, request: &RecoveryRequest) -> Result<RecoveryResult> {
    request.cancel.check()?;
    let card = reader.card_info()?;
    let layout = classic_layout(&card)?;
    let sector_count = layout.sector_count();
    let mut found: Vec<RecoveredKey> = Vec::new();
    let mut attempts: Vec<AttackKind> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    // 1. Candidate dictionary on every sector, with a trailer read as an extra
    // check when access bits allow it.
    for sector in 0..sector_count {
        request.cancel.check()?;
        scan_sector(reader, layout, sector, request, &mut found)?;
        emit(
            request,
            RecoveryProgress::Dictionary {
                sector,
                sector_count,
                found: found.len(),
            },
        );
    }
    if !found.is_empty() {
        attempts.push(AttackKind::Dictionary);
    }

    // 2. EV1/QL88 signature key as a known key when no candidate matched. The
    // signature blocks only exist on 4K cards.
    if found.is_empty() && layout.block_count() > usize::from(classify::EV1_SIGNATURE_BLOCK) {
        match classify::detect_ev1_signature(reader) {
            Ok(true) => {
                let sector = layout.sector_for_block(usize::from(classify::EV1_SIGNATURE_BLOCK))?;
                push_found(
                    &mut found,
                    sector,
                    KeyType::B,
                    classify::EV1_SIGNATURE_KEY,
                    AttackKind::Ev1Signature,
                );
                attempts.push(AttackKind::Ev1Signature);
                notes.push("EV1 signature key used as the known key".to_owned());
            }
            Ok(false) => {}
            Err(NfcError::Unsupported(message)) => {
                notes.push(format!("EV1 signature detection unavailable: {message}"));
            }
            Err(error) if fatal(&error) => return Err(error),
            Err(error) => notes.push(format!("EV1 signature detection failed: {error}")),
        }
    }

    // 3. Darkside seed when nothing authenticated.
    if found.is_empty() {
        let Some(seed) = darkside_seed(reader, layout, request, &mut notes)? else {
            return Err(NfcError::Unsupported(
                "no candidate key authenticated and darkside produced no verified seed key; the card must repeat a tag nonce for host-driven darkside",
            ));
        };
        let (sector, key_type, key) = seed;
        push_found(&mut found, sector, key_type, key, AttackKind::DarkSide);
        attempts.push(AttackKind::DarkSide);
    }

    // 4. Recover every missing sector key.
    if found.len() < sector_count * 2 {
        let seed = found[0];
        let seed_block = layout.first_block(seed.sector)? as u8;
        let static_encrypted =
            match classify::detect_static_encrypted(reader, seed_block, seed.key_type, seed.key) {
                Ok(value) => value,
                Err(NfcError::Unsupported(message)) => {
                    notes.push(format!("static encrypted detection unavailable: {message}"));
                    false
                }
                Err(error) if fatal(&error) => return Err(error),
                Err(error) => {
                    notes.push(format!("static encrypted detection failed: {error}"));
                    false
                }
            };
        if static_encrypted {
            notes.push(static_encrypted_message().to_string());
        } else {
            recover_missing(
                reader,
                layout,
                request,
                &mut found,
                &mut attempts,
                &mut notes,
            )?;
        }
    }

    // 5. Read every recovered sector with its verified key.
    let readable = verify_reads(reader, layout, request, &found, &mut notes)?;

    let mut keys: Vec<(KeyType, Key)> = Vec::new();
    for entry in &found {
        let candidate = (entry.key_type, entry.key);
        if !keys.contains(&candidate) {
            keys.push(candidate);
        }
    }
    let mut note = format!(
        "sector keys {}/{} · read verification {}/{} sectors",
        found.len(),
        sector_count * 2,
        readable,
        sector_count
    );
    for entry in &notes {
        note.push_str(" · ");
        note.push_str(entry);
    }
    Ok(RecoveryResult {
        card: None,
        keys,
        sector_keys: found,
        attempts,
        profile: None,
        note,
    })
}

fn classic_layout(card: &nfc_core::CardInfo) -> Result<Layout> {
    match card.card_type {
        CardType::MifareClassic1K => Ok(Layout::Classic1K),
        CardType::MifareClassic4K => Ok(Layout::Classic4K),
        _ => Err(NfcError::Unsupported(
            "auto recovery needs a MIFARE Classic 1K or 4K card",
        )),
    }
}

fn scan_sector(
    reader: &mut dyn Reader,
    layout: Layout,
    sector: usize,
    request: &RecoveryRequest,
    found: &mut Vec<RecoveredKey>,
) -> Result<()> {
    let first = layout.first_block(sector)? as u8;
    let trailer = layout.trailer_block(sector)? as u8;
    for key_type in [KeyType::A, KeyType::B] {
        if found
            .iter()
            .any(|entry| entry.sector == sector && entry.key_type == key_type)
        {
            continue;
        }
        for &(candidate_type, key) in &request.candidates {
            if candidate_type != key_type {
                continue;
            }
            request.cancel.check()?;
            reader.card_info()?;
            match reader.authenticate(first, key_type, key) {
                Ok(()) => {
                    // Authentication is the proof. The trailer read is an
                    // extra check; access bits may forbid it without making
                    // the key invalid for nested recovery.
                    if let Err(error) = reader.read_block(trailer)
                        && fatal(&error)
                    {
                        return Err(error);
                    }
                    push_found(found, sector, key_type, key, AttackKind::Dictionary);
                    break;
                }
                Err(NfcError::AuthenticationFailed) => continue,
                Err(error) if fatal(&error) => return Err(error),
                Err(_) => continue,
            }
        }
    }
    Ok(())
}

fn darkside_seed(
    reader: &mut dyn Reader,
    layout: Layout,
    request: &RecoveryRequest,
    notes: &mut Vec<String>,
) -> Result<Option<(usize, KeyType, Key)>> {
    let first = layout.first_block(0)? as u8;
    for key_type in [KeyType::A, KeyType::B] {
        let sample = match darkside::collect_with_progress(
            reader,
            first,
            key_type,
            &request.cancel,
            DARKSIDE_ATTEMPTS,
            &mut |attempts| emit(request, RecoveryProgress::Darkside { attempts }),
        ) {
            Ok(sample) => sample,
            Err(NfcError::Unsupported(message)) => {
                notes.push(format!("darkside unavailable: {message}"));
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let Some(sample) = sample else {
            notes.push(format!(
                "darkside found no repeated tag nonce for key {key_type:?}"
            ));
            continue;
        };
        for key in darkside::solve(&sample) {
            request.cancel.check()?;
            reader.card_info()?;
            if reader.authenticate(first, key_type, key).is_ok() {
                return Ok(Some((0, key_type, key)));
            }
        }
        notes.push(format!(
            "darkside sample for key {key_type:?} produced no verified key"
        ));
    }
    Ok(None)
}

fn recover_missing(
    reader: &mut dyn Reader,
    layout: Layout,
    request: &RecoveryRequest,
    found: &mut Vec<RecoveredKey>,
    attempts: &mut Vec<AttackKind>,
    notes: &mut Vec<String>,
) -> Result<()> {
    let sector_count = layout.sector_count();
    let mut tables: Option<hardnested::tables::Tables> = None;
    let mut hardnested_unavailable = false;
    'sectors: for sector in 0..sector_count {
        for key_type in [KeyType::A, KeyType::B] {
            request.cancel.check()?;
            if found
                .iter()
                .any(|entry| entry.sector == sector && entry.key_type == key_type)
            {
                continue;
            }
            let Some((known_block, known_type, known_key)) =
                choose_seed(layout, found, sector, key_type)
            else {
                continue;
            };
            let mut target = request.clone();
            target.target_block = layout.first_block(sector)? as u8;
            target.target_key_type = key_type;

            emit(
                request,
                RecoveryProgress::Sector {
                    sector,
                    sector_count,
                    hardnested: false,
                },
            );
            match nested_attack_with(
                reader,
                &target,
                known_block,
                known_type,
                known_key,
                AttackKind::Nested,
            ) {
                Ok(result) => {
                    record_result(found, sector, result, AttackKind::Nested);
                    push_attempt(attempts, AttackKind::Nested);
                    continue;
                }
                Err(NfcError::AuthenticationFailed) => {}
                Err(NfcError::Unsupported(message)) => {
                    notes.push(format!("nested unavailable: {message}"));
                    break 'sectors;
                }
                Err(error) if fatal(&error) => return Err(error),
                Err(error) => {
                    notes.push(format!("sector {sector} nested failed: {error}"));
                    continue;
                }
            }

            if hardnested_unavailable {
                continue;
            }
            if tables.is_none() {
                match load_hardnested_tables(request) {
                    Ok(loaded) => tables = Some(loaded),
                    Err(error) => {
                        hardnested_unavailable = true;
                        notes.push(format!("hardnested unavailable: {error}"));
                        continue;
                    }
                }
            }
            emit(
                request,
                RecoveryProgress::Sector {
                    sector,
                    sector_count,
                    hardnested: true,
                },
            );
            match hardnested_attack_with(
                reader,
                &target,
                known_block,
                known_type,
                known_key,
                tables.as_ref().expect("tables were just loaded"),
            ) {
                Ok(result) => {
                    record_result(found, sector, result, AttackKind::HardNested);
                    push_attempt(attempts, AttackKind::HardNested);
                }
                Err(NfcError::AuthenticationFailed) => {
                    notes.push(format!("sector {sector} {key_type:?}: no key recovered"));
                }
                Err(NfcError::Unsupported(message)) => {
                    notes.push(format!("hardnested unavailable: {message}"));
                    break 'sectors;
                }
                Err(error) if fatal(&error) => return Err(error),
                Err(error) => {
                    notes.push(format!("sector {sector} hardnested failed: {error}"));
                }
            }
        }
    }
    Ok(())
}

fn choose_seed(
    layout: Layout,
    found: &[RecoveredKey],
    target_sector: usize,
    target_type: KeyType,
) -> Option<(u8, KeyType, Key)> {
    found
        .iter()
        .filter(|entry| !(entry.sector == target_sector && entry.key_type == target_type))
        .min_by_key(|entry| entry.sector == target_sector)
        .and_then(|entry| {
            layout
                .first_block(entry.sector)
                .ok()
                .map(|block| (block as u8, entry.key_type, entry.key))
        })
}

fn record_result(
    found: &mut Vec<RecoveredKey>,
    sector: usize,
    result: RecoveryResult,
    method: AttackKind,
) {
    for (key_type, key) in result.keys {
        push_found(found, sector, key_type, key, method);
    }
}

fn push_found(
    found: &mut Vec<RecoveredKey>,
    sector: usize,
    key_type: KeyType,
    key: Key,
    method: AttackKind,
) {
    if !found
        .iter()
        .any(|entry| entry.sector == sector && entry.key_type == key_type && entry.key == key)
    {
        found.push(RecoveredKey {
            sector,
            key_type,
            key,
            method,
        });
    }
}

fn push_attempt(attempts: &mut Vec<AttackKind>, kind: AttackKind) {
    if !attempts.contains(&kind) {
        attempts.push(kind);
    }
}

fn verify_reads(
    reader: &mut dyn Reader,
    layout: Layout,
    request: &RecoveryRequest,
    found: &[RecoveredKey],
    notes: &mut Vec<String>,
) -> Result<usize> {
    let sector_count = layout.sector_count();
    let mut readable = 0;
    for sector in 0..sector_count {
        request.cancel.check()?;
        let entries: Vec<&RecoveredKey> = found
            .iter()
            .filter(|entry| entry.sector == sector)
            .collect();
        if entries.is_empty() {
            continue;
        }
        let first = layout.first_block(sector)?;
        let blocks = layout.blocks_in_sector(sector)?;
        let mut readable_sector = false;
        for entry in entries {
            reader.card_info()?;
            if reader
                .authenticate(first as u8, entry.key_type, entry.key)
                .is_err()
            {
                continue;
            }
            let mut complete = true;
            for block in first..first + blocks {
                if reader.read_block(block as u8).is_err() {
                    complete = false;
                    break;
                }
            }
            if complete {
                readable_sector = true;
                break;
            }
        }
        if readable_sector {
            readable += 1;
        } else {
            notes.push(format!("sector {sector} read-back was incomplete"));
        }
        emit(
            request,
            RecoveryProgress::Verify {
                sector,
                sector_count,
            },
        );
    }
    Ok(readable)
}

fn nested_attack(
    reader: &mut dyn Reader,
    request: &RecoveryRequest,
    static_nested: bool,
) -> Result<RecoveryResult> {
    let Some((key_type, key)) = find_known_key(reader, request)? else {
        return Err(NfcError::InvalidArgument(
            "nested recovery needs a candidate key that authenticates the known block".to_owned(),
        ));
    };
    nested_attack_with(
        reader,
        request,
        request.known_block,
        key_type,
        key,
        if static_nested {
            AttackKind::StaticNested
        } else {
            AttackKind::Nested
        },
    )
}

fn nested_attack_with(
    reader: &mut dyn Reader,
    request: &RecoveryRequest,
    known_block: u8,
    known_key_type: KeyType,
    known_key: Key,
    kind: AttackKind,
) -> Result<RecoveryResult> {
    let nested_request = NestedRequest {
        known_block,
        known_key_type,
        known_key,
        target_block: request.target_block,
        target_key_type: request.target_key_type,
    };
    let outcome = nested::recover_nested(reader, &nested_request, &request.cancel)?;
    let mut verified = Vec::new();
    for key in outcome.keys {
        request.cancel.check()?;
        if nested::verify_key(reader, request.target_block, request.target_key_type, key).is_ok() {
            verified.push((request.target_key_type, key));
        }
    }
    if verified.is_empty() {
        return Err(NfcError::AuthenticationFailed);
    }
    Ok(RecoveryResult {
        card: None,
        keys: verified,
        sector_keys: Vec::new(),
        attempts: vec![kind],
        profile: None,
        note: format!(
            "two nested samples, measured distance {}..={}",
            outcome.distance_min, outcome.distance_max
        ),
    })
}

fn hardnested_attack(reader: &mut dyn Reader, request: &RecoveryRequest) -> Result<RecoveryResult> {
    let tables = load_hardnested_tables(request)?;
    let Some((key_type, key)) = find_known_key(reader, request)? else {
        return Err(NfcError::InvalidArgument(
            "hardnested recovery needs a candidate key that authenticates the known block"
                .to_owned(),
        ));
    };
    hardnested_attack_with(reader, request, request.known_block, key_type, key, &tables)
}

fn load_hardnested_tables(request: &RecoveryRequest) -> Result<hardnested::tables::Tables> {
    let dir = hardnested::tables::resolve_dir(request.tables_dir.as_deref())?;
    hardnested::tables::Tables::load(&dir)
}

fn hardnested_attack_with(
    reader: &mut dyn Reader,
    request: &RecoveryRequest,
    known_block: u8,
    known_key_type: KeyType,
    known_key: Key,
    tables: &hardnested::tables::Tables,
) -> Result<RecoveryResult> {
    let hardnested_request = HardnestedRequest {
        known_block,
        known_key_type,
        known_key,
        target_block: request.target_block,
        target_key_type: request.target_key_type,
        tables_dir: request.tables_dir.clone(),
        max_nonces: request.max_nonces,
    };
    let outcome = hardnested::recover_with_tables(
        reader,
        &hardnested_request,
        tables,
        &request.cancel,
        &mut |event| match event {
            HardnestedEvent::Acquiring(event) => emit(
                request,
                RecoveryProgress::Acquiring {
                    received: event.received,
                    distinct_nonces: event.distinct_nonces,
                    first_bytes: event.first_bytes,
                    expected_states: event.expected_states,
                },
            ),
            HardnestedEvent::Filtering(event) => {
                if event.phase == hardnested::solver::SolverPhase::BruteForce {
                    emit(
                        request,
                        RecoveryProgress::BruteForce {
                            tested_states: event.tested_states,
                            total_states: event.total_states,
                            expected_states: event.expected_states,
                        },
                    );
                } else {
                    emit(
                        request,
                        RecoveryProgress::Filtering {
                            expected_states: event.expected_states,
                        },
                    );
                }
            }
        },
    )?;

    let mut verified = Vec::new();
    for key in outcome.keys {
        request.cancel.check()?;
        if nested::verify_key(reader, request.target_block, request.target_key_type, key).is_ok() {
            verified.push((request.target_key_type, key));
        }
    }
    if verified.is_empty() {
        return Err(NfcError::AuthenticationFailed);
    }
    Ok(RecoveryResult {
        card: None,
        keys: verified,
        sector_keys: Vec::new(),
        attempts: vec![AttackKind::HardNested],
        profile: None,
        note: outcome.note,
    })
}

fn fm11rf08s_attack(reader: &mut dyn Reader, request: &RecoveryRequest) -> Result<RecoveryResult> {
    let Some((key_type, key)) = find_known_key(reader, request)? else {
        return Err(NfcError::InvalidArgument(
            "static encrypted nonce detection needs a candidate key that authenticates the known block"
                .to_owned(),
        ));
    };
    if classify::detect_static_encrypted(reader, request.known_block, key_type, key)? {
        Err(static_encrypted_message())
    } else {
        Err(NfcError::InvalidArgument(
            "the card does not return a static encrypted nonce; use nested or hardnested recovery"
                .to_owned(),
        ))
    }
}

fn static_encrypted_message() -> NfcError {
    NfcError::Unsupported(
        "static encrypted nonce card detected (FM11RF08S profile); recovery needs the reader-scheduled FM11RF08S acquisition, which this transport does not expose",
    )
}

fn emit(request: &RecoveryRequest, progress: RecoveryProgress) {
    if let Some(sink) = &request.progress {
        sink(progress);
    }
}

fn darkside_attack(reader: &mut dyn Reader, request: &RecoveryRequest) -> Result<RecoveryResult> {
    let sample = darkside::collect_with_progress(
        reader,
        request.target_block,
        request.target_key_type,
        &request.cancel,
        DARKSIDE_ATTEMPTS,
        &mut |attempts| emit(request, RecoveryProgress::Darkside { attempts }),
    )?;
    let Some(sample) = sample else {
        return Err(NfcError::Unsupported(
            "the card did not repeat a tag nonce; darkside needs eight probes against one tag nonce",
        ));
    };
    let mut verified = Vec::new();
    for key in darkside::solve(&sample) {
        request.cancel.check()?;
        if nested::verify_key(reader, request.target_block, request.target_key_type, key).is_ok() {
            verified.push((request.target_key_type, key));
        }
    }
    if verified.is_empty() {
        return Err(NfcError::AuthenticationFailed);
    }
    Ok(RecoveryResult {
        card: None,
        keys: verified,
        sector_keys: Vec::new(),
        attempts: vec![AttackKind::DarkSide],
        profile: None,
        note: format!("darkside tag nonce {:08X}", sample.tag_nonce),
    })
}

fn find_known_key(
    reader: &mut dyn Reader,
    request: &RecoveryRequest,
) -> Result<Option<(KeyType, Key)>> {
    for &(key_type, key) in &request.candidates {
        request.cancel.check()?;
        reader.card_info()?;
        match reader.authenticate(request.known_block, key_type, key) {
            Ok(()) => return Ok(Some((key_type, key))),
            Err(NfcError::AuthenticationFailed | NfcError::Unsupported(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn fatal(error: &NfcError) -> bool {
    matches!(
        error,
        NfcError::Cancelled
            | NfcError::NoCard
            | NfcError::NoReader
            | NfcError::Timeout(_)
            | NfcError::CardChanged
            | NfcError::Io(_)
    )
}

fn retryable(error: &NfcError) -> bool {
    matches!(error, NfcError::Timeout(_) | NfcError::Io(_))
}

fn reopen(reader: &mut dyn Reader) -> Result<()> {
    let conn_string = reader.device().conn_string.clone();
    let _ = reader.close();
    reader.open(&conn_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_request_is_read_only_and_targets_block_zero() {
        let request = RecoveryRequest::default();
        assert_eq!(request.target_block, 0);
        assert!(request.candidates.is_empty());
    }

    #[test]
    fn retryable_errors_cover_transport_loss() {
        assert!(retryable(&NfcError::Timeout("x".to_owned())));
        assert!(retryable(&NfcError::Io("x".to_owned())));
        assert!(!retryable(&NfcError::AuthenticationFailed));
    }
}
