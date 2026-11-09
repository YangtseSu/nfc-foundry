// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Recovery strategy selection and execution.

use nfc_core::{CancelToken, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::auto::{CardProfile, PrngProfile, RecoveryStrategy};
use crate::classify;
use crate::darkside;
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

#[derive(Clone, Debug)]
pub struct RecoveryRequest {
    pub known_block: u8,
    pub known_key_type: KeyType,
    pub target_block: u8,
    pub target_key_type: KeyType,
    /// Candidate keys tried against `known_block` before a nested attack.
    pub candidates: Vec<(KeyType, Key)>,
    pub cancel: CancelToken,
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
        RecoveryStrategy::HardNested | RecoveryStrategy::Fm11rf08s => Err(
            NfcError::NotImplemented("hardnested and static-encrypted recovery are phase 07"),
        ),
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
    if let Some((key_type, key)) = find_known_key(reader, request)? {
        profile.has_known_key = true;
        request.cancel.check()?;
        let strategy = match profile.prng {
            PrngProfile::StaticNonce | PrngProfile::StaticEncrypted => {
                RecoveryStrategy::StaticNested
            }
            _ => RecoveryStrategy::Nested,
        };
        let result = match strategy {
            RecoveryStrategy::StaticNested => {
                nested_attack_with(reader, request, key_type, key, AttackKind::StaticNested)
            }
            _ => nested_attack_with(reader, request, key_type, key, AttackKind::Nested),
        };
        let mut result = result?;
        result.profile = Some(profile);
        return Ok(result);
    }

    if profile.prng == PrngProfile::Weak || profile.darkside_vulnerable {
        let mut result = darkside_attack(reader, request)?;
        result.profile = Some(profile);
        return Ok(result);
    }

    Err(NfcError::Unsupported(
        "no candidate key authenticated and the card is not darkside-vulnerable; hardnested is phase 07",
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
    known_key_type: KeyType,
    known_key: Key,
    kind: AttackKind,
) -> Result<RecoveryResult> {
    let nested_request = NestedRequest {
        known_block: request.known_block,
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
