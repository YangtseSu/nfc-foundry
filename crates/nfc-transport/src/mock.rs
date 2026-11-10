//! Scripted reader and transport used by application and workflow tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result};

use crate::{
    ApduCapabilities, ApduResponse, DeviceInfo, ManufacturerWriter, Reader, TrailerWriter,
    Transport,
};

/// Reader entry points that can be scripted with a fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockCall {
    CardInfo,
    Authenticate,
    ReadBlock,
    WriteBlock,
    Transceive,
    ReadPageQuad,
    WritePage,
    PasswordAuth,
    Apdu,
    /// An UPDATE BINARY APDU. Kept separate so `WriteIgnored` targets a
    /// state-changing exchange instead of the next SELECT or READ.
    ApduWrite,
}

/// Injected behavior for the next matching call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockFault {
    AuthenticationFailed,
    NoCard,
    Timeout,
    Cancelled,
    /// The write reports success but leaves the stored block unchanged.
    WriteIgnored,
}

impl MockFault {
    fn into_error(self) -> NfcError {
        match self {
            Self::AuthenticationFailed => NfcError::AuthenticationFailed,
            Self::NoCard => NfcError::NoCard,
            Self::Timeout => NfcError::Timeout("mock reader timed out".to_owned()),
            Self::Cancelled => NfcError::Cancelled,
            Self::WriteIgnored => NfcError::Io("mock write ignored".to_owned()),
        }
    }
}

/// A scripted NFC Forum Type 4 tag. The simulator answers the ISO 7816-4
/// SELECT, READ BINARY, and UPDATE BINARY commands used by the Type 4 adapter,
/// so workflow tests exercise the real APDU sequence instead of a canned list.
///
/// The NDEF file is stored as complete file bytes, including its two-byte NLEN
/// prefix. The default file IDs are `E103` (CC) and `E104` (NDEF).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockType4Tag {
    cc: Vec<u8>,
    ndef: Vec<u8>,
    cc_file_id: u16,
    ndef_file_id: u16,
    write_access: u8,
    selected_application: bool,
    selected_file: Option<u16>,
    commands: Vec<Vec<u8>>,
}

impl MockType4Tag {
    pub fn new(cc: impl Into<Vec<u8>>, ndef: impl Into<Vec<u8>>) -> Self {
        Self {
            cc: cc.into(),
            ndef: ndef.into(),
            cc_file_id: 0xE103,
            ndef_file_id: 0xE104,
            write_access: 0x00,
            selected_application: false,
            selected_file: None,
            commands: Vec::new(),
        }
    }

    /// A factory NTAG 424 DNA: 23-byte CC, 256-byte NDEF file, free access.
    pub fn ntag424() -> Self {
        Self::new(
            vec![
                0x00, 0x17, 0x20, 0x01, 0x00, 0x00, 0xFF, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
                0x00, 0x05, 0x06, 0xE1, 0x05, 0x00, 0x80, 0x82, 0x83,
            ],
            vec![0u8; 256],
        )
    }

    /// A DESFire-style 15-byte CC and a 256-byte NDEF file.
    pub fn desfire_256() -> Self {
        Self::new(
            vec![
                0x00, 0x0F, 0x20, 0x00, 0x3A, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
                0x00,
            ],
            vec![0u8; 256],
        )
    }

    pub fn with_write_access(mut self, access: u8) -> Self {
        self.write_access = access;
        self
    }

    pub fn with_ndef_file_id(mut self, file_id: u16) -> Self {
        self.ndef_file_id = file_id;
        self
    }

    pub fn cc(&self) -> &[u8] {
        &self.cc
    }

    pub fn ndef(&self) -> &[u8] {
        &self.ndef
    }

    pub fn commands(&self) -> &[Vec<u8>] {
        &self.commands
    }

    fn handle(&mut self, apdu: &[u8], ignore_write: bool) -> ApduResponse {
        self.commands.push(apdu.to_vec());
        if apdu.len() < 4 {
            return sw(0x67, 0x00);
        }
        match (apdu[0], apdu[1]) {
            (0x00, 0xA4) => self.select(apdu),
            (0x00, 0xB0) => self.read_binary(apdu),
            (0x00, 0xD6) => self.update_binary(apdu, ignore_write),
            _ => sw(0x6D, 0x00),
        }
    }

