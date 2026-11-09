//! Retained workspace state and application actions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gpui_kit::component::{WindowExt, button::ButtonVariant, input::InputState};
use gpui_kit::{App, AppContext as _, Context, Entity, SharedString, WeakEntity, Window};
use nfc_app::Service;
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicRead, ClassicRestore, KeyEntry, KeyKind,
    KeySource, KeyStore, KeyTypes,
};
use nfc_classic::{BlockStatus, Dump, DumpDiff, DumpFormat, exchange};
use nfc_core::{BLOCK_SIZE, NfcError, Result as NfcResult};

use crate::model::{DeviceSummary, StatusMessage, WorkspaceTab};

/// GPUI Kit MIFARE Classic workspace.
///
/// All card operations run through a shared [`Service`] on a background
/// executor. The view never performs reader I/O in `render`.
pub struct ClassicWorkspace {
    pub(crate) service: Arc<Mutex<Service>>,
    pub(crate) keys: KeyStore,
    pub(crate) key_store_path: Option<PathBuf>,
    pub(crate) backup: BackupStore,
    pub(crate) devices: Vec<DeviceSummary>,
    pub(crate) selected_device: Option<String>,
    pub(crate) tab: WorkspaceTab,
    pub(crate) dump: Option<Dump>,
    pub(crate) previous_dump: Option<Dump>,
    pub(crate) diff: Option<DumpDiff>,
    pub(crate) selected_block: Option<usize>,
    pub(crate) busy: Option<SharedString>,
    pub(crate) status: Option<StatusMessage>,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) block_input: Entity<InputState>,
    pub(crate) key_hex_input: Entity<InputState>,
    pub(crate) key_sector_input: Entity<InputState>,
    pub(crate) key_kind: usize,
    pub(crate) last_backup: Option<PathBuf>,
}

