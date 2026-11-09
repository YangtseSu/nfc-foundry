// SPDX-FileCopyrightText: 2009 Roel Verdult
// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `mfkey32`, `mfkey32v2`, `mfkey32_nested`, `mfkey64`, and the two-sample
//! nested key intersection.
//!
//! Ported from `proxmark3/client/src/mifare/mfkey.c` and the nested state
//! intersection in `mifarehost.c`.

use crate::crypto1::{lfsr_recovery32, lfsr_recovery64, prng_successor};

/// One reader authentication response observed after a tag challenge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MfKey32 {
    pub uid: u32,
    pub nt: u32,
    pub nr: u32,
    pub ar: u32,
    pub nr2: u32,
    pub ar2: u32,
}

/// Two reader authentication responses observed after different tag
/// challenges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MfKey32Moebius {
    pub uid: u32,
    pub nt: u32,
    pub nr: u32,
    pub ar: u32,
    pub nt2: u32,
    pub nr2: u32,
    pub ar2: u32,
}

/// A nested authentication with a known plain tag nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MfKey32Nested {
    pub uid: u32,
    pub nt: u32,
    pub nt_enc: u32,
    pub nr_enc: u32,
    pub ar_enc: u32,
}

/// One complete authentication exchange with the tag answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MfKey64 {
    pub uid: u32,
    pub nt: u32,
    pub nr: u32,
    pub ar: u32,
    pub at: u32,
}

/// Recovers a key from two reader responses on the same tag challenge.
pub fn mfkey32(input: &MfKey32) -> Option<u64> {
    let prng_64 = prng_successor(input.nt, 64);
    let states = lfsr_recovery32(input.ar ^ prng_64, 0);
    let mut found = None;
    let mut count = 0;
    for mut state in states {
        state.rollback_word(0, false);
        state.rollback_word(input.nr, true);
        state.rollback_word(input.uid ^ input.nt, false);
        let key = state.key();
        state.word(input.uid ^ input.nt, false);
        state.word(input.nr2, true);
        if input.ar2 == (state.word(0, false) ^ prng_64) {
            found = Some(key);
            count += 1;
            if count == 20 {
                break;
            }
        }
    }
    if count == 1 { found } else { None }
}

/// Recovers a key from two reader responses on two different tag challenges.
pub fn mfkey32_moebius(input: &MfKey32Moebius) -> Option<u64> {
    let prng_64 = prng_successor(input.nt, 64);
    let prng_64_b = prng_successor(input.nt2, 64);
    let states = lfsr_recovery32(input.ar ^ prng_64, 0);
    let mut found = None;
    let mut count = 0;
    for mut state in states {
        state.rollback_word(0, false);
        state.rollback_word(input.nr, true);
        state.rollback_word(input.uid ^ input.nt, false);
        let key = state.key();

        state.word(input.uid ^ input.nt2, false);
        state.word(input.nr2, true);
        if input.ar2 == (state.word(0, false) ^ prng_64_b) {
            found = Some(key);
            count += 1;
            if count == 2 {
                break;
            }
        }
    }
    if count == 1 { found } else { None }
}

/// Recovers a key from one nested authentication when the plain tag nonce is
/// known.
pub fn mfkey32_nested(input: &MfKey32Nested) -> Option<u64> {
    let plain_reader_response = prng_successor(input.nt, 64);
    let keystream_nonce = input.nt_enc ^ input.nt;
    let keystream_response = input.ar_enc ^ plain_reader_response;
    let states = lfsr_recovery32(keystream_nonce, input.uid ^ input.nt);
    for mut state in states {
        state.word(input.nr_enc, true);
        if keystream_response == state.word(0, false) {
            state.rollback_word(0, false);
            state.rollback_word(input.nr_enc, true);
            state.rollback_word(input.uid ^ input.nt, false);
            return Some(state.key());
        }
    }
    None
}

/// Recovers a key from one complete authentication exchange.
pub fn mfkey64(input: &MfKey64) -> Option<u64> {
    let ks2 = input.ar ^ prng_successor(input.nt, 64);
    let ks3 = input.at ^ prng_successor(input.nt, 96);
    let state = lfsr_recovery64(ks2, ks3).into_iter().next()?;
    let mut state = state;
    state.rollback_word(0, false);
    state.rollback_word(0, false);
    state.rollback_word(input.nr, true);
    state.rollback_word(input.uid ^ input.nt, false);
    Some(state.key())
}

