use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nfc_core::{CancelToken, NfcError, Result};
use nfc_transport::{DeviceInfo, NoTransport, Reader, ReaderCapabilities, Transport};

use crate::operation::{Operation, OperationKind};
use crate::reader::{
    MonitorState, ReaderMonitorConfig, ReaderSnapshot, ReaderUpdate, is_transient,
};
use crate::reader_lock::ReaderLock;

/// Reader selection and lifecycle state. A reader is opened only when an
/// operation or a monitor poll needs it and is closed on release, device loss,
/// or a selection change. `begin_operation` sets the busy status and the
/// returned lease clears it on drop, so a poll can never share the reader.
///
/// Opening also claims the cross-process lock for the reader's physical
/// identity; the lock is held until the connection closes.
pub struct DeviceManager {
    transport: Arc<dyn Transport>,
    devices: Vec<DeviceInfo>,
    selected: Option<String>,
    reader: Option<Box<dyn Reader>>,
    lock: Option<ReaderLock>,
    /// Overrides the runtime lock directory. Tests inject a private directory;
    /// hosts use the default runtime location.
    lock_dir: Option<PathBuf>,
    busy: Cell<bool>,
    /// A release parks the reader until an operation or an explicit selection.
    polling_paused: bool,
    monitor: MonitorState,
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::with_transport(Arc::new(NoTransport))
    }
}

impl DeviceManager {
    pub fn with_transport(transport: Arc<dyn Transport>) -> Self {
        Self::with_lock_dir(transport, None)
    }

    fn with_lock_dir(transport: Arc<dyn Transport>, lock_dir: Option<PathBuf>) -> Self {
        Self {
            transport,
            devices: Vec::new(),
            selected: None,
            reader: None,
            lock: None,
            lock_dir,
            busy: Cell::new(false),
            polling_paused: false,
            monitor: MonitorState::new(),
        }
    }

    #[cfg(test)]
    fn with_transport_and_lock_dir(transport: Arc<dyn Transport>, lock_dir: PathBuf) -> Self {
        Self::with_lock_dir(transport, Some(lock_dir))
    }

    pub fn devices(&self) -> &[DeviceInfo] {
        &self.devices
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn monitor_config(&self) -> ReaderMonitorConfig {
        self.monitor.config()
    }

    pub fn set_monitor_config(&mut self, config: ReaderMonitorConfig) {
        self.monitor.set_config(config);
    }

    pub fn capabilities(&self) -> ReaderCapabilities {
        self.monitor.capabilities()
    }

    pub fn state(&self) -> ReaderSnapshot {
        self.monitor
            .snapshot(self.busy.get(), self.selected.clone())
    }

    /// Refresh the device list from the transport. A selection that no longer
    /// exists is dropped and its reader is closed.
    pub fn refresh(&mut self) -> Result<()> {
        let devices = self.transport.enumerate()?;
        let selection_is_gone = self
            .selected
            .as_ref()
            .is_some_and(|selected| !devices.iter().any(|device| device.conn_string == *selected));
        self.devices = devices;
        if selection_is_gone {
            self.release()?;
            self.selected = None;
        } else if self.monitor.lifecycle() == crate::reader::ReaderStatus::Error {
            self.release()?;
            self.polling_paused = false;
            self.monitor.reset(Instant::now());
        }
        Ok(())
    }

    pub fn select(&mut self, conn_string: &str) -> Result<()> {
        if !self
            .devices
            .iter()
            .any(|device| device.conn_string == conn_string)
        {
            return Err(NfcError::InvalidArgument(format!(
                "reader {conn_string} is not in the device list"
            )));
        }
        if self.selected.as_deref() == Some(conn_string) {
            if self.polling_paused || self.monitor.lifecycle() == crate::reader::ReaderStatus::Error
            {
                self.release()?;
                self.polling_paused = false;
                self.monitor.reset(Instant::now());
            }
            return Ok(());
        }
        self.release()?;
        self.selected = Some(conn_string.to_owned());
        self.polling_paused = false;
        self.monitor.reset(Instant::now());
        Ok(())
    }

    /// Close the reader and start a new connection attempt with backoff reset.
    pub fn retry(&mut self) -> Result<()> {
        self.release()?;
        self.polling_paused = false;
        self.monitor.reset(Instant::now());
        Ok(())
    }

    pub fn release(&mut self) -> Result<()> {
        let result = if let Some(mut reader) = self.reader.take() {
            reader.close()
        } else {
            Ok(())
        };
        self.lock = None;
        self.busy.set(false);
        self.monitor.forget_card();
        self.polling_paused = true;
        result
    }

    /// Open the selected reader if needed and return the exclusive lease for
    /// one operation. Card presence observed here updates the monitor before
    /// the operation starts.
    pub fn begin_operation(
        &mut self,
        kind: OperationKind,
        cancel: CancelToken,
    ) -> Result<Operation<'_>> {
        cancel.check()?;
        self.polling_paused = false;
        let selected = self.selected.clone().ok_or(NfcError::NoReader)?;
        if self.reader.is_none()
            && let Err(error) = self.ensure_open(&selected)
        {
            self.record_open_failure(&error);
            return Err(error);
        }
        self.busy.set(true);
        self.monitor.begin_operation();

        let card = match self.reader.as_mut() {
            Some(reader) => reader.card_info(),
            None => Err(NfcError::NoReader),
        };
        let now = Instant::now();
        let card = match card {
            Ok(card) => {
                self.monitor.card_seen(card.clone(), now);
                self.monitor.sync(true);
                card
            }
            Err(NfcError::NoCard) => {
                self.monitor.card_missing(now);
                self.fail_operation();
                return Err(NfcError::NoCard);
            }
            Err(error) => {
                if is_transient(&error) {
                    self.monitor.reader_failure(&error, now);
                    self.close_reader();
                } else {
                    self.monitor.fatal(&error);
                }
                self.fail_operation();
                return Err(error);
            }
        };

        let busy = &self.busy;
        let reader = self.reader.as_mut().ok_or(NfcError::NoReader)?;
        Ok(Operation::new(kind, reader.as_mut(), card, cancel, busy))
    }

