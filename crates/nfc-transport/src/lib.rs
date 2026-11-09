//! Card-family-neutral reader transport boundary.

pub mod device;
pub mod mock;

pub use device::{DeviceInfo, ReaderCapabilities};

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, Result};

/// Recovery-specific transport facts. A driver only advertises parity
/// exchange after the primitive works on hardware.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoveryCapabilities {
    pub parity_exchange: bool,
}

/// One raw ISO/IEC 14443A frame exchanged with parity handled by the caller.
/// `parity` has one bit per data byte in the least significant bit; `bits` is
/// the number of data bits in `data` for a partial frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParityFrame {
    pub data: Vec<u8>,
    pub bits: usize,
    pub parity: Vec<u8>,
}

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

    /// Recovery parity exchange is opt-in; readers without it keep the default
    /// error instead of pretending to support active attacks.
    fn recovery_capabilities(&self) -> RecoveryCapabilities {
        RecoveryCapabilities::default()
    }

    /// Enter or leave the parity-disabled framing mode used by Crypto1 partial
    /// authentication. Ordinary and raw byte modes are unavailable while this
    /// is enabled.
    fn set_recovery_mode(&mut self, _enabled: bool) -> Result<()> {
        Err(NfcError::Unsupported(
            "this reader has no recovery parity mode",
        ))
    }

    /// Send one complete byte frame with caller-supplied parity and return the
    /// response data, received bit count, and response parity bits.
    fn transceive_parity(&mut self, _data: &[u8], _parity: &[u8]) -> Result<ParityFrame> {
        Err(NfcError::Unsupported(
            "this reader has no recovery parity exchange",
        ))
    }

    /// Parity exchange with a known response frame size. Drivers may skip the
    /// trailing-bits reset and status read so the next authentication frame is
    /// transmitted inside the card's response window.
    fn transceive_parity_expect(
        &mut self,
        data: &[u8],
        parity: &[u8],
        expected_bits: usize,
    ) -> Result<ParityFrame> {
        let _ = expected_bits;
        self.transceive_parity(data, parity)
    }
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
