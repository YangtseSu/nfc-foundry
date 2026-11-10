//! ACR122U over direct USB CCID.
//!
//! The reader exposes an internal PN532 through the ACR122 pseudo-APDU. Card
//! selection, MIFARE Classic authentication and block access, raw byte/bit
//! exchange, and the Gen1A manufacturer-block path use that command channel.
//! PC/SC is intentionally not used, so the raw bit and parity behavior is the
//! same on every supported host.

mod ccid;
mod protocol;

use std::fmt;
use std::thread;
use std::time::Duration;

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, Result};
use nfc_transport::{DeviceInfo, ManufacturerWriter, ParityFrame, Reader, RecoveryCapabilities};

use ccid::{ACR122_READER_NAME, CcidTransport};
use protocol::{Acr122Config, Acr122Protocol};

pub use crate::pn53x::FirmwareVersion;

pub(crate) const READER_NAME: &str = ACR122_READER_NAME;

/// ACR122U USB reader.
///
/// Construction performs no I/O. `Reader::open` selects the exact
/// `acr122_usb:<bus>:<address>` device, claims its CCID interface, powers the
/// ICC, and validates the internal PN532 firmware.
pub struct Acr122Usb {
    info: DeviceInfo,
    config: Acr122Config,
    protocol: Option<Acr122Protocol>,
}

impl fmt::Debug for Acr122Usb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Acr122Usb")
            .field("info", &self.info)
            .field("open", &self.protocol.is_some())
            .finish()
    }
}

impl Acr122Usb {
    pub fn new(info: DeviceInfo) -> Self {
        Self {
            info,
            config: Acr122Config::default(),
            protocol: None,
        }
    }

    /// Firmware identity of the open reader, if it is open.
    pub fn firmware(&self) -> Option<FirmwareVersion> {
        self.protocol.as_ref().map(Acr122Protocol::firmware)
    }

    /// List attached ACR122U readers as exact bus/address connstrings.
    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        CcidTransport::enumerate()
    }

    /// Return the selected card to IDLE without power-cycling the field.
    pub fn release_card(&mut self) -> Result<()> {
        self.protocol()?.release_card()
    }

    /// Power-cycle the RF field so the card returns to IDLE.
    pub fn cycle_rf_field(&mut self) -> Result<()> {
        self.protocol()?.cycle_rf_field()
    }

    fn open_path(conn_string: &str) -> Result<&str> {
        let target = conn_string
            .strip_prefix(crate::ACR122_USB_PREFIX)
            .ok_or_else(|| {
                NfcError::InvalidArgument(format!(
                    "{conn_string} is not a {} connstring",
                    crate::ACR122_USB_PREFIX
                ))
            })?;
        if target.is_empty() {
            return Err(NfcError::InvalidArgument(format!(
                "reader connstring {conn_string} has no bus:address target"
            )));
        }
        Ok(target)
    }

    fn protocol(&mut self) -> Result<&mut Acr122Protocol> {
        self.protocol
            .as_mut()
            .ok_or_else(|| NfcError::Io("the ACR122U reader is not open".to_owned()))
    }
}

impl Reader for Acr122Usb {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, conn_string: &str) -> Result<()> {
        Self::open_path(conn_string)?;
        if self.protocol.is_some() {
            self.close()?;
        }
        let mut last_error = None;
        for attempt in 0..self.config.attempts.max(1) {
            let transport = CcidTransport::open(conn_string)?;
            match Acr122Protocol::open(Box::new(transport), self.config.clone()) {
                Ok(protocol) => {
                    self.protocol = Some(protocol);
                    self.info.conn_string = conn_string.to_owned();
                    return Ok(());
                }
                Err(error @ (NfcError::Timeout(_) | NfcError::Io(_)))
                    if attempt + 1 < self.config.attempts.max(1) =>
                {
                    last_error = Some(error);
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or(NfcError::Unsupported(
            "the ACR122U did not respond as a PN532",
        )))
    }

    fn close(&mut self) -> Result<()> {
        if let Some(mut protocol) = self.protocol.take() {
            protocol.shutdown();
        }
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        self.protocol()?.select_card()
    }

    fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()> {
        self.protocol()?.authenticate(block, key_type, key)
    }

    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        self.protocol()?.read_block(block)
    }

    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.write_block(block, data)
    }

    fn read_page_quad(&mut self, start_page: u8) -> Result<[u8; nfc_core::PAGE_QUAD_SIZE]> {
        self.protocol()?.read_page_quad(start_page)
    }

    fn write_page(&mut self, page: u8, data: [u8; nfc_core::PAGE_SIZE]) -> Result<()> {
        self.protocol()?.write_page(page, data)
    }

    fn password_auth(&mut self, password: [u8; nfc_core::PAGE_SIZE]) -> Result<[u8; 2]> {
        self.protocol()?.password_auth(password)
    }

    fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>> {
        self.protocol()?.transceive_bytes(tx)
    }

    fn transceive_bits(&mut self, tx: &[u8], tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        self.protocol()?.transceive_bits(tx, tx_bits)
    }

    fn set_raw_mode(&mut self, enabled: bool) -> Result<()> {
        self.protocol()?.set_raw_mode(enabled)
    }

    fn recovery_capabilities(&self) -> RecoveryCapabilities {
        RecoveryCapabilities {
            parity_exchange: self.protocol.is_some(),
        }
    }

    fn set_recovery_mode(&mut self, enabled: bool) -> Result<()> {
        self.protocol()?.set_recovery_mode(enabled)
    }

    fn transceive_parity(&mut self, data: &[u8], parity: &[u8]) -> Result<ParityFrame> {
        self.protocol()?.transceive_parity(data, parity)
    }

    fn transceive_parity_bits(
        &mut self,
        data: &[u8],
        data_bits: usize,
        parity: &[u8],
    ) -> Result<ParityFrame> {
        self.protocol()?
            .transceive_parity_bits(data, data_bits, parity)
    }

    fn transceive_parity_expect(
        &mut self,
        data: &[u8],
        parity: &[u8],
        expected_bits: usize,
    ) -> Result<ParityFrame> {
        self.protocol()?
            .transceive_parity_expect(data, parity, expected_bits)
    }
}

impl ManufacturerWriter for Acr122Usb {
    fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.write_manufacturer_block(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader() -> Acr122Usb {
        Acr122Usb::new(DeviceInfo::new(READER_NAME, "acr122_usb:001:017"))
    }

    #[test]
    fn rejects_connstrings_without_an_exact_usb_target() {
        let mut reader = reader();
        assert!(matches!(
            reader.open("mock:0"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            reader.open("acr122_usb:"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            reader.open("pn532_uart:/dev/ttyUSB0"),
            Err(NfcError::InvalidArgument(_))
        ));
    }

    #[test]
    fn unopened_reader_reports_closed_state() {
        let mut reader = reader();
        assert!(matches!(reader.card_info(), Err(NfcError::Io(_))));
        assert!(matches!(reader.read_block(4), Err(NfcError::Io(_))));
        assert!(matches!(
            reader.write_manufacturer_block([0u8; BLOCK_SIZE]),
            Err(NfcError::Io(_))
        ));
        reader.close().expect("closing an unopened reader");
    }
}
