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
use nfc_transport::{
    ApduCapabilities, ApduResponse, DeviceInfo, ManufacturerSealer, ManufacturerWriter,
    ParityFrame, Reader, ReaderCapabilities, TrailerWriter,
};

use crate::PN532_UART_PREFIX;
use crate::pn53x::MAX_ISO_DEP_APDU;
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

    /// List serial ports as PN532 candidates with USB adapters first. A UART
    /// adapter cannot be identified as a PN532 without opening it, so
    /// selection stays explicit.
    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        let ports = serialport::available_ports()
            .map_err(|error| NfcError::Io(format!("serial port enumeration failed: {error}")))?;
        Ok(candidates(ports))
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
        .with_physical_id(physical_id(port_name))
}

/// Lock identity for a serial port. A `/dev/serial/by-id` entry is preferred
/// because it survives re-enumeration; otherwise the canonical device path is
/// used, falling back to the path from the connstring when the node is absent.
fn physical_id(port_name: &str) -> String {
    physical_id_in(std::path::Path::new("/dev/serial/by-id"), port_name)
}

fn physical_id_in(by_id_dir: &std::path::Path, port_name: &str) -> String {
    use std::path::PathBuf;

    let canonical = std::fs::canonicalize(port_name).ok();
    if let Some(canonical) = &canonical
        && let Ok(entries) = std::fs::read_dir(by_id_dir)
    {
        let mut matches = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| std::fs::canonicalize(path).is_ok_and(|target| target == *canonical))
            .collect::<Vec<PathBuf>>();
        matches.sort();
        if let Some(path) = matches.into_iter().next() {
            return format!("{PN532_UART_PREFIX}{}", path.display());
        }
    }
    let path = canonical.unwrap_or_else(|| PathBuf::from(port_name));
    format!("{PN532_UART_PREFIX}{}", path.display())
}

/// Convert enumerated ports into candidates, dropping legacy UART nodes with
/// no detected hardware and ordering USB serial adapters first. Sorting is
/// stable by connstring within each group so the automatic selection is
/// deterministic.
fn candidates(ports: Vec<serialport::SerialPortInfo>) -> Vec<DeviceInfo> {
    let mut candidates: Vec<(bool, DeviceInfo)> = ports
        .into_iter()
        .filter(|port| !is_unbacked_legacy_uart(&port.port_name))
        .map(|port| {
            let usb = matches!(port.port_type, serialport::SerialPortType::UsbPort(_));
            (usb, port_device(&port.port_name))
        })
        .collect();
    candidates.sort_by(|(left_usb, left), (right_usb, right)| {
        right_usb
            .cmp(left_usb)
            .then_with(|| left.conn_string.cmp(&right.conn_string))
    });
    candidates.dedup_by(|(_, left), (_, right)| left.conn_string == right.conn_string);
    candidates.into_iter().map(|(_, device)| device).collect()
}

