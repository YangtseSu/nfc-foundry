//! PN532 command layer: initialization, ISO/IEC 14443A selection, MIFARE
//! Classic authentication and block access, and raw frame exchange.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

use nfc_core::{
    BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result,
    TYPE2_VERSION_SIZE,
};
use nfc_transport::{ApduResponse, ParityFrame};

use super::frame::{Frame, WAKEUP_PREAMBLE, build_frame, take_frame};
use super::link::Pn532Link;
use crate::gen1a::{self, Gen1aLink};
use crate::pn53x::{
    AUTO_ATR_RES_AND_RATS, FirmwareVersion, GET_FIRMWARE_VERSION, IN_COMMUNICATE_THRU,
    IN_DATA_EXCHANGE, IN_LIST_PASSIVE_TARGET, INITIATOR, ISO14443A_106_KBPS, MAX_ISO_DEP_APDU,
    MAX_RAW_PAYLOAD, MIFARE_AUTH_A, MIFARE_AUTH_B, MIFARE_READ, MIFARE_WRITE, PARITY_DISABLE,
    PASSIVE_ACTIVATION_RETRIES, READ_REGISTER, REG_CIU_BIT_FRAMING, REG_CIU_CONTROL,
    REG_CIU_MANUAL_RCV, REG_CIU_RX_MODE, REG_CIU_STATUS2, REG_CIU_TX_MODE, RF_CONFIGURATION,
    RF_FIELD, RF_RETRY_SELECT, RX_LAST_BITS, SAM_CONFIGURATION, SET_PARAMETERS, TARGET_NUMBER,
    TX_RX_CRC_ENABLE, TYPE2_ACK, TYPE2_GET_VERSION, TYPE2_PWD_AUTH, TYPE2_READ, TYPE2_WRITE,
    WRITE_REGISTER, cascade_uid, parse_firmware, parse_passive_target, parse_response,
    split_apdu_response, status_error, type2_command, type2_response, unwrap_parity_frame,
    wrap_parity_frame_bits,
};

#[cfg(test)]
use crate::pn53x::wrap_parity_frame;

#[cfg(test)]
use crate::pn53x::{ISO14443A_SUPPORT, PN532_IC};

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
    recovery_mode: bool,
    firmware: FirmwareVersion,
    tx_last_bits: u8,
    selected: Option<CardInfo>,
}

impl Pn532Protocol {
    pub(crate) fn open(link: Box<dyn Pn532Link>, config: Pn532Config) -> Result<Self> {
        let mut protocol = Self {
            link,
            config,
            rx: Vec::new(),
            raw_mode: false,
            recovery_mode: false,
            firmware: FirmwareVersion::default(),
            tx_last_bits: 0,
            selected: None,
        };
        protocol.handshake()?;
        Ok(protocol)
    }

    pub(crate) fn shutdown(&mut self) {
        let _ = self.command(
            &[RF_CONFIGURATION, RF_FIELD, 0x00],
            Duration::from_millis(100),
            Retry::No,
        );
    }

    pub(crate) fn firmware(&self) -> FirmwareVersion {
        self.firmware
    }

    /// Power-cycle the RF field so an already selected card returns to idle.
    pub(crate) fn cycle_rf_field(&mut self) -> Result<()> {
        self.command(
            &[RF_CONFIGURATION, RF_FIELD, 0x00],
            self.config.command_timeout,
            Retry::Yes,
        )?;
        self.command(
            &[RF_CONFIGURATION, RF_FIELD, 0x01],
            self.config.command_timeout,
            Retry::Yes,
        )?;
        thread::sleep(Duration::from_millis(10));
        self.selected = None;
        Ok(())
    }

