//! Numeric Amiibo identification fields.
//!
//! The crate does not ship a character database. It exposes the numeric game,
//! character, model, and series IDs from the tag so the application can show
//! the user's own data without reproducing Nintendo's catalog.

use nfc_core::{NfcError, Result};

/// Identification block at raw offset `0x54`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmiiboIdentity {
    /// Two-byte game and character ID as stored in the tag.
    pub game_character_id: u16,
    /// `game_character_id >> 6`.
    pub game_id: u16,
    /// `game_character_id & 0x3F`.
    pub character_id: u8,
    pub variant: u8,
    pub figure_type: u8,
    pub model_number: u16,
    pub series: u8,
    pub format_version: u8,
    /// Amiibo settings write counter at plaintext offset `0x29`.
    pub write_count: u16,
    /// Application ID at plaintext offset `0xB6`; zero when uninitialized.
    pub app_id: u32,
}

impl AmiiboIdentity {
    pub(crate) fn parse(block: &[u8], plain: &[u8; 540]) -> Result<Self> {
        if block.len() != 8 {
            return Err(NfcError::InvalidArgument(
                "the Amiibo identification block is eight bytes".to_owned(),
            ));
        }
        let game_character_id = u16::from_be_bytes([block[0], block[1]]);
        Ok(Self {
            game_character_id,
            game_id: game_character_id >> 6,
            character_id: (game_character_id & 0x3F) as u8,
            variant: block[2],
            figure_type: block[3],
            model_number: u16::from_be_bytes([block[4], block[5]]),
            series: block[6],
            format_version: block[7],
            write_count: u16::from_be_bytes([plain[0x029], plain[0x02A]]),
            app_id: u32::from_be_bytes([plain[0xB6], plain[0xB7], plain[0xB8], plain[0xB9]]),
        })
    }

    pub const fn figure_type_label(self) -> &'static str {
        match self.figure_type {
            0x00 => "figure",
            0x01 => "card",
            0x02 => "yarn",
            _ => "unknown",
        }
    }

    pub const fn initialized(self) -> bool {
        self.app_id != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_fields_and_write_count() {
        let mut plain = [0u8; 540];
        let block = [0x12, 0x34, 0x01, 0x02, 0x0A, 0xBC, 0x03, 0x02];
        plain[0x029..0x02B].copy_from_slice(&0x0005u16.to_be_bytes());
        plain[0xB6..0xBA].copy_from_slice(&0x10110E00u32.to_be_bytes());
        let identity = AmiiboIdentity::parse(&block, &plain).expect("identity");
        assert_eq!(identity.game_character_id, 0x1234);
        assert_eq!(identity.game_id, 0x0048);
        assert_eq!(identity.character_id, 0x34);
        assert_eq!(identity.variant, 1);
        assert_eq!(identity.figure_type, 2);
        assert_eq!(identity.figure_type_label(), "yarn");
        assert_eq!(identity.model_number, 0x0ABC);
        assert_eq!(identity.series, 3);
        assert_eq!(identity.format_version, 2);
        assert_eq!(identity.write_count, 5);
        assert_eq!(identity.app_id, 0x10110E00);
        assert!(identity.initialized());
    }
}
