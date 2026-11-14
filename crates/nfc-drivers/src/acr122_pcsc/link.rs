//! PC/SC link for the ACR122U.
//!
//! The reader is opened in `SCARD_SHARE_DIRECT` mode and PN532 commands travel
//! through the CCID escape control, so no card APDU is ever sent and the card
//! is not activated behind the driver's back. The ACR122 pseudo-APDU framing
//! is unchanged; only `SCardControl` replaces the bulk CCID transfer.
//!
//! Card mode (`SCARD_SHARE_SHARED`, T=1) is a fallback for firmware that
//! rejects direct mode. There the same pseudo-APDUs travel through
//! `SCardTransmit`; the chosen mode is a diagnostic fact, not a hidden retry.

use std::fmt;
use std::time::Duration;

use nfc_core::{NfcError, Result};
use pcsc::{Card, Context, Disposition, Error, Protocols, Scope, ShareMode, ctl_code};

use crate::acr122::link::{AcrLink, ApduResponse};
use crate::acr122_pcsc::{READER_NAME, is_acr122_reader};

/// CCID escape control code the ACR122U accepts on Linux.
const CCID_ESCAPE: u64 = ctl_code(3500);
const MAX_APDU: usize = 255;
const MAX_RESPONSE: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Direct,
    Transmit,
}

/// One PC/SC connection to an ACR122U.
pub(crate) struct PcscLink {
    context: Option<Context>,
    card: Option<Card>,
    mode: Mode,
}

impl fmt::Debug for PcscLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PcscLink")
            .field("open", &self.card.is_some())
            .field("mode", &self.mode)
            .finish()
    }
}

impl PcscLink {
    pub(crate) fn connect(conn_string: &str) -> Result<Self> {
        let name = parse_connstring(conn_string)?;
        let context = establish()?;
        let reader = exact_reader(&context, name)?;
        let mut link = Self {
            context: Some(context),
            card: None,
            mode: Mode::Direct,
        };
        link.connect_reader(&reader)?;
        Ok(link)
    }

    fn connect_reader(&mut self, reader: &std::ffi::CStr) -> Result<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| NfcError::Io("the PC/SC context is not open".to_owned()))?;
        match context.connect(reader, ShareMode::Direct, Protocols::UNDEFINED) {
            Ok(card) => {
                self.card = Some(card);
                self.mode = Mode::Direct;
                Ok(())
            }
            Err(direct) => match context.connect(reader, ShareMode::Shared, Protocols::T1) {
                Ok(card) => {
                    self.card = Some(card);
                    self.mode = Mode::Transmit;
                    Ok(())
                }
                Err(card_mode) => Err(NfcError::Io(format!(
                    "{READER_NAME} {} could not be opened: direct mode failed ({direct}); \
                         T=1 mode failed ({card_mode}){}",
                    reader.to_string_lossy(),
                    contention_hint(direct)
                ))),
            },
        }
    }
}

impl AcrLink for PcscLink {
    fn power_on(&mut self) -> Result<()> {
        // `SCardConnect` already powered the ICC in card mode; direct mode
        // leaves the card untouched until the driver selects it.
        Ok(())
    }

    fn xfr_block(&mut self, apdu: &[u8], _timeout: Duration) -> Result<ApduResponse> {
        if apdu.is_empty() || apdu.len() > MAX_APDU {
            return Err(NfcError::InvalidArgument(format!(
                "ACR122U pseudo-APDU length {} is outside 1..={MAX_APDU}",
                apdu.len()
            )));
        }
        let card = self
            .card
            .as_ref()
            .ok_or_else(|| NfcError::Io("the ACR122U PC/SC link is not open".to_owned()))?;
        let mut response = vec![0u8; MAX_RESPONSE];
        let result = match self.mode {
            Mode::Direct => card.control(CCID_ESCAPE, apdu, &mut response),
            Mode::Transmit => card.transmit(apdu, &mut response),
        };
        let payload = result.map_err(exchange_error)?;
        split_response(payload)
    }

    fn shutdown(&mut self) {
        if let Some(card) = self.card.take() {
            let _ = card.disconnect(Disposition::LeaveCard);
        }
        if let Some(context) = self.context.take() {
            let _ = context.release();
        }
    }
}

impl Drop for PcscLink {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Parse `acr122_pcsc:<exact reader name>`.
pub(crate) fn parse_connstring(conn_string: &str) -> Result<&str> {
    let name = conn_string
        .strip_prefix(crate::ACR122_PCSC_PREFIX)
        .ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "{conn_string} is not a {} connstring",
                crate::ACR122_PCSC_PREFIX
            ))
        })?;
    if name.is_empty() {
        return Err(NfcError::InvalidArgument(format!(
            "reader connstring {conn_string} has no reader name"
        )));
    }
    if !is_acr122_reader(name) {
        return Err(NfcError::Unsupported(
            "only ACR122U readers are supported over PC/SC",
        ));
    }
    Ok(name)
}

