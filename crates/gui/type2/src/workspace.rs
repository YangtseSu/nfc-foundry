//! Retained Type 2 memory, NDEF, and protection workspace state.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gpui_kit::component::{WindowExt, button::ButtonVariant, input::InputState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, ParentElement as _, SharedString, WeakEntity, Window,
};
use nfc_app::Service;
use nfc_app::classic::BackupStore;
use nfc_app::type2::{
    Type2Format, Type2Lock, Type2NdefWrite, Type2PageWrite, Type2Password, Type2Restore,
};
use nfc_core::{CancelToken, NfcError, PAGE_SIZE, Result as NfcResult};
use nfc_ndef::{NdefMessage, TextRecord};
use nfc_type2::{
    Configuration, LockBits, NdefState, PageRole, RestoreOptions, Type2Image, Type2Ndef, read_ndef,
};

use crate::model::{DeviceSummary, StatusMessage, WorkspaceTab};

const CONFIRMATION: &str = "I_OWN_THIS_CARD";

/// GPUI Kit Type 2 / NDEF workspace.
///
/// All card work runs through the shared [`Service`] on a background executor.
/// The view never performs reader I/O in `render`.
pub struct Type2Workspace {
    pub(crate) service: Arc<Mutex<Service>>,
    pub(crate) backup: BackupStore,
    pub(crate) devices: Vec<DeviceSummary>,
    pub(crate) selected_device: Option<String>,
    pub(crate) tab: WorkspaceTab,
    pub(crate) image: Option<Type2Image>,
    pub(crate) loaded: Option<Type2Image>,
    pub(crate) ndef: Option<Type2Ndef>,
    pub(crate) selected_page: Option<u8>,
    pub(crate) busy: Option<SharedString>,
    pub(crate) status: Option<StatusMessage>,
    pub(crate) page_data_input: Entity<InputState>,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) ndef_text_input: Entity<InputState>,
    pub(crate) ndef_language_input: Entity<InputState>,
    pub(crate) lock_pages_input: Entity<InputState>,
    pub(crate) auth0_input: Entity<InputState>,
    pub(crate) access_input: Entity<InputState>,
    pub(crate) pwd_input: Entity<InputState>,
    pub(crate) pack_input: Entity<InputState>,
    pub(crate) confirm_input: Entity<InputState>,
    pub(crate) last_backup: Option<PathBuf>,
}

