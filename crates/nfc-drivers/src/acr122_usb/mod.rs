//! ACR122U over direct USB CCID.
//!
//! The reader exposes an internal PN532 through the ACR122 pseudo-APDU. Card
//! selection, MIFARE Classic authentication and block access, raw byte/bit
//! exchange, and the Gen1A manufacturer-block path use that command channel.
//! PC/SC is not used on this path, so raw bit and parity behavior does not
//! depend on the smart-card service.

mod ccid;

use std::fmt;

use nfc_core::Result;
use nfc_transport::{ApduCapabilities, DeviceInfo, ReaderCapabilities};

use crate::acr122::link::AcrLink;
use crate::acr122::reader::{Acr122Reader, forward_acr122_reader};
use ccid::{ACR122_READER_NAME, CcidTransport};

pub use crate::pn53x::FirmwareVersion;

pub(crate) const READER_NAME: &str = ACR122_READER_NAME;

/// ACR122U USB reader.
///
/// Construction performs no I/O. `Reader::open` selects the exact
/// `acr122_usb:<bus>:<address>` device, claims its CCID interface, powers the
/// ICC, and validates the internal PN532 firmware.
pub struct Acr122Usb {
    core: Acr122Reader,
}

impl fmt::Debug for Acr122Usb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.core.fmt(formatter)
    }
}

impl Acr122Usb {
    pub fn new(info: DeviceInfo) -> Self {
        Self {
            core: Acr122Reader::new(
                info,
                ReaderCapabilities {
                    classic_read: true,
                    classic_write: true,
                    raw_bytes: true,
                    raw_bits: true,
                    manufacturer_write: true,
                    manufacturer_seal: true,
                    type2_read: true,
                    type2_write: true,
                    recovery_parity: true,
                },
                ApduCapabilities {
                    supported: true,
                    max_command: crate::pn53x::MAX_ISO_DEP_APDU,
                },
            ),
        }
    }

    /// Firmware identity of the open reader, if it is open.
    pub fn firmware(&self) -> Option<FirmwareVersion> {
        self.core.firmware()
    }

    /// List attached ACR122U readers as exact bus/address connstrings.
    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        CcidTransport::enumerate()
    }

    /// Return the selected card to IDLE without power-cycling the field.
    pub fn release_card(&mut self) -> Result<()> {
        self.core.release_card()
    }

    /// Power-cycle the RF field so the card returns to IDLE.
    pub fn cycle_rf_field(&mut self) -> Result<()> {
        self.core.cycle_rf_field()
    }
}

forward_acr122_reader!(Acr122Usb, |conn_string| {
    Ok(Box::new(CcidTransport::open(conn_string)?) as Box<dyn AcrLink>)
});

#[cfg(test)]
mod tests {
    use super::*;
    use nfc_core::NfcError;
    use nfc_transport::{ManufacturerWriter, Reader};

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
            reader.write_manufacturer_block([0u8; nfc_core::BLOCK_SIZE]),
            Err(NfcError::Io(_))
        ));
        reader.close().expect("closing an unopened reader");
    }

    #[test]
    fn firmware_label_starts_empty() {
        let reader = reader();
        assert!(reader.firmware().is_none());
    }

    #[cfg(feature = "acr122-pcsc")]
    #[test]
    fn pcsc_connstring_is_not_a_usb_target() {
        let mut reader = reader();
        assert!(matches!(
            reader.open("acr122_pcsc:ACS ACR122U 00 00"),
            Err(NfcError::InvalidArgument(_))
        ));
    }
}
