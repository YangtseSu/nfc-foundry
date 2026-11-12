//! PN53x command values, register addresses, and response parsing shared by
//! the UART and ACR122U transports. The chip protocol is the same; only the
//! host frame carrying it differs.

use nfc_core::{CardInfo, CardType, NfcError, Result};
use nfc_transport::ApduResponse;

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
pub(crate) const PARITY_DISABLE: u8 = 0x10;
pub(crate) const AUTO_ATR_RES_AND_RATS: u8 = 0x14;
pub(crate) const PN532_IC: u8 = 0x32;
pub(crate) const ISO14443A_SUPPORT: u8 = 0x01;

pub(crate) const MIFARE_AUTH_A: u8 = 0x60;
pub(crate) const MIFARE_AUTH_B: u8 = 0x61;
pub(crate) const MIFARE_READ: u8 = 0x30;
pub(crate) const MIFARE_WRITE: u8 = 0xA0;

pub(crate) const TYPE2_READ: u8 = 0x30;
pub(crate) const TYPE2_WRITE: u8 = 0xA2;
pub(crate) const TYPE2_PWD_AUTH: u8 = 0x1B;
pub(crate) const TYPE2_GET_VERSION: u8 = 0x60;
pub(crate) const TYPE2_ACK: u8 = 0x0A;

pub(crate) const TARGET_NUMBER: u8 = 0x01;
pub(crate) const ISO14443A_106_KBPS: u8 = 0x00;
/// Bounded passive-activation retries. The chip default is infinite and zero
/// disables retries; phone emulations need more than one activation attempt.
pub(crate) const PASSIVE_ACTIVATION_RETRIES: u8 = 0x0A;
pub(crate) const MAX_RAW_PAYLOAD: usize = 252;
/// Largest complete command APDU carried by one exchange. The ACR122U
/// pseudo-APDU is limited to 255 bytes, and its `D4 40 01` wrapper leaves 247
/// bytes for the APDU. The PN532 UART path is capped at the same value so the
/// Type 4 adapter uses one chunk size on both readers.
pub(crate) const MAX_ISO_DEP_APDU: usize = 247;

/// Firmware identity returned by `GetFirmwareVersion`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FirmwareVersion {
    pub ic: u8,
    pub version: u8,
    pub revision: u8,
    pub support: u8,
}

