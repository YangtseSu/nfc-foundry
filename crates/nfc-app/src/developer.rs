//! Developer-only raw diagnostics.
//!
//! This module is compiled with the `developer-tools` feature only. A
//! diagnostic lease owns the reader like an operation lease, but it does not
//! select a card: the native command console can talk to the reader before a
//! tag is present, and the raw APDU exchange selects a card only when it runs.
//! Neither method is a supported card path.

use std::cell::Cell;

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_transport::{ApduResponse, Reader};

/// Exclusive reader lease for a developer diagnostic session.
///
/// The host keeps the lease for the whole session, so a monitor poll or a
/// second operation cannot interleave commands. Dropping the lease clears the
/// busy state; the reader and its cross-process lock stay open until the
/// service releases or closes them.
pub struct DiagnosticLease<'a> {
    reader: &'a mut dyn Reader,
    busy: &'a Cell<bool>,
    cancel: CancelToken,
}

impl<'a> DiagnosticLease<'a> {
    pub(crate) fn new(
        reader: &'a mut dyn Reader,
        busy: &'a Cell<bool>,
        cancel: CancelToken,
    ) -> Self {
        Self {
            reader,
            busy,
            cancel,
        }
    }

    /// Send one native reader command and return the native reply body,
    /// including a leading status byte where the protocol defines one.
    pub fn raw_command(&mut self, command: &[u8]) -> Result<Vec<u8>> {
        self.cancel.check()?;
        if command.is_empty() {
            return Err(NfcError::InvalidArgument(
                "a raw command needs at least one byte".to_owned(),
            ));
        }
        self.reader
            .raw_console()
            .ok_or(NfcError::Unsupported(
                "the selected reader has no raw command console",
            ))?
            .raw_command(command)
    }

    /// Select the card and send one complete ISO 7816-4 command APDU. Returns
    /// the selected card identity so the caller can report it.
    pub fn raw_apdu(&mut self, apdu: &[u8]) -> Result<(CardInfo, ApduResponse)> {
        self.cancel.check()?;
        if apdu.is_empty() {
            return Err(NfcError::InvalidArgument(
                "the command APDU is empty".to_owned(),
            ));
        }
        let card = self.reader.card_info()?;
        if card.card_type != CardType::Iso14443 {
            return Err(NfcError::Unsupported(
                "the selected card is not an ISO/IEC 14443-4 card",
            ));
        }
        let response = self.reader.transceive_apdu(apdu)?;
        Ok((card, response))
    }
}

impl Drop for DiagnosticLease<'_> {
    fn drop(&mut self) {
        self.busy.set(false);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nfc_core::{CancelToken, CardInfo, CardType, NfcError};
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::MockTransport;

    use crate::Service;

    fn service(conn_string: &str, card: Option<CardInfo>) -> (Service, MockTransport) {
        let transport = MockTransport::new();
        transport.add(DeviceInfo::new("Mock", conn_string), card);
        let mut service = Service::with_transport(Arc::new(transport.clone()));
        service.refresh_devices().expect("refresh");
        service.select_device(conn_string).expect("select");
        (service, transport)
    }

    #[test]
    fn a_raw_console_runs_before_a_card_is_present() {
        let (mut service, transport) = service("mock:console", None);
        let handle = transport.handle("mock:console").expect("handle");
        handle.set_raw_response(vec![0x32, 0x01, 0x06, 0x07]);

        let mut lease = service
            .begin_diagnostics(CancelToken::new())
            .expect("diagnostic lease");
        assert_eq!(
            lease.raw_command(&[0x02]).expect("firmware command"),
            vec![0x32, 0x01, 0x06, 0x07]
        );
        assert_eq!(handle.raw_commands(), vec![vec![0x02]]);
        drop(lease);
        assert_eq!(handle.open_count(), 1);
    }

    #[test]
    fn a_raw_apdu_requires_an_iso_dep_card() {
        let card = CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        );
        let (mut service, _transport) = service("mock:classic", Some(card));
        let mut lease = service
            .begin_diagnostics(CancelToken::new())
            .expect("diagnostic lease");
        let result = lease.raw_apdu(&[0x00, 0xA4, 0x04, 0x00]);
        assert!(
            matches!(result, Err(NfcError::Unsupported(_))),
            "{result:?}"
        );
    }
}