    pub(crate) fn set_recovery_mode(&mut self, enabled: bool) -> Result<()> {
        if self.recovery_mode == enabled {
            return Ok(());
        }
        if enabled {
            if self.raw_mode {
                self.set_raw_mode(false)?;
            }
            self.set_tx_last_bits(0)?;
            self.set_parameters(0x00)?;
            self.write_register(REG_CIU_TX_MODE, 0x00)?;
            self.write_register(REG_CIU_RX_MODE, 0x00)?;
            self.write_register(REG_CIU_MANUAL_RCV, PARITY_DISABLE)?;
            self.write_register(REG_CIU_STATUS2, 0x00)?;
        } else {
            self.set_parameters(AUTO_ATR_RES_AND_RATS)?;
            self.write_register(REG_CIU_TX_MODE, TX_RX_CRC_ENABLE)?;
            self.write_register(REG_CIU_RX_MODE, TX_RX_CRC_ENABLE)?;
            self.write_register(REG_CIU_MANUAL_RCV, 0x00)?;
        }
        self.recovery_mode = enabled;
        Ok(())
    }

    pub(crate) fn transceive_parity(&mut self, data: &[u8], parity: &[u8]) -> Result<ParityFrame> {
        self.transceive_parity_inner(data, parity, None)
    }

    pub(crate) fn transceive_parity_bits(
        &mut self,
        data: &[u8],
        data_bits: usize,
        parity: &[u8],
    ) -> Result<ParityFrame> {
        self.transceive_parity_bits_inner(data, data_bits, parity, None)
    }

    pub(crate) fn transceive_parity_expect(
        &mut self,
        data: &[u8],
        parity: &[u8],
        expected_bits: usize,
    ) -> Result<ParityFrame> {
        self.transceive_parity_inner(data, parity, Some(expected_bits))
    }

    fn transceive_parity_inner(
        &mut self,
        data: &[u8],
        parity: &[u8],
        expected_bits: Option<usize>,
    ) -> Result<ParityFrame> {
        self.transceive_parity_bits_inner(data, data.len() * 8, parity, expected_bits)
    }

    fn transceive_parity_bits_inner(
        &mut self,
        data: &[u8],
        data_bits: usize,
        parity: &[u8],
        expected_bits: Option<usize>,
    ) -> Result<ParityFrame> {
        self.require_recovery("recovery parity exchange")?;
        let (frame, frame_bits) = wrap_parity_frame_bits(data, data_bits, parity)?;
        if frame.len() > MAX_RAW_PAYLOAD {
            return Err(NfcError::InvalidArgument(format!(
                "parity frame is {} bytes, at most {MAX_RAW_PAYLOAD} are supported",
                frame.len()
            )));
        }
        self.set_tx_last_bits((frame_bits % 8) as u8)?;
        let raw = self.in_communicate_thru(&frame)?;
        if raw.is_empty() {
            return Ok(ParityFrame::default());
        }
        let received_bits = match expected_bits {
            Some(bits) => bits,
            None => {
                self.set_tx_last_bits(0)?;
                let last_bits = self.read_register(REG_CIU_CONTROL)? & RX_LAST_BITS;
                if last_bits == 0 {
                    raw.len() * 8
                } else {
                    (raw.len() - 1) * 8 + last_bits as usize
                }
            }
        };
        if data_bits < 9 {
            // Short frames (for example WUPA) return the raw ATQA response;
            // the caller only needs to know that the card answered.
            return Ok(ParityFrame {
                data: raw,
                bits: received_bits,
                parity: Vec::new(),
            });
        }
        let (data, bits, parity) = unwrap_parity_frame(&raw, received_bits)?;
        Ok(ParityFrame { data, bits, parity })
    }

