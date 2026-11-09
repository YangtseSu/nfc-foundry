// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recovery strategy selection and execution.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use nfc_core::{CancelToken, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::auto::{CardProfile, PrngProfile, RecoveryStrategy};
use crate::classify;
use crate::darkside;
use crate::hardnested::{self, HardnestedEvent, HardnestedRequest};
use crate::nested::{self, NestedRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackKind {
    Dictionary,
    DarkSide,
    Nested,
    StaticNested,
    HardNested,
    Fm11rf08s,
}

/// Progress events from long-running recovery acquisition and brute force.
#[derive(Clone, Copy, Debug)]
pub enum RecoveryProgress {
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
    /// Candidate keys tried against `known_block` before a nested attack.
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

#[derive(Clone, Debug, Default)]
pub struct RecoveryResult {
    pub keys: Vec<(KeyType, Key)>,
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
                keys: found.into_iter().collect(),
                attempts: vec![AttackKind::Dictionary],
                profile: None,
                note: "dictionary hit".to_owned(),
            })
        }
    }
}

fn auto(reader: &mut dyn Reader, request: &RecoveryRequest) -> Result<RecoveryResult> {
    let mut profile =
        classify::classify(reader, request.known_block, request.known_key_type, None)?;
    profile.ev1_signature = classify::detect_ev1_signature(reader)?;

    let known = match find_known_key(reader, request)? {
        Some((key_type, key)) => Some((request.known_block, key_type, key)),
        None if profile.ev1_signature => {
            let (key_type, key) = classify::ev1_signature_key();
            Some((classify::EV1_SIGNATURE_BLOCK, key_type, key))
        }
        None => None,
    };

    if let Some((known_block, key_type, key)) = known {
        profile.has_known_key = true;
        request.cancel.check()?;
        if profile.prng != PrngProfile::StaticNonce
            && classify::detect_static_encrypted(reader, known_block, key_type, key)?
        {
            return Err(static_encrypted_message());
        }
        let mut result = if profile.prng == PrngProfile::StaticNonce {
            nested_attack_with(
                reader,
                request,
                known_block,
                key_type,
                key,
                AttackKind::StaticNested,
            )?
        } else {
            match nested_attack_with(
                reader,
                request,
                known_block,
                key_type,
                key,
                AttackKind::Nested,
            ) {
                Ok(result) => result,
                Err(NfcError::AuthenticationFailed) => {
                    let tables = load_hardnested_tables(request)?;
                    reader.card_info()?;
                    reader.authenticate(known_block, key_type, key)?;
                    hardnested_attack_with(reader, request, known_block, key_type, key, &tables)?
                }
                Err(error) => return Err(error),
            }
        };
        result.profile = Some(profile);
        return Ok(result);
    }

    if profile.prng == PrngProfile::Weak || profile.darkside_vulnerable {
        match darkside_attack(reader, request) {
            Ok(mut result) => {
                result.profile = Some(profile);
                return Ok(result);
            }
            Err(NfcError::Unsupported(_)) => {
                return Err(NfcError::Unsupported(
                    "no candidate key authenticated; darkside needs reader-scheduled frames and hardnested needs a known key: supply --key, --key-file, or --key-store",
                ));
            }
            Err(error) => return Err(error),
        }
    }

    Err(NfcError::Unsupported(
        "no candidate key authenticated and the card has a hard or static PRNG: supply one known key for nested/hardnested recovery",
    ))
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
        keys: verified,
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
        keys: verified,
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
    let sample = darkside::collect(
        reader,
        request.target_block,
        request.target_key_type,
        &request.cancel,
        256,
    )?;
    let Some(sample) = sample else {
        return Err(NfcError::Unsupported(
            "the card did not repeat a tag nonce; host-driven readers cannot schedule the PRNG-synchronized authentication frames darkside needs",
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
        keys: verified,
        attempts: vec![AttackKind::DarkSide],
        profile: None,
        note: format!("darkside tag nonce {:08X}", sample.tag_nonce),
    })
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