    /// Step the monitor with the host clock. Card and status transitions are
    /// returned as events; transport errors are state, not returned errors.
    pub fn poll(&mut self, now: Instant) -> ReaderUpdate {
        if self.busy.get() {
            self.monitor.sync(true);
            return self.monitor.update(true, self.selected.clone());
        }
        self.monitor.sync(false);
        let Some(selected) = self.selected.clone() else {
            return self.monitor.update(false, None);
        };
        if self.polling_paused {
            return self.monitor.update(false, self.selected.clone());
        }
        if !self.monitor.poll_due(now) {
            return self.monitor.update(false, self.selected.clone());
        }
        if self.reader.is_none()
            && let Err(error) = self.ensure_open(&selected)
        {
            self.record_open_failure_at(&error, now);
            self.monitor.sync(false);
            return self.monitor.update(false, self.selected.clone());
        }

        let result = match self.reader.as_mut() {
            Some(reader) => reader.card_info(),
            None => Err(NfcError::NoReader),
        };
        match result {
            Ok(card) => self.monitor.card_seen(card, now),
            Err(NfcError::NoCard) => self.monitor.card_missing(now),
            Err(error) if is_transient(&error) => {
                self.monitor.reader_failure(&error, now);
                self.close_reader();
            }
            Err(error) => self.monitor.fatal(&error),
        }
        self.monitor.sync(false);
        self.monitor.update(false, self.selected.clone())
    }

    fn ensure_open(&mut self, conn_string: &str) -> Result<()> {
        if let Some(reader) = self.reader.as_ref() {
            self.monitor
                .opened(reader.capabilities(), reader.firmware());
            return Ok(());
        }
        let lock = match self
            .devices
            .iter()
            .find(|device| device.conn_string == conn_string)
            .filter(|device| device.physical_id.is_some())
        {
            Some(device) => Some(match &self.lock_dir {
                Some(directory) => ReaderLock::acquire_in(directory, device)?,
                None => ReaderLock::acquire(device)?,
            }),
            None => None,
        };
        let mut reader = self.transport.open(conn_string)?;
        reader.open(conn_string)?;
        self.monitor
            .opened(reader.capabilities(), reader.firmware());
        self.reader = Some(reader);
        self.lock = lock;
        Ok(())
    }

    fn record_open_failure(&mut self, error: &NfcError) {
        self.record_open_failure_at(error, Instant::now());
    }

    fn record_open_failure_at(&mut self, error: &NfcError, now: Instant) {
        if is_transient(error) {
            self.monitor.reader_failure(error, now);
        } else {
            self.monitor.fatal(error);
        }
    }

    fn fail_operation(&mut self) {
        self.busy.set(false);
        self.monitor.end_operation();
    }