    pub(crate) fn set_raw_mode(&mut self, enabled: bool) -> Result<()> {
        if self.raw_mode == enabled {
            return Ok(());
        }
        if self.recovery_mode {
            return Err(NfcError::InvalidArgument(
                "raw mode is unavailable while recovery mode is enabled".to_owned(),
            ));
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
        // A selected target stays ACTIVE and does not answer an unrestricted
        // second InListPassiveTarget. Re-select the known UID instead: this
        // verifies the same card, resets Crypto1, and keeps the RF field on.
        // Phone emulations are sensitive to field cycling, and repeated
        // unrestricted selection is unreliable on PN532 UART.
        if let Some(previous) = self.selected.clone() {
            match self.select_known(&previous) {
                Ok(card) => {
                    self.selected = Some(card.clone());
                    return Ok(card);
                }
                Err(NfcError::NoCard | NfcError::Timeout(_)) => {}
                Err(error) => return Err(error),
            }
            // The known target is gone or did not answer. Cycle the field so
            // a replacement card starts from IDLE and can be discovered.
            self.cycle_rf_field()?;
        }
        self.set_tx_last_bits(0)?;
        let response = self.command(
            &[IN_LIST_PASSIVE_TARGET, 0x01, ISO14443A_106_KBPS],
            self.config.command_timeout,
            Retry::Yes,
        )?;
        let card = parse_passive_target(&response, "PN532")?;
        self.selected = Some(card.clone());
        Ok(card)
    }

    /// Re-selects a known ISO/IEC 14443A UID without cycling the field.
    fn select_known(&mut self, card: &CardInfo) -> Result<CardInfo> {
        self.set_tx_last_bits(0)?;
        let uid = cascade_uid(&card.uid);
        let mut command = Vec::with_capacity(3 + uid.len());
        command.extend_from_slice(&[IN_LIST_PASSIVE_TARGET, 0x01, ISO14443A_106_KBPS]);
        command.extend_from_slice(&uid);
        let response = self.command(&command, self.config.command_timeout, Retry::No)?;
        parse_passive_target(&response, "PN532")
    }

    pub(crate) fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()> {
        self.require_ordinary("MIFARE authentication")?;
        let card = self.selected.as_ref().ok_or(NfcError::NoCard)?;
        if card.uid.len() < 4 {
            return Err(NfcError::Io(
                "selected card has no four-byte UID suffix for authentication".to_owned(),
            ));
        }
        let uid_tail = card.uid[card.uid.len() - 4..].to_vec();
        let command = match key_type {
            KeyType::A => MIFARE_AUTH_A,
            KeyType::B => MIFARE_AUTH_B,
        };
        let mut payload = Vec::with_capacity(12);
        payload.push(command);
        payload.push(block);
        payload.extend_from_slice(key.as_bytes());
        payload.extend_from_slice(&uid_tail);
        self.data_exchange(&payload, self.config.command_timeout, Retry::Yes)?;
        Ok(())
    }

    pub(crate) fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        self.require_ordinary("MIFARE block read")?;
        self.selected.as_ref().ok_or(NfcError::NoCard)?;
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
        self.selected.as_ref().ok_or(NfcError::NoCard)?;
        let mut payload = Vec::with_capacity(2 + BLOCK_SIZE);
        payload.push(MIFARE_WRITE);
        payload.push(block);
        payload.extend_from_slice(&data);
        self.data_exchange(&payload, self.config.command_timeout, Retry::No)?;
        Ok(())
    }

    /// Unlocks a Gen1A magic card with the 7-bit `0x40` / `0x43` backdoor and
    /// writes the complete manufacturer block. The caller must reselect and
    /// verify the card; a matching block 0 is the authoritative success check.
    pub(crate) fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.require_ordinary("Gen1A manufacturer write")?;
        let card = self.selected.as_ref().ok_or(NfcError::NoCard)?.clone();
        gen1a::validate(&card, &data)?;

        // Reselect the same card before the raw sequence. A card that is still
        // authenticated would encrypt the Gen1A backdoor frames with Crypto1.
        let current = self.select_card()?;
        if !current.same_identity(&card) {
            return Err(NfcError::CardChanged);
        }

        self.set_raw_mode(true)?;
        let result = gen1a::write_block0(self, &data);
        let reset = self.set_raw_mode(false);
        match (result, reset) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    /// Probes the Gen1A unlock without changing card data, then reselects the
    /// card so the backdoor state cannot leak into the next operation.
    pub(crate) fn probe_gen1a(&mut self) -> Result<bool> {
        self.require_ordinary("Gen1A probe")?;
        let card = self.selected.as_ref().ok_or(NfcError::NoCard)?.clone();
        let current = self.select_card()?;
        if !current.same_identity(&card) {
            return Err(NfcError::CardChanged);
        }

        self.set_raw_mode(true)?;
        let result = gen1a::probe(self);
        let reset = self.set_raw_mode(false);
        let result = match (result, reset) {
            (Ok(available), Ok(())) => Ok(available),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        };

        let current = self.select_card()?;
        if !current.same_identity(&card) {
            return Err(NfcError::CardChanged);
        }
        result
    }

