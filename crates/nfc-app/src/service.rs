use std::sync::Arc;

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_transport::{DeviceInfo, Transport};

use crate::classic::{
    BackupRecord, BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicKeyscanResult,
    ClassicNdefFormat, ClassicNdefFormatResult, ClassicNdefRead, ClassicNdefReadResult,
    ClassicNdefWrite, ClassicNdefWriteResult, ClassicRead, ClassicReadResult, ClassicRestore,
    ClassicRestoreResult, ClassicWriteResult, KeyStore,
};
use crate::device::DeviceManager;
use crate::operation::{Operation, OperationKind};
use crate::type2::Type2ReadResult;

/// Result of reading whichever supported card family is selected.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CardReadResult {
    Classic(ClassicReadResult),
    Type2(Type2ReadResult),
}

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

    /// Reads whichever supported card family is selected: a full Classic dump
    /// or a Type 2 image. The card is selected once for the whole read.
    pub fn read_card(
        &mut self,
        keys: &KeyStore,
        include_common_keys: bool,
        cancel: CancelToken,
    ) -> Result<CardReadResult> {
        let device = self.devices.selected().map(str::to_owned);
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let cancel = operation.cancel().clone();
        match operation.card().card_type {
            CardType::Type2 => {
                crate::type2::read(&mut operation, &cancel).map(CardReadResult::Type2)
            }
            CardType::MifareClassic1K | CardType::MifareClassic4K => {
                let mut result =
                    crate::classic::read::read_dump(&mut operation, keys, include_common_keys)?;
                result.dump.metadata.device = device;
                Ok(CardReadResult::Classic(result))
            }
            _ => Err(NfcError::Unsupported(
                "the selected card is not a supported Classic or Type 2 tag",
            )),
        }
    }

    /// Reads the whole card with candidate keys. Authentication failures
    /// produce a partial dump with explicit warnings.
    pub fn classic_read(&mut self, request: ClassicRead<'_>) -> Result<ClassicReadResult> {
        let ClassicRead {
            keys,
            include_common_keys,
            cancel,
        } = request;
        let device = self.devices.selected().map(str::to_owned);
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let mut result =
            crate::classic::read::read_dump(&mut operation, keys, include_common_keys)?;
        result.dump.metadata.device = device;
        Ok(result)
    }

    /// Reads one block after authenticating its sector with the first
    /// candidate key the card accepts.
    pub fn classic_read_block(
        &mut self,
        block: usize,
        keys: &KeyStore,
        include_common_keys: bool,
        cancel: CancelToken,
    ) -> Result<crate::classic::read::ClassicBlockReadResult> {
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

    /// Reads the card and parses the MAD, NFC sector map, and NDEF TLV.
    pub fn classic_ndef_read(
        &mut self,
        request: ClassicNdefRead<'_>,
    ) -> Result<ClassicNdefReadResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        crate::classic::ndef::read(&mut operation, request)
    }

    /// Parses a Classic dump without reading hardware.
    pub fn classic_ndef_inspect(
        &self,
        dump: &nfc_classic::Dump,
    ) -> Result<nfc_classic_ndef::ClassicNdefInfo> {
        crate::classic::ndef::inspect(dump)
    }

    /// Writes the mandatory NDEF Message TLV with a backup and verification.
    pub fn classic_ndef_write(
        &mut self,
        request: ClassicNdefWrite<'_>,
    ) -> Result<ClassicNdefWriteResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        crate::classic::ndef::write(&mut operation, request)
    }

    /// Formats the MAD and NFC sectors with a backup and confirmation.
    pub fn classic_ndef_format(
        &mut self,
        request: ClassicNdefFormat<'_>,
    ) -> Result<ClassicNdefFormatResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        crate::classic::ndef::format(&mut operation, request)
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
    use nfc_core::{BLOCK_SIZE, CardType, Key, KeyType, NfcError};
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

    fn type2_card(uid: u8) -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, uid],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )
    }

    fn seed_ntag(handle: &MockHandle) {
        for page in 0..45u8 {
            handle.set_page(page, [page; 4]);
        }
        handle.set_page(2, [0x80, 0x48, 0x00, 0x00]);
        handle.set_page(3, [0xE1, 0x10, 0x12, 0x00]);
        handle.set_page(40, [0x00, 0x00, 0x00, 0xBD]);
        handle.set_page(41, [0x00, 0x00, 0x00, 0xFF]);
        handle.set_page(42, [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn read_card_dispatches_by_family() {
        let transport = MockTransport::new();
        let classic = transport.add(
            DeviceInfo::new("Mock Classic", "mock:classic"),
            Some(card(1)),
        );
        classic.set_block(0, [1; BLOCK_SIZE]);
        let ntag = transport.add(
            DeviceInfo::new("Mock NTAG", "mock:ntag"),
            Some(type2_card(0x80)),
        );
        seed_ntag(&ntag);
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");

        service.select_device("mock:ntag").expect("select tag");
        let read = service
            .read_card(&KeyStore::new(), true, CancelToken::new())
            .expect("type 2 read");
        match read {
            CardReadResult::Type2(read) => {
                assert_eq!(read.card.card_type, CardType::Type2);
                assert_eq!(read.image.layout.total_pages, 45);
                assert!(read.image.is_complete());
            }
            other => panic!("unexpected read result: {other:?}"),
        }

        service
            .select_device("mock:classic")
            .expect("select classic");
        let read = service
            .read_card(&KeyStore::new(), true, CancelToken::new())
            .expect("classic read");
        assert!(matches!(read, CardReadResult::Classic(_)));
    }

    #[test]
    fn type2_identity_mismatch_is_refused_before_the_backup() {
        let (mut service, handle) = service_with_card(type2_card(0x80));
        seed_ntag(&handle);
        let backups = tempfile::tempdir().expect("backup dir");
        let backup = BackupStore::new(backups.path());
        handle.set_card(Some(type2_card(0x81)));
        let request = crate::type2::Type2PageWrite::new(4, [0xAA; 4], &backup)
            .expected_card(type2_card(0x80));
        assert!(matches!(
            service.type2_write_page(request),
            Err(NfcError::CardChanged)
        ));
        assert_eq!(handle.page(4), Some([4; 4]));
        assert_eq!(
            std::fs::read_dir(backups.path())
                .expect("backup dir")
                .count(),
            0
        );
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
