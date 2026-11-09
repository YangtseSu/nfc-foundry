//! Card-family-neutral reader transport boundary.

pub mod device;
pub mod mock;

pub use device::{DeviceInfo, ReaderCapabilities};

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, Result};

/// A reader opened through an exact connstring. One reader is owned by one
/// operation at a time; `nfc-app` enforces that lease. `Transport::open`
/// constructs the reader and `Reader::open` performs the device I/O, so a
/// transport factory never touches hardware by itself.
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

/// A driver factory. Enumeration returns exact connstrings; opening a
/// connstring yields a reader of that device only. The returned reader is not
/// connected until `Reader::open` succeeds.
pub trait Transport: Send + Sync {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>>;
    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>>;
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

/// Transport used by callers that have no reader backend yet. It reports no
/// devices and refuses every open instead of inventing a device.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTransport;

impl Transport for NoTransport {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        Ok(Vec::new())
    }

    fn open(&self, _conn_string: &str) -> Result<Box<dyn Reader>> {
        Err(nfc_core::NfcError::Unsupported(
            "no reader backend is configured",
        ))
    }
}

#[cfg(test)]
mod tests {
    use nfc_core::{BLOCK_SIZE, CardInfo, CardType};

    use super::*;
    use crate::mock::{MockReader, MockTransport};

    fn card() -> CardInfo {
        CardInfo::new(
            vec![1, 2, 3, 4],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )
    }

    #[test]
    fn reader_transport_and_capability_traits_are_object_safe() {
        let mut reader: Box<dyn Reader> = Box::new(MockReader::new(
            DeviceInfo::new("mock", "mock:0"),
            Some(card()),
        ));
        assert_eq!(reader.card_info().expect("card").uid, vec![1, 2, 3, 4]);
        assert!(matches!(
            reader.read_block(1),
            Err(nfc_core::NfcError::InvalidArgument(_))
        ));

        let mut trailer: Box<dyn TrailerWriter> = Box::new(MockReader::new(
            DeviceInfo::new("mock", "mock:0"),
            Some(card()),
        ));
        trailer
            .write_sector_trailer(1, [0xAA; BLOCK_SIZE])
            .expect("trailer write");

        let mut manufacturer: Box<dyn ManufacturerWriter> = Box::new(MockReader::new(
            DeviceInfo::new("mock", "mock:0"),
            Some(card()),
        ));
        manufacturer
            .write_manufacturer_block([0xBB; BLOCK_SIZE])
            .expect("manufacturer write");

        let transport: Box<dyn Transport> = Box::new(MockTransport::new());
        assert!(transport.enumerate().expect("enumerate").is_empty());
    }

    #[test]
    fn no_transport_refuses_to_open() {
        let transport = NoTransport;
        assert!(transport.enumerate().expect("enumerate").is_empty());
        assert!(matches!(
            transport.open("mock:0"),
            Err(nfc_core::NfcError::Unsupported(_))
        ));
    }
}
