//! Retained state and actions for the Type 2 screens.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gpui_kit::component::{WindowExt, input::InputState};
use gpui_kit::{App, AppContext as _, Context, Entity, ParentElement as _, WeakEntity, Window};
use nfc_app::Service;
use nfc_app::type2::{
    Type2Format, Type2Lock, Type2NdefWrite, Type2PageWrite, Type2Password, Type2Restore,
};
use nfc_core::{CancelToken, NfcError, PAGE_SIZE, Result as NfcResult};
use nfc_ndef::{NdefMessage, TextRecord, UriRecord};
use nfc_type2::{PageRole, RestoreOptions, Type2Image, Type2Ndef, read_ndef};
use nfc_ui::{
    ActivityEntry, CardIdentity, Key, Session, StatusKind, StatusMessage, error_key,
    parse_hex_array, parse_page_list, write_private,
};

use crate::model::{NdefSource, PageFilter, Type2Settings};

pub(crate) const CONFIRMATION: &str = "I_OWN_THIS_CARD";

/// GPUI Kit Type 2 / NDEF screens.
///
/// All card work runs through the shared [`Service`] on a background executor.
/// The view never performs reader I/O in `render`.
pub struct Type2Workspace {
    pub(crate) service: Arc<Mutex<Service>>,
    pub(crate) settings: Type2Settings,
    pub(crate) session: Entity<Session>,
    pub(crate) image: Option<Type2Image>,
    pub(crate) loaded: Option<Type2Image>,
    pub(crate) ndef: Option<Type2Ndef>,
    pub(crate) selected_page: Option<u8>,
    pub(crate) page_input: Entity<InputState>,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) ndef_value_input: Entity<InputState>,
    pub(crate) ndef_language_input: Entity<InputState>,
    pub(crate) ndef_file_input: Entity<InputState>,
    pub(crate) ndef_export_input: Entity<InputState>,
    pub(crate) lock_pages_input: Entity<InputState>,
    pub(crate) auth0_input: Entity<InputState>,
    pub(crate) access_input: Entity<InputState>,
    pub(crate) pwd_input: Entity<InputState>,
    pub(crate) pack_input: Entity<InputState>,
    pub(crate) confirm_input: Entity<InputState>,
    pub(crate) page_search: Entity<InputState>,
    pub(crate) page_filter: PageFilter,
    pub(crate) ndef_source: NdefSource,
    pub(crate) restore_configuration: bool,
    pub(crate) restore_cc: bool,
    pub(crate) restore_locks: bool,
}

