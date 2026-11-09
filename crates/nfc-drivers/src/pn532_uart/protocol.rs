//! PN532 command layer: initialization, ISO/IEC 14443A selection, MIFARE
//! Classic authentication and block access, and raw frame exchange.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

use nfc_core::{BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, Result};

use super::frame::{Frame, PN532_TFI, WAKEUP_PREAMBLE, build_frame, take_frame};
use super::link::Pn532Link;

const GET_FIRMWARE_VERSION: u8 = 0x02;
const READ_REGISTER: u8 = 0x06;
const WRITE_REGISTER: u8 = 0x08;
const SET_PARAMETERS: u8 = 0x12;
const SAM_CONFIGURATION: u8 = 0x14;
const RF_CONFIGURATION: u8 = 0x32;
const IN_DATA_EXCHANGE: u8 = 0x40;
const IN_COMMUNICATE_THRU: u8 = 0x42;
const IN_LIST_PASSIVE_TARGET: u8 = 0x4A;

const REG_CIU_TX_MODE: u16 = 0x6302;
const REG_CIU_RX_MODE: u16 = 0x6303;
const REG_CIU_MANUAL_RCV: u16 = 0x630D;
const REG_CIU_STATUS2: u16 = 0x6338;
const REG_CIU_CONTROL: u16 = 0x633C;
const REG_CIU_BIT_FRAMING: u16 = 0x633D;

const TX_RX_CRC_ENABLE: u8 = 0x80;
const INITIATOR: u8 = 0x10;
const RX_LAST_BITS: u8 = 0x07;
const AUTO_ATR_RES_AND_RATS: u8 = 0x14;
const PN532_IC: u8 = 0x32;
const ISO14443A_SUPPORT: u8 = 0x01;

const MIFARE_AUTH_A: u8 = 0x60;
const MIFARE_AUTH_B: u8 = 0x61;
const MIFARE_READ: u8 = 0x30;
const MIFARE_WRITE: u8 = 0xA0;

const TARGET_NUMBER: u8 = 0x01;
const ISO14443A_106_KBPS: u8 = 0x00;
const MAX_RAW_PAYLOAD: usize = 252;

#[derive(Clone, Debug)]
pub(crate) struct Pn532Config {
    pub(crate) baud_rate: u32,
    pub(crate) ack_timeout: Duration,
    pub(crate) command_timeout: Duration,
    pub(crate) open_timeout: Duration,
    pub(crate) attempts: usize,
}

impl Default for Pn532Config {
    fn default() -> Self {
        Self {
            baud_rate: 115_200,
            ack_timeout: Duration::from_millis(150),
            command_timeout: Duration::from_millis(1_200),
            open_timeout: Duration::from_millis(1_000),
            attempts: 3,
        }
    }
}

/// Firmware identity returned by `GetFirmwareVersion`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FirmwareVersion {
    pub ic: u8,
    pub version: u8,
    pub revision: u8,
    pub support: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retry {
    Yes,
    No,
}

/// One PN532 session. The link is owned by the protocol until shutdown.
pub(crate) struct Pn532Protocol {
    link: Box<dyn Pn532Link>,
    config: Pn532Config,
    rx: Vec<u8>,
    raw_mode: bool,
    firmware: FirmwareVersion,
    tx_last_bits: u8,
}

impl Pn532Protocol {
    pub(crate) fn open(link: Box<dyn Pn532Link>, config: Pn532Config) -> Result<Self> {
        let mut protocol = Self {
            link,
            config,
            rx: Vec::new(),
            raw_mode: false,
            firmware: FirmwareVersion::default(),
            tx_last_bits: 0,
        };
        protocol.handshake()?;
        Ok(protocol)
    }

    pub(crate) fn shutdown(&mut self) {
        let _ = self.command(
            &[RF_CONFIGURATION, 0x01, 0x00],
            Duration::from_millis(100),
            Retry::No,
        );
    }

    pub(crate) fn firmware(&self) -> FirmwareVersion {
        self.firmware
    }

