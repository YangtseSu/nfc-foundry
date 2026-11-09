//! Scripted reader and transport used by application and workflow tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, Result};

use crate::{DeviceInfo, ManufacturerWriter, Reader, TrailerWriter, Transport};

/// Reader entry points that can be scripted with a fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockCall {
    CardInfo,
    Authenticate,
    ReadBlock,
    WriteBlock,
    Transceive,
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

#[derive(Debug, Default)]
struct MockState {
    card: Option<CardInfo>,
    blocks: HashMap<u8, [u8; BLOCK_SIZE]>,
    raw_mode: bool,
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

    /// Fail the next matching call. `WriteIgnored` applies to
    /// `MockCall::WriteBlock`.
    pub fn fail_next(&self, call: MockCall, fault: MockFault) {
        self.lock().faults.push((call, fault));
    }

    pub fn is_raw_mode(&self) -> bool {
        self.lock().raw_mode
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
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
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
