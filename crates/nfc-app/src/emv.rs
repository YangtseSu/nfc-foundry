//! Read-only EMV payment-card public data.
//!
//! The protocol adapter lives in `nfc-pboc`; this layer enforces the selected
//! card identity and the reader lease. No payment, balance, PIN, or mutation
//! path exists.

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_pboc::emv::EmvCard;

use crate::operation::Operation;

/// A completed payment-card read with the card identity used for the envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmvReadResult {
    pub card: CardInfo,
    pub data: EmvCard,
}

/// Read the highest-priority EMV application on the selected ISO-DEP card.
pub fn read(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<EmvReadResult> {
    cancel.check()?;
    if operation.card().card_type != CardType::Iso14443 {
        return Err(NfcError::Unsupported(
            "the selected card is not an ISO/IEC 14443-4 card",
        ));
    }
    let card = operation.card().clone();
    let data = nfc_pboc::emv::read(operation.reader_mut(), cancel)?;
    operation.check_iso_dep_card()?;
    Ok(EmvReadResult { card, data })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nfc_core::{CancelToken, CardInfo, CardType, NfcError};
    use nfc_pboc::mock::MockEmvTransport;
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::MockTransport;

    use crate::Service;

    fn emv_service() -> Service {
        let transport = MockEmvTransport::new();
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");
        service.select_device("mock:emv").expect("select");
        service
    }

    #[test]
    fn reads_the_emv_card_through_the_app_service() {
        let mut service = emv_service();
        let result = service.emv_read(CancelToken::new()).expect("EMV read");
        assert_eq!(result.card.card_type, CardType::Iso14443);
        assert_eq!(result.data.label, "Mock Payment");
        assert_eq!(
            result.data.pan_masked.as_deref(),
            Some("**************0001")
        );
        assert_eq!(
            result.data.expires.expect("expiry").to_string(),
            "2030-12-31"
        );
        assert!(result.data.complete);
    }

    #[test]
    fn a_non_iso_dep_card_is_refused_before_any_apdu() {
        let transport = MockTransport::new();
        transport.add(
            DeviceInfo::new("Mock Classic", "mock:classic"),
            Some(CardInfo::new(
                vec![0x04, 0x11, 0x22, 0x33],
                [0x00, 0x04],
                0x08,
                CardType::MifareClassic1K,
            )),
        );
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");
        service.select_device("mock:classic").expect("select");
        let error = service
            .emv_read(CancelToken::new())
            .expect_err("non ISO-DEP");
        assert!(matches!(error, NfcError::Unsupported(_)), "{error}");
    }

    #[test]
    fn a_cancelled_token_stops_before_reading() {
        let mut service = emv_service();
        let cancel = CancelToken::new();
        cancel.cancel();
        let error = service.emv_read(cancel).expect_err("cancelled");
        assert!(matches!(error, NfcError::Cancelled), "{error}");
    }
}