/// Legacy `/dev/ttyS*` nodes exist for every possible 8250 UART whether or not
/// the machine has the port. The kernel reports `type` 0 (`PORT_UNKNOWN`) for
/// those placeholders; a detected UART reports its real type. Checking sysfs
/// keeps enumeration read-only instead of probing by opening the port.
#[cfg(target_os = "linux")]
fn is_unbacked_legacy_uart(port_name: &str) -> bool {
    let Some(name) = std::path::Path::new(port_name)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    if !name.starts_with("ttyS") {
        return false;
    }
    std::fs::read_to_string(format!("/sys/class/tty/{name}/type"))
        .map(|value| value.trim() == "0")
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn is_unbacked_legacy_uart(_port_name: &str) -> bool {
    false
}

impl Reader for Pn532Uart {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn firmware(&self) -> Option<String> {
        self.protocol
            .as_ref()
            .map(|protocol| protocol.firmware().label())
    }

    fn capabilities(&self) -> ReaderCapabilities {
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
        }
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

    fn read_page_quad(&mut self, start_page: u8) -> Result<[u8; nfc_core::PAGE_QUAD_SIZE]> {
        self.protocol()?.read_page_quad(start_page)
    }

    fn write_page(&mut self, page: u8, data: [u8; nfc_core::PAGE_SIZE]) -> Result<()> {
        self.protocol()?.write_page(page, data)
    }

    fn password_auth(&mut self, password: [u8; nfc_core::PAGE_SIZE]) -> Result<[u8; 2]> {
        self.protocol()?.password_auth(password)
    }

    fn type2_version(&mut self) -> Result<[u8; nfc_core::TYPE2_VERSION_SIZE]> {
        self.protocol()?.type2_version()
    }

    fn apdu_capabilities(&self) -> ApduCapabilities {
        ApduCapabilities {
            supported: true,
            max_command: MAX_ISO_DEP_APDU,
        }
    }

    fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
        self.protocol()?.transceive_apdu(apdu)
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

    fn manufacturer_writer(&mut self) -> Option<&mut dyn ManufacturerWriter> {
        Some(self)
    }

    fn manufacturer_sealer(&mut self) -> Option<&mut dyn ManufacturerSealer> {
        Some(self)
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

    #[cfg(feature = "developer-tools")]
    fn raw_console(&mut self) -> Option<&mut dyn nfc_transport::RawConsole> {
        Some(self)
    }
}

#[cfg(feature = "developer-tools")]
impl nfc_transport::RawConsole for Pn532Uart {
    fn raw_command(&mut self, command: &[u8]) -> Result<Vec<u8>> {
        self.protocol()?.raw_command(command)
    }
}

impl TrailerWriter for Pn532Uart {
    fn write_sector_trailer(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.write_block(block, data)
    }
}

impl ManufacturerWriter for Pn532Uart {
    fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.write_manufacturer_block(data)
    }
}

impl ManufacturerSealer for Pn532Uart {
    fn probe_gen1a(&mut self) -> Result<bool> {
        self.protocol()?.probe_gen1a()
    }

    fn seal_manufacturer_card(&mut self, config: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.seal_manufacturer_card(config)
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
        let node = port_device("/dev/ttyUSB0");
        assert_eq!(node.name, READER_NAME);
        assert_eq!(node.conn_string, "pn532_uart:/dev/ttyUSB0");
        assert!(
            node.physical_id
                .as_deref()
                .is_some_and(|id| id.starts_with(PN532_UART_PREFIX))
        );

        let uart = port_device("ttyAMA0");
        assert_eq!(uart.conn_string, "pn532_uart:ttyAMA0");
        assert_eq!(uart.physical_id.as_deref(), Some("pn532_uart:ttyAMA0"));
    }

    #[test]
    fn physical_id_prefers_the_by_id_symlink() {
        let directory = tempfile::tempdir().expect("tempdir");
        let by_id = directory.path().join("by-id");
        std::fs::create_dir(&by_id).expect("create by-id");
        let node = directory.path().join("ttyUSB0");
        std::fs::write(&node, b"").expect("create node");
        let link = by_id.join("usb-1a86_USB_Serial-if00-port0");
        std::os::unix::fs::symlink(&node, &link).expect("create symlink");

        let node_name = node.to_string_lossy();
        assert_eq!(
            physical_id_in(&by_id, &node_name),
            format!("pn532_uart:{}", link.display())
        );
        assert_eq!(
            physical_id_in(&by_id, "/dev/ttyUSB-does-not-exist"),
            "pn532_uart:/dev/ttyUSB-does-not-exist"
        );
    }

    fn usb_port(port_name: &str) -> serialport::SerialPortInfo {
        serialport::SerialPortInfo {
            port_name: port_name.to_owned(),
            port_type: serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0x1A86,
                pid: 0x7523,
                serial_number: None,
                manufacturer: None,
                product: None,
            }),
        }
    }

    fn other_port(
        port_name: &str,
        port_type: serialport::SerialPortType,
    ) -> serialport::SerialPortInfo {
        serialport::SerialPortInfo {
            port_name: port_name.to_owned(),
            port_type,
        }
    }

    #[test]
    fn usb_serial_candidates_sort_first() {
        let devices = candidates(vec![
            other_port("/dev/ttyAMA0", serialport::SerialPortType::Unknown),
            usb_port("/dev/ttyUSB1"),
            usb_port("/dev/ttyACM0"),
            other_port("/dev/ttyAMA1", serialport::SerialPortType::PciPort),
        ]);
        let conn_strings: Vec<_> = devices
            .iter()
            .map(|device| device.conn_string.as_str())
            .collect();
        assert_eq!(
            conn_strings,
            [
                "pn532_uart:/dev/ttyACM0",
                "pn532_uart:/dev/ttyUSB1",
                "pn532_uart:/dev/ttyAMA0",
                "pn532_uart:/dev/ttyAMA1",
            ]
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
