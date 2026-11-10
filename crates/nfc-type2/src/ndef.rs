//! NDEF Message TLV mapping onto the Type 2 data area.

use nfc_core::{NfcError, Result};
use nfc_ndef::{NdefMessage, Tlv, TlvStream, ndef_capacity};

use crate::page::Type2Image;

/// State of the NDEF Message TLV on a formatted or unformatted tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NdefState {
    /// No NDEF Message TLV is present.
    Unformatted,
    /// The NDEF Message TLV has a zero-length value.
    Empty,
    /// A decoded NDEF message.
    Message(NdefMessage),
}

/// NDEF facts for one Type 2 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type2Ndef {
    pub state: NdefState,
    /// Largest NDEF message that fits the capability container.
    pub capacity: usize,
    /// Encoded message size, zero when empty or unformatted.
    pub message_bytes: usize,
    /// The CC write-access property forbids data-area writes.
    pub read_only: bool,
}

impl Type2Ndef {
    pub fn message(&self) -> Option<&NdefMessage> {
        match &self.state {
            NdefState::Message(message) => Some(message),
            _ => None,
        }
    }
}

/// Parse the NDEF TLV from a complete Type 2 image. Unknown and proprietary
/// TLVs are not interpreted here.
pub fn read_ndef(image: &Type2Image) -> Result<Type2Ndef> {
    let area = image.ndef_area().ok_or_else(|| {
        NfcError::InvalidArgument("the NDEF data area is not completely readable".to_owned())
    })?;
    let stream = TlvStream::parse(&area)?;
    let mut ndef = None;
    for tlv in stream.tlvs() {
        if let Tlv::Ndef(value) = tlv
            && ndef.replace(value).is_some()
        {
            return Err(NfcError::InvalidArgument(
                "the Type 2 NDEF data area contains more than one NDEF TLV".to_owned(),
            ));
        }
    }
    let state = match ndef {
        None => NdefState::Unformatted,
        Some(value) if value.is_empty() => NdefState::Empty,
        Some(value) => {
            let message = NdefMessage::decode(value).map_err(|error| {
                NfcError::InvalidArgument(format!("the Type 2 NDEF TLV is invalid: {error}"))
            })?;
            NdefState::Message(message)
        }
    };
    let message_bytes = match &state {
        NdefState::Message(message) => message.encoded_len()?,
        _ => 0,
    };
    Ok(Type2Ndef {
        state,
        capacity: image.cc.ndef_capacity(),
        message_bytes,
        read_only: !image.cc.writable(),
    })
}

/// Build the complete data-area bytes for an NDEF message.
///
/// Unknown, proprietary, Lock Control, and Memory Control TLVs are preserved.
/// NULL TLVs and old padding are replaced by the Terminator and zero fill.
pub fn encode_ndef_area(image: &Type2Image, message: &NdefMessage) -> Result<Vec<u8>> {
    message.validate()?;
    let capacity = capacity_bytes(image);
    let message = message.encode()?;
    if message.len() > capacity {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF message is {} bytes but the tag holds at most {capacity}",
            message.len()
        )));
    }
    let replacement = Tlv::Ndef(message);

    let mut output = Vec::new();
    if let Some(area) = image.ndef_area() {
        let stream = TlvStream::parse(&area)?;
        let has_ndef = stream.tlvs().iter().any(|tlv| matches!(tlv, Tlv::Ndef(_)));
        let mut inserted = has_ndef;
        let mut seen_non_control = false;
        for tlv in stream.tlvs() {
            match tlv {
                Tlv::Null | Tlv::Terminator => {}
                Tlv::Ndef(_) => {
                    output.extend_from_slice(&replacement.encode()?);
                }
                Tlv::LockControl(_) | Tlv::MemoryControl(_) => {
                    if !inserted && seen_non_control {
                        output.extend_from_slice(&replacement.encode()?);
                        inserted = true;
                    }
                    output.extend_from_slice(&tlv.encode()?);
                }
                Tlv::Proprietary(_) | Tlv::Unknown { .. } => {
                    if !inserted {
                        output.extend_from_slice(&replacement.encode()?);
                        inserted = true;
                    }
                    seen_non_control = true;
                    output.extend_from_slice(&tlv.encode()?);
                }
            }
        }
        if !inserted {
            output.extend_from_slice(&replacement.encode()?);
        }
    } else {
        output.extend_from_slice(&replacement.encode()?);
    }
    pad_area(image, output, true)
}

/// Build a formatted but empty NDEF area (`03 00 FE` plus zero fill).
pub fn format_ndef_area(image: &Type2Image) -> Result<Vec<u8>> {
    pad_area(image, vec![0x03, 0x00, 0xFE], false)
}

fn capacity_bytes(image: &Type2Image) -> usize {
    let size = usize::from(image.cc.size);
    ndef_capacity(size).min(size.saturating_sub(2))
}

