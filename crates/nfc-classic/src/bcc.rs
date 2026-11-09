use nfc_core::{NfcError, Result};

use crate::Dump;

/// Result of checking the manufacturer-block BCC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BccStatus {
    Valid,
    Invalid {
        expected: u8,
        actual: u8,
    },
    /// The UID length is unknown or uses a cascade layout without a single
    /// XOR check byte in manufacturer block 0.
    NotApplicable,
}

/// BCC for a four-byte UID, the only layout with one XOR check byte at
/// manufacturer-block byte 4.
pub fn bcc_for_uid(uid: &[u8]) -> Result<u8> {
    match uid {
        [a, b, c, d] => Ok(a ^ b ^ c ^ d),
        _ => Err(NfcError::InvalidArgument(format!(
            "BCC calculation requires an exact 4-byte UID, got {} bytes",
            uid.len()
        ))),
    }
}

impl Dump {
    /// Checks the manufacturer-block BCC only when the UID length is four.
    /// Seven- and ten-byte UIDs use cascade BCC bytes elsewhere, and an
    /// unknown UID length makes the check inapplicable. The check uses the UID
    /// bytes stored in block 0, so it verifies the BCC field independently of
    /// the selected card identity.
    pub fn validate_bcc(&self, uid_length: usize) -> Result<BccStatus> {
        if uid_length != 4 {
            return Ok(BccStatus::NotApplicable);
        }
        let block = self.blocks.first().ok_or_else(|| {
            NfcError::InvalidArgument("dump does not contain manufacturer block 0".to_owned())
        })?;
        if block.known_mask & 0b1_1111 != 0b1_1111 {
            return Err(NfcError::VerificationFailed(
                "manufacturer block UID and BCC bytes are incomplete".to_owned(),
            ));
        }
        let expected = bcc_for_uid(&block.data[0..4])?;
        let actual = block.data[4];
        if actual == expected {
            Ok(BccStatus::Valid)
        } else {
            Ok(BccStatus::Invalid { expected, actual })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Layout;

    fn dump_with_bcc(layout: Layout) -> Dump {
        let mut dump = Dump::new(layout);
        dump.blocks[0].data[0..4].copy_from_slice(&[1, 2, 3, 4]);
        dump.blocks[0].data[4] = bcc_for_uid(&[1, 2, 3, 4]).expect("bcc");
        dump.blocks[0].known_mask = 0x001F;
        dump
    }

    #[test]
    fn four_byte_uid_bcc_is_the_xor_of_the_uid_bytes() {
        assert_eq!(bcc_for_uid(&[1, 2, 3, 4]).expect("bcc"), 4);
        for length in [0, 3, 7, 10] {
            assert!(bcc_for_uid(&vec![0u8; length]).is_err(), "length {length}");
        }
    }

    #[test]
    fn bcc_status_distinguishes_valid_invalid_and_inapplicable() {
        let mut dump = dump_with_bcc(Layout::Classic1K);
        assert_eq!(dump.validate_bcc(4).expect("bcc"), BccStatus::Valid);
        dump.blocks[0].data[4] ^= 0xFF;
        assert_eq!(
            dump.validate_bcc(4).expect("bcc"),
            BccStatus::Invalid {
                expected: 4,
                actual: 4 ^ 0xFF
            }
        );
        assert_eq!(dump.validate_bcc(7).expect("bcc"), BccStatus::NotApplicable);
        assert_eq!(dump.validate_bcc(0).expect("bcc"), BccStatus::NotApplicable);
    }

    #[test]
    fn incomplete_manufacturer_bytes_cannot_be_checked() {
        let mut dump = dump_with_bcc(Layout::Classic1K);
        dump.blocks[0].known_mask &= !0b10;
        assert!(dump.validate_bcc(4).is_err());
    }
}
