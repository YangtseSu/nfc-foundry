use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, Result};
use nfc_transport::{DeviceInfo, Reader};

/// ACR122U over direct USB/CCID. PC/SC is intentionally not used.
#[derive(Debug)]
pub struct Acr122Usb {
    info: DeviceInfo,
}

impl Acr122Usb {
    pub fn new(info: DeviceInfo) -> Self {
        Self { info }
    }

    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        Err(NfcError::NotImplemented("ACR122U USB enumeration"))
    }
}

impl Reader for Acr122Usb {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, _conn_string: &str) -> Result<()> {
        Err(NfcError::NotImplemented("ACR122U USB open"))
    }

    fn close(&mut self) -> Result<()> {
        Err(NfcError::NotImplemented("ACR122U USB close"))
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        Err(NfcError::NotImplemented("ACR122U USB card selection"))
    }

    fn authenticate(&mut self, _block: u8, _key_type: KeyType, _key: Key) -> Result<()> {
        Err(NfcError::NotImplemented("ACR122U USB authentication"))
    }

    fn read_block(&mut self, _block: u8) -> Result<[u8; BLOCK_SIZE]> {
        Err(NfcError::NotImplemented("ACR122U USB block read"))
    }

    fn write_block(&mut self, _block: u8, _data: [u8; BLOCK_SIZE]) -> Result<()> {
        Err(NfcError::NotImplemented("ACR122U USB block write"))
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        Err(NfcError::NotImplemented("ACR122U USB raw bytes"))
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Err(NfcError::NotImplemented("ACR122U USB raw bits"))
    }

    fn set_raw_mode(&mut self, _enabled: bool) -> Result<()> {
        Err(NfcError::NotImplemented("ACR122U USB raw mode"))
    }
}
