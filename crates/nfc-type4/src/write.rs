//! Verified Type 4 NDEF file writes.
//!
//! The Type 4 write procedure sets NLEN to zero, writes the message in
//! transport-sized chunks, writes the final NLEN, then reads the file back.
//! The update is not atomic: a cancellation or transport failure can leave
//! NLEN at zero. The caller must re-read the tag before retrying.

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_ndef::NdefMessage;
use nfc_transport::Reader;

use crate::apdu;
use crate::cc::Type4CapabilityContainer;
use crate::file::{NLEN_LENGTH, Type4Ndef};
use crate::read::{read_range, select_application, select_file};

/// Result of a verified NDEF write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type4WriteResult {
    pub ndef: Type4Ndef,
    pub written: usize,
}

/// Result of clearing the NDEF file.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type4ClearResult {
    pub ndef: Type4Ndef,
}

/// Write one NDEF message into the mandatory NDEF file and verify it.
pub fn write_ndef(
    reader: &mut dyn Reader,
    card: &CardInfo,
    cc: &Type4CapabilityContainer,
    message: &NdefMessage,
    cancel: &CancelToken,
) -> Result<Type4WriteResult> {
    cancel.check()?;
    require_iso_dep(card)?;
    let control = cc.ndef_file()?;
    if let Some(reason) = control.write_denied_reason() {
        return Err(NfcError::Unsupported(reason));
    }
    message.validate()?;
    let encoded = message.encode()?;
    if encoded.len() > control.capacity() {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF message is {} bytes but the file holds at most {}",
            encoded.len(),
            control.capacity()
        )));
    }
    let nlen = u16::try_from(encoded.len()).map_err(|_| {
        NfcError::InvalidArgument("the NDEF message exceeds 65535 bytes".to_owned())
    })?;

    select_application(reader, cancel)?;
    select_file(reader, control.file_id, "selecting the NDEF file", cancel)?;
    write_nlen(reader, 0, cancel)?;
    write_message(reader, cc, &encoded, cancel)?;
    write_nlen(reader, nlen, cancel)?;
    verify_file(reader, cc, &encoded, cancel)?;

    let mut file_bytes = nlen.to_be_bytes().to_vec();
    file_bytes.extend_from_slice(&encoded);
    Ok(Type4WriteResult {
        ndef: Type4Ndef::from_control(control, file_bytes)?,
        written: encoded.len(),
    })
}

/// Write NLEN zero and verify that the NDEF file is empty.
pub fn clear_ndef(
    reader: &mut dyn Reader,
    card: &CardInfo,
    cc: &Type4CapabilityContainer,
    cancel: &CancelToken,
) -> Result<Type4ClearResult> {
    cancel.check()?;
    require_iso_dep(card)?;
    let control = cc.ndef_file()?;
    if let Some(reason) = control.write_denied_reason() {
        return Err(NfcError::Unsupported(reason));
    }

    select_application(reader, cancel)?;
    select_file(reader, control.file_id, "selecting the NDEF file", cancel)?;
    write_nlen(reader, 0, cancel)?;
    verify_file(reader, cc, &[], cancel)?;
    Ok(Type4ClearResult {
        ndef: Type4Ndef::from_control(control, vec![0x00, 0x00])?,
    })
}

fn require_iso_dep(card: &CardInfo) -> Result<()> {
    if card.card_type != CardType::Iso14443 {
        return Err(NfcError::Unsupported(
            "the selected card is not an ISO/IEC 14443-4 Type 4 tag",
        ));
    }
    Ok(())
}

fn write_nlen(reader: &mut dyn Reader, nlen: u16, cancel: &CancelToken) -> Result<()> {
    cancel.check()?;
    let apdu = apdu::update_binary(0, &nlen.to_be_bytes())?;
    let response = apdu::transceive(reader, &apdu, "writing the NDEF length")?;
    apdu::check_ok(&response, "writing the NDEF length")?;
    Ok(())
}

fn write_message(
    reader: &mut dyn Reader,
    cc: &Type4CapabilityContainer,
    encoded: &[u8],
    cancel: &CancelToken,
) -> Result<()> {
    let capabilities = reader.apdu_capabilities();
    let chunk = usize::from(cc.max_lc)
        .min(capabilities.max_command.saturating_sub(5))
        .min(255);
    if chunk == 0 {
        return Err(NfcError::Unsupported(
            "the reader cannot carry an UPDATE BINARY payload",
        ));
    }
    let mut offset = NLEN_LENGTH;
    for data in encoded.chunks(chunk) {
        cancel.check()?;
        let apdu = apdu::update_binary(offset as u16, data)?;
        let response = apdu::transceive(reader, &apdu, "writing the NDEF message")?;
        apdu::check_ok(&response, "writing the NDEF message")?;
        offset += data.len();
    }
    Ok(())
}

