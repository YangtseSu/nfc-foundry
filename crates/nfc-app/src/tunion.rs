//! Read-only China T-Union workflow.
//!
//! The protocol adapter lives in `nfc-pboc`; this layer enforces the selected
//! card identity and the reader lease. No mutation path exists.

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_pboc::TUnionCard;

use crate::operation::Operation;

/// A completed T-Union read with the card identity used for the envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TUnionReadResult {
    pub card: CardInfo,
    pub data: TUnionCard,
}

/// Read the T-Union application on the selected ISO-DEP card.
pub fn read(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<TUnionReadResult> {
    cancel.check()?;
    if operation.card().card_type != CardType::Iso14443 {
        return Err(NfcError::Unsupported(
            "the selected card is not an ISO/IEC 14443-4 card",
        ));
    }
    let card = operation.card().clone();
    let data = nfc_pboc::tunion::read(operation.reader_mut(), cancel)?;
    operation.check_iso_dep_card()?;
    Ok(TUnionReadResult { card, data })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nfc_core::{CancelToken, CardInfo, CardType, NfcError};
    use nfc_pboc::mock::MockTUnionTransport;
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::MockTransport;

    use crate::Service;

    fn tunion_service() -> Service {
        let transport = MockTUnionTransport::new();
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");
        service.select_device("mock:tunion").expect("select");
        service
    }

    #[test]
    fn reads_the_t_union_card_through_the_app_service() {
        let mut service = tunion_service();
        let result = service
            .tunion_read(CancelToken::new())
            .expect("T-Union read");
        assert_eq!(result.card.card_type, CardType::Iso14443);
        assert_eq!(result.data.balance_cents, 5000);
        assert_eq!(result.data.transactions.len(), 2);
        assert_eq!(result.data.travels.len(), 2);
        assert_eq!(result.data.label, "MOT_T_EP");
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
            .tunion_read(CancelToken::new())
            .expect_err("non ISO-DEP");
        assert!(matches!(error, NfcError::Unsupported(_)), "{error}");
    }

    #[test]
    fn a_cancelled_token_stops_before_reading() {
        let mut service = tunion_service();
        let cancel = CancelToken::new();
        cancel.cancel();
        let error = service.tunion_read(cancel).expect_err("cancelled");
        assert!(matches!(error, NfcError::Cancelled), "{error}");
    }
}
