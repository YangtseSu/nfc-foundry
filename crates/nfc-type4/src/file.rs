//! NDEF file state for the Type 4 mapping.

use nfc_core::{NfcError, Result};
use nfc_ndef::NdefMessage;

use crate::cc::NdefFileControl;

/// The two-byte NDEF length field at the start of a Type 4 NDEF file.
pub const NLEN_LENGTH: usize = 2;

/// Decoded NDEF state of a Type 4 file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NdefState {
    /// NLEN is zero; the tag is formatted but carries no message.
    Empty,
    /// A decoded NDEF message.
    Message(NdefMessage),
}

/// NDEF facts and the current file bytes read from the tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type4Ndef {
    pub file_id: u16,
    /// Maximum file size from the NDEF File Control TLV, including NLEN.
    pub max_size: u16,
    pub read_access: u8,
    pub write_access: u8,
    pub state: NdefState,
    pub message_bytes: usize,
    /// NLEN followed by the encoded message, exactly as read.
    file_bytes: Vec<u8>,
}

impl Type4Ndef {
    pub fn from_control(control: NdefFileControl, file_bytes: Vec<u8>) -> Result<Self> {
        if file_bytes.len() < NLEN_LENGTH {
            return Err(NfcError::InvalidArgument(
                "the NDEF file is shorter than its two-byte NLEN field".to_owned(),
            ));
        }
        let nlen = usize::from(u16::from_be_bytes([file_bytes[0], file_bytes[1]]));
        if nlen > control.capacity() {
            return Err(NfcError::InvalidArgument(format!(
                "the NDEF length {nlen} exceeds the file capacity {}",
                control.capacity()
            )));
        }
        if file_bytes.len() < NLEN_LENGTH + nlen {
            return Err(NfcError::InvalidArgument(format!(
                "the NDEF file declares {nlen} bytes but only {} follow NLEN",
                file_bytes.len().saturating_sub(NLEN_LENGTH)
            )));
        }
        let message_bytes = &file_bytes[NLEN_LENGTH..NLEN_LENGTH + nlen];
        let state = if nlen == 0 {
            NdefState::Empty
        } else {
            NdefState::Message(NdefMessage::decode(message_bytes).map_err(|error| {
                NfcError::InvalidArgument(format!("the Type 4 NDEF message is invalid: {error}"))
            })?)
        };
        let mut file_bytes = file_bytes;
        file_bytes.truncate(NLEN_LENGTH + nlen);
        Ok(Self {
            file_id: control.file_id,
            max_size: control.max_size,
            read_access: control.read_access,
            write_access: control.write_access,
            state,
            message_bytes: nlen,
            file_bytes,
        })
    }

    pub fn message(&self) -> Option<&NdefMessage> {
        match &self.state {
            NdefState::Message(message) => Some(message),
            NdefState::Empty => None,
        }
    }

    pub const fn capacity(&self) -> usize {
        (self.max_size as usize).saturating_sub(NLEN_LENGTH)
    }

    pub fn readable(&self) -> bool {
        self.read_access == 0x00
    }

    pub fn writable(&self) -> bool {
        self.write_access == 0x00
    }

    /// Complete file bytes containing NLEN and the encoded message.
    pub fn file_bytes(&self) -> &[u8] {
        &self.file_bytes
    }

    /// Build canonical file bytes for an empty NDEF file.
    pub fn empty_file_bytes() -> Vec<u8> {
        vec![0x00, 0x00]
    }
}

#[cfg(test)]
mod tests {
    use nfc_ndef::{NdefRecord, TextRecord};

    use super::*;

    fn control() -> NdefFileControl {
        NdefFileControl {
            file_id: 0xE104,
            max_size: 0x0100,
            read_access: 0,
            write_access: 0,
        }
    }

    fn message(text: &str) -> NdefMessage {
        NdefMessage::new(vec![
            TextRecord::new(text, "en")
                .expect("text")
                .to_record()
                .expect("record"),
        ])
        .expect("message")
    }

    #[test]
    fn decodes_empty_and_populated_files() {
        let empty = Type4Ndef::from_control(control(), vec![0x00, 0x00]).expect("empty");
        assert_eq!(empty.state, NdefState::Empty);
        assert_eq!(empty.capacity(), 254);
        assert!(empty.writable());

        let encoded = message("hello").encode().expect("encode");
        let mut file = (encoded.len() as u16).to_be_bytes().to_vec();
        file.extend_from_slice(&encoded);
        let ndef = Type4Ndef::from_control(control(), file.clone()).expect("NDEF");
        assert_eq!(ndef.message(), Some(&message("hello")));
        assert_eq!(ndef.message_bytes, encoded.len());
        assert_eq!(ndef.file_bytes(), file.as_slice());
    }

    #[test]
    fn rejects_lengths_beyond_the_control_tlv() {
        let file = vec![0x01, 0x00];
        assert!(Type4Ndef::from_control(control(), file).is_err());

        let mut short = vec![0x00, 0x10];
        short.extend_from_slice(&[0x00; 4]);
        assert!(Type4Ndef::from_control(control(), short).is_err());

        let bad = NdefMessage::new(vec![NdefRecord::well_known(b"x", vec![1]).expect("record")])
            .expect("message")
            .encode()
            .expect("encode");
        let mut file = (bad.len() as u16).to_be_bytes().to_vec();
        file.extend_from_slice(&bad);
        file[0] = 0xFF;
        assert!(Type4Ndef::from_control(control(), file).is_err());
    }
}