    fn select(&mut self, apdu: &[u8]) -> ApduResponse {
        let (p1, p2) = (apdu[2], apdu[3]);
        if p1 == 0x04 {
            if p2 != 0x00 || apdu.len() < 6 {
                return sw(0x6A, 0x86);
            }
            let length = usize::from(apdu[4]);
            if apdu.len() < 5 + length {
                return sw(0x67, 0x00);
            }
            if apdu[5..5 + length] == [0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01] {
                self.selected_application = true;
                self.selected_file = None;
                sw(0x90, 0x00)
            } else {
                sw(0x6A, 0x82)
            }
        } else if p1 == 0x00 {
            if (p2 != 0x00 && p2 != 0x0C) || apdu.len() < 7 || apdu[4] != 0x02 {
                return sw(0x6A, 0x86);
            }
            let file_id = u16::from_be_bytes([apdu[5], apdu[6]]);
            if file_id == self.cc_file_id || file_id == self.ndef_file_id {
                self.selected_file = Some(file_id);
                sw(0x90, 0x00)
            } else {
                sw(0x6A, 0x82)
            }
        } else {
            sw(0x6A, 0x86)
        }
    }

    fn read_binary(&mut self, apdu: &[u8]) -> ApduResponse {
        let Some(file_id) = self.selected_file else {
            return sw(0x69, 0x82);
        };
        if !self.selected_application && file_id != self.cc_file_id {
            return sw(0x69, 0x82);
        }
        if apdu.len() < 5 {
            return sw(0x67, 0x00);
        }
        let offset = usize::from(u16::from_be_bytes([apdu[2], apdu[3]]));
        let requested = match apdu[4] {
            0x00 => 256,
            value => usize::from(value),
        };
        let file = if file_id == self.cc_file_id {
            &self.cc
        } else {
            &self.ndef
        };
        if offset >= file.len() {
            return sw(0x6B, 0x00);
        }
        let end = (offset + requested).min(file.len());
        let data = file[offset..end].to_vec();
        if data.len() < requested {
            ApduResponse::new(data, [0x62, 0x82])
        } else {
            ApduResponse::new(data, [0x90, 0x00])
        }
    }

    fn update_binary(&mut self, apdu: &[u8], ignore_write: bool) -> ApduResponse {
        let Some(file_id) = self.selected_file else {
            return sw(0x69, 0x82);
        };
        if file_id != self.ndef_file_id {
            return sw(0x69, 0x82);
        }
        if self.write_access != 0x00 {
            return sw(0x69, 0x82);
        }
        if apdu.len() < 5 {
            return sw(0x67, 0x00);
        }
        let length = usize::from(apdu[4]);
        if apdu.len() < 5 + length {
            return sw(0x67, 0x00);
        }
        let offset = usize::from(u16::from_be_bytes([apdu[2], apdu[3]]));
        if offset + length > self.ndef.len() {
            return sw(0x6B, 0x00);
        }
        if !ignore_write {
            self.ndef[offset..offset + length].copy_from_slice(&apdu[5..5 + length]);
        }
        sw(0x90, 0x00)
    }
}

fn sw(sw1: u8, sw2: u8) -> ApduResponse {
    ApduResponse::new(Vec::new(), [sw1, sw2])
}

#[derive(Debug, Default)]
struct MockState {
    card: Option<CardInfo>,
    blocks: HashMap<u8, [u8; BLOCK_SIZE]>,
    pages: HashMap<u8, [u8; PAGE_SIZE]>,
    password: Option<([u8; PAGE_SIZE], [u8; 2])>,
    type4: Option<MockType4Tag>,
    raw_mode: bool,
    open: bool,
    open_count: usize,
    close_count: usize,
    faults: Vec<(MockCall, MockFault)>,
}

impl MockState {
    fn take_fault(&mut self, call: MockCall) -> Option<MockFault> {
        let index = self.faults.iter().position(|(target, _)| *target == call)?;
        Some(self.faults.remove(index).1)
    }

