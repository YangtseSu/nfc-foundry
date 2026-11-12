use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_transport::{DeviceInfo, Transport};

use crate::classic::{
    BackupRecord, BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicKeyscanResult,
    ClassicNdefFormat, ClassicNdefFormatResult, ClassicNdefRead, ClassicNdefReadResult,
    ClassicNdefWrite, ClassicNdefWriteResult, ClassicRead, ClassicReadResult, ClassicRestore,
    ClassicRestoreResult, ClassicUidExecution, ClassicUidPlan, ClassicUidPreflight,
    ClassicUidResult, ClassicWorkbench, ClassicWriteExecution, ClassicWritePlan,
    ClassicWritePreflight, ClassicWriteResult, KeyStore, WorkbenchMergeReport, WorkbenchSnapshot,
    WorkbenchValidation, WriteToken,
};
use crate::device::DeviceManager;
use crate::diagnostics::{DriverInfo, ReaderDevice, SelfCheck};
use crate::operation::{Operation, OperationKind};
use crate::reader::{ReaderMonitorConfig, ReaderSnapshot, ReaderUpdate};
use crate::type2::{Type2ReadRequest, Type2ReadResult};

/// Result of reading whichever supported card family is selected.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CardReadResult {
    Classic(ClassicReadResult),
    Type2(Type2ReadResult),
    /// The card was selected but its SAK/ATQA does not identify a supported
    /// read path. Only the selection identity (UID, ATQA, SAK) is available.
    IdentityOnly(CardInfo),
}

struct PendingWrite {
    plan: ClassicWritePlan,
    baseline: Option<nfc_classic::Dump>,
    expires: Instant,
}

struct PendingUidWrite {
    plan: ClassicUidPlan,
    expires: Instant,
}

/// Application facade shared by the GUI and `nfctl`.
#[derive(Default)]
pub struct Service {
    devices: DeviceManager,
    workbench: Option<ClassicWorkbench>,
    pending_writes: HashMap<u64, PendingWrite>,
    pending_uid_writes: HashMap<u64, PendingUidWrite>,
    next_write_token: u64,
}

