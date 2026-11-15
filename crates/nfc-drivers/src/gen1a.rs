//! Gen1A magic-card unlock, manufacturer-block write, and UFUID seal sequence.
//!
//! The sequence is reader-neutral: HALT the card, send the 7-bit `0x40`
//! unlock, send `0x43`, then either write block 0 or the UFUID configuration.
//! Each driver supplies raw byte and bit exchanges that keep the RF field on.

use nfc_core::{BLOCK_SIZE, CardInfo, CardType, NfcError, Result};
use nfc_transport::UFUID_SEAL_CONFIG;

const UNLOCK_FIRST: u8 = 0x40;
const UNLOCK_SECOND: u8 = 0x43;
const MIFARE_HALT: u8 = 0x50;
const MIFARE_WRITE: u8 = 0xA0;
const MIFARE_ACK: u8 = 0x0A;
const UFUID_WRITE_CONFIG: u8 = 0xE1;

/// Raw exchanges used by the Gen1A sequence.
pub(crate) trait Gen1aLink {
    fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>>;
    fn transceive_bits(&mut self, tx: &[u8], tx_bits: usize) -> Result<(Vec<u8>, usize)>;
}

/// Rejects a card or block that the Gen1A path cannot safely write.
pub(crate) fn validate(card: &CardInfo, data: &[u8; BLOCK_SIZE]) -> Result<()> {
    if card.uid.len() != 4 || card.card_type != CardType::MifareClassic1K {
        return Err(NfcError::Unsupported(
            "Gen1A block 0 writes require a four-byte UID MIFARE Classic 1K card",
        ));
    }
    if data[4] != data[0] ^ data[1] ^ data[2] ^ data[3] {
        return Err(NfcError::InvalidArgument(
            "the manufacturer block has an invalid BCC".to_owned(),
        ));
    }
    Ok(())
}

/// Rejects a UFUID configuration other than the canonical sealed value.
pub(crate) fn validate_seal_config(config: &[u8; BLOCK_SIZE]) -> Result<()> {
    if *config != UFUID_SEAL_CONFIG {
        return Err(NfcError::InvalidArgument(
            "the UFUID seal configuration is not the canonical sealed value".to_owned(),
        ));
    }
    Ok(())
}

/// Runs the HALT and `0x40`/`0x43` unlock. The caller must enter raw mode first
/// and reselect the card so Crypto1 authentication is cleared.
fn unlock(link: &mut impl Gen1aLink) -> Result<()> {
    let halt = crc_a(&[MIFARE_HALT, 0x00]);
    match link.transceive_bytes(&halt) {
        Ok(_) => {}
        Err(NfcError::Timeout(_) | NfcError::NoCard) => {}
        Err(error) => return Err(error),
    }

    let (response, bits) = link
        .transceive_bits(&[UNLOCK_FIRST], 7)
        .map_err(|error| gen1a_error("the card did not answer the 7-bit 0x40 unlock", error))?;
    if bits != 4 || !is_ack(&response) {
        return Err(NfcError::Unsupported(
            "the card answered the 7-bit 0x40 Gen1A unlock command with an unexpected frame",
        ));
    }

    let response = link
        .transceive_bytes(&[UNLOCK_SECOND])
        .map_err(|error| gen1a_error("the card refused the 0x43 unlock command", error))?;
    if !is_ack(&response) {
        return Err(NfcError::Unsupported(
            "the card answered the Gen1A 0x43 unlock command with an unexpected frame",
        ));
    }
    Ok(())
}