    /// Unlocks the Gen1A backdoor and writes the UFUID configuration that
    /// disables it. The caller must reselect and verify the result.
    pub(crate) fn seal_manufacturer_card(&mut self, config: [u8; BLOCK_SIZE]) -> Result<()> {
        self.require_ordinary("UFUID seal")?;
        let card = self.selected.as_ref().ok_or(NfcError::NoCard)?.clone();
        gen1a::validate_seal_config(&config)?;

        let current = self.select_card()?;
        if !current.same_identity(&card) {
            return Err(NfcError::CardChanged);
        }

        self.set_raw_mode(true)?;
        let result = gen1a::seal(self, &config);
        let reset = self.set_raw_mode(false);
        match (result, reset) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    pub(crate) fn read_page_quad(&mut self, start_page: u8) -> Result<[u8; PAGE_QUAD_SIZE]> {
        self.require_type2()?;
        let data = self.with_raw(|this| {
            let command = type2_command(&[TYPE2_READ, start_page]);
            let response = this.in_communicate_thru(&command)?;
            type2_response(&response, PAGE_QUAD_SIZE, "PN532")
        })?;
        let mut output = [0u8; PAGE_QUAD_SIZE];
        output.copy_from_slice(&data);
        Ok(output)
    }

    pub(crate) fn type2_version(&mut self) -> Result<[u8; TYPE2_VERSION_SIZE]> {
        self.require_type2()?;
        let data = self.with_raw(|this| {
            let command = type2_command(&[TYPE2_GET_VERSION]);
            let response = this.in_communicate_thru(&command)?;
            type2_response(&response, TYPE2_VERSION_SIZE, "PN532")
        })?;
        let mut version = [0u8; TYPE2_VERSION_SIZE];
        version.copy_from_slice(&data);
        Ok(version)
    }

    pub(crate) fn write_page(&mut self, page: u8, data: [u8; PAGE_SIZE]) -> Result<()> {
        self.require_type2()?;
        let mut payload = Vec::with_capacity(2 + PAGE_SIZE);
        payload.push(TYPE2_WRITE);
        payload.push(page);
        payload.extend_from_slice(&data);
        let command = type2_command(&payload);
        let response = self.with_raw(|this| this.in_communicate_thru(&command))?;
        if response.first() == Some(&TYPE2_ACK) {
            Ok(())
        } else {
            Err(NfcError::Io(format!(
                "PN532 Type 2 write returned {:02X?} instead of an ACK",
                response
            )))
        }
    }

    pub(crate) fn password_auth(&mut self, password: [u8; PAGE_SIZE]) -> Result<[u8; 2]> {
        self.require_type2()?;
        let mut payload = Vec::with_capacity(1 + PAGE_SIZE);
        payload.push(TYPE2_PWD_AUTH);
        payload.extend_from_slice(&password);
        let command = type2_command(&payload);
        let response = self.with_raw(|this| {
            let response = this.in_communicate_thru(&command)?;
            type2_response(&response, 2, "PN532")
        })?;
        let mut pack = [0u8; 2];
        pack.copy_from_slice(&response);
        Ok(pack)
    }

    /// Send one complete ISO 7816-4 command APDU to the selected ISO-DEP
    /// card. `InDataExchange` carries the APDU after the PN532 auto-RATS
    /// activation; the PN532 handles ISO-DEP block framing and CRC. APDUs are
    /// never retried because an UPDATE BINARY is state-changing.
    pub(crate) fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
        self.require_ordinary("ISO-DEP APDU exchange")?;
        let card = self.selected.as_ref().ok_or(NfcError::NoCard)?;
        if card.card_type != CardType::Iso14443 {
            return Err(NfcError::Unsupported(
                "the selected card is not an ISO/IEC 14443-4 card",
            ));
        }
        if apdu.is_empty() || apdu.len() > MAX_ISO_DEP_APDU {
            return Err(NfcError::InvalidArgument(format!(
                "the command APDU is {} bytes, expected 1..={MAX_ISO_DEP_APDU}",
                apdu.len()
            )));
        }
        let data = self.data_exchange(apdu, self.config.command_timeout, Retry::No)?;
        split_apdu_response(data, "PN532")
    }

