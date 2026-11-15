//! User-supplied Amiibo master key records.
//!
//! A record is 80 bytes: a 16-byte HMAC key, a 14-byte type string, RFU,
//! magic-byte count, 16 magic bytes, and a 32-byte XOR pad. The combined file
//! is the 80-byte data record followed by the 80-byte tag record.

use std::fmt;

use nfc_core::{NfcError, Result};

/// Size of one master key record.
pub const MASTER_KEY_SIZE: usize = 80;
/// Size of the combined data + tag key file.
pub const COMBINED_KEY_SIZE: usize = MASTER_KEY_SIZE * 2;

const DATA_TYPE: &[u8] = b"unfixed infos\0";
const TAG_TYPE: &[u8] = b"locked secret\0";

#[derive(Clone, PartialEq, Eq)]
struct MasterKey {
    bytes: [u8; MASTER_KEY_SIZE],
}

impl MasterKey {
    fn parse(bytes: &[u8], expected_type: &[u8], role: &str) -> Result<Self> {
        if bytes.len() != MASTER_KEY_SIZE {
            return Err(NfcError::InvalidArgument(format!(
                "the {role} key record is {} bytes; expected {MASTER_KEY_SIZE}",
                bytes.len()
            )));
        }
        if &bytes[16..30] != expected_type {
            return Err(NfcError::InvalidArgument(format!(
                "the {role} key record does not carry the expected type string"
            )));
        }
        if usize::from(bytes[31]) > 16 {
            return Err(NfcError::InvalidArgument(format!(
                "the {role} key record has an invalid magic-byte count"
            )));
        }
        let mut record = [0u8; MASTER_KEY_SIZE];
        record.copy_from_slice(bytes);
        Ok(Self { bytes: record })
    }

    fn hmac_key(&self) -> [u8; 16] {
        let mut key = [0u8; 16];
        key.copy_from_slice(&self.bytes[..16]);
        key
    }

    fn type_string(&self) -> &[u8] {
        &self.bytes[16..30]
    }

    fn magic_size(&self) -> usize {
        usize::from(self.bytes[31])
    }

    fn magic(&self) -> &[u8] {
        &self.bytes[32..48]
    }

    fn xor_pad(&self) -> &[u8] {
        &self.bytes[48..80]
    }
}

/// The data and tag master key records used for one Amiibo key set.
///
/// The records are secret input. `Debug` never prints key bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct AmiiboKeys {
    data: MasterKey,
    tag: MasterKey,
}

impl AmiiboKeys {
    /// Parse a 160-byte combined file: data record, then tag record.
    pub fn from_combined(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != COMBINED_KEY_SIZE {
            return Err(NfcError::InvalidArgument(format!(
                "the combined Amiibo key file is {} bytes; expected {COMBINED_KEY_SIZE}",
                bytes.len()
            )));
        }
        Self::from_parts(&bytes[..MASTER_KEY_SIZE], &bytes[MASTER_KEY_SIZE..])
    }

    /// Parse `unfixed-info.bin` and `locked-secret.bin` separate records.
    pub fn from_parts(data: &[u8], tag: &[u8]) -> Result<Self> {
        Ok(Self {
            data: MasterKey::parse(data, DATA_TYPE, "data")?,
            tag: MasterKey::parse(tag, TAG_TYPE, "tag")?,
        })
    }

    pub(crate) fn data(&self) -> KeyMaterial<'_> {
        KeyMaterial::from(&self.data)
    }

    pub(crate) fn tag(&self) -> KeyMaterial<'_> {
        KeyMaterial::from(&self.tag)
    }
}

impl fmt::Debug for AmiiboKeys {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmiiboKeys")
            .field("data", &"<redacted>")
            .field("tag", &"<redacted>")
            .finish()
    }
}

/// Borrowed master key fields for key derivation.
pub(crate) struct KeyMaterial<'a> {
    pub hmac_key: [u8; 16],
    pub type_string: &'a [u8],
    pub magic_size: usize,
    pub magic: &'a [u8],
    pub xor_pad: &'a [u8],
}

impl<'a> From<&'a MasterKey> for KeyMaterial<'a> {
    fn from(key: &'a MasterKey) -> Self {
        Self {
            hmac_key: key.hmac_key(),
            type_string: key.type_string(),
            magic_size: key.magic_size(),
            magic: key.magic(),
            xor_pad: key.xor_pad(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x11; 80];
        bytes[16..30].copy_from_slice(kind);
        bytes[31] = 0x0E;
        bytes
    }

    #[test]
    fn combined_and_split_records_match() {
        let data = record(DATA_TYPE);
        let tag = record(TAG_TYPE);
        let combined: Vec<u8> = data.iter().chain(tag.iter()).copied().collect();
        assert_eq!(
            AmiiboKeys::from_combined(&combined).expect("combined"),
            AmiiboKeys::from_parts(&data, &tag).expect("parts")
        );
    }

    #[test]
    fn wrong_records_and_swapped_files_are_refused() {
        let data = record(DATA_TYPE);
        let tag = record(TAG_TYPE);
        assert!(AmiiboKeys::from_parts(&tag, &data).is_err());
        assert!(AmiiboKeys::from_parts(&data[..79], &tag).is_err());
        assert!(AmiiboKeys::from_combined(&data).is_err());
        let mut oversized = data.clone();
        oversized[31] = 17;
        assert!(AmiiboKeys::from_parts(&oversized, &tag).is_err());
    }

    #[test]
    fn debug_never_prints_key_bytes() {
        let keys = AmiiboKeys::from_parts(&record(DATA_TYPE), &record(TAG_TYPE)).expect("keys");
        let debug = format!("{keys:?}");
        assert!(debug.contains("redacted"));
        assert!(!debug.contains("1111"));
    }
}