    pub(crate) fn set_raw_mode(&mut self, enabled: bool) -> Result<()> {
        if self.raw_mode == enabled {
            return Ok(());
        }
        if enabled {
            self.set_parameters(0x00)?;
            self.write_register(REG_CIU_TX_MODE, 0x00)?;
            self.write_register(REG_CIU_RX_MODE, 0x00)?;
        } else {
            self.set_parameters(AUTO_ATR_RES_AND_RATS)?;
            self.write_register(REG_CIU_TX_MODE, TX_RX_CRC_ENABLE)?;
            self.write_register(REG_CIU_RX_MODE, TX_RX_CRC_ENABLE)?;
        }
        self.raw_mode = enabled;
        Ok(())
    }

    pub(crate) fn select_card(&mut self) -> Result<CardInfo> {
        self.require_ordinary("card selection")?;
        self.set_tx_last_bits(0)?;
        let response = self.command(
            &[IN_LIST_PASSIVE_TARGET, 0x01, ISO14443A_106_KBPS],
            self.config.command_timeout,
            Retry::Yes,
        )?;
        let Some((&count, targets)) = response.split_first() else {
            return Err(NfcError::Io(
                "PN532 card selection response is empty".to_owned(),
            ));
        };
        if count == 0 {
            return Err(NfcError::NoCard);
        }
        if count != 1 {
            return Err(NfcError::Io(format!(
                "PN532 selected {count} cards, expected one"
            )));
        }
        if targets.len() < 5 {
            return Err(NfcError::Io(
                "PN532 card selection data is truncated".to_owned(),
            ));
        }
        if targets[0] != TARGET_NUMBER {
            return Err(NfcError::Io(format!(
                "PN532 returned target number {}, expected {TARGET_NUMBER}",
                targets[0]
            )));
        }
        let atqa = [targets[1], targets[2]];
        let sak = targets[3];
        let uid_length = targets[4] as usize;
        if !(4..=10).contains(&uid_length) {
            return Err(NfcError::Io(format!(
                "PN532 reported an invalid UID length {uid_length}"
            )));
        }
        if targets.len() < 5 + uid_length {
            return Err(NfcError::Io(format!(
                "PN532 card selection data has no complete {uid_length}-byte UID"
            )));
        }
        let uid = targets[5..5 + uid_length].to_vec();
        Ok(CardInfo::new(uid, atqa, sak, CardType::infer(atqa, sak)))
    }

    pub(crate) fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()> {
        self.require_ordinary("MIFARE authentication")?;
        let command = match key_type {
            KeyType::A => MIFARE_AUTH_A,
            KeyType::B => MIFARE_AUTH_B,
        };
        let mut payload = Vec::with_capacity(8);
        payload.push(command);
        payload.push(block);
        payload.extend_from_slice(key.as_bytes());
        self.data_exchange(&payload, self.config.command_timeout, Retry::Yes)?;
        Ok(())
    }