impl ClassicWorkspace {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        service: Arc<Mutex<Service>>,
        keys: KeyStore,
        key_store_path: Option<PathBuf>,
        backup: BackupStore,
    ) -> Self {
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("Dump file path"));
        let block_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("32 hexadecimal digits"));
        let key_hex_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("12 hexadecimal digits"));
        let key_sector_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Sector (blank = all)"));
        let mut workspace = Self {
            service,
            keys,
            key_store_path,
            backup,
            devices: Vec::new(),
            selected_device: None,
            tab: WorkspaceTab::Blocks,
            dump: None,
            previous_dump: None,
            diff: None,
            selected_block: None,
            busy: None,
            status: None,
            path_input,
            block_input,
            key_hex_input,
            key_sector_input,
            key_kind: 2,
            last_backup: None,
        };
        workspace.reload_devices(cx);
        workspace
    }

    /// Current card image, if a read or load succeeded.
    pub fn dump(&self) -> Option<&Dump> {
        self.dump.as_ref()
    }

    /// Previous card image used for the diff tab.
    pub fn previous_dump(&self) -> Option<&Dump> {
        self.previous_dump.as_ref()
    }

    /// Latest dump comparison.
    pub fn diff(&self) -> Option<&DumpDiff> {
        self.diff.as_ref()
    }

    /// Key store currently loaded in the workspace.
    pub fn keys(&self) -> &KeyStore {
        &self.keys
    }

    /// Selected block number.
    pub fn selected_block(&self) -> Option<usize> {
        self.selected_block
    }

    /// Current status message.
    pub fn status(&self) -> Option<&StatusMessage> {
        self.status.as_ref()
    }

    /// In-flight operation label.
    pub fn busy(&self) -> Option<&SharedString> {
        self.busy.as_ref()
    }

    pub(crate) fn reload_devices(&mut self, cx: &mut Context<Self>) {
        let mut failure = None;
        let (devices, selected) = {
            let mut service = lock_service(&self.service);
            if let Err(error) = service.refresh_devices() {
                failure = Some(error);
            }
            let mut selected = service.selected_device().map(str::to_owned);
            if failure.is_none()
                && selected.is_none()
                && let Some(first) = service
                    .devices()
                    .first()
                    .map(|device| device.conn_string.clone())
            {
                match service.select_device(&first) {
                    Ok(()) => selected = Some(first),
                    Err(error) => failure = Some(error),
                }
            }
            let devices = service
                .devices()
                .iter()
                .map(|device| DeviceSummary {
                    name: device.name.clone(),
                    conn_string: device.conn_string.clone(),
                })
                .collect();
            (devices, selected)
        };
        self.devices = devices;
        self.selected_device = selected;
        if let Some(error) = failure {
            self.status = Some(StatusMessage::error(format!("Reader list failed: {error}")));
        } else if self.status.is_none() {
            self.status = Some(StatusMessage::info("Select a reader and read a card."));
        }
        cx.notify();
    }

    pub(crate) fn select_device(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(device) = self.devices.get(index).cloned() else {
            return;
        };
        let result = lock_service(&self.service).select_device(&device.conn_string);
        match result {
            Ok(()) => {
                self.selected_device = Some(device.conn_string);
                self.dump = None;
                self.previous_dump = None;
                self.diff = None;
                self.selected_block = None;
                self.status = Some(StatusMessage::info(format!("Selected {}", device.name)));
            }
            Err(error) => {
                self.status = Some(StatusMessage::error(format!(
                    "Couldn't select reader: {error}"
                )));
            }
        }
        cx.notify();
    }

    pub(crate) fn read_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(SharedString::from("Reading card"));
        cx.notify();
        let keys = self.keys.clone();
        let this = cx.weak_entity();
        spawn_service(
            this,
            self.service.clone(),
            window,
            cx,
            move |service: &mut Service| {
                service.classic_read(ClassicRead::new(&keys).include_common_keys(true))
            },
            |this, result, window, cx| match result {
                Ok(read) => {
                    let warnings = read.warnings.len();
                    this.previous_dump = this.dump.take();
                    this.diff = this
                        .previous_dump
                        .as_ref()
                        .and_then(|before| nfc_classic::diff::diff(before, &read.dump).ok());
                    this.dump = Some(read.dump);
                    this.selected_block = Some(0);
                    this.sync_block_input(window, cx);
                    this.status = Some(if warnings == 0 {
                        StatusMessage::success("Card read")
                    } else {
                        StatusMessage::warning(format!("Card read with {warnings} warning(s)"))
                    });
                }
                Err(error) => {
                    this.status = Some(StatusMessage::error(format!("Read failed: {error}")));
                }
            },
        );
    }

    pub(crate) fn save_dump(&mut self, cx: &mut Context<Self>) {
        let path = self.path(cx);
        if path.is_empty() {
            self.status = Some(StatusMessage::error("Enter a dump file path first."));
            cx.notify();
            return;
        }
        let result = self
            .dump
            .as_ref()
            .ok_or_else(|| NfcError::InvalidArgument("read or load a card first".to_owned()))
            .and_then(|dump| exchange::export(dump, DumpFormat::Project))
            .and_then(|bytes| write_file(Path::new(&path), &bytes));
        match result {
            Ok(()) => {
                self.status = Some(StatusMessage::success(format!("Saved {path}")));
            }
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Couldn't save dump: {error}")));
            }
        }
        cx.notify();
    }

    pub(crate) fn load_dump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path(cx);
        if path.is_empty() {
            self.status = Some(StatusMessage::error("Enter a dump file path first."));
            cx.notify();
            return;
        }
        let result = fs::read(&path)
            .map_err(|error| NfcError::Io(format!("could not read {path}: {error}")))
            .and_then(|bytes| exchange::load(&bytes));
        match result {
            Ok(load) => {
                let warnings = load.warnings.len();
                self.previous_dump = self.dump.take();
                self.diff = self
                    .previous_dump
                    .as_ref()
                    .and_then(|before| nfc_classic::diff::diff(before, &load.dump).ok());
                self.dump = Some(load.dump);
                self.selected_block = Some(0);
                self.sync_block_input(window, cx);
                self.status = Some(if warnings == 0 {
                    StatusMessage::success(format!("Loaded {path} ({})", load.format.name()))
                } else {
                    StatusMessage::warning(format!(
                        "Loaded {path} ({}) with {warnings} warning(s)",
                        load.format.name()
                    ))
                });
            }
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Couldn't load dump: {error}")));
            }
        }
        cx.notify();
    }

    pub(crate) fn restore_loaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(dump) = self.dump.clone() else {
            self.status = Some(StatusMessage::error("Read or load a dump first."));
            cx.notify();
            return;
        };
        let summary = format!(
            "The selected card will be overwritten with the {} image{}. A project backup is written before the first change.",
            dump.layout.name(),
            dump.card
                .as_ref()
                .map(|card| format!(" for UID {}", card.uid_hex()))
                .unwrap_or_default()
        );
        let this = cx.weak_entity();
        let service = self.service.clone();
        let keys = self.keys.clone();
        let backup = self.backup.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let service = service.clone();
            let keys = keys.clone();
            let backup = backup.clone();
            let dump = dump.clone();
            let summary = summary.clone();
            alert
                .title("Restore this dump?")
                .description(summary)
                .show_cancel(true)
                .ok_text("Restore")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let dump = dump.clone();
                    let keys = keys.clone();
                    let backup = backup.clone();
                    let _ = this.update(cx, |this, cx| {
                        this.busy = Some(SharedString::from("Restoring card"));
                        cx.notify();
                    });
                    spawn_service(
                        this.clone(),
                        service.clone(),
                        window,
                        cx,
                        move |service: &mut Service| {
                            service.classic_restore(
                                ClassicRestore::new(dump, &keys, &backup).confirmed(true),
                            )
                        },
                        |this, result, _window, _cx| match result {
                            Ok(restore) => {
                                this.last_backup =
                                    restore.backup.as_ref().map(|backup| backup.path.clone());
                                let failures = restore.failures.len();
                                this.status = Some(if failures == 0 {
                                    StatusMessage::success(format!(
                                        "Restore complete: {} written, {} unchanged, {} trailers",
                                        restore.written_blocks,
                                        restore.unchanged_blocks,
                                        restore.trailer_writes
                                    ))
                                } else {
                                    StatusMessage::warning(format!(
                                        "Restore finished with {failures} failure(s)"
                                    ))
                                });
                                this.diff = None;
                                if let (Some(status), Some(backup)) =
                                    (this.status.as_mut(), &this.last_backup)
                                {
                                    status.text = SharedString::from(format!(
                                        "{} · backup {}",
                                        status.text,
                                        backup.display()
                                    ));
                                }
                            }
                            Err(error) => {
                                this.status =
                                    Some(StatusMessage::error(format!("Restore failed: {error}")));
                            }
                        },
                    );
                    true
                })
        });
    }

    pub(crate) fn confirm_write_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(dump) = self.dump.as_ref() else {
            self.status = Some(StatusMessage::error("Read or load a card first."));
            cx.notify();
            return;
        };
        let Some(number) = self.selected_block else {
            self.status = Some(StatusMessage::error("Select a block first."));
            cx.notify();
            return;
        };
        if number == 0 {
            self.status = Some(StatusMessage::error("Block 0 is read-only in this build."));
            cx.notify();
            return;
        }
        let data = match parse_block_hex(&self.block_input.read(cx).value()) {
            Ok(data) => data,
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Block data: {error}")));
                cx.notify();
                return;
            }
        };
        let is_trailer = dump.layout.is_trailer(number).unwrap_or(false);
        if is_trailer {
            let sector = dump.layout.sector_for_block(number).unwrap_or(0);
            let summary = format!(
                "Block {number} is the trailer for sector {sector}. Writing it can change keys, access bits, or make the sector unreadable."
            );
            let this = cx.weak_entity();
            let service = self.service.clone();
            let keys = self.keys.clone();
            let backup = self.backup.clone();
            window.open_alert_dialog(cx, move |alert, _, _| {
                let this = this.clone();
                let service = service.clone();
                let keys = keys.clone();
                let backup = backup.clone();
                let summary = summary.clone();
                alert
                    .title("Write sector trailer?")
                    .description(summary)
                    .show_cancel(true)
                    .ok_text("Write trailer")
                    .ok_variant(ButtonVariant::Danger)
                    .on_ok(move |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.busy = Some(SharedString::from("Writing block"));
                            cx.notify();
                        });
                        spawn_write(
                            this.clone(),
                            service.clone(),
                            PendingWrite {
                                block: number,
                                data,
                                keys: keys.clone(),
                                backup: backup.clone(),
                                confirm_trailer: true,
                            },
                            window,
                            cx,
                        );
                        true
                    })
            });
        } else {
            self.busy = Some(SharedString::from("Writing block"));
            cx.notify();
            spawn_write(
                cx.weak_entity(),
                self.service.clone(),
                PendingWrite {
                    block: number,
                    data,
                    keys: self.keys.clone(),
                    backup: self.backup.clone(),
                    confirm_trailer: false,
                },
                window,
                cx,
            );
        }
    }

    pub(crate) fn scan_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(SharedString::from("Scanning keys"));
        cx.notify();
        let keys = self.keys.clone();
        spawn_service(
            cx.weak_entity(),
            self.service.clone(),
            window,
            cx,
            move |service: &mut Service| {
                service.classic_keyscan(
                    ClassicKeyscan::new(&keys, Vec::new())
                        .include_common_keys(true)
                        .key_types(KeyTypes::ALL),
                )
            },
            |this, result, _window, cx| match result {
                Ok(scan) => {
                    let added = this.keys.merge(&scan.store).unwrap_or(0);
                    let saved = this.persist_keys();
                    let suffix = match saved {
                        Ok(()) => String::new(),
                        Err(error) => format!(" · save failed: {error}"),
                    };
                    this.status = Some(StatusMessage::success(format!(
                        "Key scan: {} verified, {} added{}",
                        scan.verified, added, suffix
                    )));
                    cx.notify();
                }
                Err(error) => {
                    this.status = Some(StatusMessage::error(format!("Key scan failed: {error}")));
                }
            },
        );
    }

    pub(crate) fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = self.key_hex_input.read(cx).value().to_string();
        let key = match nfc_core::Key::from_hex(&hex) {
            Ok(key) => key,
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Key: {error}")));
                cx.notify();
                return;
            }
        };
        let sector_text = self.key_sector_input.read(cx).value().to_string();
        let sector = if sector_text.trim().is_empty() {
            None
        } else {
            match sector_text.trim().parse::<usize>() {
                Ok(sector) => Some(sector),
                Err(_) => {
                    self.status = Some(StatusMessage::error("Sector must be a number."));
                    cx.notify();
                    return;
                }
            }
        };
        let kind = match self.key_kind {
            0 => KeyKind::A,
            1 => KeyKind::B,
            _ => KeyKind::Any,
        };
        match self
            .keys
            .add(KeyEntry::new(sector, kind, key, KeySource::Manual))
        {
            Ok(added) => {
                let saved = self.persist_keys();
                self.key_hex_input.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.status = Some(match saved {
                    Ok(()) => StatusMessage::success(if added {
                        "Key added"
                    } else {
                        "Key already present"
                    }),
                    Err(error) => {
                        StatusMessage::warning(format!("Key added, save failed: {error}"))
                    }
                });
                cx.notify();
            }
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Key: {error}")));
                cx.notify();
            }
        }
    }

    pub(crate) fn remove_key(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.keys.remove(index).is_some() {
            let saved = self.persist_keys();
            self.status = Some(match saved {
                Ok(()) => StatusMessage::success("Key removed"),
                Err(error) => StatusMessage::warning(format!("Key removed, save failed: {error}")),
            });
        }
        cx.notify();
    }

    pub(crate) fn select_block(
        &mut self,
        number: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_block = Some(number);
        self.sync_block_input(window, cx);
        cx.notify();
    }

    pub(crate) fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tab = WorkspaceTab::from_index(index);
        cx.notify();
    }

    fn sync_block_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .dump
            .as_ref()
            .zip(self.selected_block)
            .and_then(|(dump, number)| dump.blocks.get(number))
            .filter(|block| block.complete())
            .map(|block| hex(&block.data))
            .unwrap_or_default();
        self.block_input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn persist_keys(&self) -> NfcResult<()> {
        match &self.key_store_path {
            Some(path) => self.keys.save(path),
            None => Ok(()),
        }
    }

    fn path(&self, cx: &App) -> String {
        self.path_input.read(cx).value().trim().to_owned()
    }
}

