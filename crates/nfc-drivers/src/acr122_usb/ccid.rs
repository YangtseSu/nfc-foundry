//! Direct USB CCID transport for the ACR122U.
//!
//! The reader accepts PN532 commands inside the ACR122 pseudo-APDU
//! `FF 00 00 00 Lc <D4 ...>` carried by a `PC_to_RDR_XfrBlock` message. Only
//! the CCID bulk endpoints are used; the interrupt endpoint is ignored because
//! no asynchronous reader events are required by the supported operations.

use std::fmt;
use std::time::Duration;

use nfc_core::{NfcError, Result};
use nfc_transport::DeviceInfo;
use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, Direction, In, Out, TransferError};
use nusb::{Endpoint, Interface, MaybeFuture};

use crate::ACR122_USB_PREFIX;
use crate::acr122::link::{AcrLink, ApduResponse};

pub(crate) const ACR122_VID: u16 = 0x072F;
pub(crate) const ACR122_PID: u16 = 0x2200;
pub(crate) const CCID_INTERFACE_CLASS: u8 = 0x0B;

const PC_TO_RDR_ICCPOWERON: u8 = 0x62;
const PC_TO_RDR_XFRBLOCK: u8 = 0x6F;
const RDR_TO_PC_DATABLOCK: u8 = 0x80;

const CCID_HEADER_LEN: usize = 10;
const MAX_CCID_MESSAGE: usize = 512;
const ICC_MUTE: u8 = 0xFE;

pub(crate) const ACR122_READER_NAME: &str = "ACR122U USB";

/// One opened ACR122U CCID interface.
pub(crate) struct CcidTransport {
    interface: Option<Interface>,
    endpoint_out: Option<Endpoint<Bulk, Out>>,
    endpoint_in: Option<Endpoint<Bulk, In>>,
    seq: u8,
}

impl fmt::Debug for CcidTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CcidTransport")
            .field("open", &self.interface.is_some())
            .field("seq", &self.seq)
            .finish()
    }
}

impl CcidTransport {
    pub(crate) fn enumerate() -> Result<Vec<DeviceInfo>> {
        let devices = nusb::list_devices()
            .wait()
            .map_err(|error| usb_error("ACR122U USB enumeration", error))?;
        let mut found: Vec<DeviceInfo> = devices
            .filter(|device| device.vendor_id() == ACR122_VID && device.product_id() == ACR122_PID)
            .map(|device| {
                DeviceInfo::new(
                    ACR122_READER_NAME,
                    format!(
                        "{ACR122_USB_PREFIX}{:03}:{:03}",
                        device.busnum(),
                        device.device_address()
                    ),
                )
            })
            .collect();
        found.sort_by(|left, right| left.conn_string.cmp(&right.conn_string));
        found.dedup_by(|left, right| left.conn_string == right.conn_string);
        Ok(found)
    }

    pub(crate) fn open(conn_string: &str) -> Result<Self> {
        let (bus, address) = parse_connstring(conn_string)?;
        let info = nusb::list_devices()
            .wait()
            .map_err(|error| usb_error("ACR122U USB enumeration", error))?
            .find(|device| {
                device.vendor_id() == ACR122_VID
                    && device.product_id() == ACR122_PID
                    && device.busnum() == bus
                    && device.device_address() == address
            })
            .ok_or_else(|| {
                NfcError::Io(format!(
                    "the ACR122U at {conn_string} is not connected; run `nfctl list`"
                ))
            })?;

        let interface_number = info
            .interfaces()
            .find(|interface| interface.class() == CCID_INTERFACE_CLASS)
            .map(|interface| interface.interface_number())
            .unwrap_or(0);

        let device = info
            .open()
            .wait()
            .map_err(|error| usb_error("ACR122U USB open", error))?;
        let interface = device
            .detach_and_claim_interface(interface_number)
            .wait()
            .map_err(|error| {
                let hint = if error.kind() == nusb::ErrorKind::Busy {
                    "; stop pcscd or any PC/SC client that holds the ACR122U alias"
                } else {
                    ""
                };
                NfcError::Io(format!(
                    "could not claim the ACR122U CCID interface{hint}: {error}"
                ))
            })?;

        let (endpoint_out, endpoint_in) = bulk_endpoints(&interface)?;
        let mut transport = Self {
            interface: Some(interface),
            endpoint_out: Some(endpoint_out),
            endpoint_in: Some(endpoint_in),
            seq: 0,
        };
        transport.drain_pending_input();
        Ok(transport)
    }

