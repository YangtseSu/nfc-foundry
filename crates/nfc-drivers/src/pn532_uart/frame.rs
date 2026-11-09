//! PN532 UART framing: normal and extended information frames, ACK, NACK, and
//! the application error frame.

use nfc_core::{NfcError, Result};

pub(crate) const HOST_TFI: u8 = 0xD4;

/// Host wake-up preamble for the High Speed Unit (HSU).
pub(crate) const WAKEUP_PREAMBLE: [u8; 16] = [
    0x55, 0x55, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const START: [u8; 3] = [0x00, 0x00, 0xFF];
const MAX_PAYLOAD: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    /// The PN532 acknowledged the command and is still processing it.
    Ack,
    /// Direction byte, command code, and command data.
    Data(Vec<u8>),
}

/// Build a host information frame for a command. The command is the PN532
/// command code followed by its parameters; the TFI and checksums are added.
pub(crate) fn build_frame(command: &[u8]) -> Result<Vec<u8>> {
    if command.is_empty() {
        return Err(NfcError::InvalidArgument(
            "PN532 command is empty".to_owned(),
        ));
    }
    // LEN counts the TFI and the command, but not LCS or DCS.
    let length = command.len() + 1;
    let mut frame = Vec::with_capacity(length + 9);
    frame.extend_from_slice(&START);
    if length <= u8::MAX as usize {
        frame.push(length as u8);
        frame.push(0u8.wrapping_sub(length as u8));
    } else {
        let high = (length >> 8) as u8;
        let low = length as u8;
        frame.extend_from_slice(&[0xFF, 0xFF, high, low]);
        frame.push(0u8.wrapping_sub(high.wrapping_add(low)));
    }
    frame.push(HOST_TFI);
    frame.extend_from_slice(command);
    let sum = command.iter().fold(HOST_TFI, |accumulator, byte| {
        accumulator.wrapping_add(*byte)
    });
    frame.push(0u8.wrapping_sub(sum));
    frame.push(0x00);
    Ok(frame)
}

/// Consume one complete frame from the front of `buffer`. Incomplete frames
/// stay buffered; junk before a start code is discarded. Invalid frames are
/// consumed and reported as errors.
pub(crate) fn take_frame(buffer: &mut Vec<u8>) -> Result<Option<Frame>> {
    let Some(start) = find_start(buffer) else {
        // Keep a possible two-byte start-code prefix for the next read.
        if buffer.len() > 2 {
            buffer.drain(..buffer.len() - 2);
        }
        return Ok(None);
    };
    if start > 0 {
        buffer.drain(..start);
    }
    if buffer.len() < 5 {
        return Ok(None);
    }

    let length = buffer[3];
    let length_checksum = buffer[4];

    // The fixed application error frame is 00 00 FF 01 FF 7F 81 00.
    if length == 0x01 && length_checksum == 0xFF {
        if buffer.len() < 8 {
            return Ok(None);
        }
        buffer.drain(..8);
        return Err(NfcError::Io(
            "PN532 reported an application error frame".to_owned(),
        ));
    }

    // 00 00 FF FF 00 00 is a host/PN532 NACK, not an extended frame.
    if length == 0xFF && length_checksum == 0x00 {
        if buffer.len() < 6 {
            return Ok(None);
        }
        buffer.drain(..6);
        return Err(NfcError::Io(
            "PN532 rejected the command with a NACK frame".to_owned(),
        ));
    }

    if length == 0xFF && length_checksum == 0xFF {
        if buffer.len() < 8 {
            return Ok(None);
        }
        let high = buffer[5];
        let low = buffer[6];
        let extended_checksum = buffer[7];
        if high.wrapping_add(low).wrapping_add(extended_checksum) != 0 {
            buffer.drain(..1);
            return Ok(None);
        }
        let payload_length = ((high as usize) << 8) | low as usize;
        if !(2..=MAX_PAYLOAD).contains(&payload_length) {
            buffer.drain(..8);
            return Err(NfcError::Io(format!(
                "PN532 extended frame length {payload_length} is invalid"
            )));
        }
        let total = 8 + payload_length + 2;
        if buffer.len() < total {
            return Ok(None);
        }
        let payload = buffer[8..8 + payload_length].to_vec();
        let data_checksum = buffer[8 + payload_length];
        let postamble = buffer[8 + payload_length + 1];
        validate_payload(&payload, data_checksum, postamble)?;
        buffer.drain(..total);
        return Ok(Some(Frame::Data(payload)));
    }

    // The ACK frame is the one frame whose length checksum is not the
    // two's complement of LEN: 00 00 FF 00 FF 00.
    if length == 0 && length_checksum == 0xFF {
        if buffer.len() < 6 {
            return Ok(None);
        }
        if buffer[5] != 0x00 {
            buffer.drain(..1);
            return Ok(None);
        }
        buffer.drain(..6);
        return Ok(Some(Frame::Ack));
    }

    if length == 0 {
        buffer.drain(..1);
        return Ok(None);
    }

    if length.wrapping_add(length_checksum) != 0 {
        buffer.drain(..1);
        return Ok(None);
    }

    let payload_length = length as usize;
    let total = payload_length + 7;
    if buffer.len() < total {
        return Ok(None);
    }
    let payload = buffer[5..5 + payload_length].to_vec();
    let data_checksum = buffer[5 + payload_length];
    let postamble = buffer[5 + payload_length + 1];
    validate_payload(&payload, data_checksum, postamble)?;
    buffer.drain(..total);
    Ok(Some(Frame::Data(payload)))
}