    /// Send one native PN532 command frame and return the reply body. Framing,
    /// acknowledgement, and response-code validation match every other
    /// command; the command code and parameters are the caller's.
    #[cfg(feature = "developer-tools")]
    pub(crate) fn raw_command(&mut self, command: &[u8]) -> Result<Vec<u8>> {
        if command.is_empty() {
            return Err(NfcError::InvalidArgument(
                "a PN532 command needs at least one byte".to_owned(),
            ));
        }
        self.command(command, self.config.command_timeout, Retry::No)
    }

    /// Run a Type 2 command in raw framing and restore the previous mode.
    fn with_raw<T>(&mut self, action: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let was_raw = self.raw_mode;
        if !was_raw {
            self.set_raw_mode(true)?;
        }
        let result = action(self);
        if !was_raw {
            let restored = self.set_raw_mode(false);
            if result.is_ok() {
                restored?;
            }
        }
        result
    }

    fn require_type2(&self) -> Result<()> {
        let card = self.selected.as_ref().ok_or(NfcError::NoCard)?;
        if card.card_type != CardType::Type2 {
            return Err(NfcError::Unsupported(
                "the selected card is not an NFC Forum Type 2 tag",
            ));
        }
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
        self.firmware = parse_firmware(&firmware)?;
        self.reset_settings()?;
        Ok(())
    }

