// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Card/PRNG classification before selecting an attack.

use nfc_core::{Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::acquire::probe_tag_nonce;
use crate::auto::{CardProfile, PrngProfile};
use crate::crypto1::validate_prng_nonce;

/// Probes the authentication nonce without a key.
pub fn probe_nonce(reader: &mut dyn Reader, block: u8, key_type: KeyType) -> Result<u32> {
    probe_tag_nonce(reader, block, key_type)?.ok_or(NfcError::Unsupported(
        "the card does not answer MIFARE authentication",
    ))
}

/// Classifies the card's nonce behaviour with three partial authentications.
pub fn classify(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
    known_key: Option<(KeyType, Key)>,
) -> Result<CardProfile> {
    let first = probe_nonce(reader, block, key_type)?;
    let second = probe_nonce(reader, block, key_type)?;
    let third = probe_nonce(reader, block, key_type)?;
    let nonces = [first, second, third];

    let prng = if nonces.windows(2).all(|pair| pair[0] == pair[1]) {
        PrngProfile::StaticNonce
    } else if validate_prng_nonce(first) {
        PrngProfile::Weak
    } else {
        PrngProfile::Hard
    };

    let has_known_key = match known_key {
        Some((key_type, key)) => {
            reader.set_recovery_mode(false)?;
            reader.card_info()?;
            reader.authenticate(block, key_type, key).is_ok()
        }
        None => false,
    };

    Ok(CardProfile {
        prng,
        has_known_key,
        darkside_vulnerable: prng == PrngProfile::Weak,
        ev1_signature: false,
    })
}

/// True when a partial authentication returns a nonce and a deliberately
/// invalid parity response returns a four-bit NACK.
pub fn darkside_probe(reader: &mut dyn Reader, block: u8, key_type: KeyType) -> Result<bool> {
    let Some(_) = probe_tag_nonce(reader, block, key_type)? else {
        return Ok(false);
    };
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_equal_nonces_classify_as_static() {
        let nonces = [0x0120_0145u32; 3];
        let prng = if nonces.windows(2).all(|pair| pair[0] == pair[1]) {
            PrngProfile::StaticNonce
        } else if validate_prng_nonce(nonces[0]) {
            PrngProfile::Weak
        } else {
            PrngProfile::Hard
        };
        assert_eq!(prng, PrngProfile::StaticNonce);
    }
}
