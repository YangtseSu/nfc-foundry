//! PN53x command values, register addresses, and response parsing shared by
//! the UART and ACR122U transports. The chip protocol is the same; only the
//! host frame carrying it differs.

use nfc_core::{CardInfo, CardType, NfcError, Result};

pub(crate) const PN532_TFI: u8 = 0xD5;

pub(crate) const GET_FIRMWARE_VERSION: u8 = 0x02;
pub(crate) const READ_REGISTER: u8 = 0x06;
pub(crate) const WRITE_REGISTER: u8 = 0x08;
pub(crate) const SET_PARAMETERS: u8 = 0x12;
pub(crate) const SAM_CONFIGURATION: u8 = 0x14;
pub(crate) const RF_CONFIGURATION: u8 = 0x32;
pub(crate) const RF_FIELD: u8 = 0x01;
pub(crate) const RF_RETRY_SELECT: u8 = 0x05;
pub(crate) const IN_DATA_EXCHANGE: u8 = 0x40;
pub(crate) const IN_COMMUNICATE_THRU: u8 = 0x42;
pub(crate) const IN_LIST_PASSIVE_TARGET: u8 = 0x4A;
pub(crate) const IN_RELEASE: u8 = 0x52;

pub(crate) const REG_CIU_TX_MODE: u16 = 0x6302;
pub(crate) const REG_CIU_RX_MODE: u16 = 0x6303;
pub(crate) const REG_CIU_MANUAL_RCV: u16 = 0x630D;
pub(crate) const REG_CIU_STATUS2: u16 = 0x6338;
pub(crate) const REG_CIU_CONTROL: u16 = 0x633C;
pub(crate) const REG_CIU_BIT_FRAMING: u16 = 0x633D;

pub(crate) const TX_RX_CRC_ENABLE: u8 = 0x80;
pub(crate) const INITIATOR: u8 = 0x10;
pub(crate) const RX_LAST_BITS: u8 = 0x07;
pub(crate) const AUTO_ATR_RES_AND_RATS: u8 = 0x14;
pub(crate) const PN532_IC: u8 = 0x32;
pub(crate) const ISO14443A_SUPPORT: u8 = 0x01;

pub(crate) const MIFARE_AUTH_A: u8 = 0x60;
pub(crate) const MIFARE_AUTH_B: u8 = 0x61;
pub(crate) const MIFARE_READ: u8 = 0x30;
pub(crate) const MIFARE_WRITE: u8 = 0xA0;

pub(crate) const TARGET_NUMBER: u8 = 0x01;
pub(crate) const ISO14443A_106_KBPS: u8 = 0x00;
pub(crate) const MAX_RAW_PAYLOAD: usize = 252;

/// Firmware identity returned by `GetFirmwareVersion`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FirmwareVersion {
    pub ic: u8,
    pub version: u8,
    pub revision: u8,
    pub support: u8,
}

pub(crate) fn parse_firmware(response: &[u8]) -> Result<FirmwareVersion> {
    if response.len() < 4 || response[0] != PN532_IC {
        return Err(NfcError::Unsupported(
            "the connected device is not a supported PN532",
        ));
    }
    if response[3] & ISO14443A_SUPPORT == 0 {
        return Err(NfcError::Unsupported(
            "the PN532 does not report ISO/IEC 14443A support",
        ));
    }
    Ok(FirmwareVersion {
        ic: response[0],
        version: response[1],
        revision: response[2],
        support: response[3],
    })
}

/// Parses the `InListPassiveTarget` body: target count, target number, ATQA,
/// SAK, UID length, and UID.
pub(crate) fn parse_passive_target(response: &[u8], source: &str) -> Result<CardInfo> {
    let Some((&count, targets)) = response.split_first() else {
        return Err(NfcError::Io(format!(
            "{source} card selection response is empty"
        )));
    };
    if count == 0 {
        return Err(NfcError::NoCard);
    }
    if count != 1 {
        return Err(NfcError::Io(format!(
            "{source} selected {count} cards, expected one"
        )));
    }
    if targets.len() < 5 {
        return Err(NfcError::Io(format!(
            "{source} card selection data is truncated"
        )));
    }
    if targets[0] != TARGET_NUMBER {
        return Err(NfcError::Io(format!(
            "{source} returned target number {}, expected {TARGET_NUMBER}",
            targets[0]
        )));
    }
    let atqa = [targets[1], targets[2]];
    let sak = targets[3];
    let uid_length = targets[4] as usize;
    if !(4..=10).contains(&uid_length) {
        return Err(NfcError::Io(format!(
            "{source} reported an invalid UID length {uid_length}"
        )));
    }
    if targets.len() < 5 + uid_length {
        return Err(NfcError::Io(format!(
            "{source} card selection data has no complete {uid_length}-byte UID"
        )));
    }
    let uid = targets[5..5 + uid_length].to_vec();
    Ok(CardInfo::new(uid, atqa, sak, CardType::infer(atqa, sak)))
}

/// Validates the `D5 <command+1>` prefix and returns the payload.
pub(crate) fn parse_response(command: u8, data: &[u8], source: &str) -> Result<Vec<u8>> {
    if data.len() < 2 {
        return Err(NfcError::Io(format!("{source} response is too short")));
    }
    if data[0] != PN532_TFI {
        return Err(NfcError::Io(format!(
            "{source} response direction {:#04X} is invalid",
            data[0]
        )));
    }
    let expected = command.wrapping_add(1);
    if data[1] != expected {
        return Err(NfcError::Io(format!(
            "{source} response code {:#04X} does not match command {:#04X}",
            data[1], command
        )));
    }
    Ok(data[2..].to_vec())
}

pub(crate) fn status_error(status: u8) -> NfcError {
    match status {
        0x01 | 0x0A => NfcError::Timeout("the card did not answer".to_owned()),
        0x14 => NfcError::AuthenticationFailed,
        0x02 => NfcError::Io("PN532 reported a CRC error".to_owned()),
        0x03 => NfcError::Io("PN532 reported a parity error".to_owned()),
        0x04 => NfcError::Io("PN532 reported a bit-count error".to_owned()),
        0x05 => NfcError::Io("PN532 reported a framing error".to_owned()),
        0x06 => NfcError::Io("PN532 reported a collision".to_owned()),
        0x13 => NfcError::Io("PN532 rejected the received frame".to_owned()),
        other => NfcError::Io(format!("PN532 reported status {other:02X}")),
    }
}