    fn reset_settings(&mut self) -> Result<()> {
        self.set_parameters(AUTO_ATR_RES_AND_RATS)?;
        self.command(
            &[
                RF_CONFIGURATION,
                RF_RETRY_SELECT,
                0x00,
                0x01,
                PASSIVE_ACTIVATION_RETRIES,
            ],
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
        self.recovery_mode = false;
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
        if self.raw_mode || self.recovery_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} is unavailable in raw or recovery mode"
            )));
        }
        Ok(())
    }

    fn require_raw(&self, operation: &'static str) -> Result<()> {
        if !self.raw_mode || self.recovery_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} requires raw mode outside recovery mode"
            )));
        }
        Ok(())
    }

    fn require_recovery(&self, operation: &'static str) -> Result<()> {
        if !self.recovery_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} requires recovery mode"
            )));
        }
        Ok(())
    }

    fn command(&mut self, command: &[u8], timeout: Duration, retry: Retry) -> Result<Vec<u8>> {
        let frame = build_frame(command)?;
        // The PN532 only wakes from low-power or power-down when the HSU
        // preamble is immediately followed by a command. Every command carries
        // the preamble, matching the known-good UART sequences.
        let mut packet = Vec::with_capacity(WAKEUP_PREAMBLE.len() + frame.len());
        packet.extend_from_slice(&WAKEUP_PREAMBLE);
        packet.extend_from_slice(&frame);
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
                .write_all(&packet)
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
                    return parse_response(command[0], &data, "PN532");
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

impl Gen1aLink for Pn532Protocol {
    fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>> {
        Pn532Protocol::transceive_bytes(self, tx)
    }

    fn transceive_bits(&mut self, tx: &[u8], tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Pn532Protocol::transceive_bits(self, tx, tx_bits)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use nfc_core::CardType;

    use crate::pn53x::PN532_TFI;

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
                .filter_map(|packet| {
                    let start = packet
                        .windows(3)
                        .position(|window| window == [0x00, 0x00, 0xFF])?;
                    if packet.len() < start + 6 {
                        return None;
                    }
                    let length = packet[start + 3] as usize;
                    if length == 0 || packet.len() < start + 5 + length {
                        return None;
                    }
                    Some(packet[start + 6..start + 5 + length].to_vec())
                })
                .collect()
        }

        fn host_frames(&self) -> usize {
            self.writes()
                .into_iter()
                .filter(|packet| packet.windows(3).any(|window| window == [0x00, 0x00, 0xFF]))
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

    fn selected_card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )
    }

    fn test_protocol(link: Box<dyn Pn532Link>) -> Pn532Protocol {
        Pn532Protocol {
            link,
            config: config(),
            rx: Vec::new(),
            raw_mode: false,
            recovery_mode: false,
            firmware: FirmwareVersion {
                ic: PN532_IC,
                version: 0x01,
                revision: 0x06,
                support: ISO14443A_SUPPORT,
            },
            tx_last_bits: 0,
            selected: Some(selected_card()),
        }
    }

    fn unselected_protocol(link: Box<dyn Pn532Link>) -> Pn532Protocol {
        let mut protocol = test_protocol(link);
        protocol.selected = None;
        protocol
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
        assert_eq!(
            &script.writes()[0][..WAKEUP_PREAMBLE.len()],
            &WAKEUP_PREAMBLE
        );
        assert_eq!(
            script.commands(),
            vec![
                vec![SAM_CONFIGURATION, 0x01],
                vec![GET_FIRMWARE_VERSION],
                vec![SET_PARAMETERS, AUTO_ATR_RES_AND_RATS],
                vec![
                    RF_CONFIGURATION,
                    RF_RETRY_SELECT,
                    0x00,
                    0x01,
                    PASSIVE_ACTIVATION_RETRIES
                ],
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
        let mut protocol = unselected_protocol(link);
        let card = protocol.select_card().expect("classic card");
        assert_eq!(card.uid, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(card.atqa, [0x00, 0x04]);
        assert_eq!(card.sak, 0x08);
        assert_eq!(card.card_type, CardType::MifareClassic1K);
        assert_eq!(
            script.commands(),
            vec![vec![IN_LIST_PASSIVE_TARGET, 0x01, ISO14443A_106_KBPS]]
        );
        assert!(
            script.writes()[0].starts_with(&WAKEUP_PREAMBLE),
            "each command must carry the HSU wake-up preamble"
        );

        let (link, script) = ScriptedLink::scripted();
        let mut response = vec![0x01, TARGET_NUMBER, 0x00, 0x44, 0x00, 0x07];
        response.extend_from_slice(&[0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80]);
        script.command_response(IN_LIST_PASSIVE_TARGET, &response);
        let mut protocol = unselected_protocol(link);
        let card = protocol.select_card().expect("type 2 card");
        assert_eq!(card.uid.len(), 7);
        assert_eq!(card.card_type, CardType::Type2);
    }

    #[test]
    fn reselects_a_known_card_without_cycling_the_field() {
        let (link, script) = ScriptedLink::scripted();
        let mut response = vec![0x01, TARGET_NUMBER, 0x00, 0x04, 0x08, 0x07];
        response.extend_from_slice(&[0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        script.command_response(IN_LIST_PASSIVE_TARGET, &response);
        let mut protocol = test_protocol(link);
        let card = protocol.select_card().expect("known card");
        assert!(card.same_identity(&selected_card()));
        assert_eq!(
            script.commands(),
            vec![vec![
                IN_LIST_PASSIVE_TARGET,
                0x01,
                ISO14443A_106_KBPS,
                0x88,
                0x04,
                0x11,
                0x22,
                0x33,
                0x44,
                0x55,
                0x66,
            ]]
        );
    }

    #[test]
    fn falls_back_to_a_full_selection_when_the_known_card_is_gone() {
        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_LIST_PASSIVE_TARGET, &[0x00]);
        script.command_response(RF_CONFIGURATION, &[]);
        script.command_response(RF_CONFIGURATION, &[]);
        script.command_response(IN_LIST_PASSIVE_TARGET, &classic_card_response());
        let mut protocol = test_protocol(link);
        let card = protocol.select_card().expect("replacement card");
        assert_eq!(card.uid, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let commands = script.commands();
        assert_eq!(commands[0][0], IN_LIST_PASSIVE_TARGET);
        assert_eq!(commands[0].len(), 11, "known 7-byte UID plus cascade tag");
        assert!(
            commands
                .iter()
                .any(|command| command == &[RF_CONFIGURATION, RF_FIELD, 0x00]),
            "fallback must cycle the field: {commands:?}"
        );
        assert_eq!(
            commands.last().expect("last command"),
            &vec![IN_LIST_PASSIVE_TARGET, 0x01, ISO14443A_106_KBPS]
        );
    }

    #[test]
    fn reports_missing_and_malformed_card_selections() {
        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_LIST_PASSIVE_TARGET, &[0x00]);
        let mut protocol = unselected_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::NoCard)));

        let (link, script) = ScriptedLink::scripted();
        script.command_response(IN_LIST_PASSIVE_TARGET, &[0x01, 0x01, 0x00]);
        let mut protocol = unselected_protocol(link);
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
                0x33,
                0x44,
                0x55,
                0x66,
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
        let mut protocol = unselected_protocol(link);
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
        let mut protocol = unselected_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::Io(_))));
    }

    #[test]
    fn type2_page_commands_add_crc_and_restore_raw_mode() {
        let (link, script) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);
        protocol.selected = Some(CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        ));
        script.command_response(SET_PARAMETERS, &[]);
        for _ in 0..2 {
            script.command_response(WRITE_REGISTER, &[]);
        }
        let pages: [u8; PAGE_QUAD_SIZE] = core::array::from_fn(|index| index as u8);
        let mut response = vec![0x00];
        response.extend_from_slice(&type2_command(&pages));
        script.command_response(IN_COMMUNICATE_THRU, &response);
        script.command_response(SET_PARAMETERS, &[]);
        for _ in 0..2 {
            script.command_response(WRITE_REGISTER, &[]);
        }
        assert_eq!(protocol.read_page_quad(4).expect("page read"), pages);
        assert_eq!(
            script
                .commands()
                .into_iter()
                .find(|command| command.first() == Some(&IN_COMMUNICATE_THRU)),
            Some(vec![IN_COMMUNICATE_THRU, 0x30, 0x04, 0x26, 0xEE])
        );
        assert!(!protocol.raw_mode);
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
        assert!(
            commands
                .iter()
                .any(|command| command == &[SET_PARAMETERS, AUTO_ATR_RES_AND_RATS])
        );
        assert!(
            commands
                .iter()
                .any(|command| command == &[WRITE_REGISTER, 0x63, 0x02, TX_RX_CRC_ENABLE])
        );
        assert!(
            commands
                .iter()
                .any(|command| command == &[WRITE_REGISTER, 0x63, 0x03, TX_RX_CRC_ENABLE])
        );
    }
    #[test]
    fn recovery_mode_uses_parity_disabled_framing() {
        let (link, script) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);

        script.command_response(SET_PARAMETERS, &[]);
        for _ in 0..4 {
            script.command_response(WRITE_REGISTER, &[]);
        }
        protocol.set_recovery_mode(true).expect("recovery mode");
        assert_eq!(
            script.commands(),
            vec![
                vec![SET_PARAMETERS, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x02, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x03, 0x00],
                vec![WRITE_REGISTER, 0x63, 0x0D, PARITY_DISABLE],
                vec![WRITE_REGISTER, 0x63, 0x38, 0x00],
            ]
        );
        assert!(matches!(
            protocol.transceive_bytes(&[0x26]),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            protocol.select_card(),
            Err(NfcError::InvalidArgument(_))
        ));

        let data = [0x60, 0x04, 0xD3, 0xF7];
        let parity = [1, 0, 1, 1];
        let (frame, frame_bits) = wrap_parity_frame(&data, &parity).expect("wrap");
        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(IN_COMMUNICATE_THRU, &{
            let mut payload = vec![0x00];
            payload.extend_from_slice(&frame);
            payload
        });
        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(READ_REGISTER, &[(frame_bits % 8) as u8]);
        let response = protocol
            .transceive_parity(&data, &parity)
            .expect("parity exchange");
        assert_eq!(response.data, data);
        assert_eq!(response.bits, 32);
        assert_eq!(response.parity, parity);

        script.command_response(SET_PARAMETERS, &[]);
        for _ in 0..3 {
            script.command_response(WRITE_REGISTER, &[]);
        }
        protocol.set_recovery_mode(false).expect("ordinary mode");
        let commands = script.commands();
        assert_eq!(
            &commands[commands.len() - 4..],
            &[
                vec![SET_PARAMETERS, AUTO_ATR_RES_AND_RATS],
                vec![WRITE_REGISTER, 0x63, 0x02, TX_RX_CRC_ENABLE],
                vec![WRITE_REGISTER, 0x63, 0x03, TX_RX_CRC_ENABLE],
                vec![WRITE_REGISTER, 0x63, 0x0D, 0x00],
            ]
        );
    }
    #[test]
    fn parity_expect_skips_post_response_register_round_trips() {
        let (link, script) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);
        script.command_response(SET_PARAMETERS, &[]);
        for _ in 0..4 {
            script.command_response(WRITE_REGISTER, &[]);
        }
        protocol.set_recovery_mode(true).expect("recovery mode");
        let setup = script.commands().len();

        let data = [0x60, 0x04, 0xD3, 0xF7];
        let parity = [1, 0, 1, 1];
        let (frame, _) = wrap_parity_frame(&data, &parity).expect("wrap");
        script.command_response(WRITE_REGISTER, &[]);
        script.command_response(IN_COMMUNICATE_THRU, &{
            let mut payload = vec![0x00];
            payload.extend_from_slice(&frame);
            payload
        });
        let response = protocol
            .transceive_parity_expect(&data, &parity, 36)
            .expect("parity exchange");
        assert_eq!(response.data, data);
        assert_eq!(response.bits, 32);
        assert_eq!(response.parity, parity);

        let commands = script.commands();
        assert_eq!(commands.len(), setup + 2);
        assert_eq!(commands[setup], vec![WRITE_REGISTER, 0x63, 0x3D, 0x04]);
        assert_eq!(commands[setup + 1][0], IN_COMMUNICATE_THRU);
    }

    fn iso_card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            [0x00, 0x44],
            0x20,
            CardType::Iso14443,
        )
    }

    fn iso_protocol(link: Box<dyn Pn532Link>) -> Pn532Protocol {
        let mut protocol = test_protocol(link);
        protocol.selected = Some(iso_card());
        protocol
    }

    #[test]
    fn iso_dep_apdu_is_carried_by_in_data_exchange() {
        let (link, script) = ScriptedLink::scripted();
        let mut protocol = iso_protocol(link);
        script.command_response(IN_DATA_EXCHANGE, &[0x00, 0x01, 0x02, 0x90, 0x00]);

        let apdu = [0x00, 0xA4, 0x04, 0x00];
        let response = protocol.transceive_apdu(&apdu).expect("APDU");
        assert_eq!(response.data, vec![0x01, 0x02]);
        assert_eq!(response.sw, [0x90, 0x00]);
        assert_eq!(
            script.commands(),
            vec![vec![
                IN_DATA_EXCHANGE,
                TARGET_NUMBER,
                0x00,
                0xA4,
                0x04,
                0x00
            ]]
        );
    }

    #[test]
    fn iso_dep_rejects_non_iso_cards_raw_mode_and_oversized_apdus() {
        let (link, _script) = ScriptedLink::scripted();
        let mut classic = test_protocol(link);
        assert!(matches!(
            classic.transceive_apdu(&[0x00, 0xA4, 0x04, 0x00]),
            Err(NfcError::Unsupported(_))
        ));

        let (link, _script) = ScriptedLink::scripted();
        let mut raw = iso_protocol(link);
        raw.raw_mode = true;
        assert!(matches!(
            raw.transceive_apdu(&[0x00, 0xA4, 0x04, 0x00]),
            Err(NfcError::InvalidArgument(_))
        ));

        let (link, _script) = ScriptedLink::scripted();
        let mut oversized = iso_protocol(link);
        assert!(matches!(
            oversized.transceive_apdu(&vec![0; MAX_ISO_DEP_APDU + 1]),
            Err(NfcError::InvalidArgument(_))
        ));
    }

    #[test]
    fn iso_dep_requires_a_status_word() {
        let (link, script) = ScriptedLink::scripted();
        let mut protocol = iso_protocol(link);
        script.command_response(IN_DATA_EXCHANGE, &[0x00, 0x90]);
        assert!(matches!(
            protocol.transceive_apdu(&[0x00, 0xB0, 0x00, 0x00, 0x02]),
            Err(NfcError::Io(_))
        ));
    }
}
