//! Concrete reader drivers. Only PN532 UART and ACR122U USB/CCID are in scope.

pub mod acr122_usb;
mod gen1a;
pub mod pn532_uart;
mod pn53x;

use nfc_core::{NfcError, Result};
use nfc_transport::{DeviceInfo, Reader, Transport};

use crate::acr122_usb::{Acr122Usb, READER_NAME as ACR122_READER_NAME};
use crate::pn532_uart::{Pn532Uart, READER_NAME};

pub const PN532_UART_PREFIX: &str = "pn532_uart:";
pub const ACR122_USB_PREFIX: &str = "acr122_usb:";

/// Enumeration and open entry point for both supported driver families.
#[derive(Clone, Copy, Debug, Default)]
pub struct DriverRegistry;

impl DriverRegistry {
    pub fn new() -> Self {
        Self
    }

    fn classify(conn_string: &str) -> Result<&'static str> {
        for (name, prefix) in [
            (READER_NAME, PN532_UART_PREFIX),
            (ACR122_READER_NAME, ACR122_USB_PREFIX),
        ] {
            if let Some(target) = conn_string.strip_prefix(prefix) {
                if target.is_empty() {
                    return Err(NfcError::InvalidArgument(format!(
                        "reader connstring {conn_string} has no target"
                    )));
                }
                return Ok(name);
            }
        }
        Err(NfcError::Unsupported("unknown reader connstring"))
    }
}

impl Transport for DriverRegistry {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        let mut devices = Vec::new();
        collect_available(&mut devices, Pn532Uart::enumerate())?;
        collect_available(&mut devices, Acr122Usb::enumerate())?;
        Ok(devices)
    }

    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>> {
        let info = DeviceInfo::new(Self::classify(conn_string)?, conn_string);
        if conn_string.starts_with(PN532_UART_PREFIX) {
            Ok(Box::new(Pn532Uart::new(info)))
        } else {
            Ok(Box::new(Acr122Usb::new(info)))
        }
    }
}

/// A driver that is not implemented yet contributes no devices; any other
/// enumeration error remains visible.
fn collect_available(devices: &mut Vec<DeviceInfo>, result: Result<Vec<DeviceInfo>>) -> Result<()> {
    match result {
        Ok(found) => {
            devices.extend(found);
            Ok(())
        }
        Err(NfcError::NotImplemented(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_opens_known_connstrings_only() {
        let registry = DriverRegistry;
        assert_eq!(
            registry
                .open("pn532_uart:/dev/ttyUSB0")
                .expect("pn532 reader")
                .device()
                .name,
            READER_NAME
        );
        assert_eq!(
            registry
                .open("acr122_usb:1-2")
                .expect("acr reader")
                .device()
                .name,
            ACR122_READER_NAME
        );
        assert!(matches!(
            registry.open("pn532_uart:"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            registry.open("pcsc:0"),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[test]
    fn unfinished_drivers_are_skipped_but_real_errors_surface() {
        let mut devices = Vec::new();
        collect_available(&mut devices, Err(NfcError::NotImplemented("test driver")))
            .expect("not implemented is skipped");
        assert!(devices.is_empty());

        assert!(matches!(
            collect_available(&mut devices, Err(NfcError::Io("broken".to_owned()))),
            Err(NfcError::Io(_))
        ));
    }
}