struct PendingWrite {
    block: usize,
    data: [u8; BLOCK_SIZE],
    keys: KeyStore,
    backup: BackupStore,
    confirm_trailer: bool,
}

fn spawn_write(
    this: WeakEntity<ClassicWorkspace>,
    service: Arc<Mutex<Service>>,
    pending: PendingWrite,
    window: &Window,
    cx: &mut App,
) {
    let PendingWrite {
        block,
        data,
        keys,
        backup,
        confirm_trailer,
    } = pending;
    spawn_service(
        this,
        service,
        window,
        cx,
        move |service: &mut Service| {
            service.classic_write_block(
                ClassicBlockWrite::new(block, data, &keys, &backup)
                    .confirm_trailer(confirm_trailer),
            )
        },
        move |this, result, _window, cx| match result {
            Ok(write) => {
                if write.changed {
                    if let Some(dump) = &mut this.dump
                        && let Some(entry) = dump.blocks.get_mut(block)
                    {
                        entry.set_read(data);
                        entry.status = BlockStatus::Verified;
                    }
                    this.last_backup = write.backup.as_ref().map(|backup| backup.path.clone());
                    this.status = Some(StatusMessage::success(format!(
                        "Block {block} written and verified"
                    )));
                } else {
                    this.status = Some(StatusMessage::info(format!(
                        "Block {block} already matches"
                    )));
                }
                cx.notify();
            }
            Err(error) => {
                this.status = Some(StatusMessage::error(format!("Write failed: {error}")));
            }
        },
    );
}