    pub(crate) fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        self.require_ordinary("MIFARE block read")?;
        let data = self.data_exchange(
            &[MIFARE_READ, block],
            self.config.command_timeout,
            Retry::Yes,
        )?;
        if data.len() != BLOCK_SIZE {
            return Err(NfcError::Io(format!(
                "PN532 MIFARE read returned {} bytes instead of {BLOCK_SIZE}",
                data.len()
            )));
        }
        let mut output = [0u8; BLOCK_SIZE];
        output.copy_from_slice(&data);
        Ok(output)
    }

    pub(crate) fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.require_ordinary("MIFARE block write")?;
        let mut payload = Vec::with_capacity(2 + BLOCK_SIZE);
        payload.push(MIFARE_WRITE);
        payload.push(block);
        payload.extend_from_slice(&data);
        self.data_exchange(&payload, self.config.command_timeout, Retry::No)?;
        Ok(())
    }

    pub(crate) fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>> {
        self.require_raw("raw byte transceive")?;
        if tx.is_empty() {
            return Err(NfcError::InvalidArgument(
                "raw transceive payload is empty".to_owned(),
            ));
        }
        if tx.len() > MAX_RAW_PAYLOAD {
            return Err(NfcError::InvalidArgument(format!(
                "raw transceive payload is {} bytes, at most {MAX_RAW_PAYLOAD} are supported",
                tx.len()
            )));
        }
        self.set_tx_last_bits(0)?;
        self.in_communicate_thru(tx)
    }

    pub(crate) fn transceive_bits(
        &mut self,
        tx: &[u8],
        tx_bits: usize,
    ) -> Result<(Vec<u8>, usize)> {
        self.require_raw("raw bit transceive")?;
        if tx.is_empty() || tx_bits == 0 {
            return Err(NfcError::InvalidArgument(
                "raw bit transceive needs at least one bit".to_owned(),
            ));
        }
        if tx.len() > MAX_RAW_PAYLOAD {
            return Err(NfcError::InvalidArgument(format!(
                "raw bit transceive payload is {} bytes, at most {MAX_RAW_PAYLOAD} are supported",
                tx.len()
            )));
        }
        if tx_bits > tx.len() * 8 {
            return Err(NfcError::InvalidArgument(format!(
                "raw bit transceive sends {tx_bits} bits from {} bytes",
                tx.len()
            )));
        }
        self.set_tx_last_bits((tx_bits % 8) as u8)?;
        let result = self.in_communicate_thru(tx);
        let reset = self.set_tx_last_bits(0);
        let data = match result {
            Ok(data) => {
                reset?;
                data
            }
            Err(error) => {
                let _ = reset;
                return Err(error);
            }
        };
        let last_bits = self.read_register(REG_CIU_CONTROL)? & RX_LAST_BITS;
        let received_bits = if last_bits == 0 {
            data.len() * 8
        } else {
            if data.is_empty() {
                return Err(NfcError::Io(
                    "PN532 reported a partial byte without data".to_owned(),
                ));
            }
            (data.len() - 1) * 8 + last_bits as usize
        };
        Ok((data, received_bits))
    }

    fn handshake(&mut self) -> Result<()> {
        self.link
            .write_all(&WAKEUP_PREAMBLE)
            .map_err(|error| NfcError::Io(format!("PN532 wake-up failed: {error}")))?;
        thread::sleep(Duration::from_millis(100));
        let firmware = match self
            .command(
                &[SAM_CONFIGURATION, 0x01],
                self.config.open_timeout,
                Retry::Yes,
            )
            .and_then(|_| {
                self.command(
                    &[GET_FIRMWARE_VERSION],
                    self.config.open_timeout,
                    Retry::Yes,
                )
            }) {
            Ok(firmware) => firmware,
            Err(NfcError::Timeout(_) | NfcError::Io(_)) => {
                return Err(NfcError::Unsupported(
                    "the serial port did not respond as a PN532",
                ));
            }
            Err(error) => return Err(error),
        };
        if firmware.len() < 4 || firmware[0] != PN532_IC {
            return Err(NfcError::Unsupported(
                "the connected device is not a supported PN532",
            ));
        }
        if firmware[3] & ISO14443A_SUPPORT == 0 {
            return Err(NfcError::Unsupported(
                "the PN532 does not report ISO/IEC 14443A support",
            ));
        }
        self.firmware = FirmwareVersion {
            ic: firmware[0],
            version: firmware[1],
            revision: firmware[2],
            support: firmware[3],
        };
        self.reset_settings()?;
        Ok(())
    }

    fn reset_settings(&mut self) -> Result<()> {
        self.set_parameters(AUTO_ATR_RES_AND_RATS)?;
        self.command(
            &[RF_CONFIGURATION, 0x00, 0x00, 0x01, 0x00],
            self.config.command_timeout,
            Retry::Yes,
        )?;
        self.write_register(REG_CIU_TX_MODE, TX_RX_CRC_ENABLE)?;
        self.write_register(REG_CIU_RX_MODE, TX_RX_CRC_ENABLE)?;
        self.write_register(REG_CIU_MANUAL_RCV, 0x00)?;
        self.write_register(REG_CIU_STATUS2, 0x00)?;
        self.write_register(REG_CIU_CONTROL, INITIATOR)?;
        self.write_register(REG_CIU_BIT_FRAMING, 0x00)?;
        self.tx_last_bits = 0;
        self.raw_mode = false;
        Ok(())
    }

    fn set_parameters(&mut self, value: u8) -> Result<()> {
        self.command(
            &[SET_PARAMETERS, value],
            self.config.command_timeout,
            Retry::Yes,
        )
        .map(|_| ())
    }

    fn write_register(&mut self, address: u16, value: u8) -> Result<()> {
        let command = [WRITE_REGISTER, (address >> 8) as u8, address as u8, value];
        self.command(&command, self.config.command_timeout, Retry::Yes)
            .map(|_| ())
    }

    fn read_register(&mut self, address: u16) -> Result<u8> {
        let command = [READ_REGISTER, (address >> 8) as u8, address as u8];
        let data = self.command(&command, self.config.command_timeout, Retry::Yes)?;
        data.first()
            .copied()
            .ok_or_else(|| NfcError::Io("PN532 ReadRegister response is empty".to_owned()))
    }

    fn set_tx_last_bits(&mut self, bits: u8) -> Result<()> {
        if bits > 7 {
            return Err(NfcError::InvalidArgument(format!(
                "PN532 cannot send {bits} trailing bits"
            )));
        }
        if self.tx_last_bits == bits {
            return Ok(());
        }
        self.write_register(REG_CIU_BIT_FRAMING, bits)?;
        self.tx_last_bits = bits;
        Ok(())
    }

    fn data_exchange(
        &mut self,
        payload: &[u8],
        timeout: Duration,
        retry: Retry,
    ) -> Result<Vec<u8>> {
        self.require_ordinary("MIFARE data exchange")?;
        self.set_tx_last_bits(0)?;
        let mut command = Vec::with_capacity(payload.len() + 2);
        command.extend_from_slice(&[IN_DATA_EXCHANGE, TARGET_NUMBER]);
        command.extend_from_slice(payload);
        let mut response = self.command(&command, timeout, retry)?;
        let mut output = Vec::new();
        loop {
            let Some((&status, rest)) = response.split_first() else {
                return Err(NfcError::Io(
                    "PN532 InDataExchange response is empty".to_owned(),
                ));
            };
            let code = status & 0x3F;
            if code != 0 {
                return Err(status_error(code));
            }
            output.extend_from_slice(rest);
            if status & 0x40 == 0 {
                break;
            }
            response = self.command(&[IN_DATA_EXCHANGE, TARGET_NUMBER], timeout, retry)?;
        }
        Ok(output)
    }

    fn in_communicate_thru(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let mut command = Vec::with_capacity(data.len() + 1);
        command.push(IN_COMMUNICATE_THRU);
        command.extend_from_slice(data);
        let mut response = self.command(&command, self.config.command_timeout, Retry::No)?;
        let mut output = Vec::new();
        loop {
            let Some((&status, rest)) = response.split_first() else {
                return Err(NfcError::Io(
                    "PN532 InCommunicateThru response is empty".to_owned(),
                ));
            };
            let code = status & 0x3F;
            if code != 0 {
                return Err(status_error(code));
            }
            output.extend_from_slice(rest);
            if status & 0x40 == 0 {
                break;
            }
            response = self.command(
                &[IN_COMMUNICATE_THRU],
                self.config.command_timeout,
                Retry::No,
            )?;
        }
        Ok(output)
    }

    fn require_ordinary(&self, operation: &'static str) -> Result<()> {
        if self.raw_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} is unavailable in raw mode"
            )));
        }
        Ok(())
    }

    fn require_raw(&self, operation: &'static str) -> Result<()> {
        if !self.raw_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} requires raw mode"
            )));
        }
        Ok(())
    }

    fn command(&mut self, command: &[u8], timeout: Duration, retry: Retry) -> Result<Vec<u8>> {
        let frame = build_frame(command)?;
        let attempts = if retry == Retry::Yes {
            self.config.attempts.max(1)
        } else {
            1
        };
        let mut last_error = None;
        for _ in 0..attempts {
            self.rx.clear();
            self.link
                .flush_input()
                .map_err(|error| NfcError::Io(format!("PN532 serial flush failed: {error}")))?;
            self.link
                .write_all(&frame)
                .map_err(|error| NfcError::Io(format!("PN532 serial write failed: {error}")))?;
            match self.read_frame(self.config.ack_timeout, "acknowledgement") {
                Ok(Frame::Ack) => {}
                Ok(Frame::Data(_)) => {
                    return Err(NfcError::Io(
                        "PN532 sent a response before acknowledging the command".to_owned(),
                    ));
                }
                Err(NfcError::Timeout(_)) => {
                    last_error = Some(NfcError::Timeout("PN532 acknowledgement".to_owned()));
                    continue;
                }
                Err(error) => return Err(error),
            }
            match self.read_frame(timeout, "response") {
                Ok(Frame::Data(data)) => {
                    return parse_response(command[0], &data);
                }
                Ok(Frame::Ack) => {
                    let error = NfcError::Io("PN532 acknowledged the command twice".to_owned());
                    if retry == Retry::No {
                        return Err(error);
                    }
                    last_error = Some(error);
                    continue;
                }
                Err(NfcError::Timeout(_)) => {
                    last_error = Some(NfcError::Timeout("PN532 response".to_owned()));
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| NfcError::Io("PN532 command failed".to_owned())))
    }

    fn read_frame(&mut self, timeout: Duration, what: &'static str) -> Result<Frame> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = take_frame(&mut self.rx)? {
                return Ok(frame);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.rx.clear();
                return Err(NfcError::Timeout(format!("PN532 {what}")));
            }
            self.link.set_timeout(remaining).map_err(|error| {
                NfcError::Io(format!("PN532 serial timeout setup failed: {error}"))
            })?;
            let mut buffer = [0u8; 128];
            match self.link.read(&mut buffer) {
                Ok(0) => thread::sleep(Duration::from_millis(1)),
                Ok(count) => self.rx.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                    self.rx.clear();
                    return Err(NfcError::Timeout(format!("PN532 {what}")));
                }
                Err(error) => {
                    return Err(NfcError::Io(format!("PN532 serial read failed: {error}")));
                }
            }
        }
    }
}

