//! Recovery service entry point. The exclusive operation lease and card
//! identity checks stay in `nfc-app`; the algorithms live in `nfc-recover`.

use nfc_core::Result;

use nfc_recover::auto::RecoveryStrategy;
use nfc_recover::{RecoveryRequest, RecoveryResult};

use crate::operation::OperationKind;
use crate::service::Service;

impl Service {
    /// Runs one recovery strategy under an exclusive reader lease. Every
    /// recovered key is authenticated before it is returned, and a card change
    /// aborts the operation.
    pub fn recover(
        &mut self,
        strategy: RecoveryStrategy,
        request: RecoveryRequest,
    ) -> Result<RecoveryResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Recovery, cancel)?;
        let result = nfc_recover::recover(operation.reader_mut(), strategy, &request)?;
        operation.check_card()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nfc_core::{CancelToken, CardInfo, CardType, Key, KeyType, NfcError};
    use nfc_recover::RecoveryRequest;
    use nfc_recover::auto::RecoveryStrategy;
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::MockTransport;

    use crate::Service;

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0xFA, 0xB9, 0xCF, 0xF4],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )
    }

    fn service_with_card() -> Service {
        let transport = MockTransport::new();
        transport.add(
            DeviceInfo::new("Mock Classic 1K", "mock:classic-1k"),
            Some(card()),
        );
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");
        service
            .select_device("mock:classic-1k")
            .expect("select mock reader");
        service
    }

    #[test]
    fn dictionary_recovery_returns_the_verified_candidate() {
        let mut service = service_with_card();
        let key = Key::from_hex("A0A1A2A3A4A5").expect("key");
        let result = service
            .recover(
                RecoveryStrategy::Dictionary,
                RecoveryRequest {
                    candidates: vec![(KeyType::A, key)],
                    ..RecoveryRequest::default()
                },
            )
            .expect("dictionary recovery");
        assert_eq!(result.keys, vec![(KeyType::A, key)]);
    }

    #[test]
    fn recovery_cancellation_is_checked_before_reader_io() {
        let mut service = service_with_card();
        let cancel = CancelToken::new();
        cancel.cancel();
        let result = service.recover(
            RecoveryStrategy::Dictionary,
            RecoveryRequest {
                cancel,
                ..RecoveryRequest::default()
            },
        );
        assert!(matches!(result, Err(NfcError::Cancelled)));
    }

    #[test]
    fn a_transport_timeout_reopens_the_reader_and_retries() {
        use nfc_transport::mock::{MockCall, MockFault};

        let transport = MockTransport::new();
        let handle = transport.add(
            DeviceInfo::new("Mock Classic 1K", "mock:classic-1k"),
            Some(card()),
        );
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");
        service
            .select_device("mock:classic-1k")
            .expect("select mock reader");
        handle.fail_next(MockCall::Authenticate, MockFault::Timeout);
        let key = Key::from_hex("A0A1A2A3A4A5").expect("key");
        let result = service
            .recover(
                RecoveryStrategy::Dictionary,
                RecoveryRequest {
                    candidates: vec![(KeyType::A, key)],
                    ..RecoveryRequest::default()
                },
            )
            .expect("retry after reopen");
        assert_eq!(result.keys, vec![(KeyType::A, key)]);
        assert!(handle.open_count() >= 2, "the reader was not reopened");
    }

    #[test]
    fn nested_recovery_without_parity_reports_unsupported() {
        let mut service = service_with_card();
        let key = Key::from_hex("A0A1A2A3A4A5").expect("key");
        let result = service.recover(
            RecoveryStrategy::Nested,
            RecoveryRequest {
                candidates: vec![(KeyType::A, key)],
                ..RecoveryRequest::default()
            },
        );
        assert!(matches!(result, Err(NfcError::Unsupported(_))));
    }
}
