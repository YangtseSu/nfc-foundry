use std::sync::Arc;

use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, Result};
use nfc_transport::{DeviceInfo, Transport};

use crate::classic::{
    BackupRecord, BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicKeyscanResult,
    ClassicRead, ClassicReadResult, ClassicRestore, ClassicRestoreResult, ClassicWriteResult,
    KeyStore,
};
use crate::device::DeviceManager;
use crate::operation::{Operation, OperationKind};

/// Application facade shared by the GUI and `nfctl`.
#[derive(Default)]
pub struct Service {
    devices: DeviceManager,
}

impl Service {
    pub fn with_transport(transport: Arc<dyn Transport>) -> Self {
        Self {
            devices: DeviceManager::with_transport(transport),
        }
    }

    pub fn devices(&self) -> &[DeviceInfo] {
        self.devices.devices()
    }

    pub fn selected_device(&self) -> Option<&str> {
        self.devices.selected()
    }

    pub fn refresh_devices(&mut self) -> Result<()> {
        self.devices.refresh()
    }

    pub fn select_device(&mut self, conn_string: &str) -> Result<()> {
        self.devices.select(conn_string)
    }

    pub fn release_reader(&mut self) -> Result<()> {
        self.devices.release()
    }

    pub fn scan(&mut self) -> Result<CardInfo> {
        let operation = self.begin_operation(OperationKind::Scan, CancelToken::new())?;
        Ok(operation.card().clone())
    }

    pub fn card_info(&mut self) -> Result<CardInfo> {
        let operation = self.begin_operation(OperationKind::CardInfo, CancelToken::new())?;
        Ok(operation.card().clone())
    }

    /// Reads the whole card with candidate keys. Authentication failures
    /// produce a partial dump with explicit warnings.
    pub fn classic_read(&mut self, request: ClassicRead<'_>) -> Result<ClassicReadResult> {
        let ClassicRead {
            keys,
            include_common_keys,
            cancel,
        } = request;
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        crate::classic::read::read_dump(&mut operation, keys, include_common_keys)
    }

    /// Reads one block. Block 0 needs no key; other blocks authenticate their
    /// sector with the first candidate key the card accepts.
    pub fn classic_read_block(
        &mut self,
        block: usize,
        keys: &KeyStore,
        include_common_keys: bool,
        cancel: CancelToken,
    ) -> Result<[u8; BLOCK_SIZE]> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        crate::classic::read::read_block(&mut operation, block, keys, include_common_keys)
    }

    /// Writes one block with a pre-write backup and read-back verification.
    pub fn classic_write_block(
        &mut self,
        request: ClassicBlockWrite<'_>,
    ) -> Result<ClassicWriteResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        crate::classic::write::write_block(&mut operation, request)
    }

    /// Restores a dump with a pre-write backup and the request's stop policy.
    pub fn classic_restore(&mut self, request: ClassicRestore<'_>) -> Result<ClassicRestoreResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        crate::classic::restore::restore(&mut operation, request)
    }

    /// Scans candidate keys per sector. Authentication and trailer reads are
    /// the only card operations.
    pub fn classic_keyscan(&mut self, request: ClassicKeyscan<'_>) -> Result<ClassicKeyscanResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::KeyScan, cancel)?;
        crate::classic::keyscan::keyscan(&mut operation, request)
    }

    /// Reads the card and writes a project-format backup without mutating it.
    pub fn classic_backup(
        &mut self,
        keys: &KeyStore,
        include_common_keys: bool,
        backup: &BackupStore,
    ) -> Result<BackupRecord> {
        let mut operation = self.begin_operation(OperationKind::Read, CancelToken::new())?;
        let dump = crate::classic::read::read_dump(&mut operation, keys, include_common_keys)?.dump;
        backup.save(
            &dump,
            &crate::classic::write::backup_label(&dump, dump.layout, "backup"),
        )
    }

    pub fn begin_operation(
        &mut self,
        kind: OperationKind,
        cancel: CancelToken,
    ) -> Result<Operation<'_>> {
        self.devices.begin_operation(kind, cancel)
    }
}

#[cfg(test)]
mod tests {
    use nfc_core::{CardType, Key, KeyType, NfcError};
    use nfc_transport::mock::{MockCall, MockFault, MockHandle, MockTransport};

