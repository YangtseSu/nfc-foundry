// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! FM11RF08S static encrypted nonce acquisition and the one-nonce solver.
//!
//! The solver is a Rust port of PM3 `tools/mfc/card_only/staticnested_1nt.c`.
//! It receives the clear static nested nonce, the encrypted nonce, and the
//! parity-error mask, enumerates 48-bit key candidates, and filters them with
//! the final parity bit. The caller verifies candidates against the card.

use nfc_core::{Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::acquire::{MIFARE_BACKDOOR_A, MIFARE_BACKDOOR_B, Session, decrypt_nested_nonce};
use crate::crypto1::{
    Crypto1State, lfsr_recovery32, nonce_distance, odd_parity8, prng_successor, validate_prng_nonce,
};
use crate::mfkey::key_from_u64;

/// One static encrypted nonce sample for one sector and key type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticSample {
    /// Last four UID bytes in the authentication byte order.
    pub uid: u32,
    /// Predicted clear nested nonce.
    pub nt: u32,
    /// Encrypted nested nonce sent by the card.
    pub nt_enc: u32,
    /// Parity errors observed on the encrypted nonce, one bit per byte.
    pub par_err: u8,
}

/// Published FM11RF08S backdoor keys, in the order PM3 tries them.
pub const FM11RF08S_BACKDOOR_KEYS: [Key; 3] = [
    Key::new([0xA3, 0x96, 0xEF, 0xA4, 0xE2, 0x4F]),
    Key::new([0xA3, 0x16, 0x67, 0xA8, 0xCE, 0xC1]),
    Key::new([0x51, 0x8B, 0x33, 0x54, 0xE7, 0x60]),
];

/// Collect one sample through the FM11RF08S backdoor.
///
/// The backdoor authentication command (0x64/0x65) decrypts the nested nonce
/// directly, so no PRNG-distance prediction is needed. This is the path PM3
/// uses with `--collect_fm11rf08s_with_data`.
pub fn collect_sample_backdoor(
    reader: &mut dyn Reader,
    target_block: u8,
    target_key_type: KeyType,
) -> Result<StaticSample> {
    let mut last_error = None;
    for backdoor_key in FM11RF08S_BACKDOOR_KEYS {
        for command in [MIFARE_BACKDOOR_A, MIFARE_BACKDOOR_B] {
            match collect_backdoor_with(
                reader,
                target_block,
                target_key_type,
                backdoor_key,
                command,
            ) {
                Ok(sample) => return Ok(sample),
                Err(error) => last_error = Some(error),
            }
        }
    }
    Err(last_error.unwrap_or(NfcError::Unsupported(
        "the FM11RF08S backdoor did not answer",
    )))
}

