use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, Result};
use nfc_transport::{DeviceInfo, Reader};

/// PN532 over a serial UART. The protocol worker is implemented in phase 02.
#[derive(Debug)]
pub struct Pn532Uart {
    info: DeviceInfo,
}

impl Pn532Uart {
    pub fn new(info: DeviceInfo) -> Self {
        Self { info }
    }

    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        Err(NfcError::NotImplemented("PN532 UART enumeration"))
    }
}

impl Reader for Pn532Uart {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, _conn_string: &str) -> Result<()> {
        Err(NfcError::NotImplemented("PN532 UART open"))
    }

    fn close(&mut self) -> Result<()> {
        Err(NfcError::NotImplemented("PN532 UART close"))
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        Err(NfcError::NotImplemented("PN532 UART card selection"))
    }

    fn authenticate(&mut self, _block: u8, _key_type: KeyType, _key: Key) -> Result<()> {
        Err(NfcError::NotImplemented("PN532 UART authentication"))
    }

    fn read_block(&mut self, _block: u8) -> Result<[u8; BLOCK_SIZE]> {
        Err(NfcError::NotImplemented("PN532 UART block read"))
    }

    fn write_block(&mut self, _block: u8, _data: [u8; BLOCK_SIZE]) -> Result<()> {
        Err(NfcError::NotImplemented("PN532 UART block write"))
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        Err(NfcError::NotImplemented("PN532 UART raw bytes"))
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Err(NfcError::NotImplemented("PN532 UART raw bits"))
    }

    fn set_raw_mode(&mut self, _enabled: bool) -> Result<()> {
        Err(NfcError::NotImplemented("PN532 UART raw mode"))
    }
}