    fn present_card(&self) -> Result<&CardInfo> {
        self.card.as_ref().ok_or(NfcError::NoCard)
    }

    fn check_call(&mut self, call: MockCall) -> Result<()> {
        if let Some(fault) = self.take_fault(call) {
            return Err(fault.into_error());
        }
        self.present_card().map(|_| ())
    }
}

/// Shared handle for scripting a mock reader while a service owns it.
#[derive(Clone, Debug, Default)]
pub struct MockHandle(Arc<Mutex<MockState>>);

impl MockHandle {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, MockState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_card(&self, card: Option<CardInfo>) {
        self.lock().card = card;
    }

    pub fn set_block(&self, block: u8, data: [u8; BLOCK_SIZE]) {
        self.lock().blocks.insert(block, data);
    }

    pub fn block(&self, block: u8) -> Option<[u8; BLOCK_SIZE]> {
        self.lock().blocks.get(&block).copied()
    }

    /// Store one Type 2 page.
    pub fn set_page(&self, page: u8, data: [u8; PAGE_SIZE]) {
        self.lock().pages.insert(page, data);
    }

    /// Remove every stored Type 2 page.
    pub fn clear_pages(&self) {
        self.lock().pages.clear();
    }

    pub fn page(&self, page: u8) -> Option<[u8; PAGE_SIZE]> {
        self.lock().pages.get(&page).copied()
    }

    /// Configure the PWD_AUTH secret and expected PACK.
    pub fn set_password(&self, password: [u8; PAGE_SIZE], pack: [u8; 2]) {
        self.lock().password = Some((password, pack));
    }

    /// Install or replace the scripted Type 4 tag.
    pub fn set_type4_tag(&self, tag: MockType4Tag) {
        self.lock().type4 = Some(tag);
    }

    pub fn type4_tag(&self) -> Option<MockType4Tag> {
        self.lock().type4.clone()
    }

    pub fn type4_ndef(&self) -> Option<Vec<u8>> {
        self.lock().type4.as_ref().map(|tag| tag.ndef.clone())
    }

    pub fn type4_cc(&self) -> Option<Vec<u8>> {
        self.lock().type4.as_ref().map(|tag| tag.cc.clone())
    }

    /// Fail the next matching call. `WriteIgnored` applies to
    /// `MockCall::WriteBlock`, `MockCall::WritePage`, and an UPDATE BINARY
    /// `MockCall::ApduWrite`.
    pub fn fail_next(&self, call: MockCall, fault: MockFault) {
        self.lock().faults.push((call, fault));
    }

    pub fn is_raw_mode(&self) -> bool {
        self.lock().raw_mode
    }

    pub fn is_open(&self) -> bool {
        self.lock().open
    }

    pub fn open_count(&self) -> usize {
        self.lock().open_count
    }

    pub fn close_count(&self) -> usize {
        self.lock().close_count
    }
}

#[derive(Debug)]
pub struct MockReader {
    info: DeviceInfo,
    state: MockHandle,
}

impl MockReader {
    pub fn new(info: DeviceInfo, card: Option<CardInfo>) -> Self {
        let state = MockHandle::new();
        state.set_card(card);
        Self { info, state }
    }

    fn from_handle(info: DeviceInfo, state: MockHandle) -> Self {
        Self { info, state }
    }

    pub fn set_card(&mut self, card: Option<CardInfo>) {
        self.state.set_card(card);
    }

    pub fn set_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) {
        self.state.set_block(block, data);
    }

    pub fn set_page(&mut self, page: u8, data: [u8; PAGE_SIZE]) {
        self.state.set_page(page, data);
    }

    pub fn page(&self, page: u8) -> Option<[u8; PAGE_SIZE]> {
        self.state.page(page)
    }

    pub fn set_password(&mut self, password: [u8; PAGE_SIZE], pack: [u8; 2]) {
        self.state.set_password(password, pack);
    }

    pub fn set_type4_tag(&mut self, tag: MockType4Tag) {
        self.state.set_type4_tag(tag);
    }

    pub fn type4_tag(&self) -> Option<MockType4Tag> {
        self.state.type4_tag()
    }

    pub fn fail_next(&mut self, call: MockCall, fault: MockFault) {
        self.state.fail_next(call, fault);
    }

    pub fn is_raw_mode(&self) -> bool {
        self.state.is_raw_mode()
    }
}

