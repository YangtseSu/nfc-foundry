// SPDX-FileCopyrightText: 2011 Adam Laurie
// SPDX-FileCopyrightText: 2014 Dario Carluccio
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: BSD-2-Clause OR GPL-3.0-or-later

//! PN532 command layer over the ACR122U pseudo-APDU.
//!
//! The ACR122U carries `D4 <command>` payloads in the `FF 00 00 00 Lc` direct
//! transmit APDU and returns `D5 <command+1>` with status word `90 00`. There
//! is no PN532 ACK/checksum framing on this path.

use std::thread;
use std::time::Duration;

use nfc_core::{BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, Result};
use nfc_transport::ParityFrame;

use super::ccid::AcrLink;
use crate::pn53x::{
    AUTO_ATR_RES_AND_RATS, FirmwareVersion, GET_FIRMWARE_VERSION, IN_COMMUNICATE_THRU,
    IN_DATA_EXCHANGE, IN_LIST_PASSIVE_TARGET, IN_RELEASE, INITIATOR, ISO14443A_106_KBPS,
    MIFARE_AUTH_A, MIFARE_AUTH_B, MIFARE_READ, MIFARE_WRITE, PARITY_DISABLE, READ_REGISTER,
    REG_CIU_BIT_FRAMING, REG_CIU_CONTROL, REG_CIU_MANUAL_RCV, REG_CIU_RX_MODE, REG_CIU_STATUS2,
    REG_CIU_TX_MODE, RF_CONFIGURATION, RF_FIELD, RF_RETRY_SELECT, RX_LAST_BITS, SET_PARAMETERS,
    TARGET_NUMBER, TX_RX_CRC_ENABLE, WRITE_REGISTER, parse_firmware, parse_passive_target,
    parse_response, status_error, unwrap_parity_frame, wrap_parity_frame,
};

const ACR_PSEUDO_APDU: [u8; 4] = [0xFF, 0x00, 0x00, 0x00];
const PICC_OPERATING_PARAMETERS: [u8; 5] = [0xFF, 0x00, 0x51, 0x00, 0x00];
const APDU_GET_RESPONSE: u8 = 0xC0;
const SW_OK: [u8; 2] = [0x90, 0x00];
const SW_MORE_DATA: u8 = 0x61;
const SW_NO_RESPONSE: [u8; 2] = [0x63, 0x00];
const SW_APPLICATION_ERROR: [u8; 2] = [0x63, 0x7F];

const MAX_ACR_FRAME: usize = 250;
const MAX_ACR_RAW_PAYLOAD: usize = 248;

const GEN1A_UNLOCK_FIRST: u8 = 0x40;
const GEN1A_UNLOCK_SECOND: u8 = 0x43;
const MIFARE_HALT: u8 = 0x50;
const MIFARE_ACK: u8 = 0x0A;

#[derive(Clone, Debug)]
pub(crate) struct Acr122Config {
    pub(crate) command_timeout: Duration,
    pub(crate) attempts: usize,
}

impl Default for Acr122Config {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_millis(1_200),
            attempts: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retry {
    Yes,
    No,
}

pub(crate) struct Acr122Protocol {
    link: Box<dyn AcrLink>,
    config: Acr122Config,
    raw_mode: bool,
    recovery_mode: bool,
    firmware: FirmwareVersion,
    tx_last_bits: u8,
    selected: Option<CardInfo>,
}

impl Acr122Protocol {
    pub(crate) fn open(link: Box<dyn AcrLink>, config: Acr122Config) -> Result<Self> {
        let mut protocol = Self {
            link,
            config,
            raw_mode: false,
            recovery_mode: false,
            firmware: FirmwareVersion::default(),
            tx_last_bits: 0,
            selected: None,
        };
        protocol.link.power_on()?;
        protocol.initialize_picc()?;
        protocol.handshake()?;
        Ok(protocol)
    }

    pub(crate) fn firmware(&self) -> FirmwareVersion {
        self.firmware
    }

    pub(crate) fn shutdown(&mut self) {
        if self.selected.is_some() {
            let _ = self.command(
                &[IN_RELEASE, TARGET_NUMBER],
                self.config.command_timeout,
                Retry::No,
            );
        }
        let _ = self.command(
            &[RF_CONFIGURATION, RF_FIELD, 0x00],
            Duration::from_millis(200),
            Retry::No,
        );
        self.selected = None;
        self.link.shutdown();
    }