    /// Discard a CCID response left over from a previous owner or timed-out
    /// command. A fresh interface can inherit one queued bulk-IN message.
    fn drain_pending_input(&mut self) {
        let Some(endpoint_in) = self.endpoint_in.as_mut() else {
            return;
        };
        for _ in 0..4 {
            let completion = endpoint_in
                .transfer_blocking(Buffer::new(MAX_CCID_MESSAGE), Duration::from_millis(5));
            if completion.status.is_err() || completion.actual_len == 0 {
                break;
            }
        }
    }

    fn power_on(&mut self) -> Result<()> {
        let message = ccid_message(
            PC_TO_RDR_ICCPOWERON,
            self.next_seq(),
            [0x01, 0x00, 0x00],
            &[],
        );
        let response = match self.send_and_receive(
            &message,
            Duration::from_millis(1_000),
            "ACR122U ICC power-on",
        ) {
            Ok(response) => response,
            // ICC_MUTE is reported when no card is present. The PICC interface
            // is still usable, and card selection reports the missing card.
            Err(NfcError::Timeout(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        if response.message_type != RDR_TO_PC_DATABLOCK {
            return Err(NfcError::Io(format!(
                "ACR122U answered ICC power-on with CCID message {:#04X}",
                response.message_type
            )));
        }
        Ok(())
    }

    fn next_seq(&mut self) -> u8 {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        seq
    }

    fn xfr_block_raw(&mut self, apdu: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        if apdu.is_empty() || apdu.len() > 255 {
            return Err(NfcError::InvalidArgument(format!(
                "ACR122U pseudo-APDU length {} is outside 1..=255",
                apdu.len()
            )));
        }
        let message = ccid_message(
            PC_TO_RDR_XFRBLOCK,
            self.next_seq(),
            [0x00, 0x00, 0x00],
            apdu,
        );
        let response = self.send_and_receive(&message, timeout, "ACR122U XfrBlock")?;
        if response.message_type != RDR_TO_PC_DATABLOCK {
            return Err(NfcError::Io(format!(
                "ACR122U answered XfrBlock with CCID message {:#04X}",
                response.message_type
            )));
        }
        Ok(response.payload)
    }

    fn send_and_receive(
        &mut self,
        message: &[u8],
        timeout: Duration,
        context: &'static str,
    ) -> Result<CcidMessage> {
        let endpoint_out = self
            .endpoint_out
            .as_mut()
            .ok_or_else(|| NfcError::Io("the ACR122U transport is not open".to_owned()))?;
        let max_packet = endpoint_out.max_packet_size();
        let completion = endpoint_out.transfer_blocking(Buffer::from(message), timeout);
        completion
            .status
            .map_err(|error| transfer_error(context, error))?;
        if max_packet > 0 && message.len().is_multiple_of(max_packet) {
            let completion =
                endpoint_out.transfer_blocking(Buffer::from(Vec::<u8>::new()), timeout);
            completion
                .status
                .map_err(|error| transfer_error(context, error))?;
        }

        let endpoint_in = self
            .endpoint_in
            .as_mut()
            .ok_or_else(|| NfcError::Io("the ACR122U transport is not open".to_owned()))?;
        let buffer = Buffer::new(MAX_CCID_MESSAGE);
        let completion = endpoint_in.transfer_blocking(buffer, timeout);
        completion
            .status
            .map_err(|error| transfer_error(context, error))?;
        let received = &completion.buffer[..completion.actual_len];
        CcidMessage::parse(received)
    }
}

impl AcrLink for CcidTransport {
    fn power_on(&mut self) -> Result<()> {
        CcidTransport::power_on(self)
    }

    fn xfr_block(&mut self, apdu: &[u8], timeout: Duration) -> Result<ApduResponse> {
        let payload = self.xfr_block_raw(apdu, timeout)?;
        split_apdu(payload)
    }

    fn shutdown(&mut self) {
        self.endpoint_in = None;
        self.endpoint_out = None;
        if let Some(interface) = self.interface.take() {
            let _ = interface.release().wait();
        }
    }
}

impl Drop for CcidTransport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CcidMessage {
    message_type: u8,
    payload: Vec<u8>,
}

impl CcidMessage {
    fn parse(message: &[u8]) -> Result<Self> {
        if message.len() < CCID_HEADER_LEN {
            return Err(NfcError::Io(format!(
                "ACR122U CCID response is {} bytes, shorter than the 10-byte header",
                message.len()
            )));
        }
        let length = u32::from_le_bytes([message[1], message[2], message[3], message[4]]) as usize;
        if message.len() < CCID_HEADER_LEN + length {
            return Err(NfcError::Io(format!(
                "ACR122U CCID response declares {length} payload bytes but only {} arrived",
                message.len() - CCID_HEADER_LEN
            )));
        }
        let message_type = message[0];
        let error = message[8];
        // The ACR122U reports vendor status 0x81 on otherwise successful
        // data blocks, so only the no-response case is treated as a failure.
        if error == ICC_MUTE && length == 0 {
            return Err(NfcError::Timeout(
                "the ACR122U card interface did not answer".to_owned(),
            ));
        }
        Ok(Self {
            message_type,
            payload: message[CCID_HEADER_LEN..CCID_HEADER_LEN + length].to_vec(),
        })
    }
}

fn ccid_message(message_type: u8, seq: u8, specific: [u8; 3], payload: &[u8]) -> Vec<u8> {
    let length = payload.len() as u32;
    let mut message = Vec::with_capacity(CCID_HEADER_LEN + payload.len());
    message.push(message_type);
    message.extend_from_slice(&length.to_le_bytes());
    message.push(0);
    message.push(seq);
    message.extend_from_slice(&specific);
    message.extend_from_slice(payload);
    message
}

fn split_apdu(payload: Vec<u8>) -> Result<ApduResponse> {
    if payload.len() < 2 {
        return Err(NfcError::Io(format!(
            "ACR122U APDU response is {} bytes and has no status word",
            payload.len()
        )));
    }
    let split = payload.len() - 2;
    Ok(ApduResponse {
        data: payload[..split].to_vec(),
        sw: [payload[split], payload[split + 1]],
    })
}

fn bulk_endpoints(interface: &Interface) -> Result<(Endpoint<Bulk, Out>, Endpoint<Bulk, In>)> {
    let descriptor = interface
        .descriptor()
        .ok_or_else(|| NfcError::Io("ACR122U CCID interface has no descriptor".to_owned()))?;
    let mut out = None;
    let mut input = None;
    for endpoint in descriptor.endpoints() {
        if endpoint.transfer_type() != TransferType::Bulk {
            continue;
        }
        match endpoint.direction() {
            Direction::Out => out = Some(endpoint.address()),
            Direction::In => input = Some(endpoint.address()),
        }
    }
    let address_out = out.ok_or_else(|| {
        NfcError::Io("ACR122U CCID interface has no bulk OUT endpoint".to_owned())
    })?;
    let address_in = input
        .ok_or_else(|| NfcError::Io("ACR122U CCID interface has no bulk IN endpoint".to_owned()))?;
    let endpoint_out = interface
        .endpoint::<Bulk, Out>(address_out)
        .map_err(|error| usb_error("ACR122U bulk OUT endpoint", error))?;
    let endpoint_in = interface
        .endpoint::<Bulk, In>(address_in)
        .map_err(|error| usb_error("ACR122U bulk IN endpoint", error))?;
    Ok((endpoint_out, endpoint_in))
}

fn parse_connstring(conn_string: &str) -> Result<(u8, u8)> {
    let target = conn_string.strip_prefix(ACR122_USB_PREFIX).ok_or_else(|| {
        NfcError::InvalidArgument(format!(
            "{conn_string} is not a {ACR122_USB_PREFIX} connstring"
        ))
    })?;
    let (bus, address) = target.split_once(':').ok_or_else(|| {
        NfcError::InvalidArgument(format!(
            "ACR122U connstring {conn_string} must be {ACR122_USB_PREFIX}<bus>:<address>"
        ))
    })?;
    if address.contains(':') {
        return Err(NfcError::InvalidArgument(format!(
            "ACR122U connstring {conn_string} must be {ACR122_USB_PREFIX}<bus>:<address>"
        )));
    }
    let bus = bus.parse::<u8>().map_err(|_| {
        NfcError::InvalidArgument(format!("ACR122U bus number {bus:?} is not 0..=255"))
    })?;
    let address = address.parse::<u8>().map_err(|_| {
        NfcError::InvalidArgument(format!("ACR122U device address {address:?} is not 0..=255"))
    })?;
    Ok((bus, address))
}

fn usb_error(context: &str, error: nusb::Error) -> NfcError {
    let detail = match error.kind() {
        nusb::ErrorKind::PermissionDenied => {
            "permission denied; install the NFC Foundry udev rule for 072F:2200".to_owned()
        }
        nusb::ErrorKind::Busy => {
            "device is busy; stop pcscd or any PC/SC client using the ACR122U alias".to_owned()
        }
        nusb::ErrorKind::Disconnected => "device disconnected".to_owned(),
        nusb::ErrorKind::NotFound => "device not found".to_owned(),
        nusb::ErrorKind::Unsupported => "operation unsupported by this USB stack".to_owned(),
        _ => error.to_string(),
    };
    NfcError::Io(format!("{context}: {detail}"))
}

fn transfer_error(context: &str, error: TransferError) -> NfcError {
    match error {
        TransferError::Cancelled => NfcError::Timeout(context.to_owned()),
        TransferError::Disconnected => NfcError::Io(format!("{context}: device disconnected")),
        TransferError::Stall => NfcError::Io(format!("{context}: endpoint stalled")),
        TransferError::Fault => {
            NfcError::Io(format!("{context}: hardware fault or protocol violation"))
        }
        TransferError::InvalidArgument => {
            NfcError::Io(format!("{context}: invalid USB transfer argument"))
        }
        TransferError::Unknown(code) => NfcError::Io(format!("{context}: USB error {code}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_ccid_messages_with_little_endian_lengths() {
        let message = ccid_message(
            PC_TO_RDR_XFRBLOCK,
            7,
            [0, 0, 0],
            &[0xFF, 0x00, 0x00, 0x00, 0x02, 0xD4, 0x02],
        );
        assert_eq!(&message[..10], &[0x6F, 0x07, 0, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(&message[10..], &[0xFF, 0x00, 0x00, 0x00, 0x02, 0xD4, 0x02]);
    }

    #[test]
    fn parses_data_block_payloads_and_reports_errors() {
        let mut message = vec![0x80, 0x04, 0, 0, 0, 0, 9, 0, 0, 0];
        message.extend_from_slice(&[0x90, 0x00, 0xD5, 0x03]);
        let parsed = CcidMessage::parse(&message).expect("data block");
        assert_eq!(parsed.message_type, RDR_TO_PC_DATABLOCK);
        assert_eq!(parsed.payload, vec![0x90, 0x00, 0xD5, 0x03]);

        let mute = [0x80, 0x00, 0, 0, 0, 0, 1, 0, ICC_MUTE, 0];
        assert!(matches!(
            CcidMessage::parse(&mute),
            Err(NfcError::Timeout(_))
        ));

        let truncated = [0x80, 0x08, 0, 0, 0, 0, 1, 0, 0, 0];
        assert!(matches!(
            CcidMessage::parse(&truncated),
            Err(NfcError::Io(_))
        ));

        let mut vendor_status = vec![0x80, 0x02, 0, 0, 0, 0, 3, 0, 0x81, 0];
        vendor_status.extend_from_slice(&[0x90, 0x00]);
        assert_eq!(
            CcidMessage::parse(&vendor_status)
                .expect("vendor status is not an error")
                .payload,
            vec![0x90, 0x00]
        );
    }

    #[test]
    fn splits_status_words_from_apdu_payloads() {
        assert_eq!(
            split_apdu(vec![0x90, 0x00]).expect("empty response"),
            ApduResponse {
                data: Vec::new(),
                sw: [0x90, 0x00]
            }
        );
        assert_eq!(
            split_apdu(vec![0x61, 0x08]).expect("more data"),
            ApduResponse {
                data: Vec::new(),
                sw: [0x61, 0x08]
            }
        );
        assert!(split_apdu(vec![0x90]).is_err());
    }

    #[test]
    fn parses_exact_bus_and_address_connstrings() {
        assert_eq!(
            parse_connstring("acr122_usb:001:017").expect("padded"),
            (1, 17)
        );
        assert_eq!(parse_connstring("acr122_usb:1:2").expect("short"), (1, 2));
        assert!(matches!(
            parse_connstring("acr122_usb:1:2:3"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_connstring("pn532_uart:/dev/ttyUSB0"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_connstring("acr122_usb:999:1"),
            Err(NfcError::InvalidArgument(_))
        ));
    }
}