impl FirmwareVersion {
    /// Diagnostic label shared by the PN532 UART and ACR122U paths. The ACR122U
    /// exposes the identity of its internal PN532.
    pub fn label(&self) -> String {
        format!(
            "PN532 IC {:02X} · firmware {}.{} · support {:02X}",
            self.ic, self.version, self.revision, self.support
        )
    }
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

/// Builds the `InListPassiveTarget` `InitiatorData` for a known ISO/IEC
/// 14443A UID. The PN532 expects the cascade tag `0x88` in front of each
/// cascaded level, matching libnfc's `iso14443_cascade_uid`.
pub(crate) fn cascade_uid(uid: &[u8]) -> Vec<u8> {
    match uid.len() {
        7 => {
            let mut data = Vec::with_capacity(8);
            data.push(0x88);
            data.extend_from_slice(uid);
            data
        }
        10 => {
            let mut data = Vec::with_capacity(12);
            data.push(0x88);
            data.extend_from_slice(&uid[..3]);
            data.push(0x88);
            data.extend_from_slice(&uid[3..]);
            data
        }
        _ => uid.to_vec(),
    }
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

/// Split a complete ISO 7816-4 response APDU into its response data and status
/// word. APDUs without a status word are protocol errors, not empty successes.
pub(crate) fn split_apdu_response(mut data: Vec<u8>, source: &str) -> Result<ApduResponse> {
    if data.len() < 2 {
        return Err(NfcError::Io(format!(
            "{source} returned an APDU response without a status word"
        )));
    }
    let sw = [data[data.len() - 2], data[data.len() - 1]];
    data.truncate(data.len() - 2);
    Ok(ApduResponse::new(data, sw))
}

/// Packs data bytes and caller-supplied parity into the PN53x parity-disabled
/// FIFO layout. Returns the frame bytes and the exact transmitted bit count.
#[cfg(test)]
pub(crate) fn wrap_parity_frame(data: &[u8], parity: &[u8]) -> Result<(Vec<u8>, usize)> {
    wrap_parity_frame_bits(data, data.len() * 8, parity)
}

/// Wraps a frame with caller parity. `data_bits` may be smaller than
/// `data.len() * 8` for 7-bit ISO/IEC 14443A short frames such as WUPA.
pub(crate) fn wrap_parity_frame_bits(
    data: &[u8],
    data_bits: usize,
    parity: &[u8],
) -> Result<(Vec<u8>, usize)> {
    if data.is_empty() {
        return Err(NfcError::InvalidArgument(
            "parity frame has no data".to_owned(),
        ));
    }
    if data_bits == 0 || data_bits > data.len() * 8 {
        return Err(NfcError::InvalidArgument(format!(
            "parity frame has {data_bits} bits in {} bytes",
            data.len()
        )));
    }
    if data.len() != parity.len() {
        return Err(NfcError::InvalidArgument(
            "parity frame needs one parity bit per data byte".to_owned(),
        ));
    }
    let frame_bits = data_bits + data.len();
    if data_bits < 9 {
        return Ok((data.to_vec(), data_bits));
    }

    let mut frame = vec![0u8; frame_bits.div_ceil(8)];
    let mut frame_index = 0;
    let mut data_index = 0;
    let mut bits_left = data_bits;
    loop {
        let mut frame_byte = 0u8;
        for bit_position in 0..8 {
            let mirrored = data[data_index].reverse_bits();
            frame_byte |= mirrored >> bit_position;
            frame[frame_index] = frame_byte.reverse_bits();

            frame_byte = if bit_position == 0 {
                0
            } else {
                mirrored << (8 - bit_position)
            };
            frame_byte |= (parity[data_index] & 1) << (7 - bit_position);
            frame_index += 1;
            frame[frame_index] = frame_byte.reverse_bits();

            data_index += 1;
            if bits_left < 9 {
                return Ok((frame, frame_bits));
            }
            bits_left -= 8;
        }
        frame_index += 1;
    }
}

/// Reverses [`wrap_parity_frame`]: returns data bits, parity bits, and the
/// number of data bits in a possibly partial final byte.
pub(crate) fn unwrap_parity_frame(
    frame: &[u8],
    frame_bits: usize,
) -> Result<(Vec<u8>, usize, Vec<u8>)> {
    if frame.is_empty() || frame_bits == 0 {
        return Err(NfcError::InvalidArgument(
            "parity response has no bits".to_owned(),
        ));
    }
    if frame_bits > frame.len() * 8 {
        return Err(NfcError::InvalidArgument(format!(
            "parity response has {frame_bits} bits in {} bytes",
            frame.len()
        )));
    }
    if frame_bits < 9 {
        return Ok((frame[..1].to_vec(), frame_bits, Vec::new()));
    }

    let data_bits = frame_bits - frame_bits / 9;
    if !data_bits.is_multiple_of(8) {
        return Err(NfcError::Unsupported(
            "parity response is not a whole number of data bytes",
        ));
    }
    let data_bytes = data_bits / 8;
    let mut data = Vec::with_capacity(data_bytes);
    let mut parity = Vec::with_capacity(data_bytes);
    let mut frame_base = 0;
    while data.len() < data_bytes {
        let output_position = data.len();
        let bit_position = output_position % 8;
        let index = frame_base + output_position;
        let first = frame
            .get(index)
            .copied()
            .ok_or_else(|| NfcError::Io("parity response frame is truncated".to_owned()))?
            .reverse_bits();
        let second = frame
            .get(index + 1)
            .copied()
            .ok_or_else(|| NfcError::Io("parity response frame is truncated".to_owned()))?
            .reverse_bits();
        let mut value = first << bit_position;
        if bit_position != 0 {
            value |= second >> (8 - bit_position);
        }
        data.push(value.reverse_bits());
        parity.push((second >> (7 - bit_position)) & 1);
        if bit_position == 7 {
            frame_base += 1;
        }
    }
    Ok((data, data_bits, parity))
}

/// ISO/IEC 14443A CRC-A. Type 2 commands carry it because raw mode disables
/// the PN532 hardware CRC.
pub(crate) fn crc_a(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0x6363;
    for &byte in data {
        let mut mixed = byte ^ crc as u8;
        mixed ^= mixed << 4;
        crc = (crc >> 8) ^ ((mixed as u16) << 8) ^ ((mixed as u16) << 3) ^ ((mixed as u16) >> 4);
    }
    [crc as u8, (crc >> 8) as u8]
}

/// Appends CRC-A to one Type 2 command.
pub(crate) fn type2_command(command: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(command.len() + 2);
    frame.extend_from_slice(command);
    frame.extend_from_slice(&crc_a(command));
    frame
}

/// Checks and strips CRC-A from a Type 2 response. Some reader stacks already
/// remove the received CRC, so an exact-length payload without CRC is accepted.
pub(crate) fn type2_response(data: &[u8], expected: usize, source: &str) -> Result<Vec<u8>> {
    if data.len() == expected {
        return Ok(data.to_vec());
    }
    if data.len() != expected + 2 {
        return Err(NfcError::Io(format!(
            "{source} Type 2 response returned {} bytes, expected {expected} plus CRC",
            data.len()
        )));
    }
    let (payload, crc) = data.split_at(expected);
    if crc_a(payload) != [crc[0], crc[1]] {
        return Err(NfcError::Io(format!(
            "{source} Type 2 response has an invalid CRC"
        )));
    }
    Ok(payload.to_vec())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity_frame_round_trips_four_bytes() {
        let data = [0x60, 0x04, 0xD3, 0xF7];
        let parity = [1, 0, 1, 1];
        let (frame, frame_bits) = wrap_parity_frame(&data, &parity).expect("wrap");
        assert_eq!(frame_bits, 36);
        assert_eq!(frame.len(), 5);
        let (decoded, bits, decoded_parity) =
            unwrap_parity_frame(&frame, frame_bits).expect("unwrap");
        assert_eq!(decoded, data);
        assert_eq!(bits, 32);
        assert_eq!(decoded_parity, parity);
    }

    #[test]
    fn parity_frame_round_trips_eight_bytes() {
        let data = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        let parity = [0, 1, 0, 1, 1, 0, 0, 1];
        let (frame, frame_bits) = wrap_parity_frame(&data, &parity).expect("wrap");
        assert_eq!(frame_bits, 72);
        assert_eq!(frame.len(), 9);
        let (decoded, bits, decoded_parity) =
            unwrap_parity_frame(&frame, frame_bits).expect("unwrap");
        assert_eq!(decoded, data);
        assert_eq!(bits, 64);
        assert_eq!(decoded_parity, parity);
    }

    #[test]
    fn parity_frame_rejects_mismatched_parity() {
        assert!(wrap_parity_frame(&[1, 2], &[1]).is_err());
        assert!(wrap_parity_frame(&[], &[]).is_err());
        assert!(unwrap_parity_frame(&[], 0).is_err());
    }

    #[test]
    fn short_wupa_frame_keeps_seven_bits() {
        let (frame, bits) = wrap_parity_frame_bits(&[0x52], 7, &[0]).expect("wrap");
        assert_eq!(frame, vec![0x52]);
        assert_eq!(bits, 7);
    }

    #[test]
    fn partial_frame_without_parity_is_returned_verbatim() {
        let (data, bits, parity) = unwrap_parity_frame(&[0x05], 4).expect("unwrap");
        assert_eq!(data, vec![0x05]);
        assert_eq!(bits, 4);
        assert!(parity.is_empty());
    }

    #[test]
    fn cascade_uid_adds_a_tag_for_each_cascaded_level() {
        assert_eq!(
            cascade_uid(&[0x04, 0x11, 0x22, 0x33]),
            vec![0x04, 0x11, 0x22, 0x33]
        );
        assert_eq!(
            cascade_uid(&[0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            vec![0x88, 0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]
        );
        assert_eq!(
            cascade_uid(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
            vec![0x88, 1, 2, 3, 0x88, 4, 5, 6, 7, 8, 9, 10]
        );
    }
}
