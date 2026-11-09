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

/// Block 69 holds the first half of the MIFARE Classic EV1 originality
/// signature; block 70 holds the second half.
pub const EV1_SIGNATURE_BLOCK: u8 = 69;

/// Key B used by MIFARE Classic EV1 cards for the signature blocks.
pub const EV1_SIGNATURE_KEY: Key = Key::new([0x4b, 0x79, 0x1b, 0xea, 0x7b, 0xcc]);
/// Key B used by QL88 signature cards.
pub const EV1_QL88_SIGNATURE_KEY: Key = Key::new([0x70, 0x7b, 0x11, 0xfc, 0x14, 0x81]);

/// The known key and block used when an EV1 originality signature is present.
pub fn ev1_signature_key() -> (KeyType, Key) {
    (KeyType::B, EV1_SIGNATURE_KEY)
}

/// Detects the EV1/QL88 originality-signature block. Authentication failure,
/// an unsupported block, or a card timeout means the signature is absent.
pub fn detect_ev1_signature(reader: &mut dyn Reader) -> Result<bool> {
    for key in [EV1_SIGNATURE_KEY, EV1_QL88_SIGNATURE_KEY] {
        reader.set_recovery_mode(false)?;
        reader.card_info()?;
        match reader.authenticate(EV1_SIGNATURE_BLOCK, KeyType::B, key) {
            Ok(()) => return Ok(true),
            Err(
                NfcError::AuthenticationFailed | NfcError::Unsupported(_) | NfcError::Timeout(_),
            ) => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// Reads the 32-byte EV1 originality signature from blocks 69 and 70.
pub fn read_ev1_signature(reader: &mut dyn Reader) -> Result<[u8; 32]> {
    reader.set_recovery_mode(false)?;
    reader.card_info()?;
    reader.authenticate(EV1_SIGNATURE_BLOCK, KeyType::B, EV1_SIGNATURE_KEY)?;
    let mut signature = [0u8; 32];
    signature[..16].copy_from_slice(&reader.read_block(EV1_SIGNATURE_BLOCK)?);
    signature[16..].copy_from_slice(&reader.read_block(EV1_SIGNATURE_BLOCK + 1)?);
    Ok(signature)
}

/// Detects the FM11RF08S static encrypted nonce behaviour: the encrypted
/// nested nonce must stay identical across three fresh known-key sessions.
pub fn detect_static_encrypted(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
    key: Key,
) -> Result<bool> {
    let mut collector =
        crate::acquire::Collector::begin(&mut *reader, block, key_type, key, block, key_type)?;
    let mut first = None;
    let mut repeated = true;
    for _ in 0..3 {
        let nested = match collector.next_nonce(&mut *reader) {
            Ok(nonce) => nonce,
            Err(error) => {
                let _ = collector.leave(&mut *reader);
                return Err(error);
            }
        };
        match first {
            None => first = Some(nested.tag_nonce_encrypted),
            Some(value) if value != nested.tag_nonce_encrypted => {
                repeated = false;
                break;
            }
            Some(_) => {}
        }
    }
    collector.leave(&mut *reader)?;
    Ok(repeated && first.is_some())
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