    use super::*;

    fn card(uid: u8) -> CardInfo {
        CardInfo::new(vec![uid; 4], [0x00, 0x04], 0x08, CardType::MifareClassic1K)
    }

    fn service_with_card(card: CardInfo) -> (Service, MockHandle) {
        let transport = MockTransport::new();
        let handle = transport.add(DeviceInfo::new("Mock", "mock:0"), Some(card));
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");
        service.select_device("mock:0").expect("select");
        (service, handle)
    }

    #[test]
    fn scan_returns_the_selected_card() {
        let (mut service, _handle) = service_with_card(card(3));
        assert_eq!(service.scan().expect("scan").uid, vec![3; 4]);
        assert_eq!(service.card_info().expect("card info").uid, vec![3; 4]);
    }

    #[test]
    fn reader_is_opened_once_and_reopened_after_release() {
        let (mut service, handle) = service_with_card(card(1));
        service.scan().expect("scan");
        service.scan().expect("scan");
        assert_eq!(handle.open_count(), 1);
        assert!(handle.is_open());

        service.release_reader().expect("release");
        assert!(!handle.is_open());
        assert_eq!(handle.close_count(), 1);

        service.scan().expect("scan after release");
        assert_eq!(handle.open_count(), 2);
    }

    #[test]
    fn operation_detects_card_change() {
        let (mut service, handle) = service_with_card(card(1));
        let mut operation = service
            .begin_operation(OperationKind::Read, CancelToken::new())
            .expect("operation");
        handle.set_card(Some(card(2)));
        assert!(matches!(operation.check_card(), Err(NfcError::CardChanged)));
    }

    #[test]
    fn authentication_failure_is_reported() {
        let (mut service, handle) = service_with_card(card(1));
        let mut operation = service
            .begin_operation(OperationKind::Read, CancelToken::new())
            .expect("operation");
        handle.fail_next(MockCall::Authenticate, MockFault::AuthenticationFailed);
        assert!(matches!(
            operation.authenticate(0, KeyType::A, Key::new([0xFF; 6])),
            Err(NfcError::AuthenticationFailed)
        ));
    }

    #[test]
    fn reader_timeout_is_reported() {
        let (mut service, handle) = service_with_card(card(1));
        let mut operation = service
            .begin_operation(OperationKind::Read, CancelToken::new())
            .expect("operation");
        handle.fail_next(MockCall::CardInfo, MockFault::Timeout);
        assert!(matches!(operation.check_card(), Err(NfcError::Timeout(_))));
    }

    #[test]
    fn cancellation_is_checked_before_io_and_at_start() {
        let (mut service, _handle) = service_with_card(card(1));
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert!(matches!(
            service.begin_operation(OperationKind::Read, cancelled),
            Err(NfcError::Cancelled)
        ));

        let mut operation = service
            .begin_operation(OperationKind::Read, CancelToken::new())
            .expect("operation");
        operation.cancel().cancel();
        assert!(matches!(operation.read_block(0), Err(NfcError::Cancelled)));
    }

    #[test]
    fn verified_write_reads_back_and_rejects_lost_writes() {
        let (mut service, handle) = service_with_card(card(1));
        handle.set_block(1, [0x00; 16]);
        let mut operation = service
            .begin_operation(OperationKind::Write, CancelToken::new())
            .expect("operation");

        operation
            .write_block_verified(1, [0xAA; 16])
            .expect("verified write");
        assert_eq!(handle.block(1), Some([0xAA; 16]));

        handle.fail_next(MockCall::WriteBlock, MockFault::WriteIgnored);
        assert!(matches!(
            operation.write_block_verified(1, [0x55; 16]),
            Err(NfcError::VerificationFailed(_))
        ));
        assert_eq!(handle.block(1), Some([0xAA; 16]));
    }

    #[test]
    fn operations_require_a_selected_reader() {
        let mut service = Service::with_transport(Arc::new(MockTransport::new()));
        service.refresh_devices().expect("refresh");
        assert!(matches!(
            service.begin_operation(OperationKind::Read, CancelToken::new()),
            Err(NfcError::NoReader)
        ));
        assert!(matches!(
            service.select_device("mock:missing"),
            Err(NfcError::InvalidArgument(_))
        ));
    }
}