/// Probes the Gen1A backdoor without changing card data. A card that does not
/// answer the unlock is reported as `false`; a transport failure is returned.
pub(crate) fn probe(link: &mut impl Gen1aLink) -> Result<bool> {
    let halt = crc_a(&[MIFARE_HALT, 0x00]);
    match link.transceive_bytes(&halt) {
        Ok(_) => {}
        Err(NfcError::Timeout(_) | NfcError::NoCard) => {}
        Err(error) => return Err(error),
    }

    let (response, bits) = match link.transceive_bits(&[UNLOCK_FIRST], 7) {
        Ok(answer) => answer,
        Err(NfcError::Timeout(_) | NfcError::NoCard) => return Ok(false),
        Err(error) => return Err(error),
    };
    if bits != 4 || !is_ack(&response) {
        return Ok(false);
    }
    match link.transceive_bytes(&[UNLOCK_SECOND]) {
        Ok(response) => Ok(is_ack(&response)),
        Err(NfcError::Timeout(_) | NfcError::NoCard) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Runs the unlock sequence and writes one manufacturer block.
pub(crate) fn write_block0(link: &mut impl Gen1aLink, data: &[u8; BLOCK_SIZE]) -> Result<()> {
    unlock(link)?;

    let mut write_command = vec![MIFARE_WRITE, 0x00];
    write_command.extend_from_slice(&crc_a(&write_command));
    let response = link
        .transceive_bytes(&write_command)
        .map_err(|error| gen1a_error("the card did not acknowledge the block 0 write", error))?;
    if !is_ack(&response) {
        return Err(NfcError::Io(
            "the card answered the Gen1A block 0 write with an unexpected frame".to_owned(),
        ));
    }

    let mut payload = Vec::with_capacity(BLOCK_SIZE + 2);
    payload.extend_from_slice(data);
    payload.extend_from_slice(&crc_a(data));
    let response = link
        .transceive_bytes(&payload)
        .map_err(|error| gen1a_error("the block 0 data frame was not acknowledged", error))?;
    if !is_ack(&response) {
        return Err(NfcError::Io(
            "the block 0 data frame was answered with an unexpected frame".to_owned(),
        ));
    }
    Ok(())
}

/// Runs the unlock sequence and writes the UFUID configuration that disables
/// the backdoor. The caller must reselect and verify that the unlock no longer
/// answers.
pub(crate) fn seal(link: &mut impl Gen1aLink, config: &[u8; BLOCK_SIZE]) -> Result<()> {
    validate_seal_config(config)?;
    unlock(link)?;

    let mut command = vec![UFUID_WRITE_CONFIG, 0x00];
    command.extend_from_slice(&crc_a(&command));
    let response = link.transceive_bytes(&command).map_err(|error| {
        gen1a_error("the card did not acknowledge the UFUID seal command", error)
    })?;
    if !is_ack(&response) {
        return Err(NfcError::Unsupported(
            "the card did not accept the UFUID seal command; it is not a sealable UFUID",
        ));
    }

    let mut payload = Vec::with_capacity(BLOCK_SIZE + 2);
    payload.extend_from_slice(config);
    payload.extend_from_slice(&crc_a(config));
    let response = link
        .transceive_bytes(&payload)
        .map_err(|error| gen1a_error("the UFUID seal configuration was not acknowledged", error))?;
    if !is_ack(&response) {
        return Err(NfcError::Io(
            "the card answered the UFUID seal configuration with an unexpected frame".to_owned(),
        ));
    }
    Ok(())
}

fn gen1a_error(context: &'static str, error: NfcError) -> NfcError {
    match error {
        NfcError::Timeout(_) | NfcError::NoCard | NfcError::AuthenticationFailed => {
            NfcError::Unsupported(context)
        }
        other => other,
    }
}

fn is_ack(response: &[u8]) -> bool {
    response.len() == 1 && (response[0] == MIFARE_ACK || response[0] == MIFARE_ACK << 4)
}

pub(crate) fn crc_a(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0x6363;
    for &byte in data {
        let mut mixed = byte ^ crc as u8;
        mixed ^= mixed << 4;
        crc = (crc >> 8) ^ ((mixed as u16) << 8) ^ ((mixed as u16) << 3) ^ ((mixed as u16) >> 4);
    }
    [crc as u8, (crc >> 8) as u8]
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    #[test]
    fn crc_a_matches_the_known_mifare_vector() {
        assert_eq!(crc_a(&[0x50, 0x00]), [0x57, 0xCD]);
    }

    #[test]
    fn ack_accepts_both_nibble_positions() {
        assert!(is_ack(&[0x0A]));
        assert!(is_ack(&[0xA0]));
        assert!(!is_ack(&[0x00]));
        assert!(!is_ack(&[0x0A, 0x00]));
    }

    #[test]
    fn validation_requires_a_four_byte_classic_1k_and_valid_bcc() {
        let card = CardInfo::new(
            vec![1, 2, 3, 4],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        );
        let mut block = [0u8; BLOCK_SIZE];
        block[0..4].copy_from_slice(&[1, 2, 3, 4]);
        block[4] = block[0] ^ block[1] ^ block[2] ^ block[3];
        validate(&card, &block).expect("valid block");

        let mut bad_bcc = block;
        bad_bcc[4] ^= 0xFF;
        assert!(matches!(
            validate(&card, &bad_bcc),
            Err(NfcError::InvalidArgument(_))
        ));

        let four_k = CardInfo::new(
            vec![1, 2, 3, 4],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic4K,
        );
        assert!(matches!(
            validate(&four_k, &block),
            Err(NfcError::Unsupported(_))
        ));
        let long_uid = CardInfo::new(
            vec![1, 2, 3, 4, 5, 6, 7],
            [0x00, 0x44],
            0x08,
            CardType::MifareClassic1K,
        );
        assert!(matches!(
            validate(&long_uid, &block),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[derive(Default)]
    struct FakeLink {
        bytes: VecDeque<Result<Vec<u8>>>,
        bits: VecDeque<Result<(Vec<u8>, usize)>>,
        sent: Vec<(Vec<u8>, Option<usize>)>,
    }

    impl FakeLink {
        fn with_unlock() -> Self {
            Self {
                bytes: VecDeque::from([Ok(vec![0x05]), Ok(vec![0x0A])]),
                bits: VecDeque::from([Ok((vec![0x0A], 4))]),
                sent: Vec::new(),
            }
        }
    }

    impl Gen1aLink for FakeLink {
        fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>> {
            self.sent.push((tx.to_vec(), None));
            self.bytes.pop_front().unwrap_or(Err(NfcError::Timeout(
                "fake link has no queued byte response".to_owned(),
            )))
        }

        fn transceive_bits(&mut self, tx: &[u8], tx_bits: usize) -> Result<(Vec<u8>, usize)> {
            self.sent.push((tx.to_vec(), Some(tx_bits)));
            self.bits.pop_front().unwrap_or(Err(NfcError::Timeout(
                "fake link has no queued bit response".to_owned(),
            )))
        }
    }

    #[test]
    fn probe_requires_both_gen1a_acknowledgements() {
        let mut link = FakeLink::with_unlock();
        assert!(probe(&mut link).expect("probe"));
        assert_eq!(
            link.sent,
            vec![
                (crc_a(&[0x50, 0x00]).to_vec(), None),
                (vec![0x40], Some(7)),
                (vec![0x43], None),
            ]
        );

        let mut denied = FakeLink {
            bytes: VecDeque::from([Ok(vec![0x05])]),
            bits: VecDeque::from([Ok((vec![0x0A], 4)), Ok((vec![0x05], 4))]),
            sent: Vec::new(),
        };
        assert!(!probe(&mut denied).expect("probe"));
    }

    #[test]
    fn seal_writes_the_canonical_config_after_the_unlock() {
        let mut link = FakeLink::with_unlock();
        link.bytes.push_back(Ok(vec![0x0A]));
        link.bytes.push_back(Ok(vec![0x0A]));
        seal(&mut link, &UFUID_SEAL_CONFIG).expect("seal");

        let mut expected_command = vec![0xE1, 0x00];
        expected_command.extend_from_slice(&crc_a(&[0xE1, 0x00]));
        let mut expected_config = UFUID_SEAL_CONFIG.to_vec();
        expected_config.extend_from_slice(&crc_a(&UFUID_SEAL_CONFIG));
        assert_eq!(
            link.sent,
            vec![
                (crc_a(&[0x50, 0x00]).to_vec(), None),
                (vec![0x40], Some(7)),
                (vec![0x43], None),
                (expected_command, None),
                (expected_config, None),
            ]
        );
    }

    #[test]
    fn seal_rejects_any_other_configuration_before_touching_the_card() {
        let mut link = FakeLink::default();
        let mut config = UFUID_SEAL_CONFIG;
        config[7] = 0x5A;
        assert!(matches!(
            seal(&mut link, &config),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(link.sent.is_empty());
    }
}
