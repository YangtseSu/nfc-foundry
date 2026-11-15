//! The read-only ISO 7816-4 command set used by the public-data adapters.
//!
//! Only SELECT by DF name, READ RECORD, and the T-Union GET BALANCE command
//! are constructed here. There is no UPDATE, credit, purchase, or PIN path.

use nfc_core::{CancelToken, NfcError, Result};
use nfc_transport::{ApduResponse, Reader};

/// SELECT by DF name: `00 A4 04 00 Lc <name> 00`.
pub fn select_apdu(name: &[u8]) -> Result<Vec<u8>> {
    if name.is_empty() || name.len() > 0xFF {
        return Err(NfcError::InvalidArgument(
            "a DF name must be 1..=255 bytes".to_owned(),
        ));
    }
    let mut apdu = Vec::with_capacity(6 + name.len());
    apdu.extend_from_slice(&[0x00, 0xA4, 0x04, 0x00, name.len() as u8]);
    apdu.extend_from_slice(name);
    apdu.push(0x00);
    Ok(apdu)
}

/// READ RECORD for one short EF identifier: `00 B2 <record> <sfi<<3|4> 00`.
///
/// The low five bits of the file identifier are the short EF identifier, as in
/// the T-Union reference reader.
pub fn read_record_apdu(record: u8, file_id: u8) -> Vec<u8> {
    let p2 = ((file_id & 0x1F) << 3) | 0x04;
    vec![0x00, 0xB2, record, p2, 0x00]
}

/// T-Union GET BALANCE: `80 5C 00 02 04`.
pub const fn balance_apdu() -> [u8; 5] {
    [0x80, 0x5C, 0x00, 0x02, 0x04]
}

/// SELECT by DF name and return the full response APDU.
pub fn select_by_name(
    reader: &mut dyn Reader,
    name: &[u8],
    context: &str,
    cancel: &CancelToken,
) -> Result<ApduResponse> {
    let apdu = select_apdu(name)?;
    exchange(reader, &apdu, context, cancel)
}

/// READ RECORD and return the full response APDU.
pub fn read_record(
    reader: &mut dyn Reader,
    record: u8,
    file_id: u8,
    context: &str,
    cancel: &CancelToken,
) -> Result<ApduResponse> {
    let apdu = read_record_apdu(record, file_id);
    exchange(reader, &apdu, context, cancel)
}

/// Send GET BALANCE and return the full response APDU.
pub fn get_balance(
    reader: &mut dyn Reader,
    context: &str,
    cancel: &CancelToken,
) -> Result<ApduResponse> {
    let apdu = balance_apdu();
    exchange(reader, &apdu, context, cancel)
}

/// Validate a successful status word.
pub fn ensure_ok(response: &ApduResponse, context: &str) -> Result<()> {
    if response.is_ok() {
        Ok(())
    } else {
        Err(NfcError::Io(format!(
            "{context} returned status {}",
            status_hex(response.sw)
        )))
    }
}

/// Whether the status word means "file or application not found".
pub const fn is_not_found(sw: [u8; 2]) -> bool {
    matches!(sw, [0x6A, 0x82] | [0x6A, 0x83])
}

/// Format a status word as four uppercase hexadecimal digits.
pub fn status_hex(sw: [u8; 2]) -> String {
    format!("{:02X}{:02X}", sw[0], sw[1])
}

pub(crate) fn exchange(
    reader: &mut dyn Reader,
    apdu: &[u8],
    context: &str,
    cancel: &CancelToken,
) -> Result<ApduResponse> {
    cancel.check()?;
    let capabilities = reader.apdu_capabilities();
    if !capabilities.supported {
        return Err(NfcError::Unsupported(
            "the selected reader has no ISO-DEP APDU exchange",
        ));
    }
    if apdu.len() > capabilities.max_command {
        return Err(NfcError::InvalidArgument(format!(
            "{context} needs {} bytes but the reader carries at most {}",
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
    fn select_apdu_uses_df_name_and_le() {
        let apdu = select_apdu(b"2PAY.SYS.DDF01").expect("select");
        assert_eq!(
            apdu,
            vec![
                0x00, 0xA4, 0x04, 0x00, 0x0E, b'2', b'P', b'A', b'Y', b'.', b'S', b'Y', b'S', b'.',
                b'D', b'D', b'F', b'0', b'1', 0x00
            ]
        );
        assert!(select_apdu(&[]).is_err());
    }

    #[test]
    fn read_record_encodes_the_short_ef_identifier() {
        assert_eq!(
            read_record_apdu(1, 0x18),
            vec![0x00, 0xB2, 0x01, 0xC4, 0x00]
        );
        assert_eq!(
            read_record_apdu(30, 0x1E),
            vec![0x00, 0xB2, 0x1E, 0xF4, 0x00]
        );
    }

    #[test]
    fn status_helpers_are_stable() {
        assert_eq!(status_hex([0x6A, 0x82]), "6A82");
        assert!(is_not_found([0x6A, 0x82]));
        assert!(is_not_found([0x6A, 0x83]));
        assert!(!is_not_found([0x90, 0x00]));
    }
}