impl Reader for MockReader {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, conn_string: &str) -> Result<()> {
        self.info.conn_string = conn_string.to_owned();
        let mut state = self.state.lock();
        state.open = true;
        state.open_count += 1;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        let mut state = self.state.lock();
        state.open = false;
        state.close_count += 1;
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        let mut state = self.state.lock();
        state.check_call(MockCall::CardInfo)?;
        state.present_card().cloned()
    }

    fn authenticate(&mut self, _block: u8, _key_type: KeyType, _key: Key) -> Result<()> {
        self.state.lock().check_call(MockCall::Authenticate)
    }

    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        let mut state = self.state.lock();
        state.check_call(MockCall::ReadBlock)?;
        state
            .blocks
            .get(&block)
            .copied()
            .ok_or_else(|| NfcError::InvalidArgument(format!("mock block {block} is not loaded")))
    }

    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let mut state = self.state.lock();
        if let Some(fault) = state.take_fault(MockCall::WriteBlock) {
            if matches!(fault, MockFault::WriteIgnored) {
                return Ok(());
            }
            return Err(fault.into_error());
        }
        state.present_card()?;
        state.blocks.insert(block, data);
        Ok(())
    }

    fn read_page_quad(&mut self, start_page: u8) -> Result<[u8; PAGE_QUAD_SIZE]> {
        let mut state = self.state.lock();
        state.check_call(MockCall::ReadPageQuad)?;
        let mut output = [0u8; PAGE_QUAD_SIZE];
        for offset in 0..4u8 {
            let page = start_page.wrapping_add(offset);
            let data = state.pages.get(&page).copied().ok_or_else(|| {
                NfcError::InvalidArgument(format!("mock Type 2 page {page} is not loaded"))
            })?;
            let index = usize::from(offset) * PAGE_SIZE;
            output[index..index + PAGE_SIZE].copy_from_slice(&data);
        }
        Ok(output)
    }

    fn write_page(&mut self, page: u8, data: [u8; PAGE_SIZE]) -> Result<()> {
        let mut state = self.state.lock();
        if let Some(fault) = state.take_fault(MockCall::WritePage) {
            if matches!(fault, MockFault::WriteIgnored) {
                return Ok(());
            }
            return Err(fault.into_error());
        }
        state.present_card()?;
        state.pages.insert(page, data);
        Ok(())
    }

    fn password_auth(&mut self, password: [u8; PAGE_SIZE]) -> Result<[u8; 2]> {
        let mut state = self.state.lock();
        state.check_call(MockCall::PasswordAuth)?;
        match state.password {
            Some((expected, pack)) if expected == password => Ok(pack),
            _ => Err(NfcError::AuthenticationFailed),
        }
    }

    fn apdu_capabilities(&self) -> ApduCapabilities {
        ApduCapabilities {
            supported: self.state.lock().type4.is_some(),
            max_command: 247,
        }
    }

    fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
        let mut state = self.state.lock();
        state.present_card()?;
        let call = if apdu.get(1) == Some(&0xD6) {
            MockCall::ApduWrite
        } else {
            MockCall::Apdu
        };
        let mut ignore_write = false;
        if let Some(fault) = state.take_fault(call) {
            match fault {
                MockFault::WriteIgnored => ignore_write = true,
                other => return Err(other.into_error()),
            }
        }
        let tag = state.type4.as_mut().ok_or(NfcError::Unsupported(
            "the mock reader has no scripted Type 4 tag",
        ))?;
        Ok(tag.handle(apdu, ignore_write))
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        self.state.lock().check_call(MockCall::Transceive)?;
        Ok(Vec::new())
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        self.state.lock().check_call(MockCall::Transceive)?;
        Ok((Vec::new(), 0))
    }

    fn set_raw_mode(&mut self, enabled: bool) -> Result<()> {
        self.state.lock().raw_mode = enabled;
        Ok(())
    }
}

impl TrailerWriter for MockReader {
    fn write_sector_trailer(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        Reader::write_block(self, block, data)
    }
}

