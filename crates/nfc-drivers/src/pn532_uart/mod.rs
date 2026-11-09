//! PN532 over a serial UART.
//!
//! `InListPassiveTarget` selects one ISO/IEC 14443A target;
//! `InDataExchange` carries MIFARE Classic authentication, reads, and writes.
//! Raw byte and bit frames use `InCommunicateThru` and are only available after
//! `set_raw_mode(true)`, which disables hardware CRC handling while the PN532
//! still generates and checks parity.

mod frame;
mod link;
mod protocol;

use std::time::Duration;

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, Result};
use nfc_transport::{DeviceInfo, ParityFrame, Reader, RecoveryCapabilities, TrailerWriter};

use crate::PN532_UART_PREFIX;
use link::SerialLink;
use protocol::{Pn532Config, Pn532Protocol};

pub use crate::pn53x::FirmwareVersion;

pub(crate) const READER_NAME: &str = "PN532 UART";
const INITIAL_SERIAL_TIMEOUT: Duration = Duration::from_millis(50);

/// A PN532 attached to a serial port.
///
/// Construction is cheap and performs no I/O. `Reader::open` opens the exact
/// serial port from the connstring and validates the PN532 firmware before any
/// card operation.
pub struct Pn532Uart {
    info: DeviceInfo,
    config: Pn532Config,
    protocol: Option<Pn532Protocol>,
}

impl Pn532Uart {
    pub fn new(info: DeviceInfo) -> Self {
        Self {
            info,
            config: Pn532Config::default(),
            protocol: None,
        }
    }

    /// Firmware identity of the open reader, if it is open.
    pub fn firmware(&self) -> Option<FirmwareVersion> {
        self.protocol.as_ref().map(Pn532Protocol::firmware)
    }

    /// Power-cycle the RF field so an already selected card returns to idle.
    pub fn cycle_rf_field(&mut self) -> Result<()> {
        self.protocol()?.cycle_rf_field()
    }

    /// List every serial port as a PN532 candidate. A UART adapter cannot be
    /// identified as a PN532 without opening it, so selection stays explicit.
    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        let ports = serialport::available_ports()
            .map_err(|error| NfcError::Io(format!("serial port enumeration failed: {error}")))?;
        let mut devices: Vec<DeviceInfo> = ports
            .into_iter()
            .map(|port| port_device(&port.port_name))
            .collect();
        devices.sort_by(|left, right| left.conn_string.cmp(&right.conn_string));
        devices.dedup_by(|left, right| left.conn_string == right.conn_string);
        Ok(devices)
    }

    fn open_path(conn_string: &str) -> Result<&str> {
        let target = conn_string.strip_prefix(PN532_UART_PREFIX).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "{conn_string} is not a {PN532_UART_PREFIX} connstring"
            ))
        })?;
        if target.is_empty() {
            return Err(NfcError::InvalidArgument(format!(
                "reader connstring {conn_string} has no serial port"
            )));
        }
        Ok(target)
    }

    fn protocol(&mut self) -> Result<&mut Pn532Protocol> {
        self.protocol
            .as_mut()
            .ok_or_else(|| NfcError::Io("the PN532 reader is not open".to_owned()))
    }
}

fn port_device(port_name: &str) -> DeviceInfo {
    DeviceInfo::new(READER_NAME, format!("{PN532_UART_PREFIX}{port_name}"))
}

impl Reader for Pn532Uart {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, conn_string: &str) -> Result<()> {
        let path = Self::open_path(conn_string)?.to_owned();
        if self.protocol.is_some() {
            self.close()?;
        }
        let port = SerialLink::open(&path, self.config.baud_rate, INITIAL_SERIAL_TIMEOUT)?;
        self.protocol = Some(Pn532Protocol::open(Box::new(port), self.config.clone())?);
        self.info.conn_string = conn_string.to_owned();
        Ok(())
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

impl TrailerWriter for Pn532Uart {
    fn write_sector_trailer(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.write_block(block, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader() -> Pn532Uart {
        Pn532Uart::new(DeviceInfo::new(READER_NAME, "pn532_uart:/dev/ttyUSB0"))
    }

    #[test]
    fn maps_serial_ports_to_exact_connstrings() {
        assert_eq!(
            port_device("/dev/ttyUSB0"),
            DeviceInfo::new(READER_NAME, "pn532_uart:/dev/ttyUSB0")
        );
        assert_eq!(
            port_device("ttyAMA0"),
            DeviceInfo::new(READER_NAME, "pn532_uart:ttyAMA0")
        );
    }

    #[test]
    fn rejects_connstrings_without_a_serial_port() {
        let mut reader = reader();
        assert!(matches!(
            reader.open("mock:0"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            reader.open("pn532_uart:"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            reader.open("acr122_usb:1-2"),
            Err(NfcError::InvalidArgument(_))
        ));
    }

    #[test]
    fn unopened_reader_reports_closed_state() {
        let mut reader = reader();
        assert!(matches!(reader.card_info(), Err(NfcError::Io(_))));
        assert!(matches!(reader.read_block(4), Err(NfcError::Io(_))));
        reader.close().expect("closing an unopened reader");
    }
}
