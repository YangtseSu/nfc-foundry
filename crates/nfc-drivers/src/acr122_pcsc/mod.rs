//! ACR122U over PC/SC.
//!
//! This is a second transport for the same reader, not generic PC/SC support:
//! only reader names containing `ACR122` are enumerated, and any other PC/SC
//! reader is refused. The reader name is the exact connstring target:
//! `acr122_pcsc:<reader name>`.
//!
//! `pcscd` must be running for this path. The direct USB path claims the same
//! CCID interface, so the two transports are never used simultaneously and
//! never fall back to each other.

mod link;

use std::fmt;

use nfc_core::Result;
use nfc_transport::{ApduCapabilities, DeviceInfo, ReaderCapabilities};
use pcsc::{Context, Error, Scope};

use crate::acr122::link::AcrLink;
use crate::acr122::reader::{Acr122Reader, forward_acr122_reader};
use link::{PcscLink, service_error};

pub const READER_NAME: &str = "ACR122U PC/SC";

/// ACR122U PC/SC reader.
///
/// Construction performs no I/O. `Reader::open` requires the exact
/// `acr122_pcsc:<reader name>` connstring and connects through `pcscd`.
pub struct Acr122Pcsc {
    core: Acr122Reader,
}

impl fmt::Debug for Acr122Pcsc {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.core.fmt(formatter)
    }
}

impl Acr122Pcsc {
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

    /// Return the selected card to IDLE without power-cycling the field.
    pub fn release_card(&mut self) -> Result<()> {
        self.core.release_card()
    }

    /// Power-cycle the RF field so the card returns to IDLE.
    pub fn cycle_rf_field(&mut self) -> Result<()> {
        self.core.cycle_rf_field()
    }

    /// List ACR122U readers visible to `pcscd` as exact named connstrings.
    ///
    /// A stopped or missing PC/SC service yields no devices, never an error
    /// that would hide the direct USB path.
    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        let context = match Context::establish(Scope::User) {
            Ok(context) => context,
            Err(error) if service_unavailable(&error) => return Ok(Vec::new()),
            Err(error) => return Err(service_error(error)),
        };
        let readers = match context.list_readers_owned() {
            Ok(readers) => readers,
            Err(error) if service_unavailable(&error) => return Ok(Vec::new()),
            Err(error) => {
                return Err(nfc_core::NfcError::Io(format!(
                    "PC/SC reader enumeration failed: {error}"
                )));
            }
        };
        Ok(readers
            .iter()
            .filter(|reader| is_acr122_reader(&reader.to_string_lossy()))
            .map(|reader| {
                DeviceInfo::new(
                    READER_NAME,
                    format!("{}{}", crate::ACR122_PCSC_PREFIX, reader.to_string_lossy()),
                )
            })
            .collect())
    }
}

forward_acr122_reader!(Acr122Pcsc, |conn_string| {
    Ok(Box::new(PcscLink::connect(conn_string)?) as Box<dyn AcrLink>)
});

/// PC/SC availability, service state, and matched ACR122U reader names.
pub fn diagnose() -> crate::PcscStatus {
    let context = match Context::establish(Scope::User) {
        Ok(context) => context,
        Err(error) => {
            return crate::PcscStatus {
                enabled: true,
                available: false,
                error: Some(service_error(error).to_string()),
                readers: Vec::new(),
            };
        }
    };
    match context.list_readers_owned() {
        Ok(readers) => crate::PcscStatus {
            enabled: true,
            available: true,
            error: None,
            readers: readers
                .iter()
                .filter(|reader| is_acr122_reader(&reader.to_string_lossy()))
                .map(|reader| reader.to_string_lossy().into_owned())
                .collect(),
        },
        Err(error) => crate::PcscStatus {
            enabled: true,
            available: true,
            error: Some(format!("PC/SC reader enumeration failed: {error}")),
            readers: Vec::new(),
        },
    }
}

/// A reader name that belongs to the ACR122U family.
pub(crate) fn is_acr122_reader(name: &str) -> bool {
    name.to_ascii_lowercase().contains("acr122")
}

fn service_unavailable(error: &Error) -> bool {
    matches!(
        error,
        Error::NoService | Error::ServiceStopped | Error::NoReadersAvailable | Error::Shutdown
    )
}

#[cfg(test)]
mod tests {
    use nfc_transport::Reader;

    use super::*;

    #[test]
    fn matches_only_acr122_reader_names() {
        assert!(is_acr122_reader("ACS ACR122U 00 00"));
        assert!(is_acr122_reader("acs acr122 00 00"));
        assert!(!is_acr122_reader("Yubico YubiKey OTP+FIDO+CCID"));
        assert!(!is_acr122_reader("ACS ACR39U ICC Reader"));
    }

    #[test]
    fn closed_reader_reports_device_facts_without_io() {
        let info = DeviceInfo::new(READER_NAME, "acr122_pcsc:ACS ACR122U 00 00");
        let reader = Acr122Pcsc::new(info);
        assert_eq!(reader.device().name, READER_NAME);
        assert!(!format!("{reader:?}").contains("open: true"));
    }
}