impl ManufacturerWriter for MockReader {
    fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        Reader::write_block(self, 0, data)
    }
}

/// Transport that creates `MockReader` instances sharing scripted state.
#[derive(Clone, Debug, Default)]
pub struct MockTransport {
    devices: Arc<Mutex<Vec<(DeviceInfo, MockHandle)>>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(DeviceInfo, MockHandle)>> {
        self.devices.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Add or replace a scripted reader and return its script handle.
    pub fn add(&self, info: DeviceInfo, card: Option<CardInfo>) -> MockHandle {
        let handle = MockHandle::new();
        handle.set_card(card);
        let mut devices = self.lock();
        if let Some((existing, existing_handle)) = devices
            .iter_mut()
            .find(|(existing, _)| existing.conn_string == info.conn_string)
        {
            *existing = info;
            *existing_handle = handle.clone();
        } else {
            devices.push((info, handle.clone()));
        }
        handle
    }

    pub fn handle(&self, conn_string: &str) -> Option<MockHandle> {
        self.lock()
            .iter()
            .find(|(info, _)| info.conn_string == conn_string)
            .map(|(_, handle)| handle.clone())
    }
}

impl Transport for MockTransport {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        Ok(self.lock().iter().map(|(info, _)| info.clone()).collect())
    }

    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>> {
        let (info, handle) = self
            .lock()
            .iter()
            .find(|(info, _)| info.conn_string == conn_string)
            .map(|(info, handle)| (info.clone(), handle.clone()))
            .ok_or_else(|| {
                NfcError::InvalidArgument(format!("mock reader {conn_string} is not available"))
            })?;
        Ok(Box::new(MockReader::from_handle(info, handle)))
    }
}

#[cfg(test)]
mod tests {
    use nfc_core::CardType;

    use super::*;

    fn card(uid: u8) -> CardInfo {
        CardInfo::new(vec![uid; 4], [0x00, 0x04], 0x08, CardType::MifareClassic1K)
    }

    #[test]
    fn scripted_reader_reports_card_changes_and_faults() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        assert_eq!(reader.card_info().expect("card").uid, vec![1; 4]);

        reader.set_card(None);
        assert!(matches!(reader.card_info(), Err(NfcError::NoCard)));

        reader.set_card(Some(card(2)));
        reader.fail_next(MockCall::CardInfo, MockFault::Timeout);
        assert!(matches!(reader.card_info(), Err(NfcError::Timeout(_))));
        assert_eq!(reader.card_info().expect("card").uid, vec![2, 2, 2, 2]);
    }

    #[test]
    fn mock_reader_round_trips_blocks_and_raw_mode() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        let data = [0xAA; BLOCK_SIZE];
        reader.set_block(1, data);
        assert_eq!(reader.read_block(1).expect("block"), data);
        reader.write_sector_trailer(2, data).expect("trailer");
        assert_eq!(reader.read_block(2).expect("trailer block"), data);
        reader.write_manufacturer_block(data).expect("manufacturer");
        assert_eq!(reader.read_block(0).expect("manufacturer block"), data);
        assert!(!reader.is_raw_mode());
        reader.set_raw_mode(true).expect("raw mode");
        assert!(reader.is_raw_mode());
    }

    #[test]
    fn ignored_write_keeps_the_previous_block() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        let original = [0x11; BLOCK_SIZE];
        reader.set_block(1, original);
        reader.fail_next(MockCall::WriteBlock, MockFault::WriteIgnored);
        reader.write_block(1, [0xAA; BLOCK_SIZE]).expect("write");
        assert_eq!(reader.read_block(1).expect("block"), original);
    }

    #[test]
    fn mock_transport_shares_state_with_opened_readers() {
        let transport = MockTransport::new();
        let handle = transport.add(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        let mut reader = transport.open("mock:0").expect("open");
        assert_eq!(
            transport.enumerate().expect("enumerate"),
            vec![DeviceInfo::new("mock", "mock:0")]
        );

        handle.set_card(None);
        assert!(matches!(reader.card_info(), Err(NfcError::NoCard)));
        assert!(matches!(
            transport.open("mock:1"),
            Err(NfcError::InvalidArgument(_))
        ));
    }
}
