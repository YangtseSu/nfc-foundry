//! Reader-backed Type 4 capability container and NDEF file reads.

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_transport::Reader;

use crate::apdu;
use crate::cc::Type4CapabilityContainer;
use crate::file::{NLEN_LENGTH, Type4Ndef};

/// Largest CC the adapter will read. Type 4 files use the short READ BINARY
/// offset range; real capability containers are far smaller.
const MAX_CC_LENGTH: usize = 0x7FFF;

/// A complete Type 4 read with the parsed CC and NDEF file.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type4ReadResult {
    pub cc: Type4CapabilityContainer,
    pub ndef: Type4Ndef,
}

impl Type4ReadResult {
    pub fn writable(&self) -> bool {
        self.ndef.writable()
    }

    pub fn read_only_reason(&self) -> Option<&'static str> {
        self.cc.read_only_reason()
    }
}

/// Select the NDEF Tag Application, read the CC file, then read NLEN and the
/// NDEF message. Any incomplete step is an error; partial data is never
/// reported as a complete tag.
pub fn read_tag(
    reader: &mut dyn Reader,
    card: &CardInfo,
    cancel: &CancelToken,
) -> Result<Type4ReadResult> {
    cancel.check()?;
    if card.card_type != CardType::Iso14443 {
        return Err(NfcError::Unsupported(
            "the selected card is not an ISO/IEC 14443-4 Type 4 tag",
        ));
    }
    select_application(reader, cancel)?;
    cancel.check()?;
    select_file(
        reader,
        apdu::CC_FILE_ID,
        "selecting the capability container",
        cancel,
    )?;

    // A Type 4 CC is at least 15 bytes and MLe is at least 15, so the fixed
    // header fits in one read even before MLe is known.
    let header = read_range(reader, 0, 15, 15, cancel, "reading the CC header")?;
    let cclen = usize::from(u16::from_be_bytes([header[0], header[1]]));
    if cclen < 15 {
        return Err(NfcError::InvalidArgument(format!(
            "the capability container length {cclen} is below 15"
        )));
    }
    if cclen > MAX_CC_LENGTH {
        return Err(NfcError::InvalidArgument(format!(
            "the capability container length {cclen} exceeds the short-file range"
        )));
    }
    let max_le = u16::from_be_bytes([header[3], header[4]]);
    if max_le < 15 {
        return Err(NfcError::InvalidArgument(format!(
            "the capability container MLe {max_le} is below 15"
        )));
    }
    let mut cc_bytes = header;
    if cclen > cc_bytes.len() {
        let mut tail = read_range(
            reader,
            cc_bytes.len() as u16,
            cclen - cc_bytes.len(),
            max_le,
            cancel,
            "reading the capability container",
        )?;
        cc_bytes.append(&mut tail);
    }
    let cc = Type4CapabilityContainer::parse(&cc_bytes)?;
    let control = cc.ndef_file()?;
    if !control.readable() {
        return Err(NfcError::Unsupported(
            "the Type 4 NDEF file requires a proprietary read security condition",
        ));
    }

    cancel.check()?;
    select_file(reader, control.file_id, "selecting the NDEF file", cancel)?;
    let nlen_bytes = read_range(
        reader,
        0,
        NLEN_LENGTH,
        cc.max_le,
        cancel,
        "reading the NDEF length",
    )?;
    let nlen = usize::from(u16::from_be_bytes([nlen_bytes[0], nlen_bytes[1]]));
    if nlen > control.capacity() {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF length {nlen} exceeds the file capacity {}",
            control.capacity()
        )));
    }
    let mut file_bytes = nlen_bytes;
    if nlen > 0 {
        let mut message = read_range(
            reader,
            NLEN_LENGTH as u16,
            nlen,
            cc.max_le,
            cancel,
            "reading the NDEF message",
        )?;
        file_bytes.append(&mut message);
    }
    let ndef = Type4Ndef::from_control(control, file_bytes)?;
    Ok(Type4ReadResult { cc, ndef })
}

pub(crate) fn select_application(reader: &mut dyn Reader, cancel: &CancelToken) -> Result<()> {
    cancel.check()?;
    let response = apdu::transceive(
        reader,
        &apdu::select_tag_application(),
        "selecting the NDEF tag application",
    )?;
    apdu::check_ok(&response, "selecting the NDEF tag application")?;
    Ok(())
}