    /// Release the selected target and clear the cached selection. Raw REQA
    /// on the ACR122U additionally needs [`Self::cycle_rf_field`].
    pub(crate) fn release_card(&mut self) -> Result<()> {
        self.require_ordinary("card release")?;
        if self.selected.is_none() {
            return Ok(());
        }
        match self.command(
            &[IN_RELEASE, TARGET_NUMBER],
            self.config.command_timeout,
            Retry::No,
        ) {
            Ok(_) | Err(NfcError::NoCard) | Err(NfcError::Timeout(_)) => {
                self.selected = None;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Power-cycle the RF field so a selected card returns to IDLE.
    pub(crate) fn cycle_rf_field(&mut self) -> Result<()> {
        self.require_ordinary("RF field cycle")?;
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
        self.set_tx_last_bits(0)?;
        if enabled {
            if self.raw_mode {
                self.set_raw_mode(false)?;
            }
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
        self.require_recovery("recovery parity exchange")?;
        let (frame, frame_bits) = wrap_parity_frame(data, parity)?;
        if frame.len() > MAX_ACR_RAW_PAYLOAD {
            return Err(NfcError::InvalidArgument(format!(
                "parity frame is {} bytes, at most {MAX_ACR_RAW_PAYLOAD} are supported",
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
        self.set_tx_last_bits(0)?;
        if enabled {
            self.set_parameters(0x00)?;
            self.write_register(REG_CIU_TX_MODE, 0x00)?;
            self.write_register(REG_CIU_RX_MODE, 0x00)?;
            self.write_register(REG_CIU_STATUS2, 0x00)?;
        } else {
            self.set_parameters(AUTO_ATR_RES_AND_RATS)?;
            self.write_register(REG_CIU_TX_MODE, TX_RX_CRC_ENABLE)?;
            self.write_register(REG_CIU_RX_MODE, TX_RX_CRC_ENABLE)?;
            self.write_register(REG_CIU_STATUS2, 0x00)?;
        }
        self.raw_mode = enabled;
        Ok(())
    }

    pub(crate) fn select_card(&mut self) -> Result<CardInfo> {
        self.require_ordinary("card selection")?;
        if self.selected.is_some() {
            self.release_card()?;
        }
        self.set_tx_last_bits(0)?;
        let response = self.command(
            &[IN_LIST_PASSIVE_TARGET, 0x01, ISO14443A_106_KBPS],
            self.config.command_timeout,
            Retry::Yes,
        )?;
        let card = parse_passive_target(&response, "ACR122U")?;
        self.selected = Some(card.clone());
        Ok(card)
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
                "ACR122U MIFARE read returned {} bytes instead of {BLOCK_SIZE}",
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

    pub(crate) fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>> {
        self.require_raw("raw byte transceive")?;
        validate_raw_payload(tx, 0, true)?;
        self.set_tx_last_bits(0)?;
        self.in_communicate_thru(tx)
    }

    pub(crate) fn transceive_bits(
        &mut self,
        tx: &[u8],
        tx_bits: usize,
    ) -> Result<(Vec<u8>, usize)> {
        self.require_raw("raw bit transceive")?;
        validate_raw_payload(tx, tx_bits, false)?;
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
                    "ACR122U reported a partial byte without data".to_owned(),
                ));
            }
            (data.len() - 1) * 8 + last_bits as usize
        };
        Ok((data, received_bits))
    }

    /// Unlocks a Gen1A magic card with the 7-bit `0x40` / `0x43` backdoor and
    /// writes the complete manufacturer block. The caller must reselect and
    /// verify the card; a matching block 0 is the authoritative success check.
    pub(crate) fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.require_ordinary("Gen1A manufacturer write")?;
        let card = self.selected.as_ref().ok_or(NfcError::NoCard)?.clone();
        if card.uid.len() != 4 || card.card_type != CardType::MifareClassic1K {
            return Err(NfcError::Unsupported(
                "Gen1A block 0 writes require a four-byte UID MIFARE Classic 1K card",
            ));
        }
        if data[4] != data[0] ^ data[1] ^ data[2] ^ data[3] {
            return Err(NfcError::InvalidArgument(
                "the manufacturer block has an invalid BCC".to_owned(),
            ));
        }

        // Reselect the same card before the raw sequence. A card that is still
        // authenticated would encrypt the Gen1A backdoor frames with Crypto1.
        let current = self.select_card()?;
        if !current.same_identity(&card) {
            return Err(NfcError::CardChanged);
        }

        self.set_raw_mode(true)?;
        let result = self.gen1a_sequence(&data);
        let reset = self.set_raw_mode(false);
        match (result, reset) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    fn initialize_picc(&mut self) -> Result<()> {
        // Set PICC operating parameters. The ACR122U may report ICC_MUTE here
        // right after power-on; firmware validation is the authoritative check.
        match self
            .link
            .xfr_block(&PICC_OPERATING_PARAMETERS, self.config.command_timeout)
        {
            Ok(_) | Err(NfcError::NoCard) | Err(NfcError::Timeout(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn handshake(&mut self) -> Result<()> {
        let mut last_error = None;
        for _ in 0..self.config.attempts.max(1) {
            match self.firmware_version() {
                Ok(firmware) => {
                    self.firmware = firmware;
                    match self.reset_settings() {
                        Ok(()) => return Ok(()),
                        Err(error) if retryable(&error) => last_error = Some(error),
                        Err(error) => return Err(error),
                    }
                }
                Err(error) if retryable(&error) => last_error = Some(error),
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or(NfcError::Unsupported(
            "the ACR122U did not respond as a PN532",
        )))
    }

    fn firmware_version(&mut self) -> Result<FirmwareVersion> {
        let data = self.command(
            &[GET_FIRMWARE_VERSION],
            self.config.command_timeout,
            Retry::No,
        )?;
        parse_firmware(&data)
    }

    fn reset_settings(&mut self) -> Result<()> {
        self.set_parameters(AUTO_ATR_RES_AND_RATS)?;
        self.write_register(REG_CIU_BIT_FRAMING, 0x00)?;
        self.write_register(REG_CIU_TX_MODE, TX_RX_CRC_ENABLE)?;
        self.write_register(REG_CIU_RX_MODE, TX_RX_CRC_ENABLE)?;
        self.write_register(REG_CIU_MANUAL_RCV, 0x00)?;
        self.write_register(REG_CIU_CONTROL, INITIATOR)?;
        // The ACR122U keeps an explicitly disabled RF field off across opens.
        // Cycle it on here before any InListPassiveTarget command.
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
        self.command(
            &[RF_CONFIGURATION, RF_RETRY_SELECT, 0x00, 0x01, 0x02],
            self.config.command_timeout,
            Retry::Yes,
        )?;
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
            .ok_or_else(|| NfcError::Io("ACR122U ReadRegister response is empty".to_owned()))
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
                    "ACR122U InDataExchange response is empty".to_owned(),
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
                    "ACR122U InCommunicateThru response is empty".to_owned(),
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

    fn command(&mut self, command: &[u8], timeout: Duration, retry: Retry) -> Result<Vec<u8>> {
        let frame_length = command.len() + 1;
        if frame_length > MAX_ACR_FRAME {
            return Err(NfcError::InvalidArgument(format!(
                "ACR122U command payload is {frame_length} bytes, at most {MAX_ACR_FRAME} are supported"
            )));
        }
        let mut apdu = Vec::with_capacity(ACR_PSEUDO_APDU.len() + 1 + frame_length);
        apdu.extend_from_slice(&ACR_PSEUDO_APDU);
        apdu.push(frame_length as u8);
        apdu.push(0xD4);
        apdu.extend_from_slice(command);

        let attempts = if retry == Retry::Yes {
            self.config.attempts.max(1)
        } else {
            1
        };
        let mut last_error = None;
        for _ in 0..attempts {
            match self.exchange_apdu(&apdu, timeout) {
                Ok(data) => return parse_response(command[0], &data, "ACR122U"),
                Err(error) if retry == Retry::Yes && retryable(&error) => {
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| NfcError::Io("ACR122U command failed".to_owned())))
    }

    fn exchange_apdu(&mut self, apdu: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        let mut response = self.link.xfr_block(apdu, timeout)?;
        let mut data = Vec::new();
        for _ in 0..4 {
            match response.sw {
                SW_OK => {
                    data.extend_from_slice(&response.data);
                    return Ok(data);
                }
                [SW_MORE_DATA, length] => {
                    data.extend_from_slice(&response.data);
                    response = self
                        .link
                        .xfr_block(&[0xFF, APDU_GET_RESPONSE, 0x00, 0x00, length], timeout)?;
                }
                SW_NO_RESPONSE => return Err(NfcError::NoCard),
                SW_APPLICATION_ERROR => {
                    return Err(NfcError::Io(
                        "the ACR122U PN532 reported an application-level error".to_owned(),
                    ));
                }
                [sw1, sw2] => {
                    return Err(NfcError::Io(format!(
                        "ACR122U returned status word {sw1:02X}{sw2:02X}"
                    )));
                }
            }
        }
        Err(NfcError::Io(
            "ACR122U returned too many GetResponse continuations".to_owned(),
        ))
    }

    fn require_recovery(&self, operation: &'static str) -> Result<()> {
        if !self.recovery_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} requires recovery mode"
            )));
        }
        Ok(())
    }

    fn require_ordinary(&self, operation: &'static str) -> Result<()> {
        if self.raw_mode || self.recovery_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} is unavailable in raw mode"
            )));
        }
        Ok(())
    }

    fn require_raw(&self, operation: &'static str) -> Result<()> {
        if !self.raw_mode || self.recovery_mode {
            return Err(NfcError::InvalidArgument(format!(
                "{operation} requires raw mode"
            )));
        }
        Ok(())
    }

    fn gen1a_sequence(&mut self, data: &[u8; BLOCK_SIZE]) -> Result<()> {
        let halt = crc_a(&[MIFARE_HALT, 0x00]);
        match self.transceive_bytes(&halt) {
            Ok(_) => {}
            Err(NfcError::Timeout(_) | NfcError::NoCard) => {}
            Err(error) => return Err(error),
        }

        let (response, bits) = self
            .transceive_bits(&[GEN1A_UNLOCK_FIRST], 7)
            .map_err(|error| gen1a_error("the card did not answer the 7-bit 0x40 unlock", error))?;
        if bits != 4 || !is_gen1a_ack(&response) {
            return Err(NfcError::Unsupported(
                "the card answered the 7-bit 0x40 Gen1A unlock command with an unexpected frame",
            ));
        }

        let response = self
            .transceive_bytes(&[GEN1A_UNLOCK_SECOND])
            .map_err(|error| gen1a_error("the card refused the 0x43 unlock command", error))?;
        if !is_gen1a_ack(&response) {
            return Err(NfcError::Unsupported(
                "the card answered the Gen1A 0x43 unlock command with an unexpected frame",
            ));
        }

        let mut write_command = vec![MIFARE_WRITE, 0x00];
        let write_crc = crc_a(&write_command);
        write_command.extend_from_slice(&write_crc);
        let response = self.transceive_bytes(&write_command).map_err(|error| {
            gen1a_error("the card did not acknowledge the block 0 write", error)
        })?;
        if !is_gen1a_ack(&response) {
            return Err(NfcError::Io(
                "the card answered the Gen1A block 0 write with an unexpected frame".to_owned(),
            ));
        }

        let mut payload = Vec::with_capacity(BLOCK_SIZE + 2);
        payload.extend_from_slice(data);
        payload.extend_from_slice(&crc_a(data));
        let response = self
            .transceive_bytes(&payload)
            .map_err(|error| gen1a_error("the block 0 data frame was not acknowledged", error))?;
        if !is_gen1a_ack(&response) {
            return Err(NfcError::Io(
                "the block 0 data frame was answered with an unexpected frame".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_raw_payload(tx: &[u8], tx_bits: usize, bytes: bool) -> Result<()> {
    if tx.is_empty() || (!bytes && tx_bits == 0) {
        return Err(NfcError::InvalidArgument(
            "raw transceive needs at least one bit".to_owned(),
        ));
    }
    if tx.len() > MAX_ACR_RAW_PAYLOAD {
        return Err(NfcError::InvalidArgument(format!(
            "raw transceive payload is {} bytes, at most {MAX_ACR_RAW_PAYLOAD} are supported",
            tx.len()
        )));
    }
    if !bytes && tx_bits > tx.len() * 8 {
        return Err(NfcError::InvalidArgument(format!(
            "raw transceive sends {tx_bits} bits from {} bytes",
            tx.len()
        )));
    }
    Ok(())
}

fn retryable(error: &NfcError) -> bool {
    matches!(error, NfcError::Timeout(_) | NfcError::Io(_))
}

fn gen1a_error(context: &'static str, error: NfcError) -> NfcError {
    match error {
        NfcError::Timeout(_) | NfcError::NoCard | NfcError::AuthenticationFailed => {
            NfcError::Unsupported(context)
        }
        other => other,
    }
}

fn is_gen1a_ack(response: &[u8]) -> bool {
    response.len() == 1 && (response[0] == MIFARE_ACK || response[0] == MIFARE_ACK << 4)
}

fn crc_a(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0x6363;
    for &byte in data {
        let mut mixed = byte ^ crc as u8;
        mixed ^= mixed << 4;
        crc = (crc >> 8) ^ ((mixed as u16) << 8) ^ ((mixed as u16) << 3) ^ ((mixed as u16) >> 4);
    }
    [crc as u8, (crc >> 8) as u8]
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use super::super::ccid::ApduResponse;
    use super::*;
    use crate::pn53x::PN532_IC;

    #[derive(Clone, Debug)]
    enum LinkStep {
        Apdu(ApduResponse),
        Timeout,
        Disconnect,
    }

    #[derive(Default)]
    struct LinkLog {
        apdus: Mutex<Vec<Vec<u8>>>,
    }

    impl LinkLog {
        fn lock(&self) -> MutexGuard<'_, Vec<Vec<u8>>> {
            self.apdus.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    struct ScriptedLink {
        steps: Arc<Mutex<VecDeque<LinkStep>>>,
        log: Arc<LinkLog>,
    }

    type ScriptedSetup = (
        Box<dyn AcrLink>,
        Arc<Mutex<VecDeque<LinkStep>>>,
        Arc<LinkLog>,
    );

    impl ScriptedLink {
        fn scripted() -> ScriptedSetup {
            let steps = Arc::new(Mutex::new(VecDeque::new()));
            let log = Arc::new(LinkLog::default());
            let link = Self {
                steps: Arc::clone(&steps),
                log: Arc::clone(&log),
            };
            (Box::new(link), steps, log)
        }
    }

    impl AcrLink for ScriptedLink {
        fn power_on(&mut self) -> Result<()> {
            Ok(())
        }

        fn xfr_block(&mut self, apdu: &[u8], _timeout: Duration) -> Result<ApduResponse> {
            self.log.lock().push(apdu.to_vec());
            let step = self
                .steps
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .expect("script has no more responses");
            match step {
                LinkStep::Apdu(response) => Ok(response),
                LinkStep::Timeout => Err(NfcError::Timeout("scripted USB timeout".to_owned())),
                LinkStep::Disconnect => Err(NfcError::Io("scripted disconnect".to_owned())),
            }
        }

        fn shutdown(&mut self) {}
    }

    fn apdu(data: &[u8]) -> ApduResponse {
        ApduResponse {
            data: data.to_vec(),
            sw: SW_OK,
        }
    }

    fn response(command: u8, payload: &[u8]) -> ApduResponse {
        let mut data = vec![0xD5, command.wrapping_add(1)];
        data.extend_from_slice(payload);
        apdu(&data)
    }

    fn config() -> Acr122Config {
        Acr122Config {
            command_timeout: Duration::from_millis(5),
            attempts: 2,
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

    fn test_protocol(link: Box<dyn AcrLink>) -> Acr122Protocol {
        Acr122Protocol {
            link,
            config: config(),
            raw_mode: false,
            recovery_mode: false,
            firmware: FirmwareVersion {
                ic: PN532_IC,
                version: 0x01,
                revision: 0x06,
                support: 0x01,
            },
            tx_last_bits: 0,
            selected: Some(selected_card()),
        }
    }

    fn unselected_protocol(link: Box<dyn AcrLink>) -> Acr122Protocol {
        let mut protocol = test_protocol(link);
        protocol.selected = None;
        protocol
    }

    fn classic_card_response() -> ApduResponse {
        response(
            IN_LIST_PASSIVE_TARGET,
            &[
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
            ],
        )
    }

    fn push(steps: &Arc<Mutex<VecDeque<LinkStep>>>, step: LinkStep) {
        steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(step);
    }

    #[test]
    fn opens_with_picc_parameters_firmware_and_registers() {
        let (link, steps, log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Apdu(apdu(&[])));
        push(
            &steps,
            LinkStep::Apdu(response(
                GET_FIRMWARE_VERSION,
                &[PN532_IC, 0x01, 0x06, 0x01],
            )),
        );
        push(&steps, LinkStep::Apdu(response(SET_PARAMETERS, &[])));
        for _ in 0..5 {
            push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        }
        for _ in 0..3 {
            push(&steps, LinkStep::Apdu(response(RF_CONFIGURATION, &[])));
        }
        let protocol = Acr122Protocol::open(link, config()).expect("open");
        assert_eq!(protocol.firmware().version, 1);

        let apdus = log.lock().clone();
        assert_eq!(apdus[0], PICC_OPERATING_PARAMETERS);
        assert_eq!(
            apdus[1],
            vec![0xFF, 0x00, 0x00, 0x00, 0x02, 0xD4, GET_FIRMWARE_VERSION]
        );
        assert_eq!(
            apdus[2],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x03,
                0xD4,
                SET_PARAMETERS,
                AUTO_ATR_RES_AND_RATS
            ]
        );
        assert_eq!(
            apdus[3],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x05,
                0xD4,
                WRITE_REGISTER,
                0x63,
                0x3D,
                0x00
            ]
        );
        assert_eq!(
            apdus[7],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x05,
                0xD4,
                WRITE_REGISTER,
                0x63,
                0x3C,
                INITIATOR
            ]
        );
        assert_eq!(
            apdus[8],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x04,
                0xD4,
                RF_CONFIGURATION,
                RF_FIELD,
                0x00
            ]
        );
        assert_eq!(
            apdus[10],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x06,
                0xD4,
                RF_CONFIGURATION,
                RF_RETRY_SELECT,
                0x00,
                0x01,
                0x02
            ]
        );
    }

    #[test]
    fn opens_after_a_transient_picc_parameters_timeout() {
        let (link, steps, _log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Timeout);
        push(
            &steps,
            LinkStep::Apdu(response(
                GET_FIRMWARE_VERSION,
                &[PN532_IC, 0x01, 0x06, 0x01],
            )),
        );
        push(&steps, LinkStep::Apdu(response(SET_PARAMETERS, &[])));
        for _ in 0..5 {
            push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        }
        for _ in 0..3 {
            push(&steps, LinkStep::Apdu(response(RF_CONFIGURATION, &[])));
        }
        let protocol = Acr122Protocol::open(link, config()).expect("open after timeout");
        assert_eq!(protocol.firmware().revision, 6);
    }

    #[test]
    fn rejects_invalid_gen1a_blocks_before_touching_the_card() {
        let (link, _steps, log) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);
        protocol.selected = Some(CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        ));
        let mut block = [0u8; BLOCK_SIZE];
        block[0..4].copy_from_slice(&[0x04, 0x11, 0x22, 0x33]);
        block[4] = 0xFF;
        assert!(matches!(
            protocol.write_manufacturer_block(block),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(log.lock().is_empty());
    }

    #[test]
    fn raw_payload_limit_matches_the_pseudo_apdu_frame() {
        assert!(validate_raw_payload(&vec![0u8; MAX_ACR_RAW_PAYLOAD], 0, true).is_ok());
        assert!(matches!(
            validate_raw_payload(&vec![0u8; MAX_ACR_RAW_PAYLOAD + 1], 0, true),
            Err(NfcError::InvalidArgument(_))
        ));
    }

    #[test]
    fn rejects_non_pn532_firmware() {
        let (link, steps, _log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Apdu(apdu(&[])));
        push(
            &steps,
            LinkStep::Apdu(response(GET_FIRMWARE_VERSION, &[0x33, 0x01, 0x06, 0x01])),
        );
        assert!(matches!(
            Acr122Protocol::open(link, config()),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[test]
    fn selects_and_releases_cards() {
        let (link, steps, log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Apdu(classic_card_response()));
        let mut protocol = unselected_protocol(link);
        let card = protocol.select_card().expect("card");
        assert_eq!(card.uid, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(card.card_type, CardType::MifareClassic1K);
        assert_eq!(
            log.lock()[0],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x04,
                0xD4,
                IN_LIST_PASSIVE_TARGET,
                0x01,
                0x00
            ]
        );

        let (link, steps, log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Apdu(response(IN_RELEASE, &[])));
        push(&steps, LinkStep::Apdu(classic_card_response()));
        let mut protocol = unselected_protocol(link);
        protocol.selected = Some(selected_card());
        protocol.select_card().expect("reselect");
        assert_eq!(
            log.lock()[0],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x03,
                0xD4,
                IN_RELEASE,
                TARGET_NUMBER
            ]
        );
    }

    #[test]
    fn reports_missing_cards_and_bad_selection_data() {
        let (link, steps, _log) = ScriptedLink::scripted();
        push(
            &steps,
            LinkStep::Apdu(response(IN_LIST_PASSIVE_TARGET, &[0x00])),
        );
        let mut protocol = unselected_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::NoCard)));

        let (link, steps, _log) = ScriptedLink::scripted();
        push(
            &steps,
            LinkStep::Apdu(response(IN_LIST_PASSIVE_TARGET, &[0x01, 0x01, 0x00])),
        );
        let mut protocol = unselected_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::Io(_))));

        let (link, steps, _log) = ScriptedLink::scripted();
        push(
            &steps,
            LinkStep::Apdu(ApduResponse {
                data: Vec::new(),
                sw: SW_NO_RESPONSE,
            }),
        );
        let mut protocol = unselected_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::NoCard)));
    }

    #[test]
    fn authenticates_with_the_requested_key_and_maps_failure() {
        let key = Key::new([0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);
        let (link, steps, log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Apdu(response(IN_DATA_EXCHANGE, &[0x00])));
        let mut protocol = test_protocol(link);
        protocol
            .authenticate(4, KeyType::B, key)
            .expect("authenticate");
        assert_eq!(
            log.lock()[0],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x0F,
                0xD4,
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
            ]
        );

        let (link, steps, _log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Apdu(response(IN_DATA_EXCHANGE, &[0x14])));
        let mut protocol = test_protocol(link);
        assert!(matches!(
            protocol.authenticate(0, KeyType::A, key),
            Err(NfcError::AuthenticationFailed)
        ));
    }

    #[test]
    fn reads_and_writes_classic_blocks() {
        let block = [0x5A; BLOCK_SIZE];
        let (link, steps, _log) = ScriptedLink::scripted();
        let mut payload = vec![0x00];
        payload.extend_from_slice(&block);
        push(&steps, LinkStep::Apdu(response(IN_DATA_EXCHANGE, &payload)));
        let mut protocol = test_protocol(link);
        assert_eq!(protocol.read_block(4).expect("read"), block);

        let (link, steps, _log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Apdu(response(IN_DATA_EXCHANGE, &[0x00])));
        let mut protocol = test_protocol(link);
        protocol.write_block(4, block).expect("write");
    }

    #[test]
    fn maps_apdu_chaining_and_status_words() {
        let (link, steps, log) = ScriptedLink::scripted();
        push(
            &steps,
            LinkStep::Apdu(ApduResponse {
                data: vec![0xD5, 0x03, 0x32],
                sw: [0x61, 0x03],
            }),
        );
        push(
            &steps,
            LinkStep::Apdu(ApduResponse {
                data: vec![0x01, 0x06, 0x01],
                sw: SW_OK,
            }),
        );
        let mut protocol = test_protocol(link);
        assert_eq!(
            protocol.firmware_version().expect("chained firmware"),
            FirmwareVersion {
                ic: PN532_IC,
                version: 1,
                revision: 6,
                support: 1
            }
        );
        assert_eq!(
            log.lock()[1],
            vec![0xFF, APDU_GET_RESPONSE, 0x00, 0x00, 0x03]
        );

        let (link, steps, _log) = ScriptedLink::scripted();
        for _ in 0..config().attempts {
            push(
                &steps,
                LinkStep::Apdu(ApduResponse {
                    data: Vec::new(),
                    sw: SW_APPLICATION_ERROR,
                }),
            );
        }
        let mut protocol = test_protocol(link);
        assert!(matches!(
            protocol.read_register(0x633C),
            Err(NfcError::Io(_))
        ));
    }

    #[test]
    fn reports_usb_disconnects() {
        let (link, steps, _log) = ScriptedLink::scripted();
        for _ in 0..config().attempts {
            push(&steps, LinkStep::Disconnect);
        }
        let mut protocol = unselected_protocol(link);
        assert!(matches!(protocol.select_card(), Err(NfcError::Io(_))));
    }

    #[test]
    fn retries_only_retryable_commands() {
        let (link, steps, _log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Timeout);
        push(&steps, LinkStep::Apdu(classic_card_response()));
        let mut protocol = unselected_protocol(link);
        protocol.select_card().expect("card after retry");

        let (link, steps, _log) = ScriptedLink::scripted();
        push(&steps, LinkStep::Timeout);
        let mut protocol = test_protocol(link);
        assert!(matches!(
            protocol.write_block(4, [0x00; BLOCK_SIZE]),
            Err(NfcError::Timeout(_))
        ));
    }

    #[test]
    fn raw_mode_gates_and_decodes_trailing_bits() {
        let (link, steps, log) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);
        assert!(matches!(
            protocol.transceive_bytes(&[0x26]),
            Err(NfcError::InvalidArgument(_))
        ));

        push(&steps, LinkStep::Apdu(response(SET_PARAMETERS, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        protocol.set_raw_mode(true).expect("raw mode");
        assert!(matches!(
            protocol.select_card(),
            Err(NfcError::InvalidArgument(_))
        ));

        push(
            &steps,
            LinkStep::Apdu(response(IN_COMMUNICATE_THRU, &[0x00, 0x0A])),
        );
        assert_eq!(
            protocol.transceive_bytes(&[0x40]).expect("raw bytes"),
            vec![0x0A]
        );

        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(
            &steps,
            LinkStep::Apdu(response(IN_COMMUNICATE_THRU, &[0x00, 0x0A])),
        );
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(READ_REGISTER, &[0x04])));
        assert_eq!(
            protocol.transceive_bits(&[0x40], 7).expect("raw bits"),
            (vec![0x0A], 4)
        );
        assert_eq!(
            log.lock()[5],
            vec![
                0xFF,
                0x00,
                0x00,
                0x00,
                0x05,
                0xD4,
                WRITE_REGISTER,
                0x63,
                0x3D,
                0x07
            ]
        );
    }

    #[test]
    fn gen1a_write_runs_the_unlock_sequence_and_restores_ordinary_mode() {
        let original = [
            0x04, 0x11, 0x22, 0x33, 0x04, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let (link, steps, log) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);
        protocol.selected = Some(CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        ));
        // Release and reselect the card before raw mode.
        push(&steps, LinkStep::Apdu(response(IN_RELEASE, &[])));
        push(
            &steps,
            LinkStep::Apdu(response(
                IN_LIST_PASSIVE_TARGET,
                &[
                    0x01,
                    TARGET_NUMBER,
                    0x00,
                    0x04,
                    0x08,
                    0x04,
                    0x04,
                    0x11,
                    0x22,
                    0x33,
                ],
            )),
        );
        // Enter raw mode.
        push(&steps, LinkStep::Apdu(response(SET_PARAMETERS, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        // HALT has no answer.
        push(&steps, LinkStep::Timeout);
        // 0x40 and 0x43 acknowledgements.
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(
            &steps,
            LinkStep::Apdu(response(IN_COMMUNICATE_THRU, &[0x00, 0x0A])),
        );
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(READ_REGISTER, &[0x04])));
        push(
            &steps,
            LinkStep::Apdu(response(IN_COMMUNICATE_THRU, &[0x00, 0x0A])),
        );
        // Write command and data acknowledgements.
        push(
            &steps,
            LinkStep::Apdu(response(IN_COMMUNICATE_THRU, &[0x00, 0x0A])),
        );
        push(
            &steps,
            LinkStep::Apdu(response(IN_COMMUNICATE_THRU, &[0x00, 0x0A])),
        );
        // Leave raw mode.
        push(&steps, LinkStep::Apdu(response(SET_PARAMETERS, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));

        protocol
            .write_manufacturer_block(original)
            .expect("Gen1A write");
        let apdus = log.lock().clone();
        assert_eq!(apdus.len(), 18);
        assert_eq!(
            &apdus[12][..],
            &[
                0xFF,
                0x00,
                0x00,
                0x00,
                0x06,
                0xD4,
                IN_COMMUNICATE_THRU,
                0xA0,
                0x00,
                0x5F,
                0xB1
            ]
        );
    }
    #[test]
    fn recovery_mode_uses_parity_disabled_framing() {
        let (link, steps, _log) = ScriptedLink::scripted();
        let mut protocol = test_protocol(link);

        push(&steps, LinkStep::Apdu(response(SET_PARAMETERS, &[])));
        for _ in 0..4 {
            push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        }
        protocol.set_recovery_mode(true).expect("recovery mode");
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
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(
            &steps,
            LinkStep::Apdu(response(IN_COMMUNICATE_THRU, &{
                let mut payload = vec![0x00];
                payload.extend_from_slice(&frame);
                payload
            })),
        );
        push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        push(
            &steps,
            LinkStep::Apdu(response(READ_REGISTER, &[(frame_bits % 8) as u8])),
        );
        let parity_response = protocol
            .transceive_parity(&data, &parity)
            .expect("parity exchange");
        assert_eq!(parity_response.data, data);
        assert_eq!(parity_response.bits, 32);
        assert_eq!(parity_response.parity, parity);

        push(&steps, LinkStep::Apdu(response(SET_PARAMETERS, &[])));
        for _ in 0..3 {
            push(&steps, LinkStep::Apdu(response(WRITE_REGISTER, &[])));
        }
        protocol.set_recovery_mode(false).expect("ordinary mode");
        assert!(matches!(
            protocol.transceive_parity(&data, &parity),
            Err(NfcError::InvalidArgument(_))
        ));
    }
}