impl Type2Workspace {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        service: Arc<Mutex<Service>>,
        backup: BackupStore,
    ) -> Self {
        let page_data_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("8 hexadecimal digits"));
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("Project JSON path"));
        let ndef_text_input = cx.new(|cx| InputState::new(window, cx).placeholder("NDEF text"));
        let ndef_language_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Language (en)"));
        let lock_pages_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Pages, for example 4,8,16-20"));
        let auth0_input = cx.new(|cx| InputState::new(window, cx).placeholder("AUTH0 page (FF)"));
        let access_input = cx.new(|cx| InputState::new(window, cx).placeholder("ACCESS hex"));
        let pwd_input = cx.new(|cx| InputState::new(window, cx).placeholder("PWD (8 hex)"));
        let pack_input = cx.new(|cx| InputState::new(window, cx).placeholder("PACK (4 hex)"));
        let confirm_input = cx.new(|cx| InputState::new(window, cx).placeholder(CONFIRMATION));
        let mut workspace = Self {
            service,
            backup,
            devices: Vec::new(),
            selected_device: None,
            tab: WorkspaceTab::Memory,
            image: None,
            loaded: None,
            ndef: None,
            selected_page: None,
            busy: None,
            status: None,
            page_data_input,
            path_input,
            ndef_text_input,
            ndef_language_input,
            lock_pages_input,
            auth0_input,
            access_input,
            pwd_input,
            pack_input,
            confirm_input,
            last_backup: None,
        };
        workspace.reload_devices(cx);
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

    pub fn tab(&self) -> WorkspaceTab {
        self.tab
    }

    pub fn status(&self) -> Option<&StatusMessage> {
        self.status.as_ref()
    }

    pub fn busy(&self) -> Option<&SharedString> {
        self.busy.as_ref()
    }

    pub fn last_backup(&self) -> Option<&Path> {
        self.last_backup.as_deref()
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
            self.status = Some(StatusMessage::info(
                "Select a reader and read a Type 2 tag.",
            ));
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
                self.image = None;
                self.loaded = None;
                self.ndef = None;
                self.selected_page = None;
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
        self.busy = Some(SharedString::from("Reading tag"));
        cx.notify();
        spawn_service(
            cx.weak_entity(),
            self.service.clone(),
            window,
            cx,
            |service: &mut Service| service.type2_read(CancelToken::new()),
            |this, result, window, cx| match result {
                Ok(read) => {
                    let warnings = read.warnings;
                    this.apply_image(read.image.clone(), window, cx);
                    this.status = Some(if warnings.is_empty() {
                        StatusMessage::success("Tag read")
                    } else {
                        StatusMessage::warning(format!(
                            "Tag read with {} warning(s): {}",
                            warnings.len(),
                            warnings.join("; ")
                        ))
                    });
                }
                Err(error) => {
                    this.status = Some(StatusMessage::error(format!("Read failed: {error}")));
                }
            },
        );
    }

    pub(crate) fn select_page(&mut self, page: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_page = Some(page);
        self.sync_page_input(window, cx);
        cx.notify();
    }

    pub(crate) fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tab = WorkspaceTab::from_index(index);
        cx.notify();
    }

    pub(crate) fn confirm_write_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(image) = self.image.as_ref() else {
            self.status = Some(StatusMessage::error("Read a Type 2 tag first."));
            cx.notify();
            return;
        };
        let Some(page) = self.selected_page else {
            self.status = Some(StatusMessage::error("Select a page first."));
            cx.notify();
            return;
        };
        let data = match parse_hex_array::<PAGE_SIZE>(&self.page_data_input.read(cx).value()) {
            Ok(data) => data,
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Page data: {error}")));
                cx.notify();
                return;
            }
        };
        let role = image.role(page);
        if matches!(role, PageRole::Manufacturer | PageRole::Reserved) {
            self.status = Some(StatusMessage::error(format!(
                "Page {page} is {} and cannot be written.",
                role.label()
            )));
            cx.notify();
            return;
        }
        if role.is_advanced() {
            let summary = format!(
                "Page {page} is {}. This is an Advanced write. Type {CONFIRMATION} to continue.",
                role.label()
            );
            let this = cx.weak_entity();
            let service = self.service.clone();
            let backup = self.backup.clone();
            let token = self.confirm_input.clone();
            window.open_alert_dialog(cx, move |alert, _, _| {
                let this = this.clone();
                let service = service.clone();
                let backup = backup.clone();
                let token = token.clone();
                alert
                    .title("Write Advanced page?")
                    .description(summary.clone())
                    .content({
                        let token = token.clone();
                        move |content, _, _| {
                            content.child(
                                gpui_kit::component::input::Input::new(&token)
                                    .id("type2-confirm-token"),
                            )
                        }
                    })
                    .show_cancel(true)
                    .ok_text("Write page")
                    .ok_variant(ButtonVariant::Danger)
                    .on_ok(move |_, window, cx| {
                        if token.read(cx).value().trim() != CONFIRMATION {
                            return false;
                        }
                        start_page_write(
                            this.clone(),
                            service.clone(),
                            PendingPageWrite {
                                page,
                                data,
                                advanced: true,
                                backup: backup.clone(),
                            },
                            window,
                            cx,
                        );
                        true
                    })
            });
        } else {
            start_page_write(
                cx.weak_entity(),
                self.service.clone(),
                PendingPageWrite {
                    page,
                    data,
                    advanced: false,
                    backup: self.backup.clone(),
                },
                window,
                cx,
            );
        }
    }

    pub(crate) fn write_ndef(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        if self.image.is_none() {
            self.status = Some(StatusMessage::error("Read a Type 2 tag first."));
            cx.notify();
            return;
        }
        let text = self.ndef_text_input.read(cx).value().trim().to_owned();
        let language = self.ndef_language_input.read(cx).value().trim().to_owned();
        let language = if language.is_empty() {
            "en".to_owned()
        } else {
            language
        };
        if text.is_empty() {
            self.status = Some(StatusMessage::error("Enter NDEF text first."));
            cx.notify();
            return;
        }
        let message = match TextRecord::new(text, language)
            .and_then(|record| record.to_record())
            .and_then(NdefMessage::single)
        {
            Ok(message) => message,
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("NDEF text: {error}")));
                cx.notify();
                return;
            }
        };
        self.busy = Some(SharedString::from("Writing NDEF"));
        cx.notify();
        let backup = self.backup.clone();
        spawn_service(
            cx.weak_entity(),
            self.service.clone(),
            window,
            cx,
            move |service: &mut Service| {
                service.type2_ndef_write(Type2NdefWrite::new(message, &backup))
            },
            |this, result, window, cx| match result {
                Ok(write) => {
                    this.apply_image(write.image, window, cx);
                    this.last_backup = Some(write.backup.path.clone());
                    this.status = Some(StatusMessage::success(format!(
                        "NDEF written and verified: {} page(s) written · backup {}",
                        write.summary.written_pages,
                        write.backup.path.display()
                    )));
                }
                Err(error) => {
                    this.status = Some(StatusMessage::error(format!("NDEF write failed: {error}")));
                }
            },
        );
    }

    pub(crate) fn confirm_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        if self.image.is_none() {
            self.status = Some(StatusMessage::error("Read a Type 2 tag first."));
            cx.notify();
            return;
        }
        let this = cx.weak_entity();
        let service = self.service.clone();
        let backup = self.backup.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let service = service.clone();
            let backup = backup.clone();
            alert
                .title("Format NDEF?")
                .description(
                    "The NDEF data area is replaced with an empty formatted tag. A project backup is written first.",
                )
                .show_cancel(true)
                .ok_text("Format")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let this = this.clone();
                    let service = service.clone();
                    let backup = backup.clone();
                    let _ = this.update(cx, |this, cx| {
                        this.busy = Some(SharedString::from("Formatting NDEF"));
                        cx.notify();
                    });
                    spawn_service(
                        this,
                        service,
                        window,
                        cx,
                        move |service: &mut Service| {
                            service.type2_ndef_format(Type2Format::new(&backup).confirm(true))
                        },
                        |this, result, window, cx| match result {
                            Ok(write) => {
                                this.apply_image(write.image, window, cx);
                                this.last_backup = Some(write.backup.path.clone());
                                this.status = Some(StatusMessage::success(format!(
                                    "NDEF formatted: {} page(s) written · backup {}",
                                    write.summary.written_pages,
                                    write.backup.path.display()
                                )));
                            }
                            Err(error) => {
                                this.status = Some(StatusMessage::error(format!(
                                    "Format failed: {error}"
                                )));
                            }
                        },
                    );
                    true
                })
        });
    }

    pub(crate) fn confirm_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        if self.image.is_none() {
            self.status = Some(StatusMessage::error("Read a Type 2 tag first."));
            cx.notify();
            return;
        }
        let pages = match parse_page_list(&self.lock_pages_input.read(cx).value()) {
            Ok(pages) if !pages.is_empty() => pages,
            Ok(_) => {
                self.status = Some(StatusMessage::error("Enter pages to lock."));
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Pages: {error}")));
                cx.notify();
                return;
            }
        };
        let this = cx.weak_entity();
        let service = self.service.clone();
        let backup = self.backup.clone();
        let token = self.confirm_input.clone();
        let summary = format!(
            "Lock bits are one-way. Pages {} become read-only after this write. Type {CONFIRMATION} to continue.",
            pages
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let service = service.clone();
            let backup = backup.clone();
            let token = token.clone();
            let pages = pages.clone();
            alert
                .title("Lock pages?")
                .description(summary.clone())
                .content({
                    let token = token.clone();
                    move |content, _, _| {
                        content.child(
                            gpui_kit::component::input::Input::new(&token)
                                .id("type2-confirm-token"),
                        )
                    }
                })
                .show_cancel(true)
                .ok_text("Lock pages")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    if token.read(cx).value().trim() != CONFIRMATION {
                        return false;
                    }
                    let this = this.clone();
                    let service = service.clone();
                    let backup = backup.clone();
                    let pages = pages.clone();
                    let _ = this.update(cx, |this, cx| {
                        this.busy = Some(SharedString::from("Locking pages"));
                        cx.notify();
                    });
                    spawn_service(
                        this,
                        service,
                        window,
                        cx,
                        move |service: &mut Service| {
                            service.type2_lock(Type2Lock::new(pages, &backup).confirm(true))
                        },
                        |this, result, window, cx| match result {
                            Ok(lock) => {
                                this.apply_image(lock.image, window, cx);
                                this.last_backup =
                                    lock.backup.as_ref().map(|backup| backup.path.clone());
                                this.status =
                                    Some(StatusMessage::success("Lock bits written and verified"));
                            }
                            Err(error) => {
                                this.status =
                                    Some(StatusMessage::error(format!("Lock failed: {error}")));
                            }
                        },
                    );
                    true
                })
        });
    }

    pub(crate) fn confirm_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        if self.image.is_none() {
            self.status = Some(StatusMessage::error("Read a Type 2 tag first."));
            cx.notify();
            return;
        }
        let auth0 = match self.auth0_input.read(cx).value().trim().parse::<u8>() {
            Ok(value) => value,
            Err(_) => {
                self.status = Some(StatusMessage::error("AUTH0 must be a page number."));
                cx.notify();
                return;
            }
        };
        let access = match parse_hex_array::<1>(&self.access_input.read(cx).value()) {
            Ok(value) => value[0],
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("ACCESS: {error}")));
                cx.notify();
                return;
            }
        };
        let password = match parse_hex_array::<PAGE_SIZE>(&self.pwd_input.read(cx).value()) {
            Ok(value) => value,
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("PWD: {error}")));
                cx.notify();
                return;
            }
        };
        let pack = match parse_hex_array::<2>(&self.pack_input.read(cx).value()) {
            Ok(value) => value,
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("PACK: {error}")));
                cx.notify();
                return;
            }
        };
        let this = cx.weak_entity();
        let service = self.service.clone();
        let backup = self.backup.clone();
        let token = self.confirm_input.clone();
        let summary = format!(
            "PWD/PACK, AUTH0 {auth0:02X}, and ACCESS {access:02X} are written permanently. Type {CONFIRMATION} to continue."
        );
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let service = service.clone();
            let backup = backup.clone();
            let token = token.clone();
            alert
                .title("Change password configuration?")
                .description(summary.clone())
                .content({
                    let token = token.clone();
                    move |content, _, _| {
                        content.child(
                            gpui_kit::component::input::Input::new(&token)
                                .id("type2-confirm-token"),
                        )
                    }
                })
                .show_cancel(true)
                .ok_text("Set password")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    if token.read(cx).value().trim() != CONFIRMATION {
                        return false;
                    }
                    let this = this.clone();
                    let service = service.clone();
                    let backup = backup.clone();
                    let _ = this.update(cx, |this, cx| {
                        this.busy = Some(SharedString::from("Writing password configuration"));
                        cx.notify();
                    });
                    spawn_service(
                        this,
                        service,
                        window,
                        cx,
                        move |service: &mut Service| {
                            service.type2_password(
                                Type2Password::new(auth0, access, password, pack, &backup)
                                    .confirm(true),
                            )
                        },
                        |this, result, window, cx| match result {
                            Ok(write) => {
                                this.apply_image(write.image, window, cx);
                                this.last_backup = Some(write.backup.path.clone());
                                this.status = Some(StatusMessage::success(format!(
                                    "Password configuration written and verified · backup {}",
                                    write.backup.path.display()
                                )));
                            }
                            Err(error) => {
                                this.status = Some(StatusMessage::error(format!(
                                    "Password configuration failed: {error}"
                                )));
                            }
                        },
                    );
                    true
                })
        });
    }

    pub(crate) fn save_dump(&mut self, cx: &mut Context<Self>) {
        let path = self.path(cx);
        if path.is_empty() {
            self.status = Some(StatusMessage::error("Enter a dump file path first."));
            cx.notify();
            return;
        }
        let result = self
            .image
            .as_ref()
            .ok_or_else(|| NfcError::InvalidArgument("read a card first".to_owned()))
            .and_then(nfc_type2::exchange::export)
            .and_then(|bytes| write_file(Path::new(&path), &bytes));
        match result {
            Ok(()) => self.status = Some(StatusMessage::success(format!("Saved {path}"))),
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Couldn't save dump: {error}")));
            }
        }
        cx.notify();
    }

    pub(crate) fn load_dump(&mut self, cx: &mut Context<Self>) {
        let path = self.path(cx);
        if path.is_empty() {
            self.status = Some(StatusMessage::error("Enter a dump file path first."));
            cx.notify();
            return;
        }
        let result = fs::read(&path)
            .map_err(|error| NfcError::Io(format!("could not read {path}: {error}")))
            .and_then(|bytes| nfc_type2::exchange::load(&bytes));
        match result {
            Ok(image) => {
                self.loaded = Some(image);
                self.status = Some(StatusMessage::success(format!(
                    "Loaded {path}; Restore… writes it to the selected tag"
                )));
            }
            Err(error) => {
                self.status = Some(StatusMessage::error(format!("Couldn't load dump: {error}")));
            }
        }
        cx.notify();
    }

    pub(crate) fn confirm_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(source) = self.loaded.clone() else {
            self.status = Some(StatusMessage::error("Load a dump first."));
            cx.notify();
            return;
        };
        let this = cx.weak_entity();
        let service = self.service.clone();
        let backup = self.backup.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let service = service.clone();
            let backup = backup.clone();
            let source = source.clone();
            alert
                .title("Restore this dump?")
                .description(
                    "User memory is overwritten with the loaded image. A project backup is written before the first change.",
                )
                .show_cancel(true)
                .ok_text("Restore")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let this = this.clone();
                    let service = service.clone();
                    let backup = backup.clone();
                    let source = source.clone();
                    let _ = this.update(cx, |this, cx| {
                        this.busy = Some(SharedString::from("Restoring tag"));
                        cx.notify();
                    });
                    spawn_service(
                        this,
                        service,
                        window,
                        cx,
                        move |service: &mut Service| {
                            service.type2_restore(
                                Type2Restore::new(source, &backup)
                                    .options(RestoreOptions {
                                        restore_user_memory: true,
                                        ..RestoreOptions::default()
                                    })
                                    .confirm(true),
                            )
                        },
                        |this, result, window, cx| match result {
                            Ok(write) => {
                                this.apply_image(write.image, window, cx);
                                this.last_backup = Some(write.backup.path.clone());
                                this.status = Some(StatusMessage::success(format!(
                                    "Restore complete: {} written, {} unchanged · backup {}",
                                    write.summary.written_pages,
                                    write.summary.unchanged_pages,
                                    write.backup.path.display()
                                )));
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
        cx.notify();
    }

    fn sync_page_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .image
            .as_ref()
            .zip(self.selected_page)
            .and_then(|(image, page)| image.page(page))
            .map(|data| hex(data))
            .unwrap_or_default();
        self.page_data_input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn path(&self, cx: &App) -> String {
        self.path_input.read(cx).value().trim().to_owned()
    }
}

struct PendingPageWrite {
    page: u8,
    data: [u8; PAGE_SIZE],
    advanced: bool,
    backup: BackupStore,
}

fn start_page_write(
    this: WeakEntity<Type2Workspace>,
    service: Arc<Mutex<Service>>,
    pending: PendingPageWrite,
    window: &Window,
    cx: &mut App,
) {
    let PendingPageWrite {
        page,
        data,
        advanced,
        backup,
    } = pending;
    let _ = this.update(cx, |this, cx| {
        this.busy = Some(SharedString::from("Writing page"));
        cx.notify();
    });
    spawn_service(
        this,
        service,
        window,
        cx,
        move |service: &mut Service| {
            service.type2_write_page(
                Type2PageWrite::new(page, data, &backup)
                    .advanced(advanced)
                    .confirm(advanced),
            )
        },
        |this, result, window, cx| match result {
            Ok(write) => {
                this.apply_image(write.image, window, cx);
                this.last_backup = write.backup.as_ref().map(|record| record.path.clone());
                this.status = Some(if write.changed {
                    StatusMessage::success(format!("Page {} written and verified", write.page))
                } else {
                    StatusMessage::info(format!("Page {} already matches", write.page))
                });
            }
            Err(error) => {
                this.status = Some(StatusMessage::error(format!("Write failed: {error}")));
            }
        },
    );
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

fn parse_page_list(value: &str) -> NfcResult<Vec<u8>> {
    let mut pages = Vec::new();
    for part in value.split([',', ' ']).filter(|part| !part.is_empty()) {
        if let Some((first, last)) = part.split_once('-') {
            let first: u8 = first
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {first:?}")))?;
            let last: u8 = last
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {last:?}")))?;
            if first > last {
                return Err(NfcError::InvalidArgument(format!(
                    "page range {part:?} is reversed"
                )));
            }
            pages.extend(first..=last);
        } else {
            let page: u8 = part
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {part:?}")))?;
            pages.push(page);
        }
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

fn parse_hex_array<const N: usize>(value: &str) -> NfcResult<[u8; N]> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != N * 2 {
        return Err(NfcError::InvalidArgument(format!(
            "expected {} hexadecimal digits",
            N * 2
        )));
    }
    let mut output = [0u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = (hex_digit(compact.as_bytes()[index * 2])? << 4)
            | hex_digit(compact.as_bytes()[index * 2 + 1])?;
    }
    Ok(output)
}

fn hex_digit(character: u8) -> NfcResult<u8> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(NfcError::InvalidArgument(
            "value contains a non-hexadecimal character".to_owned(),
        )),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

pub(crate) fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

/// Locked page list for the current image.
pub(crate) fn locked_pages(image: &Type2Image, locks: &LockBits) -> Vec<u8> {
    (0..image.layout.total_pages)
        .filter(|page| locks.is_locked(&image.layout, *page))
        .collect()
}

/// Optional configuration for the current image.
pub(crate) fn configuration(image: &Type2Image) -> Option<Configuration> {
    Configuration::from_image(image).ok()
}

/// NDEF state label used by the view and tests.
pub(crate) fn ndef_label(state: &NdefState) -> String {
    match state {
        NdefState::Unformatted => "unformatted".to_owned(),
        NdefState::Empty => "empty".to_owned(),
        NdefState::Message(message) => format!("{} record(s)", message.records().len()),
    }
}