fn spawn_service<T, F, D>(
    this: WeakEntity<ClassicWorkspace>,
    service: Arc<Mutex<Service>>,
    window: &Window,
    cx: &mut App,
    task: F,
    on_done: D,
) where
    T: Send + 'static,
    F: FnOnce(&mut Service) -> NfcResult<T> + Send + 'static,
    D: FnOnce(&mut ClassicWorkspace, NfcResult<T>, &mut Window, &mut Context<ClassicWorkspace>)
        + 'static,
{
    let background = cx.background_spawn(async move {
        let mut service = lock_service(&service);
        task(&mut service)
    });
    window
        .spawn(cx, async move |cx| {
            let result = background.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                on_done(this, result, window, cx);
                cx.notify();
            });
        })
        .detach();
}

fn lock_service(service: &Arc<Mutex<Service>>) -> MutexGuard<'_, Service> {
    service.lock().unwrap_or_else(PoisonError::into_inner)
}

fn write_file(path: &Path, bytes: &[u8]) -> NfcResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            NfcError::Io(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    fs::write(path, bytes)
        .map_err(|error| NfcError::Io(format!("could not write {}: {error}", path.display())))?;
    restrict_permissions(path);
    Ok(())
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

fn parse_block_hex(value: &str) -> NfcResult<[u8; BLOCK_SIZE]> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != BLOCK_SIZE * 2 {
        return Err(NfcError::InvalidArgument(format!(
            "expected {} hexadecimal digits",
            BLOCK_SIZE * 2
        )));
    }
    let bytes = compact.as_bytes();
    let mut data = [0u8; BLOCK_SIZE];
    for (index, byte) in data.iter_mut().enumerate() {
        *byte = (hex_digit(bytes[index * 2])? << 4) | hex_digit(bytes[index * 2 + 1])?;
    }
    Ok(data)
}

fn hex_digit(character: u8) -> NfcResult<u8> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(NfcError::InvalidArgument(
            "block data contains a non-hexadecimal character".to_owned(),
        )),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