fn parse_response(command: u8, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 2 {
        return Err(NfcError::Io("PN532 response is too short".to_owned()));
    }
    if data[0] != PN532_TFI {
        return Err(NfcError::Io(format!(
            "PN532 response direction {:#04X} is invalid",
            data[0]
        )));
    }
    let expected = command.wrapping_add(1);
    if data[1] != expected {
        return Err(NfcError::Io(format!(
            "PN532 response code {:#04X} does not match command {:#04X}",
            data[1], command
        )));
    }
    Ok(data[2..].to_vec())
}

fn status_error(status: u8) -> NfcError {
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
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use super::*;

    const ACK: [u8; 6] = [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00];

    #[derive(Debug)]
    enum ReadStep {
        Bytes(Vec<u8>),
        Timeout,
        Disconnect,
    }

    #[derive(Default)]
    struct ScriptLog {
        writes: Mutex<Vec<Vec<u8>>>,
    }

    impl ScriptLog {
        fn lock(&self) -> MutexGuard<'_, Vec<Vec<u8>>> {
            self.writes.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn writes(&self) -> Vec<Vec<u8>> {
            self.lock().clone()
        }

        fn commands(&self) -> Vec<Vec<u8>> {
            self.writes()
                .into_iter()
                .filter_map(|frame| {
                    if !frame.starts_with(&[0x00, 0x00, 0xFF]) || frame.len() < 8 {
                        return None;
                    }
                    let length = frame[3] as usize;
                    if length == 0 || frame.len() < 5 + length + 2 {
                        return None;
                    }
                    Some(frame[6..5 + length].to_vec())
                })
                .collect()
        }

        fn host_frames(&self) -> usize {
            self.writes()
                .into_iter()
                .filter(|frame| frame.starts_with(&[0x00, 0x00, 0xFF]))
                .count()
        }
    }

    #[derive(Clone)]
    struct ScriptHandle {
        reads: Arc<Mutex<VecDeque<ReadStep>>>,
        log: Arc<ScriptLog>,
    }

    impl ScriptHandle {
        fn lock_reads(&self) -> MutexGuard<'_, VecDeque<ReadStep>> {
            self.reads.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn ack(&self) {
            self.lock_reads().push_back(ReadStep::Bytes(ACK.to_vec()));
        }

        fn command_response(&self, command: u8, payload: &[u8]) {
            self.ack();
            self.lock_reads()
                .push_back(ReadStep::Bytes(response_frame(command, payload)));
        }

        fn timeout(&self) {
            self.lock_reads().push_back(ReadStep::Timeout);
        }

        fn disconnect(&self) {
            self.lock_reads().push_back(ReadStep::Disconnect);
        }

        fn commands(&self) -> Vec<Vec<u8>> {
            self.log.commands()
        }

        fn writes(&self) -> Vec<Vec<u8>> {
            self.log.writes()
        }

        fn host_frames(&self) -> usize {
            self.log.host_frames()
        }
    }

    struct ScriptedLink {
        reads: Arc<Mutex<VecDeque<ReadStep>>>,
        log: Arc<ScriptLog>,
    }

    impl ScriptedLink {
        fn scripted() -> (Box<dyn Pn532Link>, ScriptHandle) {
            let reads = Arc::new(Mutex::new(VecDeque::new()));
            let log = Arc::new(ScriptLog::default());
            let handle = ScriptHandle {
                reads: Arc::clone(&reads),
                log: Arc::clone(&log),
            };
            let link = Self { reads, log };
            (Box::new(link), handle)
        }
    }

    fn response_frame(command: u8, payload: &[u8]) -> Vec<u8> {
        let mut body = vec![PN532_TFI, command.wrapping_add(1)];
        body.extend_from_slice(payload);
        let length = body.len();
        let mut frame = vec![0x00, 0x00, 0xFF, length as u8];
        frame.push(0u8.wrapping_sub(length as u8));
        frame.extend_from_slice(&body);
        let sum = body
            .iter()
            .fold(0u8, |accumulator, byte| accumulator.wrapping_add(*byte));
        frame.push(0u8.wrapping_sub(sum));
        frame.push(0x00);
        frame
    }

    impl Pn532Link for ScriptedLink {
        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.log.lock().push(bytes.to_vec());
            Ok(())
        }

        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let step = self
                .reads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            match step {
                Some(ReadStep::Bytes(bytes)) => {
                    assert!(
                        bytes.len() <= buffer.len(),
                        "scripted chunk is larger than the read buffer"
                    );
                    buffer[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Some(ReadStep::Timeout) => {
                    Err(io::Error::new(io::ErrorKind::TimedOut, "scripted timeout"))
                }
                Some(ReadStep::Disconnect) => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "scripted disconnect",
                )),
                None => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "script has no more responses",
                )),
            }
        }

        fn set_timeout(&mut self, _timeout: Duration) -> io::Result<()> {
            Ok(())
        }

        fn flush_input(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn config() -> Pn532Config {
        Pn532Config {
            ack_timeout: Duration::from_millis(5),
            command_timeout: Duration::from_millis(5),
            open_timeout: Duration::from_millis(5),
            attempts: 2,
            ..Pn532Config::default()
        }
    }

    fn test_protocol(link: Box<dyn Pn532Link>) -> Pn532Protocol {
        Pn532Protocol {
            link,
            config: config(),
            rx: Vec::new(),
            raw_mode: false,
            firmware: FirmwareVersion {
                ic: PN532_IC,
                version: 0x01,
                revision: 0x06,
                support: ISO14443A_SUPPORT,
            },
            tx_last_bits: 0,
        }
    }

    fn classic_card_response() -> Vec<u8> {
        vec![
            0x01,
            TARGET_NUMBER,
            0x00,
            0x04,
            0x08,
            0x04,
            0xDE,
            0xAD,
            0xBE,
            0xEF,
        ]
    }

    #[test]
    fn opens_with_wakeup_firmware_check_and_configuration() {
        let (link, script) = ScriptedLink::scripted();
        script.command_response(SAM_CONFIGURATION, &[]);
        script.command_response(
            GET_FIRMWARE_VERSION,
            &[PN532_IC, 0x01, 0x06, ISO14443A_SUPPORT],
        );
        script.command_response(SET_PARAMETERS, &[]);
        script.command_response(RF_CONFIGURATION, &[]);
        for _ in 0..6 {
            script.command_response(WRITE_REGISTER, &[]);
        }
        let protocol = Pn532Protocol::open(link, config()).expect("open");
        assert_eq!(
            protocol.firmware(),
            FirmwareVersion {
                ic: PN532_IC,
                version: 0x01,
                revision: 0x06,
                support: ISO14443A_SUPPORT,
            }
        );
        assert_eq!(script.writes()[0], WAKEUP_PREAMBLE);
        assert_eq!(
            script.commands(),
            vec![
                vec![SAM_CONFIGURATION, 0x01],
                vec![GET_FIRMWARE_VERSION],
                vec![SET_PARAMETERS, AUTO_ATR_RES_AND_RATS],
                vec![RF_CONFIGURATION, 0x00, 0x00, 0x01, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x02, TX_RX_CRC_ENABLE],
                vec![WRITE_REGISTER, 0x63, 0x03, TX_RX_CRC_ENABLE],
                vec![WRITE_REGISTER, 0x63, 0x0D, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x38, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x3C, INITIATOR],
                vec![WRITE_REGISTER, 0x63, 0x3D, 0x00],
            ]
        );
    }

    #[test]
    fn rejects_a_device_that_is_not_a_pn532() {
        let (link, script) = ScriptedLink::scripted();
        script.command_response(SAM_CONFIGURATION, &[]);
        script.command_response(GET_FIRMWARE_VERSION, &[0x33, 0x01, 0x06, ISO14443A_SUPPORT]);
        assert!(matches!(
            Pn532Protocol::open(link, config()),
            Err(NfcError::Unsupported(_))
        ));

        let (link, _script) = ScriptedLink::scripted();
        assert!(matches!(
            Pn532Protocol::open(link, config()),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[test]
    fn selects_classic_and_type2_cards() {
        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_LIST_PASSIVE_TARGET, &classic_card_response());
        let mut protocol = test_protocol(link);
        let card = protocol.select_card().expect("classic card");
        assert_eq!(card.uid, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(card.atqa, [0x00, 0x04]);
        assert_eq!(card.sak, 0x08);
        assert_eq!(card.card_type, CardType::MifareClassic1K);
        assert_eq!(
            script.commands(),
            vec![vec![IN_LIST_PASSIVE_TARGET, 0x01, ISO14443A_106_KBPS]]
        );

        let (link, script) = ScriptedLink::scripted();
        let mut response = vec![0x01, TARGET_NUMBER, 0x00, 0x44, 0x00, 0x07];
        response.extend_from_slice(&[0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80]);
        script.command_response(IN_LIST_PASSIVE_TARGET, &response);
        let mut protocol = test_protocol(link);
        let card = protocol.select_card().expect("type 2 card");
        assert_eq!(card.uid.len(), 7);
        assert_eq!(card.card_type, CardType::Type2);
    }

    #[test]
    fn reports_missing_and_malformed_card_selections() {
        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_LIST_PASSIVE_TARGET, &[0x00]);
        let mut protocol = test_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::NoCard)));

        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_LIST_PASSIVE_TARGET, &[0x01, 0x01, 0x00]);
        let mut protocol = test_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::Io(_))));
    }

    #[test]
    fn authenticates_with_the_requested_key_and_maps_failure() {
        let key = Key::new([0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);
        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_DATA_EXCHANGE, &[0x00]);
        let mut protocol = test_protocol(link);
        protocol
            .authenticate(4, KeyType::B, key)
            .expect("authentication");
        assert_eq!(
            script.commands(),
            vec![vec![
                IN_DATA_EXCHANGE,
                TARGET_NUMBER,
                MIFARE_AUTH_B,
                0x04,
                0xA0,
                0xA1,
                0xA2,
                0xA3,
                0xA4,
                0xA5,
            ]]
        );

        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_DATA_EXCHANGE, &[0x14]);
        let mut protocol = test_protocol(link);
        assert!(matches!(
            protocol.authenticate(0, KeyType::A, key),
            Err(NfcError::AuthenticationFailed)
        ));
    }

    #[test]
    fn reads_and_writes_classic_blocks() {
        let block = [0x5A; BLOCK_SIZE];
        let (link, script) = ScriptedLink::scripted();
        let mut payload = vec![0x00];
        payload.extend_from_slice(&block);
        script.command_response(IN_DATA_EXCHANGE, &payload);
        let mut protocol = test_protocol(link);
        assert_eq!(protocol.read_block(4).expect("read"), block);
        assert_eq!(
            script.commands(),
            vec![vec![IN_DATA_EXCHANGE, TARGET_NUMBER, MIFARE_READ, 0x04]]
        );

        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_DATA_EXCHANGE, &[0x00]);
        let mut protocol = test_protocol(link);
        protocol.write_block(4, block).expect("write");
        let mut write = vec![IN_DATA_EXCHANGE, TARGET_NUMBER, MIFARE_WRITE, 0x04];
        write.extend_from_slice(&block);
        assert_eq!(script.commands(), vec![write]);
    }

    #[test]
    fn reports_short_reads_and_card_timeouts() {
        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_DATA_EXCHANGE, &[0x00, 0x01, 0x02]);
        let mut protocol = test_protocol(link);
        assert!(matches!(protocol.read_block(4), Err(NfcError::Io(_))));

        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_DATA_EXCHANGE, &[0x01]);
        let mut protocol = test_protocol(link);
        assert!(matches!(protocol.read_block(4), Err(NfcError::Timeout(_))));
    }

    #[test]
    fn retries_acknowledgement_timeouts_only_for_retryable_commands() {
        let (link, script) = ScriptedLink::scripted();
        script.timeout();
        script.command_response(IN_LIST_PASSIVE_TARGET, &classic_card_response());
        let mut protocol = test_protocol(link);
        protocol.select_card().expect("card after retry");
        assert_eq!(script.host_frames(), 2);

        let (link, script) = ScriptedLink::scripted();
        script.timeout();
        let mut protocol = test_protocol(link);
        assert!(matches!(
            protocol.write_block(4, [0x00; BLOCK_SIZE]),
            Err(NfcError::Timeout(_))
        ));
        assert_eq!(script.host_frames(), 1);
    }

    #[test]
    fn reports_disconnects() {
        let (link, script) = ScriptedLink::scripted();
        script.disconnect();
        let mut protocol = test_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::Io(_))));
    }

    #[test]
    fn raw_mode_controls_crc_and_raw_exchange() {
        let (link, script) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);
        assert!(matches!(
            protocol.transceive_bytes(&[0x26]),
            Err(NfcError::InvalidArgument(_))
        ));

        script.command_response(SET_PARAMETERS, &[]);
        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(WRITE_REGISTER, &[]);
        protocol.set_raw_mode(true).expect("raw mode");
        assert_eq!(
            script.commands(),
            vec![
                vec![SET_PARAMETERS, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x02, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x03, 0x00],
            ]
        );

        assert!(matches!(
            protocol.select_card(),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            protocol.transceive_bytes(&[]),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            protocol.transceive_bits(&[0x26], 9),
            Err(NfcError::InvalidArgument(_))
        ));

        script.command_response(IN_COMMUNICATE_THRU, &[0x00, 0x04, 0x00]);
        assert_eq!(
            protocol.transceive_bytes(&[0x26]).expect("raw bytes"),
            vec![0x04, 0x00]
        );

        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(IN_COMMUNICATE_THRU, &[0x00, 0x0A]);
        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(READ_REGISTER, &[0x00]);
        assert_eq!(
            protocol.transceive_bits(&[0x26], 7).expect("raw bits"),
            (vec![0x0A], 8)
        );

        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(IN_COMMUNICATE_THRU, &[0x00, 0x0A, 0xF0]);
        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(READ_REGISTER, &[0x04]);
        assert_eq!(
            protocol.transceive_bits(&[0x26], 7).expect("raw bits"),
            (vec![0x0A, 0xF0], 12)
        );

        script.command_response(SET_PARAMETERS, &[]);
        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(WRITE_REGISTER, &[]);
        protocol.set_raw_mode(false).expect("ordinary mode");
        assert!(matches!(protocol.select_card(), Err(NfcError::Timeout(_))));
        let commands = script.commands();
        assert_eq!(
            &commands[commands.len() - 5..commands.len() - 2],
            &[
                vec![SET_PARAMETERS, AUTO_ATR_RES_AND_RATS],
                vec![WRITE_REGISTER, 0x63, 0x02, TX_RX_CRC_ENABLE],
                vec![WRITE_REGISTER, 0x63, 0x03, TX_RX_CRC_ENABLE],
            ]
        );
    }
}
