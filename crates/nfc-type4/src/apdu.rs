//! ISO 7816-4 command APDUs used by the Type 4 NDEF mapping.

use nfc_core::{NfcError, Result};
use nfc_transport::{ApduResponse, Reader};

/// NDEF Tag Application DF name from the NFC Forum Type 4 Tag specification.
pub const NDEF_TAG_APPLICATION: [u8; 7] = [0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01];
/// Capability Container file identifier.
pub const CC_FILE_ID: u16 = 0xE103;
/// Status word for a successful command.
pub const SW_OK: [u8; 2] = [0x90, 0x00];
/// Status word returned when a read reaches the end of the file.
pub const SW_END_OF_FILE: [u8; 2] = [0x62, 0x82];

/// SELECT by DF name for the NDEF Tag Application.
pub fn select_tag_application() -> Vec<u8> {
    let mut apdu = Vec::with_capacity(5 + NDEF_TAG_APPLICATION.len() + 1);
    apdu.extend_from_slice(&[0x00, 0xA4, 0x04, 0x00]);
    apdu.push(NDEF_TAG_APPLICATION.len() as u8);
    apdu.extend_from_slice(&NDEF_TAG_APPLICATION);
    apdu.push(0x00);
    apdu
}

/// SELECT FILE by a two-byte ISO file identifier, without returning FCI.
pub fn select_file(file_id: u16) -> Vec<u8> {
    let [high, low] = file_id.to_be_bytes();
    vec![0x00, 0xA4, 0x00, 0x0C, 0x02, high, low]
}

/// READ BINARY at a 15-bit offset. A length of 256 is encoded as `Le = 00`.
pub fn read_binary(offset: u16, length: usize) -> Result<Vec<u8>> {
    if !(1..=256).contains(&length) {
        return Err(NfcError::InvalidArgument(format!(
            "a READ BINARY length is {length}, expected 1..=256"
        )));
    }
    if offset > 0x7FFF || usize::from(offset) + length > 0x8000 {
        return Err(NfcError::InvalidArgument(format!(
            "READ BINARY offset {offset} with length {length} is outside the short-file range"
        )));
    }
    let [high, low] = offset.to_be_bytes();
    let le = if length == 256 { 0x00 } else { length as u8 };
    Ok(vec![0x00, 0xB0, high, low, le])
}

/// UPDATE BINARY at a 15-bit offset.
pub fn update_binary(offset: u16, data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() || data.len() > 255 {
        return Err(NfcError::InvalidArgument(format!(
            "an UPDATE BINARY payload is {} bytes, expected 1..=255",
            data.len()
        )));
    }
    if offset > 0x7FFF || usize::from(offset) + data.len() > 0x8000 {
        return Err(NfcError::InvalidArgument(format!(
            "UPDATE BINARY offset {offset} with length {} is outside the short-file range",
            data.len()
        )));
    }
    let [high, low] = offset.to_be_bytes();
    let mut apdu = Vec::with_capacity(5 + data.len());
    apdu.extend_from_slice(&[0x00, 0xD6, high, low]);
    apdu.push(data.len() as u8);
    apdu.extend_from_slice(data);
    Ok(apdu)
}

/// Validate a response status word and return its response data.
pub fn check_ok<'a>(response: &'a ApduResponse, context: &str) -> Result<&'a [u8]> {
    match response.sw {
        SW_OK => Ok(&response.data),
        SW_END_OF_FILE => Err(NfcError::Io(format!(
            "{context}: the tag reached the end of the file before the requested length"
        ))),
        [0x6A, 0x82] => Err(NfcError::InvalidArgument(format!(
            "{context}: the NDEF tag application or file was not found"
        ))),
        [0x69, 0x82] | [0x69, 0x86] => Err(NfcError::Unsupported(
            "the Type 4 file requires a security condition that this reader does not provide",
        )),
        [0x6A, 0x84] => Err(NfcError::InvalidArgument(format!(
            "{context}: the tag has not enough memory"
        ))),
        [0x67, 0x00] => Err(NfcError::InvalidArgument(format!(
            "{context}: the tag rejected the APDU length"
        ))),
        [0x6B, 0x00] => Err(NfcError::InvalidArgument(format!(
            "{context}: the offset is outside the file"
        ))),
        [0x6A, 0x80] => Err(NfcError::InvalidArgument(format!(
            "{context}: the tag rejected the command parameters"
        ))),
        [0x6D, 0x00] | [0x6E, 0x00] => Err(NfcError::Unsupported(
            "the Type 4 tag does not support the required ISO 7816-4 command",
        )),
        [sw1, sw2] => Err(NfcError::Io(format!(
            "{context}: APDU status {sw1:02X}{sw2:02X}"
        ))),
    }
}

/// Send one APDU after checking the reader capability and frame limit.
pub fn transceive(reader: &mut dyn Reader, apdu: &[u8], context: &str) -> Result<ApduResponse> {
    let capabilities = reader.apdu_capabilities();
    if !capabilities.supported {
        return Err(NfcError::Unsupported(
            "this reader has no ISO-DEP APDU exchange",
        ));
    }
    if apdu.len() > capabilities.max_command {
        return Err(NfcError::InvalidArgument(format!(
            "{context}: APDU is {} bytes but the reader supports at most {}",
            apdu.len(),
            capabilities.max_command
        )));
    }
    reader.transceive_apdu(apdu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_select_and_apdu_frames() {
        assert_eq!(
            select_tag_application(),
            vec![
                0x00, 0xA4, 0x04, 0x00, 0x07, 0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01, 0x00
            ]
        );
        assert_eq!(
            select_file(CC_FILE_ID),
            vec![0x00, 0xA4, 0x00, 0x0C, 0x02, 0xE1, 0x03]
        );
        assert_eq!(
            read_binary(0x0002, 0x00FF).expect("read"),
            vec![0x00, 0xB0, 0x00, 0x02, 0xFF]
        );
        assert_eq!(
            read_binary(0, 256).expect("read 256"),
            vec![0x00, 0xB0, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            update_binary(0, &[0x00, 0x00]).expect("update"),
            vec![0x00, 0xD6, 0x00, 0x00, 0x02, 0x00, 0x00]
        );
    }

    #[test]
    fn rejects_invalid_offsets_and_lengths() {
        assert!(read_binary(0x8000, 1).is_err());
        assert!(read_binary(0, 0).is_err());
        assert!(read_binary(0, 257).is_err());
        assert!(update_binary(0, &[]).is_err());
        assert!(update_binary(0x7FFF, &[1, 2]).is_err());
        assert!(update_binary(0, &[0; 256]).is_err());
    }

    #[test]
    fn maps_status_words() {
        let ok = ApduResponse::new(vec![1, 2], SW_OK);
        assert_eq!(check_ok(&ok, "test").expect("ok"), &[1, 2]);
        for sw in [
            [0x6A, 0x82],
            [0x69, 0x82],
            [0x6A, 0x84],
            [0x67, 0x00],
            [0x6B, 0x00],
        ] {
            let response = ApduResponse::new(Vec::new(), sw);
            assert!(check_ok(&response, "test").is_err());
        }
        let unknown = ApduResponse::new(Vec::new(), [0x6F, 0x00]);
        assert!(matches!(check_ok(&unknown, "test"), Err(NfcError::Io(_))));
    }
}