fn verify_file(
    reader: &mut dyn Reader,
    cc: &Type4CapabilityContainer,
    expected: &[u8],
    cancel: &CancelToken,
) -> Result<()> {
    let control = cc.ndef_file()?;
    select_file(
        reader,
        control.file_id,
        "selecting the NDEF file for verification",
        cancel,
    )?;
    let nlen_bytes = read_range(
        reader,
        0,
        NLEN_LENGTH,
        cc.max_le,
        cancel,
        "verifying the NDEF length",
    )?;
    let nlen = usize::from(u16::from_be_bytes([nlen_bytes[0], nlen_bytes[1]]));
    if nlen != expected.len() {
        return Err(NfcError::VerificationFailed(format!(
            "the NDEF length read back as {nlen}, expected {}",
            expected.len()
        )));
    }
    if nlen > 0 {
        let actual = read_range(
            reader,
            NLEN_LENGTH as u16,
            nlen,
            cc.max_le,
            cancel,
            "verifying the NDEF message",
        )?;
        if actual != expected {
            return Err(NfcError::VerificationFailed(
                "the NDEF message did not match after the write".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nfc_ndef::TextRecord;
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::{MockCall, MockFault, MockReader, MockType4Tag};

    use super::*;
    use crate::read::read_tag;

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
    fn writes_and_verifies_a_message() {
        let mut reader = reader();
        let tag = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        let result = write_ndef(
            &mut reader,
            &card(),
            &tag.cc,
            &message("hello"),
            &CancelToken::new(),
        )
        .expect("write");
        assert_eq!(result.written, message("hello").encoded_len().expect("len"));
        assert_eq!(result.ndef.message(), Some(&message("hello")));

        let reread = read_tag(&mut reader, &card(), &CancelToken::new()).expect("reread");
        assert_eq!(reread.ndef.message(), Some(&message("hello")));
    }

    #[test]
    fn clears_and_verifies_an_empty_file() {
        let mut reader = reader();
        let tag = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        write_ndef(
            &mut reader,
            &card(),
            &tag.cc,
            &message("hello"),
            &CancelToken::new(),
        )
        .expect("write");
        let cleared =
            clear_ndef(&mut reader, &card(), &tag.cc, &CancelToken::new()).expect("clear");
        assert_eq!(cleared.ndef.state, crate::NdefState::Empty);
        let reread = read_tag(&mut reader, &card(), &CancelToken::new()).expect("reread");
        assert_eq!(reread.ndef.state, crate::NdefState::Empty);
    }

    #[test]
    fn refuses_read_only_files() {
        let mut reader = reader();
        let mut tag = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        tag.cc = crate::Type4CapabilityContainer::parse(&[
            0x00, 0x0F, 0x20, 0x00, 0x3A, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
            0xFF,
        ])
        .expect("read-only CC");
        assert!(matches!(
            write_ndef(
                &mut reader,
                &card(),
                &tag.cc,
                &message("hello"),
                &CancelToken::new(),
            ),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[test]
    fn detects_an_ignored_write() {
        let mut reader = reader();
        let tag = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        // Ignore the NLEN=0 update and the message update. The final NLEN is
        // written, but the message bytes still do not match.
        reader.fail_next(MockCall::ApduWrite, MockFault::WriteIgnored);
        reader.fail_next(MockCall::ApduWrite, MockFault::WriteIgnored);
        assert!(matches!(
            write_ndef(
                &mut reader,
                &card(),
                &tag.cc,
                &message("hello"),
                &CancelToken::new(),
            ),
            Err(NfcError::VerificationFailed(_))
        ));
    }

    #[test]
    fn oversized_message_is_rejected_before_io() {
        let mut reader = reader();
        let tag = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        let record = nfc_ndef::NdefRecord::well_known(b"x", vec![0; 300]).expect("record");
        let big = NdefMessage::new(vec![record]).expect("message");
        assert!(matches!(
            write_ndef(&mut reader, &card(), &tag.cc, &big, &CancelToken::new()),
            Err(NfcError::InvalidArgument(_))
        ));
    }

    #[test]
    fn chunks_a_message_larger_than_one_update_binary() {
        let mut reader = reader();
        let tag = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        let record = nfc_ndef::NdefRecord::well_known(b"x", vec![0x5A; 240]).expect("record");
        let message = NdefMessage::new(vec![record]).expect("message");
        let encoded = message.encode().expect("encode");
        assert!(encoded.len() > 242);
        assert!(encoded.len() <= 254);
        let result = write_ndef(&mut reader, &card(), &tag.cc, &message, &CancelToken::new())
            .expect("write");
        assert_eq!(result.written, encoded.len());
        let reread = read_tag(&mut reader, &card(), &CancelToken::new()).expect("reread");
        assert_eq!(reread.ndef.message(), Some(&message));
    }

    #[test]
    fn reads_a_message_in_several_mle_chunks() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:type4"), Some(card()));
        let cc = vec![
            0x00, 0x0F, 0x20, 0x00, 0x0F, 0x00, 0x0F, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
            0x00,
        ];
        let message = NdefMessage::new(vec![
            TextRecord::new("x".repeat(24), "en")
                .expect("text")
                .to_record()
                .expect("record"),
        ])
        .expect("message");
        let encoded = message.encode().expect("encode");
        let mut ndef = (encoded.len() as u16).to_be_bytes().to_vec();
        ndef.extend_from_slice(&encoded);
        ndef.resize(256, 0);
        reader.set_type4_tag(MockType4Tag::new(cc, ndef));

        let read = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        assert_eq!(read.cc.max_le, 15);
        assert_eq!(read.ndef.message(), Some(&message));
    }
}