fn validate_payload(payload: &[u8], data_checksum: u8, postamble: u8) -> Result<()> {
    let sum = payload
        .iter()
        .fold(0u8, |accumulator, byte| accumulator.wrapping_add(*byte));
    if data_checksum != 0u8.wrapping_sub(sum) {
        return Err(NfcError::Io(
            "PN532 frame data checksum mismatch".to_owned(),
        ));
    }
    if postamble != 0x00 {
        return Err(NfcError::Io("PN532 frame postamble mismatch".to_owned()));
    }
    Ok(())
}

fn find_start(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(START.len())
        .position(|window| window == START)
}

#[cfg(test)]
mod tests {
    use crate::pn53x::PN532_TFI;

    use super::*;

    fn response_frame(command: u8, payload: &[u8]) -> Vec<u8> {
        let mut body = vec![PN532_TFI, command.wrapping_add(1)];
        body.extend_from_slice(payload);
        let length = body.len();
        let mut frame = Vec::new();
        frame.extend_from_slice(&START);
        frame.push(length as u8);
        frame.push(0u8.wrapping_sub(length as u8));
        frame.extend_from_slice(&body);
        let sum = body
            .iter()
            .fold(0u8, |accumulator, byte| accumulator.wrapping_add(*byte));
        frame.push(0u8.wrapping_sub(sum));
        frame.push(0x00);
        frame
    }

    #[test]
    fn builds_the_documented_firmware_version_frame() {
        let frame = build_frame(&[0x02]).expect("frame");
        assert_eq!(
            frame,
            vec![0x00, 0x00, 0xFF, 0x02, 0xFE, 0xD4, 0x02, 0x2A, 0x00]
        );
    }

    #[test]
    fn parses_ack_normal_and_extended_frames() {
        let mut buffer = vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00];
        assert_eq!(take_frame(&mut buffer).expect("ack"), Some(Frame::Ack));

        let mut buffer = response_frame(0x02, &[0x32, 0x01, 0x06, 0x07]);
        let parsed = take_frame(&mut buffer).expect("frame").expect("complete");
        assert_eq!(
            parsed,
            Frame::Data(vec![0xD5, 0x03, 0x32, 0x01, 0x06, 0x07])
        );

        let command = vec![0x42; 280];
        let extended = build_frame(&command).expect("extended frame");
        let mut buffer = extended;
        let parsed = take_frame(&mut buffer).expect("frame").expect("complete");
        match parsed {
            Frame::Data(data) => {
                assert_eq!(data[0], HOST_TFI);
                assert_eq!(&data[1..], command.as_slice());
            }
            Frame::Ack => panic!("extended frame parsed as an ACK"),
        }
    }

    #[test]
    fn resynchronizes_after_junk_and_split_frames() {
        let frame = response_frame(0x02, &[0x32, 0x01, 0x06, 0x07]);
        let mut buffer = vec![0xAA, 0x55, 0x00, 0x00];
        assert_eq!(take_frame(&mut buffer).expect("partial"), None);
        assert_eq!(buffer, vec![0x00, 0x00]);

        buffer.extend_from_slice(&frame[2..6]);
        assert_eq!(take_frame(&mut buffer).expect("partial"), None);
        buffer.extend_from_slice(&frame[6..]);
        let parsed = take_frame(&mut buffer).expect("frame").expect("complete");
        assert_eq!(parsed, Frame::Data(frame[5..11].to_vec()));
        assert!(buffer.is_empty());
    }

    #[test]
    fn reports_nack_error_frame_and_bad_checksums() {
        let mut nack = vec![0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00];
        assert!(matches!(take_frame(&mut nack), Err(NfcError::Io(_))));
        assert!(nack.is_empty());

        let mut error = vec![0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7F, 0x81, 0x00];
        assert!(matches!(take_frame(&mut error), Err(NfcError::Io(_))));
        assert!(error.is_empty());

        let mut damaged = response_frame(0x02, &[0x32, 0x01]);
        let last = damaged.len();
        damaged[last - 2] ^= 0xFF;
        assert!(matches!(take_frame(&mut damaged), Err(NfcError::Io(_))));
    }
}
