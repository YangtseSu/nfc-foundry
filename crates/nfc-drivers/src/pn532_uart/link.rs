//! Byte transport below the PN532 frame codec. The trait keeps the protocol
//! testable without a serial port.

use std::io;
use std::time::Duration;

use nfc_core::{NfcError, Result};
use serialport::{ClearBuffer, SerialPort};

pub(crate) trait Pn532Link: Send {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize>;
    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()>;
    fn flush_input(&mut self) -> io::Result<()>;
}

pub(crate) struct SerialLink {
    port: Box<dyn SerialPort>,
}

impl SerialLink {
    pub(crate) fn open(path: &str, baud_rate: u32, timeout: Duration) -> Result<Self> {
        let port = serialport::new(path, baud_rate)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(timeout)
            .open()
            .map_err(|error| {
                let hint = if matches!(
                    error.kind(),
                    serialport::ErrorKind::Io(io::ErrorKind::PermissionDenied)
                ) {
                    "; add the user to the serial group (uucp on Arch, dialout on Debian) or fix the device permissions"
                } else {
                    ""
                };
                NfcError::Io(format!("cannot open serial port {path}: {error}{hint}"))
            })?;
        Ok(Self { port })
    }
}

impl Pn532Link for SerialLink {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.port.write_all(bytes)
    }

    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.port.read(buffer)
    }

    fn set_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.port.set_timeout(timeout).map_err(io::Error::other)
    }

    fn flush_input(&mut self) -> io::Result<()> {
        self.port
            .clear(ClearBuffer::Input)
            .map_err(io::Error::other)
    }
}