fn collect_backdoor_with(
    reader: &mut dyn Reader,
    target_block: u8,
    target_key_type: KeyType,
    backdoor_key: Key,
    backdoor_command: u8,
) -> Result<StaticSample> {
    let (mut session, card) = Session::begin(reader)?;
    let result = (|| {
        session.authenticate_command(reader, target_block, backdoor_command, backdoor_key)?;
        let clear_encrypted =
            session.nested_nonce_command(reader, target_block, backdoor_command)?;
        let nt = decrypt_nested_nonce(
            session.uid,
            clear_encrypted.tag_nonce_encrypted,
            backdoor_key,
        );
        session.abort_nested(reader);

        session.fast_select(reader, &card)?;
        session.authenticate_command(reader, target_block, backdoor_command, backdoor_key)?;
        let target = session.nested_nonce(reader, target_block, target_key_type)?;
        session.abort_nested(reader);
        Ok(StaticSample {
            uid: session.uid,
            nt,
            nt_enc: target.tag_nonce_encrypted,
            par_err: parity_error(target.parity, target.tag_nonce_encrypted),
        })
    })();
    let leave = session.leave(reader);
    match (result, leave) {
        (Ok(sample), Ok(())) => Ok(sample),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

/// Collect one sample while authenticated with a known key.
///
/// This follows PM3's `--collect_fm11rf08s_without_backdoor` sequence: get the
/// clear nested nonce and PRNG distance from the known block, capture the
/// target encrypted nonce, reselect, re-authenticate the known block, and
/// rewind its first nonce by the measured distance.
pub fn collect_sample(
    reader: &mut dyn Reader,
    known_block: u8,
    known_key_type: KeyType,
    known_key: Key,
    target_block: u8,
    target_key_type: KeyType,
) -> Result<StaticSample> {
    let (mut session, card) = Session::begin(reader)?;
    let result = collect_with_session(
        reader,
        &mut session,
        &card,
        known_block,
        known_key_type,
        known_key,
        target_block,
        target_key_type,
    );
    let leave = session.leave(reader);
    match (result, leave) {
        (Ok(sample), Ok(())) => Ok(sample),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_with_session(
    reader: &mut dyn Reader,
    session: &mut Session,
    card: &nfc_core::CardInfo,
    known_block: u8,
    known_key_type: KeyType,
    known_key: Key,
    target_block: u8,
    target_key_type: KeyType,
) -> Result<StaticSample> {
    session.authenticate_known(reader, known_block, known_key_type, known_key)?;
    let first = session.tag_nonce;
    let known = session.nested_nonce(reader, known_block, known_key_type)?;
    session.abort_nested(reader);
    let clear_known = decrypt_nested_nonce(session.uid, known.tag_nonce_encrypted, known_key);
    if !validate_prng_nonce(clear_known) || !validate_prng_nonce(first) {
        return Err(NfcError::Unsupported(
            "the card did not return a weak-PRNG nonce; the FM11RF08S distance is unavailable",
        ));
    }
    let distance = nonce_distance(clear_known, first);

    session.fast_select(reader, card)?;
    session.authenticate_known(reader, known_block, known_key_type, known_key)?;
    let target = session.nested_nonce(reader, target_block, target_key_type)?;
    session.abort_nested(reader);

    session.fast_select(reader, card)?;
    session.authenticate_known(reader, known_block, known_key_type, known_key)?;
    let after = session.tag_nonce;
    if !validate_prng_nonce(after) {
        return Err(NfcError::Unsupported(
            "the card did not return a weak-PRNG first nonce after reselection",
        ));
    }
    let nt = rewind_nonce(after, distance);
    Ok(StaticSample {
        uid: session.uid,
        nt,
        nt_enc: target.tag_nonce_encrypted,
        par_err: parity_error(target.parity, target.tag_nonce_encrypted),
    })
}

fn parity_error(received: u8, nt_enc: u32) -> u8 {
    let expected = |shift| odd_parity8((nt_enc >> shift) as u8);
    (((received >> 7) & 1) ^ expected(24)) << 3
        | (((received >> 6) & 1) ^ expected(16)) << 2
        | (((received >> 5) & 1) ^ expected(8)) << 1
        | (((received >> 4) & 1) ^ expected(0))
}

fn parity_encrypted(sample: &StaticSample) -> u8 {
    let expected = |shift| odd_parity8((sample.nt_enc >> shift) as u8);
    (sample.par_err & 0x0F)
        ^ (expected(24) << 3 | expected(16) << 2 | expected(8) << 1 | expected(0))
}

/// Rewind a first nonce by a measured PRNG distance, then reconstruct the full
/// nonce from its upper half. Port of PM3 `rewind_nonce`.
pub fn rewind_nonce(from: u32, distance: u16) -> u32 {
    let mut x = (from >> 16) as u16;
    for _ in 0..distance {
        x = x.rotate_left(1) ^ ((x >> 1 ^ x >> 2 ^ x >> 4) & 0x100);
    }
    let nt = u32::from(x);
    nt << 16 | prng_successor(nt, 16)
}

/// Enumerate key candidates from one static nonce sample.
///
/// The returned list is filtered by the final parity bit, matching PM3's
/// `staticnested_1nt`. It is not yet verified against the card.
pub fn candidates(sample: &StaticSample) -> Vec<Key> {
    const KEY_SPACE_SIZE: usize = 1 << 18;
    let nt_par_enc = parity_encrypted(sample);
    let mut result = Vec::new();
    let states = lfsr_recovery32(sample.nt ^ sample.nt_enc, sample.nt ^ sample.uid);
    for mut state in states {
        state.rollback_word(sample.nt ^ sample.uid, false);
        let key = state.key();
        let mut cipher = Crypto1State::from_key(key);
        cipher.word(sample.nt ^ sample.uid, false);
        let ks2 = cipher.word(0, false);
        let lastpar1 = odd_parity8((sample.nt & 0xff) as u8);
        let kslastp = ((ks2 >> 24) & 1) as u8;
        let lastpar2 = (nt_par_enc & 1) ^ kslastp;
        if lastpar1 == lastpar2 {
            result.push(key_from_u64(key));
            if result.len() == KEY_SPACE_SIZE {
                break;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto1::{nonce_distance, prng_successor};

    fn sample_for(key: u64, uid: u32, nt_enc: u32) -> StaticSample {
        let mut cipher = Crypto1State::from_key(key);
        let ks = cipher.word(nt_enc ^ uid, true);
        let nt = ks ^ nt_enc;
        // Make the one filtered parity bit accept the correct key.
        let mut check = Crypto1State::from_key(key);
        check.word(uid ^ nt, false);
        let ks2 = check.word(0, false);
        let lastpar1 = odd_parity8((nt & 0xff) as u8);
        let kslastp = ((ks2 >> 24) & 1) as u8;
        let par_bit = (lastpar1 ^ kslastp) ^ odd_parity8((nt_enc & 0xff) as u8);
        StaticSample {
            uid,
            nt,
            nt_enc,
            par_err: par_bit,
        }
    }

    #[test]
    fn candidate_list_contains_the_correct_key() {
        let key = 0xA0A1_A2A3_A4A5;
        let uid = 0x9C81_95D4;
        let sample = sample_for(key, uid, 0x2E9E_49FC);
        let keys = candidates(&sample);
        assert!(!keys.is_empty());
        assert!(keys.contains(&key_from_u64(key)));
    }

    #[test]
    fn rewind_reconstructs_the_original_nonce() {
        let first = 0xADCA_0697;
        let advanced = prng_successor(first, 32);
        let distance = nonce_distance(first, advanced);
        assert_eq!(rewind_nonce(advanced, distance), first);
    }
}