impl Type2Workspace {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        service: Arc<Mutex<Service>>,
        settings: Type2Settings,
        session: Entity<Session>,
    ) -> Self {
        let page_input = cx.new(|cx| InputState::new(window, cx).placeholder("00 11 22 33"));
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/tag.json"));
        let ndef_value_input = cx.new(|cx| InputState::new(window, cx).placeholder("NDEF"));
        let ndef_language_input = cx.new(|cx| InputState::new(window, cx).placeholder("en"));
        let ndef_file_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/message.ndef"));
        let ndef_export_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/export.ndef"));
        let lock_pages_input = cx.new(|cx| InputState::new(window, cx).placeholder("4,8,16-20"));
        let auth0_input = cx.new(|cx| InputState::new(window, cx).placeholder("FF"));
        let access_input = cx.new(|cx| InputState::new(window, cx).placeholder("00"));
        let pwd_input = cx.new(|cx| InputState::new(window, cx).placeholder("00000000"));
        let pack_input = cx.new(|cx| InputState::new(window, cx).placeholder("0000"));
        let confirm_input = cx.new(|cx| InputState::new(window, cx).placeholder(CONFIRMATION));
        let page_search = cx.new(|cx| InputState::new(window, cx).placeholder("0..134"));
        let workspace = Self {
            service,
            settings,
            session,
            image: None,
            loaded: None,
            ndef: None,
            selected_page: None,
            page_input,
            path_input,
            ndef_value_input,
            ndef_language_input,
            ndef_file_input,
            ndef_export_input,
            lock_pages_input,
            auth0_input,
            access_input,
            pwd_input,
            pack_input,
            confirm_input,
            page_search,
            page_filter: PageFilter::All,
            ndef_source: NdefSource::Text,
            restore_configuration: false,
            restore_cc: false,
            restore_locks: false,
        };
        cx.observe(&workspace.session, |_, _, cx| cx.notify())
            .detach();
        workspace
    }

    pub fn image(&self) -> Option<&Type2Image> {
        self.image.as_ref()
    }

    pub fn loaded(&self) -> Option<&Type2Image> {
        self.loaded.as_ref()
    }

    pub fn ndef(&self) -> Option<&Type2Ndef> {
        self.ndef.as_ref()
    }

    pub fn selected_page(&self) -> Option<u8> {
        self.selected_page
    }

    pub fn set_settings(&mut self, settings: Type2Settings, cx: &mut Context<Self>) {
        self.settings = settings;
        cx.notify();
    }

    pub fn reset_card(&mut self, cx: &mut Context<Self>) {
        self.image = None;
        self.loaded = None;
        self.ndef = None;
        self.selected_page = None;
        cx.notify();
    }

    pub fn read_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        self.begin(Key::StatusReadingTag, cx);
        spawn_service(
            cx.weak_entity(),
            self.service.clone(),
            window,
            cx,
            |service| service.type2_read(CancelToken::new()),
            |this, result, window, cx| match result {
                Ok(read) => {
                    let warnings = read.warnings.len();
                    this.apply_image(read.image, window, cx);
                    this.finish(cx);
                    this.set_status(
                        if warnings == 0 {
                            StatusKind::Success
                        } else {
                            StatusKind::Warning
                        },
                        if warnings == 0 {
                            Key::StatusTagRead
                        } else {
                            Key::StatusTagReadWarnings
                        },
                        (warnings > 0).then(|| format!("{warnings} warning(s)")),
                        cx,
                    );
                    this.activity(StatusKind::Success, Key::ReadTag, this.card_detail(cx), cx);
                }
                Err(error) => {
                    this.finish(cx);
                    this.fail(Key::StatusReadFailed, &error, cx);
                }
            },
        );
    }

    pub(crate) fn select_page(&mut self, page: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_page = Some(page);
        self.sync_page_input(window, cx);
        cx.notify();
    }

    pub(crate) fn select_filter(&mut self, index: usize, cx: &mut Context<Self>) {
        self.page_filter = PageFilter::ALL
            .get(index)
            .copied()
            .unwrap_or(PageFilter::All);
        cx.notify();
    }

    pub(crate) fn select_ndef_source(&mut self, index: usize, cx: &mut Context<Self>) {
        self.ndef_source = NdefSource::ALL
            .get(index)
            .copied()
            .unwrap_or(NdefSource::Text);
        cx.notify();
    }

    pub(crate) fn write_selected_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        let Some(image) = self.image.as_ref() else {
            self.set_status(StatusKind::Error, Key::StatusNoCard, None, cx);
            return;
        };
        let Some(page) = self.selected_page else {
            self.set_status(StatusKind::Error, Key::StatusNoPages, None, cx);
            return;
        };
        let data = match parse_hex_array::<PAGE_SIZE>(&self.page_input.read(cx).value()) {
            Ok(data) => data,
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
                return;
            }
        };
        let role = image.role(page);
        if matches!(role, PageRole::Manufacturer | PageRole::Reserved) {
            self.set_status(StatusKind::Error, Key::PageNotReadable, None, cx);
            return;
        }
        let advanced = role.is_advanced();
        let translator = self.translator(cx);
        let backup = self.settings.backup.clone();
        let service = self.service.clone();
        let session = self.session.clone();
        let this = cx.weak_entity();
        let start = Rc::new(move |window: &mut Window, cx: &mut App| {
            let this = this.clone();
            let service = service.clone();
            let backup = backup.clone();
            let session = session.clone();
            spawn_page_write(
                this,
                session,
                service,
                window,
                cx,
                PendingPageWrite {
                    page,
                    data,
                    advanced,
                    backup,
                },
            );
        });
        if advanced {
            let role_label = translator.text(crate::model::role_key(role)).to_owned();
            let body = translator.fmt2(Key::ConfirmWriteAdvancedBody, page, role_label);
            confirm_with_token(
                window,
                cx,
                self.confirm_input.clone(),
                translator.text(Key::ConfirmWriteAdvancedTitle).to_owned(),
                body,
                translator.text(Key::WritePage).to_owned(),
                start,
            );
        } else {
            start(window, cx);
        }
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
        let result = self
            .image
            .as_ref()
            .ok_or_else(|| NfcError::InvalidArgument("read a card first".to_owned()))
            .and_then(nfc_type2::exchange::export)
            .and_then(|bytes| write_private(Path::new(&path), &bytes));
        match result {
            Ok(()) => {
                self.set_status(
                    StatusKind::Success,
                    Key::StatusDumpSaved,
                    Some(path.clone()),
                    cx,
                );
                self.activity(StatusKind::Success, Key::SaveDump, path, cx);
            }
            Err(error) => self.fail(Key::StatusSaveFailed, &error, cx),
        }
    }

    pub(crate) fn load_dump(&mut self, cx: &mut Context<Self>) {
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
            .and_then(|bytes| nfc_type2::exchange::load(&bytes));
        match result {
            Ok(image) => {
                self.loaded = Some(image);
                self.set_status(
                    StatusKind::Success,
                    Key::StatusDumpLoaded,
                    Some(path.clone()),
                    cx,
                );
                self.activity(StatusKind::Success, Key::LoadDump, path, cx);
            }
            Err(error) => self.fail(Key::StatusLoadFailed, &error, cx),
        }
        cx.notify();
    }

    pub(crate) fn restore_loaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        let Some(source) = self.loaded.clone() else {
            self.set_status(StatusKind::Error, Key::StatusNoDump, None, cx);
            return;
        };
        let translator = self.translator(cx);
        let backup = self.settings.backup.clone();
        let service = self.service.clone();
        let session = self.session.clone();
        let options = RestoreOptions {
            restore_user_memory: true,
            restore_capability_container: self.restore_cc,
            restore_configuration: self.restore_configuration,
            restore_lock_bytes: self.restore_locks,
        };
        let this = cx.weak_entity();
        confirm_with_token(
            window,
            cx,
            self.confirm_input.clone(),
            translator.text(Key::RestoreTitle).to_owned(),
            translator.text(Key::RestoreBody).to_owned(),
            translator.text(Key::RestoreTag).to_owned(),
            Rc::new(move |window, cx| {
                let this = this.clone();
                let service = service.clone();
                let backup = backup.clone();
                let session = session.clone();
                let source = source.clone();
                let _ = this.update(cx, |this, cx| this.begin(Key::BusyRestoring, cx));
                spawn_service_with_session(
                    this,
                    session,
                    service,
                    window,
                    cx,
                    move |service| {
                        service.type2_restore(
                            Type2Restore::new(source, &backup)
                                .options(options)
                                .confirm(true),
                        )
                    },
                    |this, result, window, cx| match result {
                        Ok(write) => {
                            this.finish(cx);
                            this.apply_image(write.image, window, cx);
                            this.last_backup(Some(write.backup.path.clone()), cx);
                            this.set_status(
                                StatusKind::Success,
                                Key::StatusRestoreComplete,
                                Some(format!(
                                    "{} written · {} unchanged",
                                    write.summary.written_pages, write.summary.unchanged_pages
                                )),
                                cx,
                            );
                            this.activity(
                                StatusKind::Success,
                                Key::StatusRestoreComplete,
                                format!(
                                    "{} written · {}",
                                    write.summary.written_pages,
                                    write.backup.path.display()
                                ),
                                cx,
                            );
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

    pub(crate) fn write_ndef(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        if self.image.is_none() {
            self.set_status(StatusKind::Error, Key::StatusNoCard, None, cx);
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
        let backup = self.settings.backup.clone();
        let session = self.session.clone();
        spawn_service_with_session(
            cx.weak_entity(),
            session,
            self.service.clone(),
            window,
            cx,
            move |service| service.type2_ndef_write(Type2NdefWrite::new(message, &backup)),
            |this, result, window, cx| match result {
                Ok(write) => {
                    this.finish(cx);
                    this.apply_image(write.image, window, cx);
                    this.last_backup(Some(write.backup.path.clone()), cx);
                    this.set_status(
                        StatusKind::Success,
                        Key::StatusNdefWritten,
                        Some(format!("{} page(s)", write.summary.written_pages)),
                        cx,
                    );
                    this.activity(
                        StatusKind::Success,
                        Key::WriteNdef,
                        format!(
                            "{} page(s) · {}",
                            write.summary.written_pages,
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
        let Some(message) = self.ndef.as_ref().and_then(Type2Ndef::message) else {
            self.set_status(StatusKind::Error, Key::NdefEmpty, None, cx);
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

    pub(crate) fn confirm_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        if self.image.is_none() {
            self.set_status(StatusKind::Error, Key::StatusNoCard, None, cx);
            return;
        }
        let translator = self.translator(cx);
        let backup = self.settings.backup.clone();
        let service = self.service.clone();
        let session = self.session.clone();
        let this = cx.weak_entity();
        confirm_with_token(
            window,
            cx,
            self.confirm_input.clone(),
            translator.text(Key::ConfirmFormatTitle).to_owned(),
            translator.text(Key::FormatTagNote).to_owned(),
            translator.text(Key::FormatTag).to_owned(),
            Rc::new(move |window, cx| {
                let this = this.clone();
                let service = service.clone();
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
                        service.type2_ndef_format(Type2Format::new(&backup).confirm(true))
                    },
                    |this, result, window, cx| match result {
                        Ok(write) => {
                            this.finish(cx);
                            this.apply_image(write.image, window, cx);
                            this.last_backup(Some(write.backup.path.clone()), cx);
                            this.set_status(
                                StatusKind::Success,
                                Key::StatusNdefFormatted,
                                Some(format!("{} page(s)", write.summary.written_pages)),
                                cx,
                            );
                            this.activity(
                                StatusKind::Success,
                                Key::FormatTag,
                                format!(
                                    "{} page(s) · {}",
                                    write.summary.written_pages,
                                    write.backup.path.display()
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

    pub(crate) fn confirm_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        if self.image.is_none() {
            self.set_status(StatusKind::Error, Key::StatusNoCard, None, cx);
            return;
        }
        let pages = match parse_page_list(&self.lock_pages_input.read(cx).value()) {
            Ok(pages) if !pages.is_empty() => pages,
            Ok(_) => {
                self.set_status(StatusKind::Error, Key::StatusNoPages, None, cx);
                return;
            }
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
                return;
            }
        };
        let translator = self.translator(cx);
        let backup = self.settings.backup.clone();
        let service = self.service.clone();
        let session = self.session.clone();
        let this = cx.weak_entity();
        let body = format!(
            "{} {} · {} {}",
            translator.text(Key::FieldPages),
            pages
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            translator.text(Key::LockNote),
            translator.text(Key::AdvancedActionNote)
        );
        confirm_with_token(
            window,
            cx,
            self.confirm_input.clone(),
            translator.text(Key::LockPagesTitle).to_owned(),
            body,
            translator.text(Key::ApplyLocks).to_owned(),
            Rc::new(move |window, cx| {
                let this = this.clone();
                let service = service.clone();
                let backup = backup.clone();
                let session = session.clone();
                let pages = pages.clone();
                let _ = this.update(cx, |this, cx| this.begin(Key::BusyFormatting, cx));
                spawn_service_with_session(
                    this,
                    session,
                    service,
                    window,
                    cx,
                    move |service| service.type2_lock(Type2Lock::new(pages, &backup).confirm(true)),
                    |this, result, window, cx| match result {
                        Ok(lock) => {
                            this.finish(cx);
                            this.apply_image(lock.image, window, cx);
                            this.last_backup(
                                lock.backup.as_ref().map(|backup| backup.path.clone()),
                                cx,
                            );
                            this.set_status(
                                StatusKind::Success,
                                if lock.update.is_empty() {
                                    Key::StatusLocksAlreadySet
                                } else {
                                    Key::StatusLockWritten
                                },
                                None,
                                cx,
                            );
                            this.activity(
                                StatusKind::Success,
                                Key::ApplyLocks,
                                "lock bits".to_owned(),
                                cx,
                            );
                        }
                        Err(error) => {
                            this.finish(cx);
                            this.fail(Key::StatusWriteFailed, &error, cx);
                        }
                    },
                );
            }),
        );
    }

    pub(crate) fn confirm_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) {
            return;
        }
        if self.image.is_none() {
            self.set_status(StatusKind::Error, Key::StatusNoCard, None, cx);
            return;
        }
        let auth0 = match self.auth0_input.read(cx).value().trim().parse::<u8>() {
            Ok(value) => value,
            Err(_) => {
                self.set_status(
                    StatusKind::Error,
                    Key::StatusInvalidInput,
                    Some("AUTH0 must be a page".into()),
                    cx,
                );
                return;
            }
        };
        let access = match parse_hex_array::<1>(&self.access_input.read(cx).value()) {
            Ok(value) => value[0],
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
                return;
            }
        };
        let password = match parse_hex_array::<PAGE_SIZE>(&self.pwd_input.read(cx).value()) {
            Ok(value) => value,
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
                return;
            }
        };
        let pack = match parse_hex_array::<2>(&self.pack_input.read(cx).value()) {
            Ok(value) => value,
            Err(error) => {
                self.fail(Key::StatusInvalidInput, &error, cx);
                return;
            }
        };
        let translator = self.translator(cx);
        let backup = self.settings.backup.clone();
        let service = self.service.clone();
        let session = self.session.clone();
        let this = cx.weak_entity();
        let body = format!(
            "AUTH0 {auth0:02X} · ACCESS {access:02X} · {}",
            translator.text(Key::PasswordNote)
        );
        confirm_with_token(
            window,
            cx,
            self.confirm_input.clone(),
            translator.text(Key::PasswordConfigTitle).to_owned(),
            body,
            translator.text(Key::SetPassword).to_owned(),
            Rc::new(move |window, cx| {
                let this = this.clone();
                let service = service.clone();
                let backup = backup.clone();
                let session = session.clone();
                let _ = this.update(cx, |this, cx| this.begin(Key::BusyWritingBlock, cx));
                spawn_service_with_session(
                    this,
                    session,
                    service,
                    window,
                    cx,
                    move |service| {
                        service.type2_password(
                            Type2Password::new(auth0, access, password, pack, &backup)
                                .confirm(true),
                        )
                    },
                    |this, result, window, cx| match result {
                        Ok(write) => {
                            this.finish(cx);
                            this.apply_image(write.image, window, cx);
                            this.last_backup(Some(write.backup.path.clone()), cx);
                            this.set_status(
                                StatusKind::Success,
                                Key::StatusPasswordWritten,
                                None,
                                cx,
                            );
                            this.activity(
                                StatusKind::Success,
                                Key::SetPassword,
                                write.backup.path.display().to_string(),
                                cx,
                            );
                        }
                        Err(error) => {
                            this.finish(cx);
                            this.fail(Key::StatusWriteFailed, &error, cx);
                        }
                    },
                );
            }),
        );
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

    fn apply_image(&mut self, image: Type2Image, window: &mut Window, cx: &mut Context<Self>) {
        self.ndef = read_ndef(&image).ok();
        if self.selected_page.is_none()
            || self
                .selected_page
                .is_some_and(|page| !image.layout.contains(page))
        {
            self.selected_page = Some(image.layout.user_first);
        }
        self.image = Some(image);
        self.sync_page_input(window, cx);
        let identity = self.card_identity(self.translator(cx));
        self.session.update(cx, |session, cx| {
            session.type2_card = identity;
            cx.notify();
        });
    }

    fn sync_page_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .image
            .as_ref()
            .zip(self.selected_page)
            .and_then(|(image, page)| image.page(page))
            .map(|data| nfc_ui::hex_bytes(data))
            .unwrap_or_default();
        self.page_input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn card_identity(&self, translator: nfc_ui::Translator) -> Option<CardIdentity> {
        let image = self.image.as_ref()?;
        let ndef = self.ndef.as_ref();
        Some(CardIdentity {
            card_type: image.layout.product.name().to_owned(),
            family: nfc_ui::CardFamily::Type2,
            uid: image.uid_hex(),
            atqa: "—".to_owned(),
            sak: "—".to_owned(),
            detail: format!(
                "{}/{} {} · NDEF {}/{} {}",
                image.known_pages(),
                image.layout.page_count(),
                translator.text(Key::PagesUnit),
                ndef.map_or(0, |ndef| ndef.message_bytes),
                ndef.map_or(image.cc.ndef_capacity(), |ndef| ndef.capacity),
                translator.text(Key::BytesUnit)
            ),
        })
    }

    fn card_detail(&self, cx: &App) -> String {
        self.card_identity(self.translator(cx))
            .map(|identity| identity.detail)
            .unwrap_or_default()
    }

    fn path(&self, cx: &App) -> String {
        self.path_input.read(cx).value().trim().to_owned()
    }

    fn translator(&self, cx: &App) -> nfc_ui::Translator {
        self.session.read(cx).translator
    }

    pub(crate) fn is_busy(&self, cx: &App) -> bool {
        self.session.read(cx).is_busy()
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

    fn last_backup(&self, path: Option<PathBuf>, cx: &mut App) {
        if let Some(path) = path {
            self.session.update(cx, |session, cx| {
                session.set_last_backup(Some(path));
                cx.notify();
            });
        }
    }
}

struct PendingPageWrite {
    page: u8,
    data: [u8; PAGE_SIZE],
    advanced: bool,
    backup: nfc_app::classic::BackupStore,
}

fn spawn_page_write(
    this: WeakEntity<Type2Workspace>,
    session: Entity<Session>,
    service: Arc<Mutex<Service>>,
    window: &Window,
    cx: &mut App,
    pending: PendingPageWrite,
) {
    let PendingPageWrite {
        page,
        data,
        advanced,
        backup,
    } = pending;
    let _ = this.update(cx, |this, cx| this.begin(Key::BusyWritingBlock, cx));
    spawn_service_with_session(
        this,
        session,
        service,
        window,
        cx,
        move |service| {
            service.type2_write_page(
                Type2PageWrite::new(page, data, &backup)
                    .advanced(advanced)
                    .confirm(advanced),
            )
        },
        |this, result, window, cx| match result {
            Ok(write) => {
                this.finish(cx);
                this.apply_image(write.image, window, cx);
                this.last_backup(write.backup.map(|backup| backup.path), cx);
                this.set_status(
                    if write.changed {
                        StatusKind::Success
                    } else {
                        StatusKind::Info
                    },
                    if write.changed {
                        Key::StatusPageWritten
                    } else {
                        Key::StatusPageUnchanged
                    },
                    Some(format!("page {}", write.page)),
                    cx,
                );
                this.activity(
                    StatusKind::Success,
                    Key::WritePage,
                    format!("page {}", write.page),
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
                        gpui_kit::component::input::Input::new(&token).id("type2-confirm-token"),
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
    this: WeakEntity<Type2Workspace>,
    service: Arc<Mutex<Service>>,
    window: &Window,
    cx: &mut App,
    task: F,
    on_done: D,
) where
    T: Send + 'static,
    F: FnOnce(&mut Service) -> NfcResult<T> + Send + 'static,
    D: FnOnce(&mut Type2Workspace, NfcResult<T>, &mut Window, &mut Context<Type2Workspace>)
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
    this: WeakEntity<Type2Workspace>,
    session: Entity<Session>,
    service: Arc<Mutex<Service>>,
    window: &Window,
    cx: &mut App,
    task: F,
    on_done: D,
) where
    T: Send + 'static,
    F: FnOnce(&mut Service) -> NfcResult<T> + Send + 'static,
    D: FnOnce(&mut Type2Workspace, NfcResult<T>, &mut Window, &mut Context<Type2Workspace>)
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