    fn close_reader(&mut self) {
        if let Some(mut reader) = self.reader.take() {
            let _ = reader.close();
        }
        self.lock = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nfc_core::CardType;
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::{MockCall, MockFault, MockTransport};

    use super::*;
    use crate::reader::ReaderEvent;

    fn card(uid: u8) -> nfc_core::CardInfo {
        nfc_core::CardInfo::new(vec![uid; 4], [0x00, 0x04], 0x08, CardType::MifareClassic1K)
    }

    fn manager() -> (DeviceManager, nfc_transport::mock::MockHandle) {
        let transport = MockTransport::new();
        let handle = transport.add(DeviceInfo::new("Mock", "mock:0"), Some(card(1)));
        let mut manager = DeviceManager::with_transport(Arc::new(transport));
        manager.refresh().expect("refresh");
        manager.select("mock:0").expect("select");
        (manager, handle)
    }

    #[test]
    fn monitor_reports_card_appearance_and_polls_at_the_interval() {
        let (mut manager, _handle) = manager();
        let now = Instant::now();
        let update = manager.poll(now);
        assert_eq!(update.snapshot.status, crate::reader::ReaderStatus::Polling);
        assert_eq!(update.snapshot.card, Some(card(1)));
        assert!(update.events.contains(&ReaderEvent::CardAppeared(card(1))));

        let update = manager.poll(now + crate::reader::DEFAULT_POLL_INTERVAL / 2);
        assert!(update.events.is_empty());
        let update = manager.poll(now + crate::reader::DEFAULT_POLL_INTERVAL);
        assert!(update.events.is_empty());
    }

    #[test]
    fn card_removal_is_reported_after_two_misses() {
        let (mut manager, handle) = manager();
        manager.set_monitor_config(ReaderMonitorConfig {
            poll_interval: std::time::Duration::ZERO,
            ..ReaderMonitorConfig::default()
        });
        let now = Instant::now();
        manager.poll(now);
        handle.set_card(None);

        let update = manager.poll(now + std::time::Duration::from_millis(1));
        assert_eq!(update.snapshot.card, Some(card(1)));
        assert!(
            !update
                .events
                .iter()
                .any(|event| matches!(event, ReaderEvent::CardRemoved(_)))
        );

        let update = manager.poll(now + std::time::Duration::from_millis(2));
        assert_eq!(update.snapshot.card, None);
        assert!(update.events.contains(&ReaderEvent::CardRemoved(card(1))));
    }

    #[test]
    fn refresh_restarts_a_reader_after_a_fatal_error() {
        let (mut manager, handle) = manager();
        let now = Instant::now();
        manager.poll(now);
        handle.fail_next(MockCall::CardInfo, MockFault::Cancelled);
        let update = manager.poll(now + crate::reader::DEFAULT_POLL_INTERVAL);
        assert_eq!(update.snapshot.status, crate::reader::ReaderStatus::Error);
        manager.refresh().expect("refresh");
        assert_eq!(
            manager.state().status,
            crate::reader::ReaderStatus::Connecting
        );
    }

    #[test]
    fn transient_timeout_closes_and_reconnects_the_reader() {
        let (mut manager, handle) = manager();
        manager.set_monitor_config(ReaderMonitorConfig {
            poll_interval: std::time::Duration::ZERO,
            reconnect_initial: std::time::Duration::from_millis(100),
            ..ReaderMonitorConfig::default()
        });
        let now = Instant::now();
        manager.poll(now);
        assert_eq!(handle.open_count(), 1);

        handle.fail_next(MockCall::CardInfo, MockFault::Timeout);
        let update = manager.poll(now + std::time::Duration::from_millis(1));
        assert_eq!(
            update.snapshot.status,
            crate::reader::ReaderStatus::Recovering
        );
        assert!(handle.close_count() >= 1);
        assert!(!handle.is_open());

        let update = manager.poll(now + std::time::Duration::from_millis(50));
        assert_eq!(
            update.snapshot.status,
            crate::reader::ReaderStatus::Recovering
        );
        assert_eq!(handle.open_count(), 1);

        let update = manager.poll(now + std::time::Duration::from_millis(200));
        assert_eq!(update.snapshot.status, crate::reader::ReaderStatus::Polling);
        assert_eq!(handle.open_count(), 2);
        assert_eq!(update.snapshot.card, Some(card(1)));
    }

    #[test]
    fn operation_lease_clears_busy_on_drop() {
        let (mut manager, _handle) = manager();
        let operation = manager
            .begin_operation(OperationKind::Read, CancelToken::new())
            .expect("operation");
        drop(operation);
        assert!(!manager.busy.get());
        assert_eq!(manager.state().status, crate::reader::ReaderStatus::Polling);
    }

    #[test]
    fn another_holder_blocks_the_reader_until_release() {
        let directory = tempfile::tempdir().expect("tempdir");
        let transport = MockTransport::new();
        let info = DeviceInfo::new("Mock", "mock:0").with_physical_id("test:manager:0");
        let handle = transport.add(info.clone(), Some(card(1)));
        let mut manager = DeviceManager::with_transport_and_lock_dir(
            Arc::new(transport),
            directory.path().to_path_buf(),
        );
        manager.refresh().expect("refresh");
        manager.select("mock:0").expect("select");

        let held = ReaderLock::acquire_in(directory.path(), &info).expect("other holder");
        let now = Instant::now();
        let update = manager.poll(now);
        assert_eq!(update.snapshot.status, crate::reader::ReaderStatus::Error);
        assert!(
            update
                .snapshot
                .error
                .as_deref()
                .is_some_and(|error| error.contains("in use")),
            "{:?}",
            update.snapshot.error
        );
        assert!(!handle.is_open(), "the busy reader was opened anyway");

        drop(held);
        manager.refresh().expect("refresh after contention");
        let update = manager.poll(now + std::time::Duration::from_millis(1));
        assert_eq!(update.snapshot.status, crate::reader::ReaderStatus::Polling);
        assert!(handle.is_open());
        assert!(ReaderLock::acquire_in(directory.path(), &info).is_err());

        manager.release().expect("release");
        ReaderLock::acquire_in(directory.path(), &info).expect("lock after release");
    }
}