/// One nested-authentication nonce sample with its predicted plaintext.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NestedSample {
    pub uid: u32,
    pub nt: u32,
    pub nt_enc: u32,
}

/// Recovers candidate keys from one nested nonce sample.
pub fn nested_candidates(sample: &NestedSample) -> Vec<u64> {
    let keystream = sample.nt_enc ^ sample.nt;
    let input = sample.nt ^ sample.uid;
    let states = lfsr_recovery32(keystream, input);
    let mut keys = Vec::with_capacity(states.len());
    for mut state in states {
        state.rollback_word(input, false);
        keys.push(state.key());
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Intersects the candidate keys from two nested samples.
pub fn nested_keys(first: &NestedSample, second: &NestedSample) -> Vec<u64> {
    let mut left = nested_candidates(first);
    let right = nested_candidates(second);
    intersect(&mut left, &right)
}

fn intersect(left: &mut Vec<u64>, right: &[u64]) -> Vec<u64> {
    left.sort_unstable();
    left.dedup();
    let mut result = Vec::new();
    let mut right_index = 0;
    for &key in left.iter() {
        while right_index < right.len() && right[right_index] < key {
            right_index += 1;
        }
        if right_index < right.len() && right[right_index] == key {
            result.push(key);
            right_index += 1;
        }
    }
    result
}

/// Converts a recovered 48-bit key to the transport key type.
pub fn key_from_u64(key: u64) -> nfc_core::Key {
    nfc_core::Key::new(key_bytes(key))
}

/// Converts a 48-bit key to the six wire bytes used by authentication.
pub fn key_bytes(key: u64) -> [u8; 6] {
    [
        (key >> 40) as u8,
        (key >> 32) as u8,
        (key >> 24) as u8,
        (key >> 16) as u8,
        (key >> 8) as u8,
        key as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mfkey32_pm3_example() {
        let input = MfKey32 {
            uid: 0x57DA_41DA,
            nt: 0x0120_0145,
            nr: 0x7B70_C62C,
            ar: 0x9091_21F2,
            nr2: 0xF920_6A8B,
            ar2: 0x908B_8981,
        };
        assert_eq!(mfkey32(&input), Some(0xFFFF_FFFF_FFFF));
    }

    #[test]
    fn mfkey32_moebius_pm3_vector() {
        let input = MfKey32Moebius {
            uid: 0x1234_5678,
            nt: 0x1AD8_DF2B,
            nr: 0x1D31_6024,
            ar: 0x620E_F048,
            nt2: 0x30D6_CB07,
            nr2: 0xC520_77E2,
            ar2: 0x837A_C61A,
        };
        assert_eq!(mfkey32_moebius(&input), Some(0xA0A1_A2A3_A4A5));
    }

    #[test]
    fn mfkey32_nested_pm3_vector() {
        let input = MfKey32Nested {
            uid: 0x5C46_7F63,
            nt: 0x4BBF_8A12,
            nt_enc: 0xABB3_0BD1,
            nr_enc: 0x4603_3966,
            ar_enc: 0xADC1_8162,
        };
        assert_eq!(mfkey32_nested(&input), Some(0x059E_2905_BFCC));
    }

    #[test]
    fn mfkey64_pm3_vector() {
        let input = MfKey64 {
            uid: 0x9C59_9B32,
            nt: 0x82A4_166C,
            nr: 0xA1E4_58CE,
            ar: 0x6EEA_41E0,
            at: 0x5CAD_F439,
        };
        assert_eq!(mfkey64(&input), Some(0xFFFF_FFFF_FFFF));
    }

    #[test]
    fn nonce_to_key_pm3_vector() {
        let keys = crate::crypto1::nonce2key(
            0xE9CA_DD9C,
            0xA8BF_4A12,
            0,
            0,
            0xA020_A828_5858_B090,
            0x050F_0106_0706_0E07,
        );
        assert!(
            keys.contains(&0xFC00_0187_78F7),
            "expected PM3 darkside key in {keys:?}"
        );
    }

    #[test]
    fn key_bytes_are_big_endian() {
        assert_eq!(
            key_bytes(0xA0A1_A2A3_A4A5),
            [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]
        );
    }

    #[test]
    fn intersect_keeps_common_keys_once() {
        let mut left = vec![3, 1, 2, 2, 5];
        let right = vec![2, 3, 4];
        assert_eq!(intersect(&mut left, &right), vec![2, 3]);
    }
}