/// Establish a user-scope PC/SC context with an actionable service error.
pub(crate) fn establish() -> Result<Context> {
    Context::establish(Scope::User).map_err(service_error)
}

fn exact_reader(context: &Context, name: &str) -> Result<std::ffi::CString> {
    let readers = context.list_readers_owned().map_err(service_error)?;
    readers
        .into_iter()
        .find(|reader| reader.to_string_lossy() == name)
        .ok_or_else(|| {
            NfcError::Io(format!(
                "the PC/SC reader {name:?} is not present; run `nfctl list`"
            ))
        })
}

fn split_response(payload: &[u8]) -> Result<ApduResponse> {
    if payload.len() < 2 {
        return Err(NfcError::Io(format!(
            "ACR122U PC/SC response is {} bytes and has no status word",
            payload.len()
        )));
    }
    let split = payload.len() - 2;
    Ok(ApduResponse {
        data: payload[..split].to_vec(),
        sw: [payload[split], payload[split + 1]],
    })
}

pub(crate) fn service_error(error: Error) -> NfcError {
    match error {
        Error::NoService | Error::ServiceStopped => NfcError::Io(
            "the PC/SC service is not running; start pcscd with `systemctl start pcscd`".to_owned(),
        ),
        other => NfcError::Io(format!("PC/SC context failed: {other}")),
    }
}

fn exchange_error(error: Error) -> NfcError {
    match error {
        Error::NoSmartcard | Error::RemovedCard => NfcError::NoCard,
        Error::Timeout => NfcError::Timeout("the ACR122U did not answer over PC/SC".to_owned()),
        Error::SharingViolation => NfcError::Io(
            "another process holds the ACR122U over PC/SC; close it and retry".to_owned(),
        ),
        Error::NotTransacted => NfcError::Io(
            "the ACR122U PC/SC connection is not transacted; close and reopen the reader"
                .to_owned(),
        ),
        Error::NoService | Error::ServiceStopped => NfcError::Io(
            "the PC/SC service stopped; start pcscd with `systemctl start pcscd`".to_owned(),
        ),
        Error::ReaderUnavailable | Error::UnknownReader => {
            NfcError::Io("the ACR122U PC/SC reader is no longer available".to_owned())
        }
        other => NfcError::Io(format!("ACR122U PC/SC exchange failed: {other}")),
    }
}

fn contention_hint(error: Error) -> &'static str {
    if matches!(error, Error::SharingViolation) {
        "; close any other PC/SC client using the reader"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_acr122_pcsc_connstrings() {
        assert_eq!(
            parse_connstring("acr122_pcsc:ACS ACR122U 00 00").expect("reader name"),
            "ACS ACR122U 00 00"
        );
        assert!(matches!(
            parse_connstring("acr122_pcsc:"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_connstring("acr122_usb:001:002"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_connstring("acr122_pcsc:Generic Smart Card Reader"),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[test]
    fn splits_status_words_from_control_responses() {
        assert_eq!(
            split_response(&[0xD5, 0x4B, 0x00, 0x90, 0x00]).expect("response"),
            ApduResponse {
                data: vec![0xD5, 0x4B, 0x00],
                sw: [0x90, 0x00]
            }
        );
        assert_eq!(
            split_response(&[0x90, 0x00]).expect("status only"),
            ApduResponse {
                data: Vec::new(),
                sw: [0x90, 0x00]
            }
        );
        assert!(split_response(&[0x90]).is_err());
        assert!(split_response(&[]).is_err());
    }

    #[test]
    fn maps_removed_and_busy_cards_to_stable_errors() {
        assert!(matches!(
            exchange_error(Error::RemovedCard),
            NfcError::NoCard
        ));
        assert!(matches!(
            exchange_error(Error::NoSmartcard),
            NfcError::NoCard
        ));
        assert!(matches!(
            exchange_error(Error::Timeout),
            NfcError::Timeout(_)
        ));
        assert!(matches!(
            exchange_error(Error::SharingViolation),
            NfcError::Io(message) if message.contains("another process")
        ));
        assert!(matches!(
            exchange_error(Error::UnknownReader),
            NfcError::Io(message) if message.contains("no longer available")
        ));
    }

    #[test]
    fn reports_a_stopped_service_with_an_action() {
        assert!(matches!(
            service_error(Error::NoService),
            NfcError::Io(message) if message.contains("pcscd")
        ));
    }
}
