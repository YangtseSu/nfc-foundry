//! Retained state and actions for the Classic screens.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gpui_kit::component::{WindowExt, input::InputState};
use gpui_kit::{App, AppContext as _, Context, Entity, ParentElement as _, WeakEntity, Window};
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicNdefFormat, ClassicNdefWrite,
    ClassicRead, ClassicRestore, KeyEntry, KeyKind, KeySource, KeyStore,
};
use nfc_app::{ClassicNdefInfo, ClassicNdefState, Service};
use nfc_classic::{BlockStatus, Dump, DumpDiff, exchange};
use nfc_core::{BLOCK_SIZE, CancelToken, KeyType, NfcError, Result as NfcResult};
use nfc_ndef::{NdefMessage, TextRecord, UriRecord};
use nfc_recover::{RecoveryProgress, RecoveryRequest};
use nfc_ui::{
    ActivityEntry, CardIdentity, Key, Session, StatusKind, StatusMessage, error_key,
    parse_hex_array, write_private,
};

use crate::model::{
    ClassicSettings, DUMP_FORMATS, NdefSource, STRATEGIES, ScanTypes, dump_format_index,
    dump_format_key,
};

pub(crate) const CONFIRMATION: &str = "I_OWN_THIS_CARD";

/// Recovery state retained while the user navigates away.
pub(crate) struct RecoveryState {
    pub strategy: usize,
    pub known_block: Entity<InputState>,
    pub target_block: Entity<InputState>,
    pub known_key: Entity<InputState>,
    pub max_nonces: Entity<InputState>,
    pub target_type: usize,
    pub cancel: Option<CancelToken>,
    pub progress: Arc<Mutex<RecoveryViewState>>,
    pub running: bool,
    pub view: nfc_ui::RecoveryView,
    pub verified: Vec<String>,
    pub note: String,
    pub profile: String,
}

#[derive(Default)]
pub(crate) struct RecoveryViewState {
    pub view: nfc_ui::RecoveryView,
}

/// GPUI Kit MIFARE Classic screens.
///
/// All card work runs through the shared [`Service`] on a background executor.
/// The view never performs reader I/O in `render`.
pub struct ClassicWorkspace {
    pub(crate) service: Arc<Mutex<Service>>,
    pub(crate) keys: KeyStore,
    pub(crate) settings: ClassicSettings,
    pub(crate) session: Entity<Session>,
    pub(crate) dump: Option<Dump>,
    pub(crate) previous_dump: Option<Dump>,
    pub(crate) diff: Option<DumpDiff>,
    pub(crate) file_diff: Option<DumpDiff>,
    pub(crate) ndef: Option<ClassicNdefInfo>,
    pub(crate) ndef_error: Option<String>,
    pub(crate) selected_block: Option<usize>,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) block_input: Entity<InputState>,
    pub(crate) ndef_value_input: Entity<InputState>,
    pub(crate) ndef_language_input: Entity<InputState>,
    pub(crate) ndef_file_input: Entity<InputState>,
    pub(crate) ndef_export_input: Entity<InputState>,
    pub(crate) key_hex_input: Entity<InputState>,
    pub(crate) key_sector_input: Entity<InputState>,
    pub(crate) import_input: Entity<InputState>,
    pub(crate) diff_before_input: Entity<InputState>,
    pub(crate) diff_after_input: Entity<InputState>,
    pub(crate) confirm_input: Entity<InputState>,
    pub(crate) block_search: Entity<InputState>,
    pub(crate) block_filter: usize,
    pub(crate) dump_format: usize,
    pub(crate) ndef_source: NdefSource,
    pub(crate) key_kind: usize,
    pub(crate) scan_types: ScanTypes,
    pub(crate) format_read_only: bool,
    pub(crate) restore_continue: bool,
    pub(crate) reveal_keys: bool,
    pub(crate) recovery: RecoveryState,
}

