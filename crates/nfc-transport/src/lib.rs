//! Card-family-neutral reader transport boundary.

pub mod device;
pub mod mock;

pub use device::{DeviceInfo, ReaderCapabilities};

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, Result};

/// A reader opened through an exact connstring. One reader is owned by one
/// operation at a time; `nfc-app` enforces that lease.
pub trait Reader: Send {
    fn device(&self) -> &DeviceInfo;
    fn open(&mut self, conn_string: &str) -> Result<()>;
    fn close(&mut self) -> Result<()>;
    fn card_info(&mut self) -> Result<CardInfo>;
    fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()>;
    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]>;
    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()>;
    fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>>;
    fn transceive_bits(&mut self, tx: &[u8], tx_bits: usize) -> Result<(Vec<u8>, usize)>;
    fn set_raw_mode(&mut self, enabled: bool) -> Result<()>;
}

/// Capability for writing a complete sector trailer. Kept separate because not
/// every reader exposes a protected trailer command.
pub trait TrailerWriter: Reader {
    fn write_sector_trailer(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()>;
}

/// Capability for writing the manufacturer block, used only by the guarded
/// special-card UID/clone path.
pub trait ManufacturerWriter: Reader {
    fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()>;
}