fn pad_area(image: &Type2Image, mut content: Vec<u8>, add_terminator: bool) -> Result<Vec<u8>> {
    let size = usize::from(image.cc.size);
    if content.len() > size {
        return Err(NfcError::InvalidArgument(format!(
            "the Type 2 TLV content is {} bytes but the data area is {size}",
            content.len()
        )));
    }
    if add_terminator && content.len() < size {
        content.push(0xFE);
    }
    if content.len() > size {
        return Err(NfcError::InvalidArgument(
            "the Type 2 Terminator does not fit the data area".to_owned(),
        ));
    }
    content.resize(size, 0x00);
    Ok(content)
}

#[cfg(test)]
mod tests {
    use nfc_ndef::{NdefRecord, TextRecord};

    use crate::cc::CapabilityContainer;
    use crate::layout::Type2Layout;

    use super::*;

    fn image() -> Type2Image {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0x00]).expect("CC");
        let layout = Type2Layout::from_cc(&cc).expect("layout");
        Type2Image::new(layout, cc, vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80])
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

    fn area_with(message: &NdefMessage) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&Tlv::ndef(message).expect("TLV").encode().expect("encode"));
        bytes.push(0xFE);
        bytes
    }

    #[test]
    fn reads_unformatted_empty_and_message_states() {
        let mut tag = image();
        tag.set_ndef_area(&[0u8; 144]).expect("unformatted");
        assert_eq!(read_ndef(&tag).expect("read").state, NdefState::Unformatted);

        let mut bytes = area_with(&message("hello"));
        bytes.resize(144, 0);
        tag.set_ndef_area(&bytes).expect("message");
        let ndef = read_ndef(&tag).expect("read");
        assert!(matches!(ndef.state, NdefState::Message(_)));
        assert_eq!(
            ndef.message_bytes,
            message("hello").encoded_len().expect("len")
        );

        let mut bytes = format_ndef_area(&tag).expect("format");
        assert_eq!(&bytes[..3], &[0x03, 0x00, 0xFE]);
        bytes.resize(144, 0);
        tag.set_ndef_area(&bytes).expect("empty");
        assert_eq!(read_ndef(&tag).expect("read").state, NdefState::Empty);
    }

    #[test]
    fn write_area_preserves_unknown_tlvs_and_pads() {
        let mut tag = image();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0xFD, 0x02, 0xAA, 0xBB]);
        bytes.extend_from_slice(
            &Tlv::ndef(&message("old"))
                .expect("TLV")
                .encode()
                .expect("enc"),
        );
        bytes.extend_from_slice(&[0xFE]);
        bytes.resize(144, 0);
        tag.set_ndef_area(&bytes).expect("area");

        let area = encode_ndef_area(&tag, &message("new")).expect("encode");
        assert_eq!(area.len(), 144);
        assert!(
            area.windows(4)
                .any(|window| window == [0xFD, 0x02, 0xAA, 0xBB]),
            "proprietary TLV was not preserved"
        );
        let stream = TlvStream::parse(&area).expect("parse");
        assert_eq!(
            stream.ndef_message().expect("message").expect("present"),
            message("new")
        );
        assert!(matches!(stream.tlvs().last(), Some(Tlv::Terminator)));
    }

    #[test]
    fn exact_capacity_fills_the_data_area() {
        let mut tag = image();
        let capacity = tag.cc.ndef_capacity();
        let payload = vec![0x5A; capacity - 4];
        let record = NdefRecord::well_known(b"x", payload).expect("record");
        let message = NdefMessage::new(vec![record]).expect("message");
        let encoded = message.encode().expect("encoded");
        assert_eq!(encoded.len(), capacity);
        tag.set_ndef_area(&[0u8; 144]).expect("area");
        let area = encode_ndef_area(&tag, &message).expect("encode");
        assert_eq!(area.len(), 144);
        assert_eq!(&area[..2], &[0x03, 0x8D]);
        assert_eq!(&area[2..143], encoded.as_slice());
        assert_eq!(area[143], 0xFE);
    }

    #[test]
    fn rejects_message_larger_than_capacity() {
        let tag = image();
        let record = NdefRecord::well_known(b"x", vec![0; 200]).expect("record");
        let message = NdefMessage::new(vec![record]).expect("message");
        assert!(encode_ndef_area(&tag, &message).is_err());
    }

    #[test]
    fn format_clears_previous_content() {
        let mut tag = image();
        let mut bytes = area_with(&message("hello"));
        bytes.resize(144, 0);
        tag.set_ndef_area(&bytes).expect("area");
        let area = format_ndef_area(&tag).expect("format");
        assert_eq!(&area[..3], &[0x03, 0x00, 0xFE]);
        assert!(area[3..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn partial_area_is_not_decoded() {
        let tag = image();
        assert!(read_ndef(&tag).is_err());
    }
}