impl ClassicWorkspace {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        service: Arc<Mutex<Service>>,
        keys: KeyStore,
        settings: ClassicSettings,
        session: Entity<Session>,
    ) -> Self {
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/dump.json"));
        let block_input = cx.new(|cx| InputState::new(window, cx).placeholder("00 11 22 33 …"));
        let ndef_value_input = cx.new(|cx| InputState::new(window, cx).placeholder("NDEF"));
        let ndef_language_input = cx.new(|cx| InputState::new(window, cx).placeholder("en"));
        let ndef_file_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/message.ndef"));
        let ndef_export_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/export.ndef"));
        let key_hex_input = cx.new(|cx| InputState::new(window, cx).placeholder("A0A1A2A3A4A5"));
        let key_sector_input = cx.new(|cx| InputState::new(window, cx).placeholder("0..15"));
        let import_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/keys.txt"));
        let diff_before_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/before"));
        let diff_after_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/after"));
        let confirm_input = cx.new(|cx| InputState::new(window, cx).placeholder(CONFIRMATION));
        let block_search = cx.new(|cx| InputState::new(window, cx).placeholder("0..63"));
        let known_block = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("4")
                .default_value("4")
        });
        let target_block = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("0")
                .default_value("0")
        });
        let known_key = cx.new(|cx| InputState::new(window, cx).placeholder("D3F7D3F7D3F7"));
        let max_nonces = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("4000")
                .default_value("4000")
        });
        let dump_format = dump_format_index(settings.default_dump_format);
        let recovery = RecoveryState {
            strategy: 0,
            known_block,
            target_block,
            known_key,
            max_nonces,
            target_type: 0,
            cancel: None,
            progress: Arc::new(Mutex::new(RecoveryViewState::default())),
            running: false,
            view: nfc_ui::RecoveryView::default(),
            verified: Vec::new(),
            note: String::new(),
            profile: String::new(),
        };
        let workspace = Self {
            service,
            keys,
            settings,
            session,
            dump: None,
            previous_dump: None,
            diff: None,
            file_diff: None,
            ndef: None,
            ndef_error: None,
            selected_block: None,
            path_input,
            block_input,
            ndef_value_input,
            ndef_language_input,
            ndef_file_input,
            ndef_export_input,
            key_hex_input,
            key_sector_input,
            import_input,
            diff_before_input,
            diff_after_input,
            confirm_input,
            block_search,
            block_filter: 0,
            dump_format,
            ndef_source: NdefSource::Text,
            key_kind: 2,
            scan_types: ScanTypes::Ab,
            format_read_only: false,
            restore_continue: false,
            reveal_keys: false,
            recovery,
        };
        cx.observe(&workspace.session, |_, _, cx| cx.notify())
            .detach();
        workspace
    }

    /// Current card image, if a read or load succeeded.
    pub fn dump(&self) -> Option<&Dump> {
        self.dump.as_ref()
    }

    /// Previous card image used for the diff.
    pub fn previous_dump(&self) -> Option<&Dump> {
        self.previous_dump.as_ref()
    }

    /// Latest dump comparison.
    pub fn diff(&self) -> Option<&DumpDiff> {
        self.file_diff.as_ref().or(self.diff.as_ref())
    }

    /// Selected block number.
    pub fn selected_block(&self) -> Option<usize> {
        self.selected_block
    }

    /// Key store currently loaded in the workspace.
    pub fn keys(&self) -> &KeyStore {
        &self.keys
    }

    /// NDEF facts for the current dump.
    pub fn ndef(&self) -> Option<&ClassicNdefInfo> {
        self.ndef.as_ref()
    }

    /// Whether the recovery engine is running.
    pub fn recovery_running(&self) -> bool {
        self.recovery.running
    }

    /// Number of blocks in the current image.
    pub fn block_count(&self) -> usize {
        self.dump.as_ref().map_or(0, |dump| dump.blocks.len())
    }

    /// NDEF record count in the current image.
    pub fn ndef_record_count(&self) -> usize {
        match self.ndef.as_ref().map(|info| &info.ndef.state) {
            Some(ClassicNdefState::Message(message)) => message.records().len(),
            _ => 0,
        }
    }

    /// Replace the key store after a settings change.
    pub fn set_keys(
        &mut self,
        keys: KeyStore,
        key_store_path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.keys = keys;
        self.settings.key_store_path = key_store_path;
        cx.notify();
    }

    /// Apply new settings from the shell.
    pub fn set_settings(&mut self, settings: ClassicSettings, cx: &mut Context<Self>) {
        self.dump_format = dump_format_index(settings.default_dump_format);
        self.settings = settings;
        cx.notify();
    }

    /// Drop the current card image after a reader change.
    pub fn reset_card(&mut self, cx: &mut Context<Self>) {
        self.dump = None;
        self.previous_dump = None;
        self.diff = None;
        self.file_diff = None;
        self.ndef = None;
        self.ndef_error = None;
        self.selected_block = None;
        cx.notify();
    }

    pub fn read_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        self.begin(Key::StatusReadingCard, cx);
        let keys = self.keys.clone();
        let use_common = self.settings.use_common_keys;
        spawn_service(
            cx.weak_entity(),
            self.service.clone(),
            window,
            cx,
            move |service| {
                service.classic_read(ClassicRead::new(&keys).include_common_keys(use_common))
            },
            |this, result, window, cx| match result {
                Ok(read) => {
                    let warnings = read.warnings.len();
                    this.apply_dump(read.dump, window, cx);
                    this.finish(cx);
                    this.set_status(
                        if warnings == 0 {
                            StatusKind::Success
                        } else {
                            StatusKind::Warning
                        },
                        if warnings == 0 {
                            Key::StatusCardRead
                        } else {
                            Key::StatusCardReadWarnings
                        },
                        (warnings > 0).then(|| format!("{warnings} warning(s)")),
                        cx,
                    );
                    let detail = this.card_detail(cx);
                    this.activity(StatusKind::Success, Key::ReadCard, detail, cx);
                }
                Err(error) => {
                    this.finish(cx);
                    this.fail(Key::StatusReadFailed, &error, cx);
                }
            },
        );
    }

    pub(crate) fn read_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        let Some(block) = self.selected_block else {
            self.set_status(StatusKind::Error, Key::SelectBlock, None, cx);
            return;
        };
        let keys = self.keys.clone();
        let use_common = self.settings.use_common_keys;
        self.begin(Key::StatusReadingCard, cx);
        spawn_service(
            cx.weak_entity(),
            self.service.clone(),
            window,
            cx,
            move |service| service.classic_read_block(block, &keys, use_common, CancelToken::new()),
            move |this, result, _, cx| match result {
                Ok(data) => {
                    this.finish(cx);
                    if let Some(dump) = &mut this.dump
                        && let Some(entry) = dump.blocks.get_mut(block)
                    {
                        entry.set_read(data);
                        entry.status = BlockStatus::Verified;
                    }
                    this.set_status(
                        StatusKind::Success,
                        Key::StatusCardRead,
                        Some(format!("block {block}")),
                        cx,
                    );
                    cx.notify();
                }
                Err(error) => {
                    this.finish(cx);
                    this.fail(Key::StatusReadFailed, &error, cx);
                }
            },
        );
    }

    pub(crate) fn save_dump(&mut self, cx: &mut Context<Self>) {
        let path = self.path(cx);
        if path.is_empty() {
            self.set_status(
                StatusKind::Error,
                Key::StatusInvalidInput,
                Some("enter a dump path".into()),
                cx,
            );
            return;
        }
        let format = DUMP_FORMATS[self.dump_format.min(DUMP_FORMATS.len() - 1)];
        let result = self
            .dump
            .as_ref()
            .ok_or_else(|| NfcError::InvalidArgument("read or load a card first".to_owned()))
            .and_then(|dump| exchange::export(dump, format))
            .and_then(|bytes| write_private(Path::new(&path), &bytes));
        match result {
            Ok(()) => {
                self.set_status(
                    StatusKind::Success,
                    Key::StatusDumpSaved,
                    Some(path.clone()),
                    cx,
                );
                let format_name = self.translator(cx).text(dump_format_key(format));
                self.activity(
                    StatusKind::Success,
                    Key::SaveDump,
                    format!("{path} · {format_name}"),
                    cx,
                );
            }
            Err(error) => self.fail(Key::StatusSaveFailed, &error, cx),
        }
    }

    pub(crate) fn load_dump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path(cx);
        if path.is_empty() {
            self.set_status(
                StatusKind::Error,
                Key::StatusInvalidInput,
                Some("enter a dump path".into()),
                cx,
            );
            return;
        }
        let result = fs::read(&path)
            .map_err(|error| NfcError::Io(format!("could not read {path}: {error}")))
            .and_then(|bytes| exchange::load(&bytes));
        match result {
            Ok(load) => {
                let format = load.format.name().to_owned();
                let warnings = load.warnings.len();
                self.apply_dump(load.dump, window, cx);
                self.set_status(
                    StatusKind::Success,
                    Key::StatusDumpLoaded,
                    Some(format!("{path} · {format}")),
                    cx,
                );
                self.activity(
                    StatusKind::Success,
                    Key::LoadDump,
                    format!("{path} · {format}"),
                    cx,
                );
                if warnings > 0 {
                    self.set_status(
                        StatusKind::Warning,
                        Key::StatusCardReadWarnings,
                        Some(format!("{warnings} warning(s)")),
                        cx,
                    );
                }
            }
            Err(error) => self.fail(Key::StatusLoadFailed, &error, cx),
        }
    }

    pub(crate) fn restore_loaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        let Some(dump) = self.dump.clone() else {
            self.set_status(StatusKind::Error, Key::StatusNoDump, None, cx);
            return;
        };
        let translator = self.translator(cx);
        let body = format!(
            "{} {}",
            translator.text(Key::ConfirmRestoreBody),
            dump.card
                .as_ref()
                .map(|card| format!("UID {}", card.uid_hex()))
                .unwrap_or_default()
        );
        let token = self.confirm_input.clone();
        let this = cx.weak_entity();
        let service = self.service.clone();
        let keys = self.keys.clone();
        let backup = self.settings.backup.clone();
        let continue_on_error = self.restore_continue;
        let use_common = self.settings.use_common_keys;
        let session = self.session.clone();
        confirm_with_token(
            window,
            cx,
            token,
            translator.text(Key::ConfirmRestoreTitle).to_owned(),
            body,
            translator.text(Key::RestoreTag).to_owned(),
            Rc::new(move |window, cx| {
                let this = this.clone();
                let service = service.clone();
                let keys = keys.clone();
                let backup = backup.clone();
                let dump = dump.clone();
                let session = session.clone();
                let _ = this.update(cx, |this, cx| {
                    this.begin(Key::BusyRestoring, cx);
                });
                spawn_service_with_session(
                    this,
                    session,
                    service,
                    window,
                    cx,
                    move |service| {
                        service.classic_restore(
                            ClassicRestore::new(dump, &keys, &backup)
                                .include_common_keys(use_common)
                                .confirmed(true)
                                .continue_on_failure(continue_on_error),
                        )
                    },
                    |this, result, _window, cx| match result {
                        Ok(restore) => {
                            this.finish(cx);
                            let failures = restore.failures.len();
                            if let Some(backup) = &restore.backup {
                                this.last_backup(backup.path.clone(), cx);
                            }
                            this.set_status(
                                if failures == 0 {
                                    StatusKind::Success
                                } else {
                                    StatusKind::Warning
                                },
                                Key::StatusRestoreComplete,
                                Some(format!(
                                    "{} written · {} unchanged · {} trailers",
                                    restore.written_blocks,
                                    restore.unchanged_blocks,
                                    restore.trailer_writes
                                )),
                                cx,
                            );
                            this.activity(
                                if failures == 0 {
                                    StatusKind::Success
                                } else {
                                    StatusKind::Warning
                                },
                                Key::StatusRestoreComplete,
                                format!(
                                    "{} written · {} unchanged · {failures} failure(s)",
                                    restore.written_blocks, restore.unchanged_blocks
                                ),
                                cx,
                            );
                            this.diff = None;
                        }
                        Err(error) => {
                            this.finish(cx);
                            this.fail(Key::StatusRestoreFailed, &error, cx);
                        }
                    },
                );
            }),
        );
    }

    pub(crate) fn write_selected_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        let Some(dump) = self.dump.as_ref() else {
            self.set_status(StatusKind::Error, Key::StatusNoDump, None, cx);
            return;
        };
        let Some(block) = self.selected_block else {
            self.set_status(StatusKind::Error, Key::SelectBlock, None, cx);
            return;
        };
        if block == 0 {
            self.set_status(StatusKind::Error, Key::BlockZeroReadOnly, None, cx);
            return;
        }
        let data = match parse_hex_array::<BLOCK_SIZE>(&self.block_input.read(cx).value()) {
            Ok(data) => data,
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
                return;
            }
        };
        let is_trailer = dump.layout.is_trailer(block).unwrap_or(false);
        let keys = self.keys.clone();
        let backup = self.settings.backup.clone();
        let use_common = self.settings.use_common_keys;
        let session = self.session.clone();
        let service = self.service.clone();
        if is_trailer {
            let translator = self.translator(cx);
            let sector = dump.layout.sector_for_block(block).unwrap_or(0);
            let body = translator.fmt2(Key::ConfirmWriteTrailerBody, block, sector);
            let token = self.confirm_input.clone();
            let this = cx.weak_entity();
            let session = self.session.clone();
            confirm_with_token(
                window,
                cx,
                token,
                translator.text(Key::ConfirmWriteTrailerTitle).to_owned(),
                body,
                translator.text(Key::WriteTrailer).to_owned(),
                Rc::new(move |window, cx| {
                    let this = this.clone();
                    let service = service.clone();
                    let keys = keys.clone();
                    let backup = backup.clone();
                    let session = session.clone();
                    spawn_write(
                        this,
                        session,
                        service,
                        window,
                        cx,
                        PendingWrite {
                            block,
                            data,
                            keys,
                            backup,
                            use_common,
                            confirm_trailer: true,
                        },
                        |this, cx| {
                            this.set_status(StatusKind::Info, Key::StatusBlockWritten, None, cx);
                            this.begin(Key::BusyWritingBlock, cx);
                        },
                    );
                }),
            );
        } else {
            spawn_write(
                cx.weak_entity(),
                session,
                service,
                window,
                cx,
                PendingWrite {
                    block,
                    data,
                    keys,
                    backup,
                    use_common,
                    confirm_trailer: false,
                },
                |this, cx| {
                    this.set_status(StatusKind::Info, Key::StatusBlockWritten, None, cx);
                    this.begin(Key::BusyWritingBlock, cx);
                },
            );
        }
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

    pub(crate) fn select_ndef_source(&mut self, index: usize, cx: &mut Context<Self>) {
        self.ndef_source = NdefSource::ALL
            .get(index)
            .copied()
            .unwrap_or(NdefSource::Text);
        cx.notify();
    }

    pub(crate) fn select_dump_format(&mut self, index: usize, cx: &mut Context<Self>) {
        self.dump_format = index.min(DUMP_FORMATS.len() - 1);
        cx.notify();
    }

    pub(crate) fn select_block_filter(&mut self, index: usize, cx: &mut Context<Self>) {
        self.block_filter = index;
        cx.notify();
    }

    pub(crate) fn select_scan_types(&mut self, index: usize, cx: &mut Context<Self>) {
        self.scan_types = ScanTypes::ALL.get(index).copied().unwrap_or(ScanTypes::Ab);
        cx.notify();
    }

    pub(crate) fn select_key_kind(&mut self, index: usize, cx: &mut Context<Self>) {
        self.key_kind = index.min(2);
        cx.notify();
    }

    pub(crate) fn toggle_reveal(&mut self, cx: &mut Context<Self>) {
        self.reveal_keys = !self.reveal_keys;
        cx.notify();
    }

    pub(crate) fn toggle_format_read_only(&mut self, cx: &mut Context<Self>) {
        self.format_read_only = !self.format_read_only;
        cx.notify();
    }

    pub(crate) fn refresh_ndef(&mut self, cx: &mut Context<Self>) {
        self.ndef = None;
        self.ndef_error = None;
        if let Some(dump) = &self.dump {
            match lock_service(&self.service).classic_ndef_inspect(dump) {
                Ok(info) => self.ndef = Some(info),
                Err(error) => self.ndef_error = Some(error.to_string()),
            }
        }
        cx.notify();
    }

    pub fn scan_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        self.begin(Key::BusyScanningKeys, cx);
        let keys = self.keys.clone();
        let use_common = self.settings.use_common_keys;
        let key_types = self.scan_types.key_types();
        spawn_service(
            cx.weak_entity(),
            self.service.clone(),
            window,
            cx,
            move |service| {
                service.classic_keyscan(
                    ClassicKeyscan::new(&keys, Vec::new())
                        .include_common_keys(use_common)
                        .key_types(key_types),
                )
            },
            |this, result, _, cx| match result {
                Ok(scan) => {
                    let added = this.keys.merge(&scan.store).unwrap_or(0);
                    let saved = this.persist_keys();
                    this.finish(cx);
                    let suffix = match saved {
                        Ok(()) => String::new(),
                        Err(error) => format!(" · {error}"),
                    };
                    this.set_status(
                        StatusKind::Success,
                        Key::StatusKeyScanDone,
                        Some(format!(
                            "{} verified · {added} added{}",
                            scan.verified, suffix
                        )),
                        cx,
                    );
                    this.activity(
                        StatusKind::Success,
                        Key::StatusKeyScanDone,
                        format!("{} verified · {added} added", scan.verified),
                        cx,
                    );
                    cx.notify();
                }
                Err(error) => {
                    this.finish(cx);
                    this.fail(Key::StatusKeyScanFailed, &error, cx);
                }
            },
        );
    }

    pub(crate) fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = self.key_hex_input.read(cx).value().to_string();
        let key = match nfc_core::Key::from_hex(&hex) {
            Ok(key) => key,
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
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
                    self.set_status(
                        StatusKind::Error,
                        Key::StatusInvalidInput,
                        Some("sector must be a number".to_owned()),
                        cx,
                    );
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
                self.key_hex_input
                    .update(cx, |state, cx| state.set_value("", window, cx));
                match saved {
                    Ok(()) => self.set_status(
                        StatusKind::Success,
                        if added {
                            Key::StatusKeyAdded
                        } else {
                            Key::StatusKeyAlreadyPresent
                        },
                        None,
                        cx,
                    ),
                    Err(error) => {
                        self.set_status(
                            StatusKind::Warning,
                            Key::StatusKeySaveFailed,
                            Some(error.to_string()),
                            cx,
                        );
                    }
                }
                cx.notify();
            }
            Err(error) => self.fail(Key::StatusInvalidInput, &error, cx),
        }
    }

    pub(crate) fn remove_key(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.keys.remove(index).is_some() {
            match self.persist_keys() {
                Ok(()) => self.set_status(StatusKind::Success, Key::StatusKeyRemoved, None, cx),
                Err(error) => self.fail(Key::StatusKeySaveFailed, &error, cx),
            }
        }
        cx.notify();
    }

    pub(crate) fn import_keys(&mut self, cx: &mut Context<Self>) {
        let path = self.import_input.read(cx).value().trim().to_owned();
        if path.is_empty() {
            self.set_status(
                StatusKind::Error,
                Key::StatusInvalidInput,
                Some("enter a key file path".into()),
                cx,
            );
            return;
        }
        match self.keys.import_text(Path::new(&path)) {
            Ok(added) => {
                let saved = self.persist_keys();
                self.set_status(
                    StatusKind::Success,
                    Key::KeyImportDone,
                    Some(match saved {
                        Ok(()) => format!("{added} key(s) · {path}"),
                        Err(error) => format!("{added} key(s) · {path} · {error}"),
                    }),
                    cx,
                );
            }
            Err(error) => self.fail(Key::StatusLoadFailed, &error, cx),
        }
        cx.notify();
    }

    pub(crate) fn export_ndef(&mut self, cx: &mut Context<Self>) {
        let path = self.ndef_export_input.read(cx).value().trim().to_owned();
        if path.is_empty() {
            self.set_status(
                StatusKind::Error,
                Key::StatusInvalidInput,
                Some("enter an export path".into()),
                cx,
            );
            return;
        }
        let Some(ClassicNdefState::Message(message)) =
            self.ndef.as_ref().map(|info| &info.ndef.state)
        else {
            self.set_status(StatusKind::Error, Key::NdefUnformatted, None, cx);
            return;
        };
        match message
            .encode()
            .and_then(|bytes| write_private(Path::new(&path), &bytes))
        {
            Ok(()) => self.set_status(StatusKind::Success, Key::ExportMessage, Some(path), cx),
            Err(error) => self.fail(Key::StatusSaveFailed, &error, cx),
        }
        cx.notify();
    }

    pub(crate) fn write_ndef(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        let Some(info) = &self.ndef else {
            self.set_status(StatusKind::Error, Key::StatusNoDump, None, cx);
            return;
        };
        let Some(layout) = &info.layout else {
            self.set_status(StatusKind::Error, Key::NoMad, None, cx);
            return;
        };
        if layout.read_only || !layout.writable {
            self.set_status(StatusKind::Error, Key::NdefReadOnly, None, cx);
            return;
        }
        if !matches!(
            info.ndef.state,
            ClassicNdefState::Empty | ClassicNdefState::Message(_)
        ) {
            self.set_status(StatusKind::Error, Key::NdefUnformatted, None, cx);
            return;
        }
        let message = match self.build_ndef_message(cx) {
            Ok(message) => message,
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
                return;
            }
        };
        self.begin(Key::BusyWritingNdef, cx);
        let keys = self.keys.clone();
        let backup = self.settings.backup.clone();
        let use_common = self.settings.use_common_keys;
        let session = self.session.clone();
        spawn_service_with_session(
            cx.weak_entity(),
            session,
            self.service.clone(),
            window,
            cx,
            move |service| {
                service.classic_ndef_write(
                    ClassicNdefWrite::new(message, &keys, &backup).include_common_keys(use_common),
                )
            },
            |this, result, window, cx| match result {
                Ok(write) => {
                    this.finish(cx);
                    this.apply_ndef_result(write.dump, write.info, window, cx);
                    this.last_backup(write.backup.path.clone(), cx);
                    this.set_status(
                        StatusKind::Success,
                        Key::StatusNdefWritten,
                        Some(format!("{} block(s)", write.written_blocks)),
                        cx,
                    );
                    this.activity(
                        StatusKind::Success,
                        Key::WriteNdef,
                        format!(
                            "{} block(s) · {}",
                            write.written_blocks,
                            write.backup.path.display()
                        ),
                        cx,
                    );
                }
                Err(error) => {
                    this.finish(cx);
                    this.fail(Key::StatusWriteFailed, &error, cx);
                }
            },
        );
    }

    pub(crate) fn confirm_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        if self.dump.is_none() {
            self.set_status(StatusKind::Error, Key::StatusNoDump, None, cx);
            return;
        }
        let translator = self.translator(cx);
        let body = translator.text(Key::ConfirmFormatBody).to_owned();
        let read_only = self.format_read_only;
        let token = self.confirm_input.clone();
        let this = cx.weak_entity();
        let service = self.service.clone();
        let keys = self.keys.clone();
        let backup = self.settings.backup.clone();
        let use_common = self.settings.use_common_keys;
        let session = self.session.clone();
        confirm_with_token(
            window,
            cx,
            token,
            translator.text(Key::ConfirmFormatTitle).to_owned(),
            body,
            translator.text(Key::FormatCard).to_owned(),
            Rc::new(move |window, cx| {
                let this = this.clone();
                let service = service.clone();
                let keys = keys.clone();
                let backup = backup.clone();
                let session = session.clone();
                let _ = this.update(cx, |this, cx| this.begin(Key::BusyFormatting, cx));
                spawn_service_with_session(
                    this,
                    session,
                    service,
                    window,
                    cx,
                    move |service| {
                        service.classic_ndef_format(
                            ClassicNdefFormat::new(&keys, &backup)
                                .include_common_keys(use_common)
                                .read_only(read_only)
                                .confirm(true),
                        )
                    },
                    |this, result, window, cx| match result {
                        Ok(format) => {
                            this.finish(cx);
                            this.apply_ndef_result(format.dump, format.info, window, cx);
                            this.last_backup(format.backup.path.clone(), cx);
                            this.set_status(
                                StatusKind::Success,
                                Key::StatusNdefFormatted,
                                Some(format!(
                                    "{} block(s) · {} trailer(s)",
                                    format.written_blocks, format.trailer_writes
                                )),
                                cx,
                            );
                            this.activity(
                                StatusKind::Success,
                                Key::FormatCard,
                                format!(
                                    "{} block(s) · {}",
                                    format.written_blocks,
                                    format.backup.path.display()
                                ),
                                cx,
                            );
                        }
                        Err(error) => {
                            this.finish(cx);
                            this.fail(Key::StatusFormatFailed, &error, cx);
                        }
                    },
                );
            }),
        );
    }

    pub(crate) fn compare_files(&mut self, cx: &mut Context<Self>) {
        let before = self.diff_before_input.read(cx).value().trim().to_owned();
        let after = self.diff_after_input.read(cx).value().trim().to_owned();
        let result = fs::read(&before)
            .map_err(|error| NfcError::Io(format!("could not read {before}: {error}")))
            .and_then(|bytes| exchange::load(&bytes))
            .and_then(|before| {
                fs::read(&after)
                    .map_err(|error| NfcError::Io(format!("could not read {after}: {error}")))
                    .and_then(|bytes| exchange::load(&bytes))
                    .map(|after| (before, after))
            })
            .and_then(|(before, after)| nfc_classic::diff::diff(&before.dump, &after.dump));
        match result {
            Ok(diff) => {
                self.file_diff = Some(diff);
                self.set_status(
                    StatusKind::Success,
                    Key::StatusCompareDone,
                    Some(format!("{before} ↔ {after}")),
                    cx,
                );
            }
            Err(error) => self.fail(Key::StatusCompareFailed, &error, cx),
        }
        cx.notify();
    }

    pub(crate) fn start_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) || self.recovery.running {
            return;
        }
        let translator = self.translator(cx);
        let known_block = parse_u8(&self.recovery.known_block.read(cx).value());
        let target_block = parse_u8(&self.recovery.target_block.read(cx).value());
        let max_nonces = self
            .recovery
            .max_nonces
            .read(cx)
            .value()
            .trim()
            .parse::<usize>();
        let (Ok(known_block), Ok(target_block), Ok(max_nonces)) =
            (known_block, target_block, max_nonces)
        else {
            self.set_status(
                StatusKind::Error,
                Key::StatusInvalidInput,
                Some("block or nonce value".into()),
                cx,
            );
            return;
        };
        let mut candidates = self.recovery_candidates(cx);
        let manual = self.recovery.known_key.read(cx).value().trim().to_owned();
        if !manual.is_empty() {
            match nfc_core::Key::from_hex(&manual) {
                Ok(key) => candidates.push((KeyType::A, key)),
                Err(error) => {
                    self.fail(Key::StatusInvalidInput, &error, cx);
                    return;
                }
            }
        }
        candidates.sort_by_key(|(kind, key)| (kind.label(), key.to_hex()));
        candidates.dedup();
        if candidates.is_empty() {
            self.set_status(StatusKind::Warning, Key::NoCandidates, None, cx);
            return;
        }
        let strategy = STRATEGIES[self.recovery.strategy.min(STRATEGIES.len() - 1)];
        let target_type = if self.recovery.target_type == 1 {
            KeyType::B
        } else {
            KeyType::A
        };
        let cancel = CancelToken::new();
        self.recovery.cancel = Some(cancel.clone());
        self.recovery.running = true;
        self.recovery.verified.clear();
        self.recovery.note.clear();
        self.recovery.profile.clear();
        self.recovery.view = nfc_ui::RecoveryView::running(nfc_ui::RecoveryStage::Acquire, 0, "");
        let progress_state = Arc::clone(&self.recovery.progress);
        if let Ok(mut progress) = progress_state.lock() {
            progress.view = self.recovery.view.clone();
        }
        self.begin(Key::StatusRecoveryRunning, cx);
        self.session.update(cx, |session, cx| {
            session.set_recovery(Some(nfc_ui::RecoveryView::running(
                nfc_ui::RecoveryStage::Acquire,
                0,
                translator.text(Key::StatusRecoveryRunning),
            )));
            cx.notify();
        });
        let progress_sink = Arc::clone(&progress_state);
        let request = RecoveryRequest {
            known_block,
            target_block,
            target_key_type: target_type,
            known_key_type: KeyType::A,
            candidates,
            cancel: cancel.clone(),
            progress: Some({
                let sink: nfc_recover::ProgressSink = Arc::new(move |event: RecoveryProgress| {
                    if let Ok(mut progress) = progress_sink.lock() {
                        progress.view = match event {
                            RecoveryProgress::Acquiring {
                                received,
                                distinct_nonces,
                                first_bytes,
                                expected_states,
                            } => nfc_ui::RecoveryView::running(
                                nfc_ui::RecoveryStage::Acquire,
                                u8::try_from(first_bytes.saturating_mul(100) / 256).unwrap_or(100),
                                format!(
                                    "{received} frames · {distinct_nonces} nonces · {first_bytes}/256 · {expected_states:.3e} states"
                                ),
                            ),
                            RecoveryProgress::Filtering { expected_states } => {
                                nfc_ui::RecoveryView::running(
                                    nfc_ui::RecoveryStage::Filter,
                                    55,
                                    format!("{expected_states:.3e} states"),
                                )
                            }
                            RecoveryProgress::BruteForce {
                                tested_states,
                                total_states,
                                expected_states,
                            } => nfc_ui::RecoveryView::running(
                                nfc_ui::RecoveryStage::Solve,
                                u8::try_from(
                                    tested_states
                                        .saturating_mul(100)
                                        .checked_div(total_states)
                                        .unwrap_or(0),
                                )
                                .unwrap_or(100),
                                format!(
                                    "{tested_states}/{total_states} · {expected_states:.3e} states"
                                ),
                            ),
                        };
                    }
                });
                sink
            }),
            tables_dir: self.settings.tables_dir.clone(),
            max_nonces,
        };
        let service = self.service.clone();
        let session = self.session.clone();
        let this = cx.weak_entity();
        let poll_this = cx.weak_entity();
        let poll_progress = Arc::clone(&self.recovery.progress);
        window
            .spawn(cx, async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    let running = poll_this
                        .update(cx, |this, cx| {
                            this.sync_recovery(&poll_progress, cx);
                            this.recovery.running
                        })
                        .unwrap_or(false);
                    if !running {
                        break;
                    }
                }
            })
            .detach();
        let background = cx.background_spawn(async move {
            let mut service = lock_service(&service);
            service.recover(strategy, request)
        });
        window
            .spawn(cx, async move |cx| {
                let result = background.await;
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.recovery.running = false;
                    this.recovery.cancel = None;
                    this.finish(cx);
                    this.sync_recovery(&progress_state, cx);
                    match result {
                        Ok(result) => {
                            let keys: Vec<String> = result
                                .keys
                                .iter()
                                .map(|(kind, key)| format!("{} {}", kind.label(), key.to_hex()))
                                .collect();
                            this.recovery.verified = keys.clone();
                            this.recovery.note = result.note.clone();
                            this.recovery.profile = result
                                .profile
                                .as_ref()
                                .map(|profile| {
                                    format!(
                                        "{:?} · known key {}",
                                        profile.prng, profile.has_known_key
                                    )
                                })
                                .unwrap_or_default();
                            let mut view = this.recovery.view.clone();
                            view.stage = nfc_ui::RecoveryStage::Complete;
                            view.percent = 100;
                            view.verified = keys.clone();
                            view.detail = result.note.clone();
                            this.recovery.view = view.clone();
                            session.update(cx, |session, cx| {
                                session.set_recovery(Some(view));
                                session.finish();
                                session.set_status(StatusMessage::success(Key::StatusRecoveryDone));
                                session.record(ActivityEntry::new(
                                    StatusKind::Success,
                                    Key::StatusRecoveryDone,
                                    keys.join(", "),
                                ));
                                cx.notify();
                            });
                            this.set_status(
                                StatusKind::Success,
                                Key::StatusRecoveryDone,
                                Some(keys.join(", ")),
                                cx,
                            );
                        }
                        Err(error) => {
                            let cancelled = matches!(error, NfcError::Cancelled);
                            let mut view = this.recovery.view.clone();
                            view.stage = nfc_ui::RecoveryStage::Idle;
                            view.failed = !cancelled;
                            view.cancelled = cancelled;
                            view.detail = error.to_string();
                            this.recovery.view = view.clone();
                            session.update(cx, |session, cx| {
                                session.set_recovery(Some(view));
                                session.finish();
                                session.set_status(StatusMessage::new(
                                    if cancelled {
                                        StatusKind::Info
                                    } else {
                                        StatusKind::Error
                                    },
                                    if cancelled {
                                        Key::StatusRecoveryCancelled
                                    } else {
                                        Key::StatusRecoveryFailed
                                    },
                                    Some(error.to_string()),
                                ));
                                cx.notify();
                            });
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub fn cancel_recovery(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.recovery.cancel {
            cancel.cancel();
            self.set_status(StatusKind::Info, Key::StatusRecoveryCancelled, None, cx);
        }
    }

    fn sync_recovery(&mut self, progress: &Arc<Mutex<RecoveryViewState>>, cx: &mut Context<Self>) {
        let view = progress
            .lock()
            .map(|progress| progress.view.clone())
            .unwrap_or_default();
        if view.stage == nfc_ui::RecoveryStage::Idle && !self.recovery.running {
            return;
        }
        self.recovery.view = view.clone();
        self.session.update(cx, |session, cx| {
            session.set_recovery(Some(view));
            cx.notify();
        });
        cx.notify();
    }

    fn build_ndef_message(&self, cx: &App) -> NfcResult<NdefMessage> {
        let value = self.ndef_value_input.read(cx).value().trim().to_owned();
        match self.ndef_source {
            NdefSource::Text => {
                if value.is_empty() {
                    return Err(NfcError::InvalidArgument("enter NDEF text".to_owned()));
                }
                let language = self.ndef_language_input.read(cx).value().trim().to_owned();
                let language = if language.is_empty() {
                    "en".to_owned()
                } else {
                    language
                };
                NdefMessage::single(TextRecord::new(value, language)?.to_record()?)
            }
            NdefSource::Uri => {
                if value.is_empty() {
                    return Err(NfcError::InvalidArgument("enter a URI".to_owned()));
                }
                NdefMessage::single(UriRecord::new(value)?.to_record()?)
            }
            NdefSource::File => {
                let path = self.ndef_file_input.read(cx).value().trim().to_owned();
                if path.is_empty() {
                    return Err(NfcError::InvalidArgument(
                        "enter a raw NDEF file".to_owned(),
                    ));
                }
                let bytes = fs::read(&path)
                    .map_err(|error| NfcError::Io(format!("could not read {path}: {error}")))?;
                NdefMessage::decode(&bytes)
            }
        }
    }

    fn apply_dump(&mut self, dump: Dump, window: &mut Window, cx: &mut Context<Self>) {
        self.file_diff = None;
        self.previous_dump = self.dump.take();
        self.diff = self
            .previous_dump
            .as_ref()
            .and_then(|before| nfc_classic::diff::diff(before, &dump).ok());
        self.dump = Some(dump);
        self.selected_block = Some(0);
        self.sync_block_input(window, cx);
        self.refresh_ndef(cx);
        let detail = this_detail(self.dump.as_ref(), self.translator(cx));
        let identity = self
            .dump
            .as_ref()
            .and_then(|dump| dump.card.as_ref())
            .map(|card| CardIdentity::from_card(card, detail));
        self.session.update(cx, |session, cx| {
            session.classic_card = identity;
            cx.notify();
        });
    }

    fn apply_ndef_result(
        &mut self,
        dump: Dump,
        info: ClassicNdefInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_diff = None;
        self.previous_dump = self.dump.take();
        self.diff = self
            .previous_dump
            .as_ref()
            .and_then(|before| nfc_classic::diff::diff(before, &dump).ok());
        self.dump = Some(dump);
        self.selected_block = Some(0);
        self.sync_block_input(window, cx);
        self.ndef = Some(info);
        self.ndef_error = None;
        let detail = this_detail(self.dump.as_ref(), self.translator(cx));
        let identity = self
            .dump
            .as_ref()
            .and_then(|dump| dump.card.as_ref())
            .map(|card| CardIdentity::from_card(card, detail));
        self.session.update(cx, |session, cx| {
            session.classic_card = identity;
            cx.notify();
        });
    }

    fn sync_block_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .dump
            .as_ref()
            .zip(self.selected_block)
            .and_then(|(dump, number)| dump.blocks.get(number))
            .filter(|block| block.complete())
            .map(|block| nfc_ui::hex_bytes(&block.data))
            .unwrap_or_default();
        self.block_input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn recovery_candidates(&self, cx: &App) -> Vec<(KeyType, nfc_core::Key)> {
        let mut candidates = Vec::new();
        for entry in self.keys.entries() {
            for key_type in [KeyType::A, KeyType::B] {
                if entry.kind.includes(key_type) {
                    candidates.push((key_type, entry.key));
                }
            }
        }
        if self.settings.use_common_keys {
            for key in nfc_app::classic::common_keys() {
                for key_type in [KeyType::A, KeyType::B] {
                    candidates.push((key_type, key));
                }
            }
        }
        let _ = cx;
        candidates
    }

    fn persist_keys(&self) -> NfcResult<()> {
        match &self.settings.key_store_path {
            Some(path) => self.keys.save(path),
            None => Ok(()),
        }
    }

    fn path(&self, cx: &App) -> String {
        self.path_input.read(cx).value().trim().to_owned()
    }

    fn translator(&self, cx: &App) -> nfc_ui::Translator {
        self.session.read(cx).translator
    }

    fn card_detail(&self, cx: &App) -> String {
        this_detail(self.dump.as_ref(), self.translator(cx))
    }

    pub(crate) fn is_busy(&self, cx: &App) -> bool {
        self.session.read(cx).is_busy() || self.recovery.running
    }

    fn begin(&self, label: Key, cx: &mut App) {
        self.session.update(cx, |session, cx| {
            session.begin(label);
            cx.notify();
        });
    }

    fn finish(&self, cx: &mut App) {
        self.session.update(cx, |session, cx| {
            session.finish();
            cx.notify();
        });
    }

    fn set_status(&self, kind: StatusKind, summary: Key, detail: Option<String>, cx: &mut App) {
        self.session.update(cx, |session, cx| {
            session.set_status(StatusMessage::new(kind, summary, detail));
            cx.notify();
        });
    }

    fn fail(&self, summary: Key, error: &NfcError, cx: &mut App) {
        self.set_status(
            StatusKind::Error,
            summary,
            Some(format!(
                "{} · {error}",
                self.translator(cx).text(error_key(error))
            )),
            cx,
        );
        self.activity(StatusKind::Error, summary, error.to_string(), cx);
    }

    fn activity(&self, kind: StatusKind, title: Key, detail: impl Into<String>, cx: &mut App) {
        self.session.update(cx, |session, cx| {
            session.record(ActivityEntry::new(kind, title, detail));
            cx.notify();
        });
    }

    fn last_backup(&self, path: PathBuf, cx: &mut App) {
        self.session.update(cx, |session, cx| {
            session.set_last_backup(Some(path));
            cx.notify();
        });
    }
}

fn this_detail(dump: Option<&Dump>, translator: nfc_ui::Translator) -> String {
    dump.map(|dump| {
        let readability = dump.readability();
        format!(
            "{}/{} {} · {}/{} {}",
            readability.readable_blocks,
            readability.total_blocks,
            translator.text(Key::BlocksUnit),
            readability.readable_sectors,
            readability.total_sectors,
            translator.text(Key::SectorsUnit)
        )
    })
    .unwrap_or_default()
}

fn parse_u8(value: &str) -> NfcResult<u8> {
    value
        .trim()
        .parse::<u8>()
        .map_err(|_| NfcError::InvalidArgument(format!("block {value:?} is outside 0..=255")))
}

struct PendingWrite {
    block: usize,
    data: [u8; BLOCK_SIZE],
    keys: KeyStore,
    backup: BackupStore,
    use_common: bool,
    confirm_trailer: bool,
}

fn spawn_write(
    this: WeakEntity<ClassicWorkspace>,
    session: Entity<Session>,
    service: Arc<Mutex<Service>>,
    window: &Window,
    cx: &mut App,
    pending: PendingWrite,
    start: impl FnOnce(&mut ClassicWorkspace, &mut Context<ClassicWorkspace>) + 'static,
) {
    let PendingWrite {
        block,
        data,
        keys,
        backup,
        use_common,
        confirm_trailer,
    } = pending;
    let _ = this.update(cx, |this, cx| start(this, cx));
    spawn_service_with_session(
        this,
        session,
        service,
        window,
        cx,
        move |service| {
            service.classic_write_block(
                ClassicBlockWrite::new(block, data, &keys, &backup)
                    .include_common_keys(use_common)
                    .confirm_trailer(confirm_trailer),
            )
        },
        move |this, result, _window, cx| match result {
            Ok(write) => {
                this.finish(cx);
                if write.changed {
                    if let Some(dump) = &mut this.dump
                        && let Some(entry) = dump.blocks.get_mut(block)
                    {
                        entry.set_read(data);
                        entry.status = BlockStatus::Verified;
                    }
                    if let Some(backup) = &write.backup {
                        this.last_backup(backup.path.clone(), cx);
                    }
                    this.set_status(
                        StatusKind::Success,
                        Key::StatusBlockWritten,
                        Some(format!("block {block}")),
                        cx,
                    );
                    this.activity(
                        StatusKind::Success,
                        Key::WriteBlock,
                        format!(
                            "block {block} · {}",
                            write
                                .backup
                                .as_ref()
                                .map(|backup| backup.path.display().to_string())
                                .unwrap_or_default()
                        ),
                        cx,
                    );
                    this.refresh_ndef(cx);
                } else {
                    this.set_status(
                        StatusKind::Info,
                        Key::StatusBlockUnchanged,
                        Some(format!("block {block}")),
                        cx,
                    );
                }
                cx.notify();
            }
            Err(error) => {
                this.finish(cx);
                this.fail(Key::StatusWriteFailed, &error, cx);
            }
        },
    );
}

fn confirm_with_token(
    window: &mut Window,
    cx: &mut App,
    token: Entity<InputState>,
    title: String,
    body: String,
    ok_text: String,
    action: nfc_ui::ConfirmAction,
) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        let token = token.clone();
        let action = Rc::clone(&action);
        let body = body.clone();
        alert
            .title(title.clone())
            .description(body)
            .content({
                let token = token.clone();
                move |content, _, _| {
                    content.child(
                        gpui_kit::component::input::Input::new(&token).id("classic-confirm-token"),
                    )
                }
            })
            .show_cancel(true)
            .ok_text(ok_text.clone())
            .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
            .on_ok(move |_, window, cx| {
                let value = token.read(cx).value().trim().to_owned();
                if value != CONFIRMATION {
                    return false;
                }
                token.update(cx, |state, cx| state.set_value("", window, cx));
                action(window, cx);
                true
            })
    });
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
                on_done(this, result, window, cx);
                cx.notify();
            });
        })
        .detach();
}

fn spawn_service_with_session<T, F, D>(
    this: WeakEntity<ClassicWorkspace>,
    session: Entity<Session>,
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
                session.update(cx, |session, cx| {
                    session.finish();
                    cx.notify();
                });
                on_done(this, result, window, cx);
                cx.notify();
            });
        })
        .detach();
}

fn lock_service(service: &Arc<Mutex<Service>>) -> MutexGuard<'_, Service> {
    service.lock().unwrap_or_else(PoisonError::into_inner)
}
