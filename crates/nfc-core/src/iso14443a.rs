use crate::{NfcError, Result};

pub const BLOCK_SIZE: usize = 16;
pub const KEY_SIZE: usize = 6;
pub const ATQA_LENGTH: usize = 2;
/// NFC Forum Type 2 page size. A READ returns four pages at once.
pub const PAGE_SIZE: usize = 4;
/// Payload size of one NFC Forum Type 2 READ command.
pub const PAGE_QUAD_SIZE: usize = PAGE_SIZE * 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    A,
    B,
}

impl KeyType {
    pub const fn label(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
        }
    }
}

/// A six-byte MIFARE Classic key. Debug output is redacted.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key([u8; KEY_SIZE]);

impl Key {
    pub const fn new(bytes: [u8; KEY_SIZE]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.0
    }

    pub fn from_hex(value: &str) -> Result<Self> {
        let compact: String = value
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect();
        if compact.len() != KEY_SIZE * 2 {
            return Err(NfcError::InvalidArgument(
                "a MIFARE Classic key must contain 12 hexadecimal digits".to_owned(),
            ));
        }
        let mut bytes = [0u8; KEY_SIZE];
        let digits = compact.as_bytes();
        for index in 0..KEY_SIZE {
            let high = hex_nibble(digits[index * 2])?;
            let low = hex_nibble(digits[index * 2 + 1])?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }

    pub fn to_hex(self) -> String {
        self.0
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join("")
    }
}

fn hex_nibble(character: u8) -> Result<u8> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(NfcError::InvalidArgument(
            "key contains a non-hexadecimal character".to_owned(),
        )),
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Key(redacted)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_keys() {
        let key = Key::from_hex("a0 a1 a2 a3 a4 a5").expect("valid key");
        assert_eq!(key.to_hex(), "A0A1A2A3A4A5");
        assert_eq!(key.as_bytes(), &[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);
    }

    #[test]
    fn rejects_invalid_keys() {
        assert!(Key::from_hex("A0A1").is_err());
        assert!(Key::from_hex("A0A1A2A3A4GG").is_err());
    }
}