impl Service {
    pub fn with_transport(transport: Arc<dyn Transport>) -> Self {
        Self {
            devices: DeviceManager::with_transport(transport),
            ..Self::default()
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

    /// Close the selected reader and start a fresh connection attempt. A
    /// monitor in `Error` also retries on re-selecting the same reader.
    pub fn retry_reader(&mut self) -> Result<()> {
        self.devices.retry()
    }

    /// Step the reader monitor with the wall clock.
    pub fn poll_reader(&mut self) -> ReaderUpdate {
        self.poll_reader_at(Instant::now())
    }

    /// Step the reader monitor with a caller-supplied clock. Hosts that drive
    /// polling from another thread use [`Self::poll_reader`]; tests and
    /// deterministic schedulers use this entry point.
    pub fn poll_reader_at(&mut self, now: Instant) -> ReaderUpdate {
        self.devices.poll(now)
    }

    /// Current connection state, card presence, and reader capabilities.
    pub fn reader_state(&self) -> ReaderSnapshot {
        self.devices.state()
    }

    pub fn reader_monitor_config(&self) -> ReaderMonitorConfig {
        self.devices.monitor_config()
    }

    pub fn set_reader_monitor_config(&mut self, config: ReaderMonitorConfig) {
        self.devices.set_monitor_config(config);
    }

    pub fn scan(&mut self) -> Result<CardInfo> {
        let operation = self.begin_operation(OperationKind::Scan, CancelToken::new())?;
        Ok(operation.card().clone())
    }

    /// Builds the self-check document. Reader enumeration is the only device
    /// access: the report never selects, reads, or authenticates a card.
    pub fn self_check(&mut self, drivers: Vec<DriverInfo>, tables_dir: Option<&Path>) -> SelfCheck {
        let devices = match self.devices.refresh() {
            Ok(()) => Ok(self
                .devices
                .devices()
                .iter()
                .map(|device| ReaderDevice {
                    name: device.name.clone(),
                    conn_string: device.conn_string.clone(),
                })
                .collect()),
            Err(error) => Err(error.to_string()),
        };
        let snapshot = self.devices.state();
        SelfCheck::new(drivers, devices, &snapshot, tables_dir)
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
        type2_auth: Option<nfc_type2::Type2Auth>,
        cancel: CancelToken,
    ) -> Result<CardReadResult> {
        let device = self.devices.selected().map(str::to_owned);
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let cancel = operation.cancel().clone();
        match operation.card().card_type {
            CardType::Type2 => crate::type2::read(
                &mut operation,
                Type2ReadRequest::new(cancel).auth(type2_auth),
            )
            .map(CardReadResult::Type2),
            CardType::MifareClassic1K | CardType::MifareClassic4K => {
                let mut result = crate::classic::read::read_dump(
                    &mut operation,
                    keys,
                    include_common_keys,
                    true,
                )?;
                result.dump.metadata.device = device;
                Ok(CardReadResult::Classic(result))
            }
            CardType::MifareMini => Err(NfcError::Unsupported(
                "MIFARE Mini is identified but has no supported layout; read and write are unavailable",
            )),
            _ => Ok(CardReadResult::IdentityOnly(operation.card().clone())),
        }
    }

    /// Reads the whole card with candidate keys. Authentication failures
    /// produce a partial dump with explicit warnings.
    pub fn classic_read(&mut self, request: ClassicRead<'_>) -> Result<ClassicReadResult> {
        let ClassicRead {
            keys,
            include_common_keys,
            cancel,
            allow_identity_only,
        } = request;
        let device = self.devices.selected().map(str::to_owned);
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let mut result = crate::classic::read::read_dump(
            &mut operation,
            keys,
            include_common_keys,
            allow_identity_only,
        )?;
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
        let source_fingerprint = crate::classic::dump_fingerprint(&request.dump);
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        let result = crate::classic::restore::restore(&mut operation, request)?;
        drop(operation);
        self.apply_written_baseline(source_fingerprint, &result);
        Ok(result)
    }

    /// Reads the live card and returns a validated write plan without writing.
    pub fn classic_write_preflight(
        &mut self,
        request: ClassicWritePreflight<'_>,
    ) -> Result<ClassicWritePlan> {
        let token = self.new_write_token();
        let cancel = request.cancel.clone();
        let plan = {
            let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
            crate::classic::restore::preflight(&mut operation, &request, token)?
        };
        let now = Instant::now();
        self.pending_writes
            .retain(|_, pending| pending.expires > now);
        self.pending_writes.insert(
            token.value(),
            PendingWrite {
                plan: plan.clone(),
                baseline: request.baseline,
                expires: Instant::now() + crate::classic::WRITE_PLAN_TTL,
            },
        );
        Ok(plan)
    }

    /// Executes a preflight plan after rechecking the token, expiry, card, and
    /// source image. The token is consumed even when execution fails.
    pub fn classic_write_execute(
        &mut self,
        request: ClassicWriteExecution<'_>,
    ) -> Result<ClassicRestoreResult> {
        let Some(pending) = self.pending_writes.remove(&request.token.value()) else {
            return Err(NfcError::InvalidArgument(
                "the write token is unknown or was already used".to_owned(),
            ));
        };
        if Instant::now() > pending.expires {
            return Err(NfcError::InvalidArgument(
                "the write plan expired; run the preflight again".to_owned(),
            ));
        }
        if crate::classic::dump_fingerprint(&request.dump) != pending.plan.source_fingerprint {
            return Err(NfcError::CardChanged);
        }
        let source_fingerprint = pending.plan.source_fingerprint;
        let selection = pending.plan.selection.clone();
        let expected_card = pending.plan.card.clone();
        let continue_on_failure = pending.plan.continue_on_failure;
        let mut restore = ClassicRestore::new(request.dump, request.keys, request.backup)
            .include_common_keys(request.include_common_keys)
            .confirmed(request.confirmed)
            .continue_on_failure(continue_on_failure)
            .selection(selection)
            .expected_card(expected_card)
            .cancel(request.cancel.clone());
        if let Some(baseline) = pending.baseline {
            restore = restore.baseline(baseline);
        }
        let mut operation = self.begin_operation(OperationKind::Write, request.cancel)?;
        let result = crate::classic::restore::restore_with_plan(
            &mut operation,
            restore,
            request.advanced,
            &pending.plan,
        )?;
        drop(operation);
        if let Some(uid) = &result.uid
            && matches!(
                uid.state,
                crate::classic::RestoreUidState::Applied
                    | crate::classic::RestoreUidState::Uncertain
            )
        {
            let belongs_to_old_card = self.workbench.as_ref().is_some_and(|workbench| {
                workbench
                    .working()
                    .card
                    .as_ref()
                    .is_some_and(|card| card.same_identity(&uid.card_before))
            });
            if belongs_to_old_card {
                self.workbench = None;
            }
        }
        self.apply_written_baseline(source_fingerprint, &result);
        Ok(result)
    }

    /// Reads sector 0 and returns a guarded UID write plan without writing.
    pub fn classic_uid_preflight(
        &mut self,
        request: ClassicUidPreflight<'_>,
    ) -> Result<ClassicUidPlan> {
        let token = self.new_write_token();
        let cancel = request.cancel.clone();
        let plan = {
            let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
            crate::classic::uid::preflight(&mut operation, &request, token)?
        };
        let now = Instant::now();
        self.pending_uid_writes
            .retain(|_, pending| pending.expires > now);
        self.pending_uid_writes.insert(
            plan.token.value(),
            PendingUidWrite {
                plan: plan.clone(),
                expires: now + crate::classic::WRITE_PLAN_TTL,
            },
        );
        Ok(plan)
    }

    /// Executes a UID plan after rechecking the card and the sector-0 state.
    /// The token is consumed even when execution fails. A changed UID
    /// invalidates a workbench that belonged to the old card identity.
    pub fn classic_uid_execute(
        &mut self,
        request: ClassicUidExecution<'_>,
    ) -> Result<ClassicUidResult> {
        let Some(pending) = self.pending_uid_writes.remove(&request.token.value()) else {
            return Err(NfcError::InvalidArgument(
                "the UID write token is unknown or was already used".to_owned(),
            ));
        };
        if Instant::now() > pending.expires {
            return Err(NfcError::InvalidArgument(
                "the UID write plan expired; run the preflight again".to_owned(),
            ));
        }
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        let result = crate::classic::uid::execute(&mut operation, request, &pending.plan)?;
        drop(operation);
        if result.changed {
            let belongs_to_old_card = self.workbench.as_ref().is_some_and(|workbench| {
                workbench
                    .working()
                    .card
                    .as_ref()
                    .is_some_and(|card| card.same_identity(&result.card_before))
            });
            if belongs_to_old_card {
                self.workbench = None;
            }
        }
        Ok(result)
    }

    fn apply_written_baseline(&mut self, source_fingerprint: u64, result: &ClassicRestoreResult) {
        if result.dry_run {
            return;
        }
        let Some(workbench) = self.workbench.as_mut() else {
            return;
        };
        if workbench.fingerprint() != source_fingerprint {
            return;
        }
        let written: Vec<usize> = result
            .blocks
            .iter()
            .filter(|block| block.state == crate::classic::RestoreBlockState::Written)
            .map(|block| block.block)
            .collect();
        if !written.is_empty() {
            let _ = workbench.mark_written(&written);
        }
    }

    fn new_write_token(&mut self) -> WriteToken {
        let value = self.next_write_token.wrapping_add(1);
        self.next_write_token = value;
        WriteToken::new(value)
    }

    pub fn workbench_snapshot(&self) -> Option<WorkbenchSnapshot> {
        self.workbench.as_ref().map(ClassicWorkbench::snapshot)
    }

    pub fn workbench_dump(&self) -> Option<nfc_classic::Dump> {
        self.workbench
            .as_ref()
            .map(|workbench| workbench.working().clone())
    }

    pub fn workbench_baseline_dump(&self) -> Option<nfc_classic::Dump> {
        self.workbench
            .as_ref()
            .map(|workbench| workbench.baseline().clone())
    }

    pub fn workbench_load(&mut self, dump: nfc_classic::Dump) -> Result<WorkbenchSnapshot> {
        let workbench = ClassicWorkbench::new(dump)?;
        let snapshot = workbench.snapshot();
        self.workbench = Some(workbench);
        Ok(snapshot)
    }

    pub fn workbench_clear(&mut self) {
        self.workbench = None;
    }

    /// Exports the working image in the requested dump format.
    pub fn workbench_export(&self, format: nfc_classic::DumpFormat) -> Result<Vec<u8>> {
        let workbench = self
            .workbench
            .as_ref()
            .ok_or_else(|| NfcError::InvalidArgument("no workbench image is loaded".to_owned()))?;
        nfc_classic::exchange::export(workbench.working(), format)
    }

    pub fn workbench_edit(
        &mut self,
        block: usize,
        data: [u8; nfc_core::BLOCK_SIZE],
    ) -> Result<WorkbenchSnapshot> {
        let workbench = self.workbench_mut()?;
        workbench.edit_block(block, data)?;
        Ok(workbench.snapshot())
    }

    pub fn workbench_edit_ascii(&mut self, block: usize, text: &str) -> Result<WorkbenchSnapshot> {
        let workbench = self.workbench_mut()?;
        workbench.edit_ascii_block(block, text)?;
        Ok(workbench.snapshot())
    }

    pub fn workbench_undo(&mut self) -> Result<WorkbenchSnapshot> {
        let workbench = self.workbench_mut()?;
        workbench.undo()?;
        Ok(workbench.snapshot())
    }

    pub fn workbench_revert(&mut self) -> Result<WorkbenchSnapshot> {
        let workbench = self.workbench_mut()?;
        workbench.revert()?;
        Ok(workbench.snapshot())
    }

    pub fn workbench_mark_saved(&mut self) -> Result<WorkbenchSnapshot> {
        let workbench = self.workbench_mut()?;
        workbench.mark_saved();
        Ok(workbench.snapshot())
    }

    pub fn workbench_mark_written(&mut self, blocks: &[usize]) -> Result<WorkbenchSnapshot> {
        let workbench = self.workbench_mut()?;
        workbench.mark_written(blocks)?;
        Ok(workbench.snapshot())
    }

    pub fn workbench_merge(&mut self, read: &nfc_classic::Dump) -> Result<WorkbenchMergeReport> {
        self.workbench_mut()?.merge_read(read)
    }

    pub fn workbench_merge_block(
        &mut self,
        block: usize,
        data: [u8; nfc_core::BLOCK_SIZE],
    ) -> Result<WorkbenchMergeReport> {
        self.workbench_mut()?.merge_block(block, data)
    }

    pub fn workbench_validate(&self) -> Result<WorkbenchValidation> {
        self.workbench
            .as_ref()
            .map(ClassicWorkbench::validate)
            .ok_or_else(|| NfcError::InvalidArgument("no workbench image is loaded".to_owned()))
    }

    fn workbench_mut(&mut self) -> Result<&mut ClassicWorkbench> {
        self.workbench
            .as_mut()
            .ok_or_else(|| NfcError::InvalidArgument("no workbench image is loaded".to_owned()))
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
        let dump =
            crate::classic::read::read_dump(&mut operation, keys, include_common_keys, false)?.dump;
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
    fn mini_reports_its_type_but_has_no_read_path() {
        let mini = CardInfo::new(
            vec![0x04, 0xA2, 0x24, 0x8A],
            [0x00, 0x04],
            0x09,
            CardType::MifareMini,
        );
        let (mut service, _handle) = service_with_card(mini);
        assert_eq!(
            service.card_info().expect("card info").card_type,
            CardType::MifareMini
        );
        let result = service.read_card(&KeyStore::new(), false, None, CancelToken::new());
        match result {
            Err(NfcError::Unsupported(message)) => {
                assert!(message.contains("MIFARE Mini"), "{message}");
            }
            other => panic!("Mini read must be refused, got {other:?}"),
        }
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
        let update = service.poll_reader_at(Instant::now() + std::time::Duration::from_secs(1));
        assert_eq!(update.snapshot.status, crate::ReaderStatus::Connecting);
        assert_eq!(handle.open_count(), 1, "a release must park the monitor");

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
        transport.add(
            DeviceInfo::new("Mock Unknown", "mock:unknown"),
            Some(CardInfo::new(
                vec![0x08, 0x11, 0x22, 0x33],
                [0x00, 0x04],
                0x00,
                CardType::Unknown,
            )),
        );
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("refresh");

        service.select_device("mock:ntag").expect("select tag");
        let read = service
            .read_card(&KeyStore::new(), true, None, CancelToken::new())
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
            .read_card(&KeyStore::new(), true, None, CancelToken::new())
            .expect("classic read");
        assert!(matches!(read, CardReadResult::Classic(_)));

        service
            .select_device("mock:unknown")
            .expect("select unknown");
        let read = service
            .read_card(&KeyStore::new(), true, None, CancelToken::new())
            .expect("identity-only read");
        match read {
            CardReadResult::IdentityOnly(card) => {
                assert_eq!(card.card_type, CardType::Unknown);
                assert_eq!(card.uid, vec![0x08, 0x11, 0x22, 0x33]);
            }
            other => panic!("unexpected read result: {other:?}"),
        }
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
    fn expired_and_unknown_write_tokens_are_refused_before_io() {
        use nfc_classic::{Dump, Layout};

        let (mut service, handle) = service_with_card(card(1));
        let directory = tempfile::tempdir().expect("temp dir");
        let backup = BackupStore::new(directory.path());
        let keys = KeyStore::new();
        let dump = Dump::new(Layout::Classic1K).with_card(card(1));
        let token = WriteToken::new(77);
        let plan = ClassicWritePlan {
            token,
            layout: Layout::Classic1K,
            card: card(1),
            selection: crate::classic::RestoreSelection::All,
            selected_blocks: Vec::new(),
            blocks: Vec::new(),
            unchanged_blocks: 0,
            skipped_unknown_blocks: 0,
            warnings: Vec::new(),
            expires_at: 0,
            source_fingerprint: crate::classic::dump_fingerprint(&dump),
            card_fingerprint: 0,
            continue_on_failure: false,
            uid: None,
        };
        service.pending_writes.insert(
            77,
            PendingWrite {
                plan,
                baseline: None,
                expires: Instant::now() - std::time::Duration::from_secs(1),
            },
        );
        let result = service.classic_write_execute(
            ClassicWriteExecution::new(token, dump.clone(), &keys, &backup).confirmed(true),
        );
        assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
        assert_eq!(handle.block(4), None);
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("backup dir")
                .count(),
            0
        );

        let result = service.classic_write_execute(
            ClassicWriteExecution::new(WriteToken::new(78), dump, &keys, &backup).confirmed(true),
        );
        assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
        assert_eq!(handle.block(4), None);
    }

    #[test]
    fn expired_and_unknown_uid_tokens_are_refused_before_io() {
        use nfc_classic::Layout;

        let (mut service, handle) = service_with_card(card(1));
        let directory = tempfile::tempdir().expect("temp dir");
        let backup = BackupStore::new(directory.path());
        let keys = KeyStore::new();
        let plan = ClassicUidPlan {
            token: WriteToken::new(91),
            card: card(1),
            layout: Layout::Classic1K,
            current_uid: [1; 4],
            target_uid: [2; 4],
            old_block0: [0; BLOCK_SIZE],
            new_block0: [0; BLOCK_SIZE],
            old_trailer: [0; BLOCK_SIZE],
            sector0_key: (KeyType::A, Key::new([0xFF; 6])),
            write_key: Some((KeyType::A, Key::new([0xFF; 6]))),
            write_keys: crate::classic::WriteKeyTypes { a: true, b: true },
            backends: vec![crate::classic::UidWriteBackend::Standard],
            changed: true,
            warnings: Vec::new(),
            expires_at: 0,
            card_fingerprint: 0,
        };
        service.pending_uid_writes.insert(
            91,
            PendingUidWrite {
                plan,
                expires: Instant::now() - std::time::Duration::from_secs(1),
            },
        );
        let expired = ClassicUidExecution::new(WriteToken::new(91), &keys, &backup)
            .advanced(true)
            .confirmed(true);
        assert!(matches!(
            service.classic_uid_execute(expired),
            Err(NfcError::InvalidArgument(_))
        ));
        let unknown = ClassicUidExecution::new(WriteToken::new(92), &keys, &backup)
            .advanced(true)
            .confirmed(true);
        assert!(matches!(
            service.classic_uid_execute(unknown),
            Err(NfcError::InvalidArgument(_))
        ));
        assert_eq!(handle.block(0), None);
    }

    #[test]
    fn an_operation_lease_excludes_pollers_and_other_operations() {
        let (service, _handle) = service_with_card(card(1));
        let shared = Arc::new(std::sync::Mutex::new(service));
        let mut guard = shared.lock().expect("service lock");
        let operation = guard
            .begin_operation(OperationKind::Read, CancelToken::new())
            .expect("operation");
        let probe = shared.clone();
        let blocked = std::thread::spawn(move || probe.try_lock().is_err())
            .join()
            .expect("probe thread");
        assert!(blocked, "a second locker entered during the operation");
        drop(operation);
        drop(guard);
        assert!(shared.try_lock().is_ok(), "the lease did not release");
    }

    #[test]
    fn uid_service_requires_the_manufacturer_write_capability() {
        use nfc_transport::ReaderCapabilities;

        let (mut service, handle) = service_with_card(card(1));
        handle.set_capabilities(ReaderCapabilities::default());
        let keys = KeyStore::new();
        let request = ClassicUidPreflight::new([2, 2, 2, 2], &keys).advanced(true);
        let result = service.classic_uid_preflight(request);
        assert!(matches!(result, Err(NfcError::Unsupported(_))));
        assert_eq!(handle.block(0), None);
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

    #[test]
    fn self_check_enumerates_readers_and_reports_open_firmware() {
        let (mut service, _handle) = service_with_card(card(1));
        service.poll_reader();
        let report = service.self_check(vec![DriverInfo::new("Mock transport", "mock:")], None);
        assert!(report.readers.available);
        assert_eq!(report.readers.devices.len(), 1);
        assert_eq!(report.readers.devices[0].conn_string, "mock:0");
        assert_eq!(
            report.readers.firmware.as_deref(),
            Some("mock reader firmware")
        );
        assert!(report.readers.capabilities.classic_read);
        assert_eq!(report.app.version, env!("CARGO_PKG_VERSION"));
    }
}