pub(crate) fn select_file(
    reader: &mut dyn Reader,
    file_id: u16,
    context: &str,
    cancel: &CancelToken,
) -> Result<()> {
    cancel.check()?;
    let response = apdu::transceive(reader, &apdu::select_file(file_id), context)?;
    apdu::check_ok(&response, context)?;
    Ok(())
}

/// Read an exact range, splitting it by the tag's MLe and the short READ
/// BINARY limit of 256 bytes.
pub(crate) fn read_range(
    reader: &mut dyn Reader,
    offset: u16,
    length: usize,
    max_le: u16,
    cancel: &CancelToken,
    context: &str,
) -> Result<Vec<u8>> {
    let chunk_limit = usize::from(max_le).min(256);
    if chunk_limit == 0 {
        return Err(NfcError::InvalidArgument(
            "the tag reports a zero MLe".to_owned(),
        ));
    }
    let mut output = Vec::with_capacity(length);
    let mut remaining = length;
    let mut offset = offset;
    while remaining > 0 {
        cancel.check()?;
        let chunk = remaining.min(chunk_limit);
        let apdu = apdu::read_binary(offset, chunk)?;
        let response = apdu::transceive(reader, &apdu, context)?;
        let data = apdu::check_ok(&response, context)?;
        if data.len() != chunk {
            return Err(NfcError::Io(format!(
                "{context}: the tag returned {} bytes instead of {chunk}",
                data.len()
            )));
        }
        output.extend_from_slice(data);
        remaining -= chunk;
        offset = u16::try_from(usize::from(offset) + chunk).map_err(|_| {
            NfcError::InvalidArgument("the file offset exceeds the short-file range".to_owned())
        })?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::{MockReader, MockType4Tag};

    use super::*;

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            [0x00, 0x44],
            0x20,
            CardType::Iso14443,
        )
    }

    fn reader() -> MockReader {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:type4"), Some(card()));
        reader.set_type4_tag(MockType4Tag::ntag424());
        reader
    }

    #[test]
    fn reads_ntag424_cc_and_empty_ndef() {
        let mut reader = reader();
        let result = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        assert_eq!(result.cc.cclen, 23);
        assert_eq!(result.cc.max_le, 0x0100);
        assert_eq!(result.cc.max_lc, 0x00FF);
        assert_eq!(result.ndef.max_size, 0x0100);
        assert_eq!(result.ndef.state, crate::NdefState::Empty);
        assert_eq!(result.ndef.file_bytes(), &[0x00, 0x00]);
        assert!(result.writable());
    }

    #[test]
    fn rejects_non_type4_cards() {
        let mut reader = reader();
        let classic = CardInfo::new(vec![1; 4], [0, 4], 0x08, CardType::MifareClassic1K);
        assert!(matches!(
            read_tag(&mut reader, &classic, &CancelToken::new()),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[test]
    fn reads_a_populated_ndef_file() {
        use nfc_ndef::{NdefMessage, TextRecord};

        let message = NdefMessage::new(vec![
            TextRecord::new("tag", "en")
                .expect("text")
                .to_record()
                .expect("record"),
        ])
        .expect("message")
        .encode()
        .expect("encode");
        let mut ndef = (message.len() as u16).to_be_bytes().to_vec();
        ndef.extend_from_slice(&message);
        ndef.resize(256, 0);
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:type4"), Some(card()));
        reader.set_type4_tag(MockType4Tag::new(
            MockType4Tag::ntag424().cc().to_vec(),
            ndef,
        ));

        let result = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        assert_eq!(result.ndef.message_bytes, message.len());
        assert_eq!(
            result
                .ndef
                .message()
                .expect("message")
                .encode()
                .expect("encode"),
            message
        );
    }

    #[test]
    fn read_only_cc_is_reported() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:type4"), Some(card()));
        let cc = vec![
            0x00, 0x0F, 0x20, 0x00, 0x3A, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
            0xFF,
        ];
        reader.set_type4_tag(MockType4Tag::new(cc, vec![0u8; 256]).with_write_access(0xFF));
        let result = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        assert!(!result.writable());
        assert_eq!(
            result.read_only_reason(),
            Some("the NDEF File Control TLV marks the file read-only")
        );
    }

    #[test]
    fn cancellation_is_checked_before_io() {
        let mut reader = nfc_transport::mock::MockReader::new(
            DeviceInfo::new("mock", "mock:type4"),
            Some(card()),
        );
        reader.set_type4_tag(MockType4Tag::ntag424());
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            read_tag(&mut reader, &card(), &cancel),
            Err(NfcError::Cancelled)
        ));
    }
}
