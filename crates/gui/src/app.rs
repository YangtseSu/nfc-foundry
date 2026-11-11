//! GTK4 workbench shell and the first real NFC Foundry slice.
//!
//! The view owns navigation and presentation only. Every card operation goes
//! through `nfc-app`; reader I/O runs on a worker thread and returns to the GTK
//! main context with an `async-channel`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{Orientation, gio, glib};
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicKeyscanResult, ClassicNdefFormat,
    ClassicNdefFormatResult, ClassicNdefWrite, ClassicNdefWriteResult, ClassicRestore,
    ClassicRestoreResult, ClassicWriteResult, KeyEntry, KeyKind, KeySource, KeyStore, KeyTypes,
    KeyscanProgress, KeyscanSink,
};
use nfc_app::type2::{
    Type2Format, Type2Lock, Type2LockResult, Type2MutationResult, Type2NdefWrite, Type2PageResult,
    Type2PageWrite, Type2Password, Type2Restore,
};
use nfc_app::{CardReadResult, ClassicNdefInfo, ClassicNdefState, Service};
use nfc_classic::{Dump, DumpFormat, DumpLoad, diff, exchange};
use nfc_core::{CancelToken, Key, NfcError, Result as NfcResult};
use nfc_drivers::DriverRegistry;
use nfc_ndef::{
    NdefMessage, NdefRecord, TextRecord, Tnf, UriRecord, WifiAuthentication, WifiEncryption,
    WifiRecord,
};
use nfc_recover::auto::RecoveryStrategy;
use nfc_recover::{ProgressSink, RecoveryProgress, RecoveryRequest, RecoveryResult};
use nfc_transport::DeviceInfo;
use nfc_type2::{NdefState, RestoreOptions, Type2Image, Type2Ndef, exchange as type2_exchange};

use crate::classic::{self, BlockRow, NdefDraft, NdefSource};
use crate::i18n::{self, Language, Localized};
use crate::settings::{UiPreferences, preferences_path};
use crate::type2::{self as type2_ui, ADVANCED_PHRASE, Session as Type2Session};

pub fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        r#"
        .mono { font-family: monospace; font-size: 0.75em; }
        .rail-title {
            font-size: 0.78em;
            font-weight: 700;
            opacity: 0.65;
        }
        .block-title { font-size: 1.35em; font-weight: 700; }
        stackswitcher > button {
            padding-left: 8px;
            padding-right: 8px;
            min-width: 0;
        }
        "#,
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Dependencies of one workbench instance. Production builds use the real
/// driver registry and the user's key store; tests supply a scripted service
/// and an isolated key file.
pub struct Workbench {
    pub service: Arc<Mutex<Service>>,
    pub keys: KeyStore,
    pub keys_path: Option<PathBuf>,
    pub backup: BackupStore,
    pub key_error: Option<String>,
    pub preferences: UiPreferences,
    pub preferences_path: Option<PathBuf>,
    pub settings_error: Option<String>,
}

impl Workbench {
    pub fn hardware() -> Self {
        let service = Arc::new(Mutex::new(Service::with_transport(Arc::new(
            DriverRegistry,
        ))));
        let preferences_path = preferences_path();
        let (preferences, settings_error) = match &preferences_path {
            Some(path) => match UiPreferences::load(path) {
                Ok(preferences) => {
                    let error = preferences.validate().err().map(|error| error.to_string());
                    (preferences, error)
                }
                Err(error) => (UiPreferences::default(), Some(error.to_string())),
            },
            None => (UiPreferences::default(), None),
        };
        let keys_path = preferences
            .key_store
            .clone()
            .or_else(KeyStore::default_path);
        let (keys, key_error) = match &keys_path {
            Some(path) => match KeyStore::load_or_default(path) {
                Ok(keys) => (keys, None),
                Err(error) => (KeyStore::new(), Some(error.to_string())),
            },
            None => (KeyStore::new(), None),
        };
        let backup = preferences
            .backup_dir
            .clone()
            .map(BackupStore::new)
            .unwrap_or_default();
        Self {
            service,
            keys,
            keys_path,
            backup,
            key_error,
            preferences,
            preferences_path,
            settings_error,
        }
    }
}

pub fn build_ui(app: &adw::Application) {
    gtk::Window::set_default_icon_name("nfc-foundry");
    build_ui_with(app, Workbench::hardware());
}

fn build_ui_with(app: &adw::Application, workbench: Workbench) -> Rc<Ui> {
    install_css();
    let Workbench {
        service,
        keys,
        keys_path,
        backup,
        key_error,
        preferences,
        preferences_path,
        settings_error,
    } = workbench;

    i18n::set_current_language(preferences.language());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("NFC Foundry")
        .default_width(1360)
        .default_height(880)
        .icon_name("nfc-foundry")
        .build();

    let toast = adw::ToastOverlay::new();

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection.clone()));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<BlockRow>("Block", 44, false, |row| {
        format!("{:03}", row.number)
    }));
    table.append_column(&text_column::<BlockRow>("Sector", 48, false, |row| {
        format!("{:02}", row.sector)
    }));
    table.append_column(&text_column::<BlockRow>("Type", 34, false, |row| {
        row.kind.to_owned()
    }));
    table.append_column(&text_column::<BlockRow>("Status", 58, false, |row| {
        row.status.to_owned()
    }));
    let data_column = text_column::<BlockRow>("Data", 320, true, |row| row.data.clone());
    data_column.set_expand(true);
    table.append_column(&data_column);
    let table_scroll = gtk::ScrolledWindow::builder().child(&table).build();
    table_scroll.set_has_frame(true);
    table_scroll.set_overlay_scrolling(false);
    table_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    table_scroll.set_vexpand(true);
    table_scroll.set_hexpand(true);

    let empty = gtk::Box::new(Orientation::Vertical, 8);
    empty.set_valign(gtk::Align::Center);
    empty.set_halign(gtk::Align::Center);
    let empty_title = gtk::Label::new(Some("No card loaded"));
    empty_title.add_css_class("title-2");
    let empty_hint = gtk::Label::new(Some(
        "Select a reader and press Read card.\nThe block table shows the real card image.",
    ));
    empty_hint.set_justify(gtk::Justification::Center);
    empty_hint.add_css_class("dim-label");
    empty.append(&empty_title);
    empty.append(&empty_hint);

    let table_stack = gtk::Stack::new();
    table_stack.set_vexpand(true);
    table_stack.set_hexpand(true);
    table_stack.add_named(&empty, Some("empty"));
    table_stack.add_named(&table_scroll, Some("table"));
    table_stack.set_visible_child_name("empty");

    let inspector = build_inspector();

    let card_page = gtk::Box::new(Orientation::Horizontal, 12);
    card_page.set_margin_top(12);
    card_page.set_margin_bottom(12);
    card_page.set_margin_start(12);
    card_page.set_margin_end(12);
    card_page.append(&table_stack);
    card_page.append(&inspector.root);
    table_stack.set_hexpand(true);
    inspector.root.set_valign(gtk::Align::Start);

    let ndef_view = build_ndef_view();

    let keys_view = build_keys_view();
    let keyscan_view = build_keyscan_view();
    let recovery_view = build_recovery_view(&preferences);
    let diff_view = build_diff_view();
    let restore_view = build_restore_view();
    let type2_view = build_type2_view();
    let diagnostics_view = build_diagnostics_view();
    let activity_view = build_activity_view();
    let settings_view = build_settings_view(&preferences);

    let views = gtk::Stack::new();
    views.set_transition_type(gtk::StackTransitionType::Crossfade);
    views.set_vexpand(true);
    views.add_titled(&card_page, Some("card"), "Card");
    views.add_titled(&type2_view.root, Some("type2"), "Type 2");
    views.add_titled(&ndef_view.root, Some("ndef"), "NDEF");
    views.add_titled(&keys_view.root, Some("keys"), "Keys");
    views.add_titled(&keyscan_view.root, Some("keyscan"), "Key scan");
    views.add_titled(&recovery_view.root, Some("recovery"), "Recovery");
    views.add_titled(&diff_view.root, Some("diff"), "Diff");
    views.add_titled(&restore_view.root, Some("restore"), "Restore");
    views.add_titled(&diagnostics_view.root, Some("diagnostics"), "Diagnostics");
    views.add_titled(&activity_view.root, Some("activity"), "Activity");
    views.add_titled(&settings_view.root, Some("settings"), "Settings");
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&views));
    switcher.set_halign(gtk::Align::Start);
    switcher.set_hexpand(false);
    let switcher_scroll = gtk::ScrolledWindow::builder()
        .child(&switcher)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .has_frame(false)
        .propagate_natural_height(true)
        .build();
    switcher_scroll.set_hexpand(true);
    let switcher_row = gtk::Box::new(Orientation::Horizontal, 0);
    switcher_row.set_halign(gtk::Align::Fill);
    switcher_row.set_margin_top(8);
    switcher_row.set_margin_start(12);
    switcher_row.set_margin_end(12);
    switcher_row.append(&switcher_scroll);
    let center = gtk::Box::new(Orientation::Vertical, 0);
    center.set_hexpand(true);
    center.set_vexpand(true);
    center.set_halign(gtk::Align::Fill);
    center.append(&switcher_row);
    center.append(&views);

    let (reader_card, reader_head, reader_body) = rail_card("READER");
    let reader_model = gtk::StringList::new(&[]);
    let reader_combo = gtk::DropDown::new(Some(reader_model.clone()), None::<gtk::Expression>);
    reader_combo.set_hexpand(true);
    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.add_css_class("flat");
    refresh_button.set_tooltip_text(Some("Refresh readers"));
    reader_head.append(&refresh_button);
    let reader_value = gtk::Label::new(Some("No reader selected"));
    reader_value.set_xalign(0.0);
    reader_value.set_wrap(true);
    reader_value.add_css_class("caption");
    reader_value.add_css_class("dim-label");
    reader_body.append(&reader_combo);
    reader_body.append(&reader_value);

    let (card_card, _, card_body) = rail_card("CARD");
    let card_title = gtk::Label::new(Some("No card"));
    card_title.set_xalign(0.0);
    card_title.add_css_class("title-3");
    let card_detail = gtk::Label::new(Some("Select a reader and read a card."));
    card_detail.set_xalign(0.0);
    card_detail.set_wrap(true);
    card_detail.add_css_class("caption");
    card_detail.add_css_class("dim-label");
    card_body.append(&card_title);
    card_body.append(&card_detail);

    let (key_card, _, key_body) = rail_card("SECTOR KEYS");
    let key_value = gtk::Label::new(None);
    key_value.set_xalign(0.0);
    key_value.set_wrap(true);
    key_value.add_css_class("caption");
    key_value.add_css_class("dim-label");
    key_body.append(&key_value);

    let (selected_card, _, selected_body) = rail_card("SELECTED");
    let selected_label = gtk::Label::new(Some("No block selected"));
    selected_label.set_xalign(0.0);
    selected_label.set_wrap(true);
    selected_label.add_css_class("caption");
    selected_body.append(&selected_label);

    let rail = gtk::Box::new(Orientation::Vertical, 10);
    rail.set_size_request(280, -1);
    rail.set_halign(gtk::Align::Start);
    rail.append(&reader_card);
    rail.append(&card_card);
    rail.append(&key_card);
    rail.append(&selected_card);

    let body = gtk::Paned::new(Orientation::Horizontal);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.set_vexpand(true);
    body.set_resize_start_child(false);
    body.set_shrink_start_child(false);
    body.set_position(292);
    body.set_start_child(Some(&rail));
    body.set_end_child(Some(&center));

    let status = gtk::Label::new(Some("Ready"));
    status.set_xalign(0.0);
    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    let detail = gtk::Label::new(Some("No card"));
    detail.set_xalign(1.0);
    detail.set_hexpand(true);
    detail.add_css_class("caption");
    detail.add_css_class("dim-label");
    let status_bar = gtk::Box::new(Orientation::Horizontal, 8);
    status_bar.add_css_class("toolbar");
    status_bar.append(&spinner);
    status_bar.append(&status);
    status_bar.append(&detail);

    let content = gtk::Box::new(Orientation::Vertical, 0);
    content.set_vexpand(true);
    content.append(&body);
    content.append(&status_bar);

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "NFC Foundry",
        i18n::Translator::new(i18n::current_language()).localized(i18n::Key::AppSubtitle),
    )));
    let read_button = gtk::Button::with_label("Read card");
    read_button.add_css_class("suggested-action");
    header.pack_end(&read_button);
    let actions_button = gtk::MenuButton::new();
    actions_button.set_localized("Actions");
    actions_button.add_css_class("flat");
    header.pack_end(&actions_button);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    toast.set_child(Some(&toolbar));
    window.set_content(Some(&toast));

    let ui = Rc::new(Ui {
        window: window.clone(),
        toast: toast.clone(),
        service,
        keys: RefCell::new(keys),
        keys_path: RefCell::new(keys_path),
        backup: RefCell::new(backup),
        preferences: RefCell::new(preferences),
        preferences_path,
        settings_error: RefCell::new(settings_error),
        dump: RefCell::new(None),
        ndef: RefCell::new(None),
        live: Cell::new(false),
        busy: Cell::new(false),
        rows,
        selection,
        table_stack,
        status,
        detail,
        spinner,
        read_button,
        reader_combo,
        reader_model,
        reader_value,
        card_title,
        card_detail,
        key_value,
        selected_label,
        ndef_view,
        inspector,
        keys_view,
        keyscan_view,
        recovery_view,
        diff_view,
        restore_view,
        type2_view,
        diagnostics_view,
        activity_view,
        settings_view,
        activity: RefCell::new(Vec::new()),
        views: views.clone(),
        type2: RefCell::new(None),
        type2_restore: RefCell::new(None),
        diff_before: RefCell::new(None),
        diff_after: RefCell::new(None),
        restore_load: RefCell::new(None),
        pending_restore_dialog: RefCell::new(None),
        pending_write_dialog: RefCell::new(None),
        pending_ndef_write_dialog: RefCell::new(None),
        pending_ndef_format_dialog: RefCell::new(None),
        pending_type2_page_dialog: RefCell::new(None),
        pending_type2_advanced_dialog: RefCell::new(None),
        pending_type2_lock_dialog: RefCell::new(None),
        pending_type2_password_dialog: RefCell::new(None),
        pending_type2_restore_dialog: RefCell::new(None),
        pending_type2_format_dialog: RefCell::new(None),
        devices: RefCell::new(Vec::new()),
        keyscan_cancel: RefCell::new(None),
        keyscan_result: RefCell::new(None),
        keyscan_run: Cell::new(0),
        recovery_cancel: RefCell::new(None),
        recovery_result: RefCell::new(None),
        recovery_started: RefCell::new(None),
        recovery_timer: RefCell::new(None),
        recovery_run: Cell::new(0),
        language_updating: Cell::new(false),
    });

    connect_signals(&ui);
    build_actions_menu(&ui, &actions_button);
    update_card(&ui);
    update_ndef(&ui);
    render_type2(&ui);
    refresh_keys(&ui);
    render_settings(&ui);
    update_keyscan_sensitivity(&ui);
    update_recovery_sensitivity(&ui);
    update_restore_sensitivity(&ui);
    refresh_devices(&ui);
    i18n::localize_widget_tree(&ui.window);
    window.present();

    if let Some(error) = key_error {
        toast_error(&ui, &format!("Key store: {error}"));
    }
    if let Some(error) = ui.settings_error.borrow().clone() {
        toast_error(&ui, &format!("Preferences: {error}"));
    }
    ui
}

struct Inspector {
    root: gtk::Box,
    title: gtk::Label,
    meta: gtk::Label,
    error: gtk::Label,
    details: gtk::Label,
    hex: gtk::Entry,
    read: gtk::Button,
    write: gtk::Button,
}

struct KeysView {
    root: gtk::Box,
    rows: gio::ListStore,
    status: gtk::Label,
    hex: gtk::Entry,
    sector: gtk::Entry,
    kind: gtk::DropDown,
    add: gtk::Button,
    import: gtk::Button,
    save: gtk::Button,
    export: gtk::Button,
}

struct DiffView {
    root: gtk::Box,
    before: gtk::Button,
    after: gtk::Button,
    before_label: gtk::Label,
    after_label: gtk::Label,
    compare: gtk::Button,
    summary: gtk::Label,
    rows: gio::ListStore,
}

struct RestoreView {
    root: gtk::Box,
    load: gtk::Button,
    file: gtk::Label,
    summary: gtk::Label,
    continue_on_error: gtk::CheckButton,
    restore: gtk::Button,
    rows: gio::ListStore,
    result: gtk::Label,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RestoreRow {
    block: String,
    state: &'static str,
    detail: String,
}

fn restore_rows(result: &ClassicRestoreResult) -> Vec<RestoreRow> {
    result
        .blocks
        .iter()
        .map(|block| RestoreRow {
            block: format!("{:03}", block.block),
            state: block.state.label(),
            detail: block.detail.clone().unwrap_or_default(),
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActivityRow {
    time: String,
    operation: String,
    result: String,
    detail: String,
}

struct KeyscanView {
    root: gtk::Box,
    common: gtk::CheckButton,
    key_types: gtk::DropDown,
    dictionary: gtk::Entry,
    choose_dictionary: gtk::Button,
    start: gtk::Button,
    cancel: gtk::Button,
    progress: gtk::Label,
    summary: gtk::Label,
    save: gtk::Button,
    export: gtk::Button,
    rows: gio::ListStore,
    table_stack: gtk::Stack,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SectorKeyRow {
    sector: String,
    key_a: String,
    key_b: String,
}

struct RecoveryView {
    root: gtk::Box,
    strategy: gtk::DropDown,
    advanced: gtk::Expander,
    known_row: gtk::Box,
    target_row: gtk::Box,
    known_block: gtk::Entry,
    known_type: gtk::DropDown,
    target_block: gtk::Entry,
    target_type: gtk::DropDown,
    tables: gtk::Entry,
    choose_tables: gtk::Button,
    max_nonces: gtk::SpinButton,
    common: gtk::CheckButton,
    start: gtk::Button,
    cancel: gtk::Button,
    elapsed: gtk::Label,
    log: RecoveryLog,
    save: gtk::Button,
}

/// Scrollable, read-only recovery log. Live rows (acquisition, filtering,
/// brute force, per-sector progress) are replaced in place; phase transitions
/// and results are appended.
struct RecoveryLog {
    scroller: gtk::ScrolledWindow,
    view: gtk::TextView,
    buffer: gtk::TextBuffer,
    live_start: Cell<Option<i32>>,
}

impl RecoveryLog {
    fn new() -> Self {
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_cursor_visible(false);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_top_margin(6);
        view.set_bottom_margin(6);
        view.set_left_margin(6);
        view.set_right_margin(6);
        let buffer = view.buffer();
        for (name, weight) in [
            ("info", 400),
            ("success", 700),
            ("warning", 700),
            ("error", 700),
        ] {
            let tag = gtk::TextTag::builder().name(name).weight(weight).build();
            buffer.tag_table().add(&tag);
        }
        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(180)
            .build();
        scroller.set_has_frame(true);
        scroller.set_vexpand(true);
        Self {
            scroller,
            view,
            buffer,
            live_start: Cell::new(None),
        }
    }

    fn clear(&self) {
        self.buffer.set_text("");
        self.live_start.set(None);
    }

    #[cfg(test)]
    fn text(&self) -> String {
        self.buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), true)
            .to_string()
    }

    fn append(&self, tag: &str, line: &str) {
        self.end_live();
        let mut end = self.buffer.end_iter();
        self.buffer.insert_with_tags_by_name(&mut end, line, &[tag]);
        self.buffer.insert(&mut end, "\n");
        self.scroll_to_end();
    }

    fn live(&self, tag: &str, line: &str) {
        if let Some(start) = self.live_start.get() {
            let mut start_iter = self.buffer.iter_at_offset(start);
            let mut end_iter = self.buffer.end_iter();
            self.buffer.delete(&mut start_iter, &mut end_iter);
            let mut insert = self.buffer.iter_at_offset(start);
            self.buffer
                .insert_with_tags_by_name(&mut insert, line, &[tag]);
        } else {
            let mut end = self.buffer.end_iter();
            let start = end.offset();
            self.buffer.insert_with_tags_by_name(&mut end, line, &[tag]);
            self.live_start.set(Some(start));
        }
        self.scroll_to_end();
    }

    fn end_live(&self) {
        if self.live_start.take().is_some() {
            let mut end = self.buffer.end_iter();
            self.buffer.insert(&mut end, "\n");
        }
    }

    fn scroll_to_end(&self) {
        let mut end = self.buffer.end_iter();
        self.view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
    }
}

#[derive(Clone, Debug)]
struct RecoveryOutcome {
    result: RecoveryResult,
    target_block: u8,
    target_key_type: nfc_core::KeyType,
}

struct DiagnosticsView {
    root: gtk::Box,
    version: gtk::Label,
    drivers: gtk::Label,
    tables: gtk::Label,
    readers: gtk::Label,
    build: gtk::Label,
    refresh: gtk::Button,
}

struct ActivityView {
    root: gtk::Box,
    rows: gio::ListStore,
    clear: gtk::Button,
}

struct SettingsView {
    root: gtk::Box,
    key_store: gtk::Entry,
    key_store_choose: gtk::Button,
    backup_dir: gtk::Entry,
    backup_choose: gtk::Button,
    tables: gtk::Entry,
    tables_choose: gtk::Button,
    common: gtk::Switch,
    dump_format: gtk::DropDown,
    language: gtk::DropDown,
    save: gtk::Button,
    config_path: gtk::Label,
    status: gtk::Label,
}

struct Ui {
    window: adw::ApplicationWindow,
    toast: adw::ToastOverlay,
    service: Arc<Mutex<Service>>,
    keys: RefCell<KeyStore>,
    keys_path: RefCell<Option<PathBuf>>,
    backup: RefCell<BackupStore>,
    preferences: RefCell<UiPreferences>,
    preferences_path: Option<PathBuf>,
    settings_error: RefCell<Option<String>>,
    dump: RefCell<Option<Dump>>,
    ndef: RefCell<Option<ClassicNdefInfo>>,
    live: Cell<bool>,
    busy: Cell<bool>,
    rows: gio::ListStore,
    selection: gtk::SingleSelection,
    table_stack: gtk::Stack,
    status: gtk::Label,
    detail: gtk::Label,
    spinner: gtk::Spinner,
    read_button: gtk::Button,
    reader_combo: gtk::DropDown,
    reader_model: gtk::StringList,
    reader_value: gtk::Label,
    card_title: gtk::Label,
    card_detail: gtk::Label,
    key_value: gtk::Label,
    selected_label: gtk::Label,
    ndef_view: NdefView,
    inspector: Inspector,
    keys_view: KeysView,
    keyscan_view: KeyscanView,
    recovery_view: RecoveryView,
    diff_view: DiffView,
    restore_view: RestoreView,
    type2_view: Type2View,
    diagnostics_view: DiagnosticsView,
    activity_view: ActivityView,
    settings_view: SettingsView,
    activity: RefCell<Vec<ActivityRow>>,
    views: gtk::Stack,
    type2: RefCell<Option<Type2Session>>,
    type2_restore: RefCell<Option<(PathBuf, Type2Image)>>,
    diff_before: RefCell<Option<DumpLoad>>,
    diff_after: RefCell<Option<DumpLoad>>,
    restore_load: RefCell<Option<(PathBuf, DumpLoad)>>,
    pending_restore_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_write_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_ndef_write_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_ndef_format_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_page_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_advanced_dialog: RefCell<Option<TypedDialog>>,
    pending_type2_lock_dialog: RefCell<Option<TypedDialog>>,
    pending_type2_password_dialog: RefCell<Option<TypedDialog>>,
    pending_type2_restore_dialog: RefCell<Option<TypedDialog>>,
    pending_type2_format_dialog: RefCell<Option<TypedDialog>>,
    devices: RefCell<Vec<DeviceInfo>>,
    keyscan_cancel: RefCell<Option<CancelToken>>,
    keyscan_result: RefCell<Option<ClassicKeyscanResult>>,
    keyscan_run: Cell<u64>,
    recovery_cancel: RefCell<Option<CancelToken>>,
    recovery_result: RefCell<Option<RecoveryOutcome>>,
    recovery_started: RefCell<Option<Instant>>,
    recovery_timer: RefCell<Option<glib::SourceId>>,
    recovery_run: Cell<u64>,
    language_updating: Cell<bool>,
}

#[derive(Clone)]
struct TypedDialog {
    dialog: adw::AlertDialog,
    entry: gtk::Entry,
}

struct Type2View {
    root: gtk::Box,
    tabs: gtk::Stack,
    advanced: gtk::Switch,
    summary: gtk::Label,
    warnings: gtk::Label,
    rows: gio::ListStore,
    selection: gtk::SingleSelection,
    table_stack: gtk::Stack,
    editor: Type2Editor,
    save: gtk::Button,
    load: gtk::Button,
    restore_load: gtk::Button,
    restore_label: gtk::Label,
    include_memory: gtk::CheckButton,
    include_configuration: gtk::CheckButton,
    include_cc: gtk::CheckButton,
    include_locks: gtk::CheckButton,
    restore: gtk::Button,
    restore_status: gtk::Label,
    configuration: gtk::Label,
    lock_pages: gtk::Entry,
    lock_preview: gtk::Label,
    lock_apply: gtk::Button,
    auth0: gtk::Entry,
    access: gtk::Entry,
    pwd: gtk::Entry,
    pack: gtk::Entry,
    password_apply: gtk::Button,
    password_note: gtk::Label,
}

struct Type2Editor {
    root: gtk::Box,
    title: gtk::Label,
    meta: gtk::Label,
    details: gtk::Label,
    hex: gtk::Entry,
    ascii: gtk::Label,
    feedback: gtk::Label,
    write: gtk::Button,
}

struct NdefView {
    root: gtk::Box,
    state: gtk::Label,
    info: gtk::Box,
    source: gtk::DropDown,
    input_stack: gtk::Stack,
    language: gtk::Entry,
    text: gtk::TextView,
    uri: gtk::Entry,
    raw_choose: gtk::Button,
    raw_label: gtk::Label,
    preview: gtk::Label,
    feedback: gtk::Label,
    write: gtk::Button,
    format: gtk::Button,
    export: gtk::Button,
    read_only: gtk::CheckButton,
    draft: RefCell<NdefDraft>,
}

fn rail_card(title: &str) -> (gtk::Box, gtk::Box, gtk::Box) {
    let card = gtk::Box::new(Orientation::Vertical, 6);
    card.add_css_class("card");
    let head = gtk::Box::new(Orientation::Horizontal, 6);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.add_css_class("rail-title");
    head.append(&label);
    let body = gtk::Box::new(Orientation::Vertical, 4);
    card.append(&head);
    card.append(&body);
    (card, head, body)
}

fn build_inspector() -> Inspector {
    let root = gtk::Box::new(Orientation::Vertical, 8);
    root.add_css_class("card");
    root.set_width_request(340);
    root.set_hexpand(false);
    root.set_margin_start(4);

    let kicker = gtk::Label::new(Some("BLOCK INSPECTOR"));
    kicker.set_xalign(0.0);
    kicker.add_css_class("rail-title");
    let title = gtk::Label::new(Some("No block"));
    title.set_xalign(0.0);
    title.add_css_class("block-title");
    let meta = gtk::Label::new(Some("Read a card to inspect a block."));
    meta.set_xalign(0.0);
    meta.set_wrap(true);
    meta.add_css_class("caption");
    meta.add_css_class("dim-label");
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("caption");
    error.add_css_class("error");
    error.set_visible(false);

    let details = gtk::Label::new(None);
    details.set_xalign(0.0);
    details.set_wrap(true);
    details.set_selectable(true);
    details.add_css_class("caption");
    details.add_css_class("mono");

    let hex = gtk::Entry::new();
    hex.set_placeholder_text(Some("block not read"));
    hex.add_css_class("mono");
    hex.set_hexpand(true);

    let read = gtk::Button::with_label("Read block");
    let write = gtk::Button::with_label("Write block");
    write.add_css_class("suggested-action");
    read.set_sensitive(false);
    write.set_sensitive(false);
    let actions = gtk::Box::new(Orientation::Horizontal, 6);
    actions.append(&read);
    actions.append(&write);

    let note = gtk::Label::new(Some(
        "Writes are confirmed, backed up, and read back. Block 0 stays read-only.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    root.append(&kicker);
    root.append(&title);
    root.append(&meta);
    root.append(&error);
    root.append(&details);
    root.append(&gtk::Separator::new(Orientation::Horizontal));
    root.append(&hex);
    root.append(&actions);
    root.append(&note);

    Inspector {
        root,
        title,
        meta,
        error,
        details,
        hex,
        read,
        write,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyRow {
    sector: String,
    kind: &'static str,
    key: String,
    source: &'static str,
}

fn key_rows(store: &KeyStore) -> Vec<KeyRow> {
    store
        .entries()
        .iter()
        .map(|entry| KeyRow {
            sector: entry.sector_label(),
            kind: entry.kind.label(),
            key: entry.key.to_hex(),
            source: entry.source.label(),
        })
        .collect()
}

fn page_box() -> gtk::Box {
    let page = gtk::Box::new(Orientation::Vertical, 10);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page
}

fn build_ndef_view() -> NdefView {
    let root = page_box();
    let state = gtk::Label::new(Some("No card loaded"));
    state.set_xalign(0.0);
    state.add_css_class("title-3");

    let info = gtk::Box::new(Orientation::Vertical, 8);
    let info_scroll = gtk::ScrolledWindow::builder().child(&info).build();
    info_scroll.set_vexpand(true);
    info_scroll.set_has_frame(false);

    let composer = gtk::Box::new(Orientation::Vertical, 8);
    composer.add_css_class("card");
    composer.set_margin_top(6);
    composer.set_margin_bottom(6);
    let title = gtk::Label::new(Some("Compose message"));
    title.set_xalign(0.0);
    title.add_css_class("title-4");

    let source_model = gtk::StringList::new(
        &NdefSource::ALL
            .iter()
            .map(|source| source.label())
            .collect::<Vec<_>>(),
    );
    let source = gtk::DropDown::new(Some(source_model), None::<gtk::Expression>);
    let source_row = gtk::Box::new(Orientation::Horizontal, 8);
    let source_label = gtk::Label::new(Some("Record"));
    source_label.set_xalign(0.0);
    source_row.append(&source_label);
    source_row.append(&source);

    let input_stack = gtk::Stack::new();
    input_stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    let text_page = gtk::Box::new(Orientation::Vertical, 6);
    let language_row = gtk::Box::new(Orientation::Horizontal, 8);
    let language_label = gtk::Label::new(Some("Language"));
    language_label.set_xalign(0.0);
    let language = gtk::Entry::builder()
        .text("en")
        .placeholder_text("en")
        .width_chars(8)
        .build();
    language_row.append(&language_label);
    language_row.append(&language);
    let text = gtk::TextView::new();
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_accepts_tab(false);
    text.set_top_margin(6);
    text.set_bottom_margin(6);
    let text_scroll = gtk::ScrolledWindow::builder()
        .child(&text)
        .min_content_height(84)
        .has_frame(true)
        .build();
    text_page.append(&language_row);
    text_page.append(&text_scroll);
    input_stack.add_named(&text_page, Some(NdefSource::Text.stack_name()));

    let uri_page = gtk::Box::new(Orientation::Vertical, 6);
    let uri = gtk::Entry::builder()
        .placeholder_text("https://example.com/")
        .build();
    uri_page.append(&uri);
    input_stack.add_named(&uri_page, Some(NdefSource::Uri.stack_name()));

    let raw_page = gtk::Box::new(Orientation::Horizontal, 8);
    let raw_choose = gtk::Button::with_label("Choose NDEF file…");
    let raw_label = gtk::Label::new(Some("No file chosen"));
    raw_label.set_xalign(0.0);
    raw_label.set_hexpand(true);
    raw_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    raw_label.add_css_class("dim-label");
    raw_page.append(&raw_choose);
    raw_page.append(&raw_label);
    input_stack.add_named(&raw_page, Some(NdefSource::Raw.stack_name()));

    let preview = gtk::Label::new(Some("No message composed"));
    preview.set_xalign(0.0);
    preview.set_wrap(true);
    preview.set_selectable(true);
    preview.add_css_class("mono");

    let feedback = gtk::Label::new(None);
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    feedback.add_css_class("caption");
    feedback.add_css_class("dim-label");

    let actions = gtk::Box::new(Orientation::Horizontal, 8);
    let write = gtk::Button::with_label("Write NDEF");
    write.add_css_class("suggested-action");
    write.set_sensitive(false);
    let format = gtk::Button::with_label("Format NDEF area…");
    format.add_css_class("destructive-action");
    format.set_sensitive(false);
    let export = gtk::Button::with_label("Export message…");
    export.set_sensitive(false);
    let read_only = gtk::CheckButton::with_label("Leave the NDEF area read-only");
    actions.append(&write);
    actions.append(&format);
    actions.append(&export);
    actions.append(&read_only);

    composer.append(&title);
    composer.append(&source_row);
    composer.append(&input_stack);
    composer.append(&preview);
    composer.append(&feedback);
    composer.append(&actions);

    root.append(&state);
    root.append(&composer);
    root.append(&info_scroll);

    NdefView {
        root,
        state,
        info,
        source,
        input_stack,
        language,
        text,
        uri,
        raw_choose,
        raw_label,
        preview,
        feedback,
        write,
        format,
        export,
        read_only,
        draft: RefCell::new(NdefDraft::default()),
    }
}

fn build_type2_view() -> Type2View {
    let root = gtk::Box::new(Orientation::Vertical, 10);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let header = gtk::Box::new(Orientation::Horizontal, 8);
    let heading = gtk::Box::new(Orientation::Vertical, 2);
    let title = gtk::Label::new(Some("Type 2 / NTAG"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let summary = gtk::Label::new(Some("Read a Type 2 tag to inspect its pages."));
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.add_css_class("caption");
    summary.add_css_class("dim-label");
    heading.append(&title);
    heading.append(&summary);
    heading.set_hexpand(true);
    let advanced_label = gtk::Label::new(Some("Advanced mode"));
    advanced_label.add_css_class("caption");
    let advanced = gtk::Switch::new();
    advanced.set_valign(gtk::Align::Center);
    advanced.set_sensitive(false);
    header.append(&heading);
    header.append(&advanced_label);
    header.append(&advanced);

    let warnings = gtk::Label::new(None);
    warnings.set_xalign(0.0);
    warnings.set_wrap(true);
    warnings.add_css_class("caption");
    warnings.add_css_class("dim-label");
    warnings.set_visible(false);

    let tabs = gtk::Stack::new();
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.set_vexpand(true);
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&tabs));
    switcher.set_halign(gtk::Align::Start);
    let switcher_row = gtk::Box::new(Orientation::Horizontal, 0);
    switcher_row.append(&switcher);

    let memory = gtk::Box::new(Orientation::Vertical, 8);
    let toolbar = gtk::Box::new(Orientation::Horizontal, 8);
    let save = gtk::Button::with_label("Save dump…");
    save.set_sensitive(false);
    let load = gtk::Button::with_label("Load dump…");
    toolbar.append(&save);
    toolbar.append(&load);
    memory.append(&toolbar);

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection.clone()));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<type2_ui::PageRow>(
        "Page",
        56,
        false,
        |row| format!("{:03}", row.page),
    ));
    table.append_column(&text_column::<type2_ui::PageRow>(
        "Role",
        104,
        false,
        |row| row.role.to_owned(),
    ));
    table.append_column(&text_column::<type2_ui::PageRow>(
        "Lock",
        90,
        false,
        |row| row.lock.clone(),
    ));
    let data_column = text_column::<type2_ui::PageRow>("Data", 180, true, |row| row.data.clone());
    data_column.set_expand(true);
    table.append_column(&data_column);
    let table_scroll = gtk::ScrolledWindow::builder().child(&table).build();
    table_scroll.set_has_frame(true);
    table_scroll.set_overlay_scrolling(false);
    table_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    table_scroll.set_vexpand(true);
    table_scroll.set_hexpand(true);

    let empty = gtk::Label::new(Some(
        "Read a Type 2 tag to list its pages.\nPartial reads keep unknown pages as —.",
    ));
    empty.set_justify(gtk::Justification::Center);
    empty.add_css_class("dim-label");
    let table_stack = gtk::Stack::new();
    table_stack.set_vexpand(true);
    table_stack.set_hexpand(true);
    table_stack.add_named(&empty, Some("empty"));
    table_stack.add_named(&table_scroll, Some("table"));
    table_stack.set_visible_child_name("empty");

    let editor = build_type2_editor();
    let split = gtk::Paned::new(Orientation::Horizontal);
    split.set_vexpand(true);
    split.set_resize_start_child(true);
    split.set_shrink_start_child(false);
    split.set_position(560);
    split.set_start_child(Some(&table_stack));
    split.set_end_child(Some(&editor.root));
    memory.append(&split);

    let restore_card = gtk::Box::new(Orientation::Vertical, 6);
    restore_card.add_css_class("card");
    let restore_title = gtk::Label::new(Some("Restore project image"));
    restore_title.set_xalign(0.0);
    restore_title.add_css_class("title-4");
    let restore_row = gtk::Box::new(Orientation::Horizontal, 8);
    let restore_load = gtk::Button::with_label("Choose project image…");
    let restore_label = gtk::Label::new(Some("No restore image loaded"));
    restore_label.set_xalign(0.0);
    restore_label.set_hexpand(true);
    restore_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    restore_label.add_css_class("dim-label");
    restore_row.append(&restore_load);
    restore_row.append(&restore_label);
    let include_memory = gtk::CheckButton::with_label("User memory");
    include_memory.set_active(true);
    let include_configuration = gtk::CheckButton::with_label("Configuration pages");
    let include_cc = gtk::CheckButton::with_label("Capability container");
    let include_locks = gtk::CheckButton::with_label("Lock bytes");
    let options = gtk::Box::new(Orientation::Horizontal, 12);
    options.append(&include_memory);
    options.append(&include_configuration);
    options.append(&include_cc);
    options.append(&include_locks);
    let restore_actions = gtk::Box::new(Orientation::Horizontal, 8);
    let restore = gtk::Button::with_label("Restore image…");
    restore.add_css_class("destructive-action");
    restore.set_sensitive(false);
    let restore_status = gtk::Label::new(Some(
        "Restore writes only the selected page classes after the typed confirmation.",
    ));
    restore_status.set_xalign(0.0);
    restore_status.set_wrap(true);
    restore_status.add_css_class("caption");
    restore_status.add_css_class("dim-label");
    restore_actions.append(&restore);
    restore_actions.append(&restore_status);
    restore_card.append(&restore_title);
    restore_card.append(&restore_row);
    restore_card.append(&options);
    restore_card.append(&restore_actions);
    memory.append(&restore_card);
    tabs.add_titled(&memory, Some("memory"), "Memory");

    let protection_root = gtk::Box::new(Orientation::Vertical, 10);
    let protection_scroll = gtk::ScrolledWindow::builder()
        .child(&protection_root)
        .build();
    protection_scroll.set_vexpand(true);
    protection_scroll.set_has_frame(false);

    let config_card = gtk::Box::new(Orientation::Vertical, 6);
    config_card.add_css_class("card");
    let config_title = gtk::Label::new(Some("Configuration"));
    config_title.set_xalign(0.0);
    config_title.add_css_class("title-4");
    let configuration = gtk::Label::new(Some("Read a Type 2 tag."));
    configuration.set_xalign(0.0);
    configuration.set_wrap(true);
    configuration.set_selectable(true);
    configuration.add_css_class("mono");
    config_card.append(&config_title);
    config_card.append(&configuration);

    let lock_card = gtk::Box::new(Orientation::Vertical, 6);
    lock_card.add_css_class("card");
    let lock_title = gtk::Label::new(Some("One-way locks"));
    lock_title.set_xalign(0.0);
    lock_title.add_css_class("title-4");
    let lock_row = gtk::Box::new(Orientation::Horizontal, 8);
    let lock_label = gtk::Label::new(Some("Pages"));
    let lock_pages = gtk::Entry::builder()
        .placeholder_text("4,8,16-20")
        .hexpand(true)
        .build();
    let lock_apply = gtk::Button::with_label("Apply lock bits…");
    lock_apply.add_css_class("destructive-action");
    lock_apply.set_sensitive(false);
    lock_row.append(&lock_label);
    lock_row.append(&lock_pages);
    lock_row.append(&lock_apply);
    let lock_preview = gtk::Label::new(Some("Enter pages to preview the lock bytes."));
    lock_preview.set_xalign(0.0);
    lock_preview.set_wrap(true);
    lock_preview.add_css_class("caption");
    lock_preview.add_css_class("dim-label");
    lock_card.append(&lock_title);
    lock_card.append(&lock_row);
    lock_card.append(&lock_preview);

    let password_card = gtk::Box::new(Orientation::Vertical, 6);
    password_card.add_css_class("card");
    let password_title = gtk::Label::new(Some("Password and access"));
    password_title.set_xalign(0.0);
    password_title.add_css_class("title-4");
    let auth0 = field_row("AUTH0", "255");
    let access = field_row("ACCESS", "00");
    let pwd = field_row("PWD", "8 hex digits");
    let pack = field_row("PACK", "4 hex digits");
    let password_apply = gtk::Button::with_label("Apply password configuration…");
    password_apply.add_css_class("destructive-action");
    password_apply.set_sensitive(false);
    let password_note = gtk::Label::new(Some(
        "PWD and PACK are write-only; the service verifies PACK with PWD_AUTH after the write.",
    ));
    password_note.set_xalign(0.0);
    password_note.set_wrap(true);
    password_note.add_css_class("caption");
    password_note.add_css_class("dim-label");
    password_card.append(&password_title);
    password_card.append(&auth0.0);
    password_card.append(&access.0);
    password_card.append(&pwd.0);
    password_card.append(&pack.0);
    password_card.append(&password_apply);
    password_card.append(&password_note);

    protection_root.append(&config_card);
    protection_root.append(&lock_card);
    protection_root.append(&password_card);
    tabs.add_titled(&protection_scroll, Some("protection"), "Protection");

    root.append(&header);
    root.append(&warnings);
    root.append(&switcher_row);
    root.append(&tabs);

    Type2View {
        root,
        tabs,
        advanced,
        summary,
        warnings,
        rows,
        selection,
        table_stack,
        editor,
        save,
        load,
        restore_load,
        restore_label,
        include_memory,
        include_configuration,
        include_cc,
        include_locks,
        restore,
        restore_status,
        configuration,
        lock_pages,
        lock_preview,
        lock_apply,
        auth0: auth0.1,
        access: access.1,
        pwd: pwd.1,
        pack: pack.1,
        password_apply,
        password_note,
    }
}

fn field_row(label: &str, placeholder: &str) -> (gtk::Box, gtk::Entry) {
    let row = gtk::Box::new(Orientation::Horizontal, 8);
    let name = gtk::Label::new(Some(label));
    name.set_xalign(0.0);
    name.set_width_chars(10);
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .hexpand(true)
        .build();
    row.append(&name);
    row.append(&entry);
    (row, entry)
}

fn build_type2_editor() -> Type2Editor {
    let root = gtk::Box::new(Orientation::Vertical, 8);
    root.add_css_class("card");
    root.set_width_request(340);
    root.set_hexpand(false);
    root.set_margin_start(4);

    let kicker = gtk::Label::new(Some("PAGE INSPECTOR"));
    kicker.set_xalign(0.0);
    kicker.add_css_class("rail-title");
    let title = gtk::Label::new(Some("No page"));
    title.set_xalign(0.0);
    title.add_css_class("block-title");
    let meta = gtk::Label::new(Some("Read a Type 2 tag to inspect a page."));
    meta.set_xalign(0.0);
    meta.set_wrap(true);
    meta.add_css_class("caption");
    meta.add_css_class("dim-label");
    let details = gtk::Label::new(None);
    details.set_xalign(0.0);
    details.set_wrap(true);
    details.set_selectable(true);
    details.add_css_class("caption");
    let hex_label = gtk::Label::new(Some("Data"));
    hex_label.set_xalign(0.0);
    hex_label.add_css_class("rail-title");
    let hex = gtk::Entry::builder()
        .placeholder_text("8 hex digits")
        .build();
    hex.set_sensitive(false);
    let ascii = gtk::Label::new(None);
    ascii.set_xalign(0.0);
    ascii.add_css_class("mono");
    let feedback = gtk::Label::new(None);
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    feedback.add_css_class("caption");
    feedback.add_css_class("dim-label");
    let write = gtk::Button::with_label("Write page…");
    write.add_css_class("suggested-action");
    write.set_sensitive(false);
    let note = gtk::Label::new(Some(
        "User pages are backed up and read back. Advanced pages need Advanced mode and the typed confirmation.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    root.append(&kicker);
    root.append(&title);
    root.append(&meta);
    root.append(&details);
    root.append(&hex_label);
    root.append(&hex);
    root.append(&ascii);
    root.append(&feedback);
    root.append(&write);
    root.append(&note);

    Type2Editor {
        root,
        title,
        meta,
        details,
        hex,
        ascii,
        feedback,
        write,
    }
}

fn build_keys_view() -> KeysView {
    let root = page_box();
    let title = gtk::Label::new(Some("Sector keys"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "The JSON store keeps sector, A/B, and provenance. Proxmark3 .dic export is lossy: \
         unique uppercase keys in candidate order, with imported keys applying to every sector as any.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<KeyRow>("Sector", 120, false, |row| {
        row.sector.clone()
    }));
    table.append_column(&text_column::<KeyRow>("Type", 70, false, |row| {
        row.kind.to_owned()
    }));
    table.append_column(&text_column::<KeyRow>("Key", 160, true, |row| {
        row.key.clone()
    }));
    table.append_column(&text_column::<KeyRow>("Source", 100, false, |row| {
        row.source.to_owned()
    }));
    let scroll = gtk::ScrolledWindow::builder()
        .child(&table)
        .vexpand(true)
        .build();
    scroll.set_has_frame(true);
    scroll.set_overlay_scrolling(false);

    let hex = gtk::Entry::new();
    hex.set_placeholder_text(Some("12 hex digits"));
    hex.add_css_class("mono");
    hex.set_width_chars(14);
    let sector = gtk::Entry::new();
    sector.set_placeholder_text(Some("all sectors"));
    sector.set_width_chars(10);
    let kind = gtk::DropDown::from_strings(&["any", "A", "B"]);
    let add = gtk::Button::with_label("Add key");
    add.add_css_class("suggested-action");
    let add_row = gtk::Box::new(Orientation::Horizontal, 6);
    add_row.append(&hex);
    add_row.append(&sector);
    add_row.append(&kind);
    add_row.append(&add);

    let import = gtk::Button::with_label("Import .dic / JSON…");
    let save = gtk::Button::with_label("Save store");
    let export = gtk::Button::with_label("Export .dic…");
    let actions = gtk::Box::new(Orientation::Horizontal, 6);
    actions.append(&import);
    actions.append(&save);
    actions.append(&export);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("caption");
    status.add_css_class("dim-label");

    root.append(&title);
    root.append(&note);
    root.append(&scroll);
    root.append(&add_row);
    root.append(&actions);
    root.append(&status);

    KeysView {
        root,
        rows,
        status,
        hex,
        sector,
        kind,
        add,
        import,
        save,
        export,
    }
}

fn build_diff_view() -> DiffView {
    let root = page_box();
    let title = gtk::Label::new(Some("Dump diff"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Compare two images without a reader. A block with unknown bytes is classified unknown, never equal.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let before_label = gtk::Label::new(Some("Before: no image loaded"));
    before_label.set_xalign(0.0);
    before_label.set_wrap(true);
    let after_label = gtk::Label::new(Some("After: no image loaded"));
    after_label.set_xalign(0.0);
    after_label.set_wrap(true);
    let before = gtk::Button::with_label("Load before…");
    let after = gtk::Button::with_label("Load after…");
    let compare = gtk::Button::with_label("Compare");
    compare.add_css_class("suggested-action");
    compare.set_sensitive(false);
    let files = gtk::Box::new(Orientation::Vertical, 6);
    let before_row = gtk::Box::new(Orientation::Horizontal, 6);
    before_row.append(&before);
    before_row.append(&before_label);
    before_label.set_hexpand(true);
    let after_row = gtk::Box::new(Orientation::Horizontal, 6);
    after_row.append(&after);
    after_row.append(&after_label);
    after_label.set_hexpand(true);
    files.append(&before_row);
    files.append(&after_row);

    let summary = gtk::Label::new(Some("Load both images, then compare."));
    summary.set_xalign(0.0);
    summary.set_wrap(true);

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<classic::DiffRow>(
        "Block",
        60,
        false,
        |row| format!("{:03}", row.number),
    ));
    table.append_column(&text_column::<classic::DiffRow>(
        "Sector",
        54,
        false,
        |row| format!("{:02}", row.sector),
    ));
    table.append_column(&text_column::<classic::DiffRow>("Type", 42, false, |row| {
        row.kind.to_owned()
    }));
    table.append_column(&text_column::<classic::DiffRow>(
        "State",
        66,
        false,
        |row| row.state.to_owned(),
    ));
    let detail = text_column::<classic::DiffRow>("Detail", 420, true, |row| row.detail.clone());
    detail.set_expand(true);
    table.append_column(&detail);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&table)
        .vexpand(true)
        .build();
    scroll.set_has_frame(true);
    scroll.set_overlay_scrolling(false);

    root.append(&title);
    root.append(&note);
    root.append(&files);
    root.append(&compare);
    root.append(&summary);
    root.append(&scroll);

    DiffView {
        root,
        before,
        after,
        before_label,
        after_label,
        compare,
        summary,
        rows,
    }
}

fn build_restore_view() -> RestoreView {
    let root = page_box();
    let title = gtk::Label::new(Some("Restore a Classic dump"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let load = gtk::Button::with_label("Load dump…");
    let file = gtk::Label::new(Some("No image loaded"));
    file.set_xalign(0.0);
    file.set_wrap(true);
    file.set_hexpand(true);
    let file_row = gtk::Box::new(Orientation::Horizontal, 6);
    file_row.append(&load);
    file_row.append(&file);

    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.add_css_class("caption");
    summary.add_css_class("dim-label");

    let continue_on_error = gtk::CheckButton::with_label("Continue after a failed block");
    continue_on_error.set_active(false);
    let restore = gtk::Button::with_label("Restore to card");
    restore.add_css_class("destructive-action");
    restore.set_sensitive(false);
    let note = gtk::Label::new(Some(
        "Restore backs up the current card first, writes data blocks before sector trailers, \
         and reads every block back. Block 0 is never written. A card with a different identity \
         from the image is refused before the backup.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let result = gtk::Label::new(None);
    result.set_xalign(0.0);
    result.set_wrap(true);
    result.set_selectable(true);

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<RestoreRow>("Block", 60, false, |row| {
        row.block.clone()
    }));
    table.append_column(&text_column::<RestoreRow>("Result", 90, false, |row| {
        row.state.to_owned()
    }));
    let detail = text_column::<RestoreRow>("Detail", 360, true, |row| row.detail.clone());
    detail.set_expand(true);
    table.append_column(&detail);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&table)
        .vexpand(true)
        .build();
    scroll.set_has_frame(true);
    scroll.set_overlay_scrolling(false);

    root.append(&title);
    root.append(&file_row);
    root.append(&summary);
    root.append(&continue_on_error);
    root.append(&restore);
    root.append(&note);
    root.append(&result);
    root.append(&scroll);

    RestoreView {
        root,
        load,
        file,
        summary,
        continue_on_error,
        restore,
        rows,
        result,
    }
}

fn build_keyscan_view() -> KeyscanView {
    let root = page_box();
    let title = gtk::Label::new(Some("Key scan"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Authenticate candidate keys per sector and verify a hit by reading the sector trailer. \
         The scan never writes the card. Discovered keys are saved only when you choose to merge them.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let common = gtk::CheckButton::with_label("Include common keys");
    common.set_active(true);
    let key_types = gtk::DropDown::from_strings(&["A and B", "A only", "B only"]);
    let dictionary = gtk::Entry::new();
    dictionary.set_placeholder_text(Some("Optional Proxmark3 .dic dictionary"));
    dictionary.set_hexpand(true);
    let choose_dictionary = gtk::Button::with_label("Choose…");
    let dictionary_row = gtk::Box::new(Orientation::Horizontal, 6);
    dictionary_row.append(&dictionary);
    dictionary_row.append(&choose_dictionary);
    let options = gtk::Box::new(Orientation::Horizontal, 8);
    options.append(&common);
    options.append(&key_types);

    let start = gtk::Button::with_label("Start scan");
    start.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Cancel");
    let actions = gtk::Box::new(Orientation::Horizontal, 6);
    actions.append(&start);
    actions.append(&cancel);

    let progress = gtk::Label::new(Some("No scan running."));
    progress.set_xalign(0.0);
    progress.set_wrap(true);
    progress.add_css_class("caption");
    progress.add_css_class("dim-label");
    let summary = gtk::Label::new(Some("Scan a MIFARE Classic card to list per-sector keys."));
    summary.set_xalign(0.0);
    summary.set_wrap(true);

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<SectorKeyRow>("Sector", 70, false, |row| {
        row.sector.clone()
    }));
    table.append_column(&text_column::<SectorKeyRow>("Key A", 180, true, |row| {
        row.key_a.clone()
    }));
    table.append_column(&text_column::<SectorKeyRow>("Key B", 180, true, |row| {
        row.key_b.clone()
    }));
    let table_scroll = gtk::ScrolledWindow::builder()
        .child(&table)
        .vexpand(true)
        .build();
    table_scroll.set_has_frame(true);
    table_scroll.set_overlay_scrolling(false);
    let empty = gtk::Label::new(Some(
        "No scan result yet.\nChoose candidates and start a scan.",
    ));
    empty.set_justify(gtk::Justification::Center);
    empty.add_css_class("dim-label");
    let table_stack = gtk::Stack::new();
    table_stack.set_vexpand(true);
    table_stack.add_named(&empty, Some("empty"));
    table_stack.add_named(&table_scroll, Some("table"));
    table_stack.set_visible_child_name("empty");

    let save = gtk::Button::with_label("Save discovered keys");
    save.add_css_class("suggested-action");
    let export = gtk::Button::with_label("Export .dic…");
    let result_actions = gtk::Box::new(Orientation::Horizontal, 6);
    result_actions.append(&save);
    result_actions.append(&export);

    root.append(&title);
    root.append(&note);
    root.append(&options);
    root.append(&dictionary_row);
    root.append(&actions);
    root.append(&progress);
    root.append(&summary);
    root.append(&table_stack);
    root.append(&result_actions);

    KeyscanView {
        root,
        common,
        key_types,
        dictionary,
        choose_dictionary,
        start,
        cancel,
        progress,
        summary,
        save,
        export,
        rows,
        table_stack,
    }
}

const RECOVERY_STRATEGIES: [RecoveryStrategy; 6] = [
    RecoveryStrategy::Auto,
    RecoveryStrategy::DarkSide,
    RecoveryStrategy::Nested,
    RecoveryStrategy::StaticNested,
    RecoveryStrategy::HardNested,
    RecoveryStrategy::Fm11rf08s,
];

fn recovery_strategy(index: u32) -> RecoveryStrategy {
    RECOVERY_STRATEGIES
        .get(index as usize)
        .copied()
        .unwrap_or(RecoveryStrategy::Auto)
}

fn recovery_strategy_label(strategy: RecoveryStrategy) -> &'static str {
    match strategy {
        RecoveryStrategy::Auto => "Auto",
        RecoveryStrategy::Dictionary => "Dictionary",
        RecoveryStrategy::DarkSide => "Darkside",
        RecoveryStrategy::Nested => "Nested",
        RecoveryStrategy::StaticNested => "Static nested",
        RecoveryStrategy::HardNested => "Hardnested",
        RecoveryStrategy::Fm11rf08s => "FM11RF08S",
    }
}

fn build_recovery_view(preferences: &UiPreferences) -> RecoveryView {
    let root = page_box();
    let title = gtk::Label::new(Some("Recovery"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Native Crypto1 recovery is read/authenticate-only. A recovered key is accepted only after \
         the service authenticates it again. Cancelling stops before the next reader exchange.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let strategy = gtk::DropDown::from_strings(&[
        "Auto",
        "Darkside",
        "Nested",
        "Static nested",
        "Hardnested",
        "FM11RF08S",
    ]);
    let known_block = gtk::Entry::new();
    known_block.set_text("4");
    known_block.set_width_chars(6);
    let known_type = gtk::DropDown::from_strings(&["A", "B"]);
    let known_controls = gtk::Box::new(Orientation::Horizontal, 6);
    known_controls.append(&known_block);
    known_controls.append(&known_type);
    let known_row = setting_row("Known block / type", &known_controls);

    let target_block = gtk::Entry::new();
    target_block.set_text("0");
    target_block.set_width_chars(6);
    let target_type = gtk::DropDown::from_strings(&["A", "B"]);
    let target_controls = gtk::Box::new(Orientation::Horizontal, 6);
    target_controls.append(&target_block);
    target_controls.append(&target_type);
    let target_row = setting_row("Target block / type", &target_controls);

    let tables = gtk::Entry::new();
    if let Some(path) = &preferences.hardnested_tables {
        tables.set_text(&path.display().to_string());
    }
    tables.set_placeholder_text(Some("Installed table directory"));
    tables.set_hexpand(true);
    let choose_tables = gtk::Button::with_label("Choose…");
    let tables_controls = gtk::Box::new(Orientation::Horizontal, 6);
    tables_controls.append(&tables);
    tables_controls.append(&choose_tables);
    let tables_row = setting_row("Tables directory", &tables_controls);

    let max_nonces = gtk::SpinButton::with_range(0.0, 100_000.0, 100.0);
    max_nonces.set_value(0.0);
    max_nonces.set_tooltip_text(Some(
        "Hardnested acquisition cap; zero uses the engine default (4000 frames).",
    ));
    let common = gtk::CheckButton::with_label("Include common keys");
    common.set_active(preferences.use_common_keys);

    let advanced_form = gtk::Box::new(Orientation::Vertical, 6);
    advanced_form.append(&known_row);
    advanced_form.append(&target_row);
    advanced_form.append(&tables_row);
    advanced_form.append(&setting_row("Max nonces", &max_nonces));
    let advanced = gtk::Expander::new(Some("Advanced strategy options"));
    advanced.set_expanded(false);
    advanced.set_child(Some(&advanced_form));

    let start = gtk::Button::with_label("Start recovery");
    start.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Cancel");
    let elapsed = gtk::Label::new(Some("Elapsed —"));
    elapsed.set_xalign(1.0);
    elapsed.set_hexpand(true);
    elapsed.add_css_class("caption");
    elapsed.add_css_class("dim-label");
    let actions = gtk::Box::new(Orientation::Horizontal, 6);
    actions.append(&start);
    actions.append(&cancel);
    actions.append(&elapsed);

    let log_title = gtk::Label::new(Some("Recovery log"));
    log_title.set_xalign(0.0);
    log_title.add_css_class("caption");
    log_title.add_css_class("dim-label");
    let log = RecoveryLog::new();

    let save = gtk::Button::with_label("Save verified keys");
    save.add_css_class("suggested-action");
    save.set_halign(gtk::Align::Start);

    let form = gtk::Box::new(Orientation::Vertical, 6);
    form.append(&setting_row("Strategy", &strategy));
    form.append(&advanced);
    form.append(&common);

    root.append(&title);
    root.append(&note);
    root.append(&form);
    root.append(&actions);
    root.append(&log_title);
    root.append(&log.scroller);
    root.append(&save);

    RecoveryView {
        root,
        strategy,
        advanced,
        known_row,
        target_row,
        known_block,
        known_type,
        target_block,
        target_type,
        tables,
        choose_tables,
        max_nonces,
        common,
        start,
        cancel,
        elapsed,
        log,
        save,
    }
}

fn build_diagnostics_view() -> DiagnosticsView {
    let root = page_box();
    let title = gtk::Label::new(Some("Diagnostics"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Build and resource facts for this installation. Diagnostics reads the filesystem and \
         enumerates readers; it never reads a card and never reports key material.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let version = diagnostic_label();
    let drivers = diagnostic_label();
    let tables = diagnostic_label();
    let readers = diagnostic_label();
    let build = diagnostic_label();
    let refresh = gtk::Button::with_label("Refresh readers");

    root.append(&title);
    root.append(&note);
    root.append(&version);
    root.append(&drivers);
    root.append(&tables);
    root.append(&readers);
    root.append(&build);
    root.append(&refresh);

    DiagnosticsView {
        root,
        version,
        drivers,
        tables,
        readers,
        build,
        refresh,
    }
}

fn diagnostic_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    label
}

fn build_activity_view() -> ActivityView {
    let root = page_box();
    let title = gtk::Label::new(Some("Activity"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Session-only record of operations, results, warnings, and backup paths. Key material and \
         NDEF payloads are not logged. Clearing the list does not affect stored files.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<ActivityRow>("Time", 80, false, |row| {
        row.time.clone()
    }));
    table.append_column(&text_column::<ActivityRow>(
        "Operation",
        120,
        false,
        |row| row.operation.clone(),
    ));
    let result = text_column::<ActivityRow>("Result", 220, true, |row| row.result.clone());
    result.set_expand(true);
    table.append_column(&result);
    let detail = text_column::<ActivityRow>("Detail", 320, true, |row| row.detail.clone());
    detail.set_expand(true);
    table.append_column(&detail);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&table)
        .vexpand(true)
        .build();
    scroll.set_has_frame(true);
    scroll.set_overlay_scrolling(false);

    let clear = gtk::Button::with_label("Clear activity");

    root.append(&title);
    root.append(&note);
    root.append(&scroll);
    root.append(&clear);

    ActivityView { root, rows, clear }
}

const DUMP_FORMAT_CHOICES: [(DumpFormat, &str); 5] = [
    (DumpFormat::Auto, "Automatic (raw or project)"),
    (DumpFormat::Raw, "Proxmark3 raw (.bin)"),
    (DumpFormat::Proxmark3, "Proxmark3 JSON (.mfc.json)"),
    (DumpFormat::Mct, "MCT text (.mct)"),
    (
        DumpFormat::Project,
        "NFC Foundry project (.nfc-foundry.json)",
    ),
];

fn dump_format_index(format: DumpFormat) -> u32 {
    DUMP_FORMAT_CHOICES
        .iter()
        .position(|(choice, _)| *choice == format)
        .unwrap_or(0) as u32
}

fn build_settings_view(preferences: &UiPreferences) -> SettingsView {
    let root = page_box();
    let title = gtk::Label::new(Some("Settings"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Preferences are written atomically with mode 0600 under the XDG configuration directory. \
         Advanced mode and recovery cancellation stay session-only.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let key_store = gtk::Entry::new();
    key_store.set_placeholder_text(Some("Default key store"));
    let key_store_choose = gtk::Button::with_label("Choose…");
    let key_store_row = gtk::Box::new(Orientation::Horizontal, 6);
    key_store_row.append(&key_store);
    key_store_row.append(&key_store_choose);

    let backup_dir = gtk::Entry::new();
    backup_dir.set_placeholder_text(Some("Default backup directory"));
    let backup_choose = gtk::Button::with_label("Choose…");
    let backup_row = gtk::Box::new(Orientation::Horizontal, 6);
    backup_row.append(&backup_dir);
    backup_row.append(&backup_choose);

    let tables = gtk::Entry::new();
    tables.set_placeholder_text(Some("Installed table directory"));
    let tables_choose = gtk::Button::with_label("Choose…");
    let tables_row = gtk::Box::new(Orientation::Horizontal, 6);
    tables_row.append(&tables);
    tables_row.append(&tables_choose);

    let common = gtk::Switch::new();
    common.set_active(preferences.use_common_keys);
    common.set_halign(gtk::Align::Start);
    let dump_format = gtk::DropDown::from_strings(&DUMP_FORMAT_CHOICES.map(|(_, label)| label));
    let language = gtk::DropDown::from_strings(&Language::ALL.map(language_label));
    language.set_selected(language_index(preferences.language()));

    let save = gtk::Button::with_label("Save preferences");
    save.add_css_class("suggested-action");
    let config_path = diagnostic_label();
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("caption");
    status.add_css_class("dim-label");

    root.append(&title);
    root.append(&note);
    root.append(&setting_row("Key store", &key_store_row));
    root.append(&setting_row("Backup directory", &backup_row));
    root.append(&setting_row("Hardnested tables", &tables_row));
    root.append(&setting_row("Use common keys", &common));
    root.append(&setting_row("Default dump format", &dump_format));
    root.append(&setting_row("Language", &language));
    root.append(&save);
    root.append(&config_path);
    root.append(&status);

    SettingsView {
        root,
        key_store,
        key_store_choose,
        backup_dir,
        backup_choose,
        tables,
        tables_choose,
        common,
        dump_format,
        language,
        save,
        config_path,
        status,
    }
}

fn language_index(language: Language) -> u32 {
    match language {
        Language::En => 0,
        Language::ZhCn => 1,
    }
}

/// Language names are self-labelled so they stay findable in either locale.
fn language_label(language: Language) -> &'static str {
    match language {
        Language::En => "English",
        Language::ZhCn => "简体中文",
    }
}

fn language_at(index: u32) -> Language {
    match index {
        1 => Language::ZhCn,
        _ => Language::En,
    }
}

fn setting_row(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_width_chars(18);
    row.append(&label);
    widget.set_hexpand(true);
    row.append(widget);
    row
}

fn connect_signals(ui: &Rc<Ui>) {
    let ui2 = ui.clone();
    ui.selection
        .connect_selected_notify(move |_| update_inspector(&ui2));

    let ui2 = ui.clone();
    ui.read_button
        .connect_clicked(move |_| start_read(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .read
        .connect_clicked(move |_| start_read_block(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .write
        .connect_clicked(move |_| confirm_write(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .hex
        .connect_changed(move |_| update_write_sensitivity(&ui2));

    let ui2 = ui.clone();
    ui.ndef_view
        .source
        .connect_selected_notify(move |dropdown| {
            let Some(source) = NdefSource::ALL.get(dropdown.selected() as usize) else {
                return;
            };
            {
                ui2.ndef_view.draft.borrow_mut().source = *source;
            }
            ui2.ndef_view
                .input_stack
                .set_visible_child_name(source.stack_name());
            update_ndef_actions(&ui2);
        });
    let ui2 = ui.clone();
    ui.ndef_view.language.connect_changed(move |entry| {
        ui2.ndef_view.draft.borrow_mut().language = entry.text().to_string();
        update_ndef_actions(&ui2);
    });
    let ui2 = ui.clone();
    let text_buffer = ui.ndef_view.text.buffer();
    text_buffer.connect_changed(move |buffer| {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        ui2.ndef_view.draft.borrow_mut().text = text.to_string();
        update_ndef_actions(&ui2);
    });
    let ui2 = ui.clone();
    ui.ndef_view.uri.connect_changed(move |entry| {
        ui2.ndef_view.draft.borrow_mut().uri = entry.text().to_string();
        update_ndef_actions(&ui2);
    });
    let ui2 = ui.clone();
    ui.ndef_view
        .raw_choose
        .connect_clicked(move |_| choose_ndef_raw(&ui2));
    let ui2 = ui.clone();
    ui.ndef_view
        .write
        .connect_clicked(move |_| confirm_ndef_write(ui2.clone()));
    let ui2 = ui.clone();
    ui.ndef_view
        .format
        .connect_clicked(move |_| confirm_ndef_format(ui2.clone()));
    let ui2 = ui.clone();
    ui.ndef_view
        .export
        .connect_clicked(move |_| export_ndef_message(&ui2));

    let ui2 = ui.clone();
    ui.type2_view
        .selection
        .connect_selected_notify(move |_| update_type2_inspector(&ui2));
    let ui2 = ui.clone();
    ui.type2_view.advanced.connect_active_notify(move |_| {
        update_type2_page_table(&ui2);
        update_type2_inspector(&ui2);
        update_type2_actions(&ui2);
        update_ndef_actions(&ui2);
    });
    let ui2 = ui.clone();
    ui.type2_view
        .editor
        .hex
        .connect_changed(move |_| update_type2_write_sensitivity(&ui2));
    let ui2 = ui.clone();
    ui.type2_view
        .editor
        .write
        .connect_clicked(move |_| confirm_type2_page_write(ui2.clone()));
    let ui2 = ui.clone();
    ui.type2_view
        .save
        .connect_clicked(move |_| save_type2_dump(&ui2));
    let ui2 = ui.clone();
    ui.type2_view.load.connect_clicked(move |_| load_dump(&ui2));
    let ui2 = ui.clone();
    ui.type2_view
        .restore_load
        .connect_clicked(move |_| choose_type2_restore(&ui2));
    for option in [
        &ui.type2_view.include_memory,
        &ui.type2_view.include_configuration,
        &ui.type2_view.include_cc,
        &ui.type2_view.include_locks,
    ] {
        let ui2 = ui.clone();
        option.connect_toggled(move |_| update_type2_restore_sensitivity(&ui2));
    }
    let ui2 = ui.clone();
    ui.type2_view
        .restore
        .connect_clicked(move |_| confirm_type2_restore(ui2.clone()));
    let ui2 = ui.clone();
    ui.type2_view
        .lock_pages
        .connect_changed(move |_| update_type2_actions(&ui2));
    let ui2 = ui.clone();
    ui.type2_view
        .lock_apply
        .connect_clicked(move |_| confirm_type2_lock(ui2.clone()));
    for entry in [
        &ui.type2_view.auth0,
        &ui.type2_view.access,
        &ui.type2_view.pwd,
        &ui.type2_view.pack,
    ] {
        let ui2 = ui.clone();
        entry.connect_changed(move |_| update_type2_actions(&ui2));
    }
    let ui2 = ui.clone();
    ui.type2_view
        .password_apply
        .connect_clicked(move |_| confirm_type2_password(ui2.clone()));

    let ui2 = ui.clone();
    ui.keys_view.add.connect_clicked(move |_| add_key(&ui2));
    let ui2 = ui.clone();
    ui.keys_view
        .import
        .connect_clicked(move |_| import_keys(&ui2));
    let ui2 = ui.clone();
    ui.keys_view
        .save
        .connect_clicked(move |_| save_keys(&ui2, true));
    let ui2 = ui.clone();
    ui.keys_view
        .export
        .connect_clicked(move |_| export_keys(&ui2));

    let ui2 = ui.clone();
    ui.keyscan_view
        .start
        .connect_clicked(move |_| start_keyscan(ui2.clone()));
    let ui2 = ui.clone();
    ui.keyscan_view
        .cancel
        .connect_clicked(move |_| cancel_keyscan(&ui2));
    let ui2 = ui.clone();
    ui.keyscan_view
        .choose_dictionary
        .connect_clicked(move |_| choose_keyscan_dictionary(&ui2));
    let ui2 = ui.clone();
    ui.keyscan_view
        .save
        .connect_clicked(move |_| save_keyscan_keys(&ui2));
    let ui2 = ui.clone();
    ui.keyscan_view
        .export
        .connect_clicked(move |_| export_keyscan_dic(&ui2));

    let ui2 = ui.clone();
    ui.recovery_view
        .strategy
        .connect_selected_notify(move |_| update_recovery_sensitivity(&ui2));
    let ui2 = ui.clone();
    ui.recovery_view
        .start
        .connect_clicked(move |_| start_recovery(ui2.clone()));
    let ui2 = ui.clone();
    ui.recovery_view
        .cancel
        .connect_clicked(move |_| cancel_recovery(&ui2));
    let ui2 = ui.clone();
    ui.recovery_view
        .choose_tables
        .connect_clicked(move |_| choose_recovery_tables(&ui2));
    let ui2 = ui.clone();
    ui.recovery_view
        .save
        .connect_clicked(move |_| save_recovered_key(&ui2));

    let ui2 = ui.clone();
    ui.diagnostics_view.refresh.connect_clicked(move |_| {
        refresh_devices(&ui2);
        render_diagnostics(&ui2);
    });

    let ui2 = ui.clone();
    ui.activity_view
        .clear
        .connect_clicked(move |_| clear_activity(&ui2));

    let ui2 = ui.clone();
    ui.settings_view
        .key_store_choose
        .connect_clicked(move |_| choose_settings_file(&ui2));
    let ui2 = ui.clone();
    ui.settings_view
        .backup_choose
        .connect_clicked(move |_| choose_settings_folder(&ui2, SettingsPath::Backup));
    let ui2 = ui.clone();
    ui.settings_view
        .tables_choose
        .connect_clicked(move |_| choose_settings_folder(&ui2, SettingsPath::Tables));
    let ui2 = ui.clone();
    ui.settings_view
        .language
        .connect_selected_notify(move |dropdown| {
            if ui2.language_updating.get() {
                return;
            }
            switch_language(&ui2, language_at(dropdown.selected()));
        });
    let ui2 = ui.clone();
    ui.settings_view
        .save
        .connect_clicked(move |_| save_settings(&ui2));

    let ui2 = ui.clone();
    ui.views.connect_visible_child_name_notify(move |stack| {
        if stack.visible_child_name().as_deref() == Some("diagnostics") {
            render_diagnostics(&ui2);
        }
    });

    let ui2 = ui.clone();
    ui.diff_view
        .compare
        .connect_clicked(move |_| compare_dumps(&ui2));
    let ui2 = ui.clone();
    ui.diff_view
        .before
        .connect_clicked(move |_| choose_diff_dump(&ui2, DiffSide::Before));
    let ui2 = ui.clone();
    ui.diff_view
        .after
        .connect_clicked(move |_| choose_diff_dump(&ui2, DiffSide::After));
    let ui2 = ui.clone();
    ui.restore_view
        .load
        .connect_clicked(move |_| load_restore_dump(&ui2));
    let ui2 = ui.clone();
    ui.restore_view
        .restore
        .connect_clicked(move |_| confirm_restore(&ui2));

    let ui2 = ui.clone();
    ui.reader_combo.connect_selected_notify(move |dropdown| {
        let index = dropdown.selected();
        if index == gtk::INVALID_LIST_POSITION {
            return;
        }
        let device = {
            let devices = ui2.devices.borrow();
            let Some(device) = devices.get(index as usize) else {
                return;
            };
            device.clone()
        };
        let id = device.conn_string.clone();
        let result = match ui2.service.lock() {
            Ok(mut service) => service.select_device(&id),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        match result {
            Ok(()) => {
                ui2.live.set(false);
                reset_advanced(&ui2);
                render_type2(&ui2);
                update_inspector(&ui2);
                update_card(&ui2);
                update_restore_sensitivity(&ui2);
                update_ndef_actions(&ui2);
                ui2.status
                    .set_localized("Reader changed · read the card again");
                ui2.reader_value
                    .set_localized(&format!("{}\n{}", device.name, device.conn_string));
                append_log(&ui2, &format!("reader selected · {id}"));
            }
            Err(error) => toast_error(&ui2, &error.to_string()),
        }
    });
}

fn build_actions_menu(ui: &Rc<Ui>, button: &gtk::MenuButton) {
    let popover = gtk::Popover::new();
    let menu = gtk::Box::new(Orientation::Vertical, 2);
    menu.set_margin_top(8);
    menu.set_margin_bottom(8);
    menu.set_margin_start(8);
    menu.set_margin_end(8);

    let refresh = menu_button("Refresh readers");
    {
        let ui = ui.clone();
        let popover = popover.clone();
        refresh.connect_clicked(move |_| {
            popover.popdown();
            refresh_devices(&ui);
        });
    }
    menu.append(&refresh);

    menu.append(&gtk::Separator::new(Orientation::Horizontal));

    let save = menu_button("Save dump…");
    {
        let ui = ui.clone();
        let popover = popover.clone();
        save.connect_clicked(move |_| {
            popover.popdown();
            save_dump(&ui);
        });
    }
    menu.append(&save);

    let load = menu_button("Load dump…");
    {
        let ui = ui.clone();
        let popover = popover.clone();
        load.connect_clicked(move |_| {
            popover.popdown();
            load_dump(&ui);
        });
    }
    menu.append(&load);

    popover.set_child(Some(&menu));
    button.set_popover(Some(&popover));
}

fn menu_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.set_halign(gtk::Align::Fill);
    button.add_css_class("flat");
    button
}

fn text_column<T: 'static>(
    title: &str,
    width: i32,
    mono: bool,
    getter: fn(&T) -> String,
) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
        let label = gtk::Label::builder().xalign(0.0).build();
        label.set_margin_start(6);
        label.set_margin_end(6);
        if mono {
            label.add_css_class("mono");
        }
        item.set_child(Some(&label));
    });
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
        let Some(object) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let value = object.borrow::<T>();
        let label = item
            .child()
            .and_downcast::<gtk::Label>()
            .expect("column label");
        label.set_localized(&getter(&value));
    });
    let column = gtk::ColumnViewColumn::new(Some(title), Some(factory));
    column.set_fixed_width(width);
    column.set_resizable(true);
    column
}

fn selected_row(ui: &Ui) -> Option<BlockRow> {
    ui.selection
        .selected_item()
        .and_downcast::<glib::BoxedAnyObject>()
        .map(|object| object.borrow::<BlockRow>().clone())
}

fn update_inspector(ui: &Ui) {
    let Some(row) = selected_row(ui) else {
        ui.inspector.title.set_localized("No block");
        ui.inspector
            .meta
            .set_localized("Read a card to inspect a block.");
        ui.inspector.error.set_visible(false);
        ui.inspector.details.set_localized("");
        ui.inspector.hex.set_text("");
        ui.inspector.hex.set_sensitive(false);
        ui.inspector.read.set_sensitive(false);
        ui.inspector.write.set_sensitive(false);
        ui.selected_label.set_localized("No block selected");
        return;
    };

    ui.inspector
        .title
        .set_localized(&format!("Block {:03}", row.number));
    let role = if row.trailer {
        "sector trailer"
    } else if row.number == 0 {
        "manufacturer"
    } else {
        "data block"
    };
    ui.inspector.meta.set_localized(&format!(
        "Sector {:02} · {role} · {}",
        row.sector, row.status
    ));
    match &row.error {
        Some(error) => {
            ui.inspector.error.set_localized(error);
            ui.inspector.error.set_visible(true);
        }
        None => ui.inspector.error.set_visible(false),
    }
    let insight = ui
        .dump
        .borrow()
        .as_ref()
        .and_then(|dump| classic::inspect_block(dump, row.number))
        .unwrap_or_default();
    let mut details = insight.details;
    for warning in &insight.warnings {
        if !details.is_empty() {
            details.push('\n');
        }
        details.push_str("Warning: ");
        details.push_str(warning);
    }
    ui.inspector.details.set_localized(&details);
    ui.inspector
        .hex
        .set_text(if row.complete { &row.data } else { "" });
    let can_edit = ui.live.get() && !ui.busy.get() && row.complete && row.number != 0;
    ui.inspector.hex.set_sensitive(can_edit);
    ui.inspector
        .read
        .set_sensitive(ui.live.get() && !ui.busy.get());
    ui.selected_label.set_localized(&format!(
        "Block {:03} · sector {:02}\n{} · {}",
        row.number, row.sector, role, row.status
    ));
    update_write_sensitivity(ui);
}

fn update_write_sensitivity(ui: &Ui) {
    let Some(row) = selected_row(ui) else {
        ui.inspector.write.set_sensitive(false);
        return;
    };
    if !ui.live.get() || ui.busy.get() || !row.complete || row.number == 0 {
        ui.inspector.write.set_sensitive(false);
        return;
    }
    let text = ui.inspector.hex.text();
    let Ok(data) = classic::parse_hex_16(text.as_str()) else {
        ui.inspector.write.set_sensitive(false);
        return;
    };
    let same = ui
        .dump
        .borrow()
        .as_ref()
        .and_then(|dump| dump.blocks.get(row.number))
        .is_some_and(|block| block.data == data);
    ui.inspector.write.set_sensitive(!same);
}

fn update_card(ui: &Ui) {
    let type2 = ui.type2.borrow();
    if let Some(session) = type2.as_ref() {
        let ndef = session
            .ndef()
            .map(|ndef| type2_ui::ndef_state_label(&ndef.state))
            .unwrap_or_else(|_| "invalid".to_owned());
        let live = if ui.live.get() {
            ""
        } else {
            "\nInspection only · read again to write"
        };
        ui.card_title
            .set_localized(session.image.layout.product.name());
        ui.card_detail.set_localized(&format!(
            "UID {}\n{} pages · CC {} · NDEF {}\n{}/{} pages readable{}",
            session.card.uid_hex(),
            session.image.layout.page_count(),
            type2_ui::hex_bytes(&session.image.cc.to_bytes()),
            ndef,
            session.image.known_pages(),
            session.image.layout.page_count(),
            live
        ));
        ui.detail.set_localized(&format!(
            "{} · {} pages",
            session.image.layout.product.name(),
            session.image.layout.page_count()
        ));
        return;
    }
    drop(type2);
    let dump = ui.dump.borrow();
    match dump.as_ref() {
        Some(dump) => {
            let layout = classic::display_layout(dump.layout);
            let readability = dump.readability();
            let uid = dump
                .card
                .as_ref()
                .map(|card| card.uid_hex())
                .unwrap_or_else(|| "—".to_owned());
            ui.card_title.set_localized(layout);
            let live = if ui.live.get() {
                ""
            } else {
                "\nInspection only · read again to write"
            };
            ui.card_detail.set_localized(&format!(
                "UID {uid}\n{} sectors · {} blocks\n{}/{} blocks readable{}",
                dump.layout.sector_count(),
                dump.layout.block_count(),
                readability.readable_blocks,
                readability.total_blocks,
                live
            ));
            ui.detail
                .set_localized(&format!("{layout} · {} blocks", dump.layout.block_count()));
        }
        None => {
            ui.card_title.set_localized("No card");
            ui.card_detail
                .set_localized("Select a reader and read a card.");
            ui.detail.set_localized("No card");
        }
    }
    drop(dump);
    ui.key_value.set_localized(&format!(
        "{} saved · {} common\nA + B candidates",
        ui.keys.borrow().len(),
        nfc_app::classic::COMMON_KEYS.len()
    ));
}

fn update_ndef(ui: &Ui) {
    let type2 = ui.type2.borrow().clone();
    if let Some(session) = type2 {
        ui.ndef.replace(None);
        render_type2_ndef(ui, &session);
        return;
    }
    let dump = match ui.dump.borrow().clone() {
        Some(dump) => dump,
        None => {
            ui.ndef.replace(None);
            render_ndef(ui, None, None);
            return;
        }
    };
    let result = match ui.service.lock() {
        Ok(service) => service.classic_ndef_inspect(&dump),
        Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
    };
    match result {
        Ok(info) => {
            ui.ndef.replace(Some(info.clone()));
            render_ndef(ui, Some(&info), None);
        }
        Err(error) => {
            ui.ndef.replace(None);
            render_ndef(ui, None, Some(&error.to_string()));
        }
    }
}

/// Renders NDEF facts supplied by the last read or service result. The composer
/// state is refreshed from those facts and the current draft.
fn render_ndef(ui: &Ui, info: Option<&ClassicNdefInfo>, error: Option<&str>) {
    let view = &ui.ndef_view;
    view.read_only.set_visible(true);
    while let Some(child) = view.info.first_child() {
        view.info.remove(&child);
    }

    if ui.dump.borrow().is_none() {
        view.state.set_localized("No card loaded");
        let hint = ndef_note("Read a card to inspect its MAD mapping and NDEF message.");
        view.info.append(&hint);
        update_ndef_actions(ui);
        return;
    }
    if let Some(error) = error {
        view.state.set_localized("NDEF unavailable");
        let label = ndef_note(error);
        label.add_css_class("error");
        view.info.append(&label);
        update_ndef_actions(ui);
        return;
    }
    let Some(info) = info else {
        view.state.set_localized("NDEF unavailable");
        let hint = ndef_note("The NDEF facts are unavailable.");
        view.info.append(&hint);
        update_ndef_actions(ui);
        return;
    };

    view.state.set_localized(match &info.ndef.state {
        ClassicNdefState::Unformatted => "Not NDEF formatted",
        ClassicNdefState::Empty => "NDEF formatted · empty message",
        ClassicNdefState::Message(message) => {
            if message.records().len() == 1 {
                "NDEF · 1 record"
            } else {
                "NDEF · multiple records"
            }
        }
    });

    if let Some(layout) = &info.layout {
        let sectors = layout
            .sectors
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let mapping = format!(
            "MAD sectors: {}\nMapping version {}.{}\n{}",
            if sectors.is_empty() {
                "none".to_owned()
            } else {
                sectors
            },
            layout.version.major,
            layout.version.minor,
            if layout.read_only {
                "read-only"
            } else if layout.writable {
                "read/write"
            } else {
                "not writable"
            }
        );
        let label = ndef_note(&mapping);
        label.add_css_class("caption");
        view.info.append(&label);
    } else {
        view.info.append(&ndef_note(
            "No MAD or NFC sector mapping was found on this card.",
        ));
    }

    match &info.ndef.state {
        ClassicNdefState::Unformatted => {
            view.info.append(&ndef_note(
                "Format the NDEF area before writing; Format creates the MAD and NFC sectors.",
            ));
        }
        ClassicNdefState::Empty => {
            view.info.append(&ndef_note(&format!(
                "Capacity {} bytes · message empty",
                info.ndef.capacity
            )));
        }
        ClassicNdefState::Message(message) => {
            let capacity = ndef_note(&format!(
                "Capacity {} bytes · message {} bytes",
                info.ndef.capacity, info.ndef.message_bytes
            ));
            capacity.add_css_class("caption");
            view.info.append(&capacity);
            for (index, record) in message.records().iter().enumerate() {
                view.info.append(&record_card(index, record));
            }
        }
    }

    for warning in &info.warnings {
        let label = ndef_note(warning);
        label.add_css_class("caption");
        view.info.append(&label);
    }
    update_ndef_actions(ui);
}

fn ndef_note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_localized(text);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class("dim-label");
    label
}

/// Button state and preview for the composer. The service keeps its own gates.
fn update_ndef_actions(ui: &Ui) {
    let type2 = ui.type2.borrow().clone();
    if let Some(session) = type2 {
        update_type2_composer(ui, &session);
        return;
    }
    let view = &ui.ndef_view;
    let info = ui.ndef.borrow();
    let info = info.as_ref();
    let draft = view.draft.borrow();
    let live = ui.live.get();
    let busy = ui.busy.get();
    let (has_dump, complete) = match ui.dump.borrow().as_ref() {
        Some(dump) => (true, dump.complete()),
        None => (false, false),
    };
    let encoded = draft
        .message()
        .map_err(|error| format!("Cannot encode the message: {error}"))
        .and_then(|message| {
            message
                .encode()
                .map(|bytes| (message, bytes))
                .map_err(|error| format!("Cannot encode the message: {error}"))
        });

    let area = info.and_then(|info| info.layout.as_ref());
    let area_ready = area
        .is_some_and(|layout| !layout.sectors.is_empty() && layout.writable && !layout.read_only);
    let state_ready = matches!(
        info.map(|info| &info.ndef.state),
        Some(ClassicNdefState::Empty | ClassicNdefState::Message(_))
    );
    let capacity = info.map(|info| info.ndef.capacity);

    let mut ready = false;
    let mut feedback_error = false;
    let feedback = if !live {
        "Read a live card to write or format.".to_owned()
    } else if busy {
        "An operation is running.".to_owned()
    } else if !has_dump {
        "Read a card to inspect its MAD mapping and NDEF message.".to_owned()
    } else if !complete {
        "The image is partial; read the whole card first.".to_owned()
    } else if info.is_none() {
        "NDEF state is unavailable.".to_owned()
    } else if !state_ready {
        "Not NDEF formatted · Format is required before writing.".to_owned()
    } else if !area_ready {
        feedback_error = true;
        if area.is_some_and(|layout| layout.read_only) {
            "The NDEF area is read-only; writing is disabled.".to_owned()
        } else {
            "The NFC mapping is not writable; format the card.".to_owned()
        }
    } else {
        match &encoded {
            Err(error) => {
                feedback_error = true;
                error.clone()
            }
            Ok((message, bytes)) => match capacity {
                Some(capacity) => match classic::ndef_capacity_error(bytes.len(), capacity) {
                    Some(error) => {
                        feedback_error = true;
                        error
                    }
                    None if info.is_some_and(|info| info.ndef.message() == Some(message)) => {
                        "The NDEF area already stores this message.".to_owned()
                    }
                    None => {
                        ready = true;
                        format!("Ready to write {} bytes.", bytes.len())
                    }
                },
                None => "Capacity is unavailable; read the card again.".to_owned(),
            },
        }
    };

    let mut lines = vec![draft.summary()];
    match &encoded {
        Ok((_, bytes)) => {
            lines.push(match capacity {
                Some(capacity) => {
                    format!(
                        "Message {} bytes · capacity {} bytes",
                        bytes.len(),
                        capacity
                    )
                }
                None => format!("Message {} bytes", bytes.len()),
            });
            if !bytes.is_empty() {
                lines.push(classic::hex_preview(bytes, 48));
            }
        }
        Err(error) => lines.push(error.clone()),
    }
    view.preview.set_localized(&lines.join("\n"));
    view.feedback.set_localized(&feedback);
    if feedback_error {
        view.feedback.add_css_class("error");
    } else {
        view.feedback.remove_css_class("error");
    }
    view.write.set_sensitive(ready);
    view.format.set_sensitive(live && !busy && complete);
    view.export
        .set_sensitive(info.and_then(|info| info.ndef.message()).is_some());
}

fn record_card(index: usize, record: &NdefRecord) -> gtk::Box {
    let card = gtk::Box::new(Orientation::Vertical, 4);
    card.add_css_class("card");
    let kicker = gtk::Label::new(None);
    kicker.set_localized(&format!("RECORD {}", index + 1));
    kicker.set_xalign(0.0);
    kicker.add_css_class("rail-title");
    let (kind, content) = record_summary(record);
    let title = gtk::Label::new(None);
    title.set_localized(&kind);
    title.set_xalign(0.0);
    title.add_css_class("title-4");
    let body = gtk::Label::new(Some(&content));
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_selectable(true);
    body.add_css_class("mono");
    card.append(&kicker);
    card.append(&title);
    card.append(&body);
    for (field, value) in wifi_fields(record) {
        let row = gtk::Box::new(Orientation::Horizontal, 6);
        let field = i18n::translate_lines(field);
        let text = gtk::Label::new(Some(&format!("{field}: {value}")));
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_selectable(true);
        text.set_hexpand(true);
        text.add_css_class("mono");
        let copy = gtk::Button::new();
        copy.set_localized("Copy");
        copy.add_css_class("flat");
        copy.set_valign(gtk::Align::Start);
        let value = value.clone();
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&value);
        });
        row.append(&text);
        row.append(&copy);
        card.append(&row);
    }
    card
}

fn record_summary(record: &NdefRecord) -> (String, String) {
    if record.tnf() == Tnf::WellKnown && record.record_type() == b"T" {
        return match TextRecord::from_record(record) {
            Ok(text) => (format!("Text · {}", text.language), text.text),
            Err(error) => ("Text (invalid)".to_owned(), error.to_string()),
        };
    }
    if record.tnf() == Tnf::WellKnown && record.record_type() == b"U" {
        return match UriRecord::from_record(record) {
            Ok(uri) => ("URI".to_owned(), uri.uri),
            Err(error) => ("URI (invalid)".to_owned(), error.to_string()),
        };
    }
    if record.tnf() == Tnf::Media && record.record_type() == b"application/vnd.wfa.wsc" {
        return match WifiRecord::from_record(record) {
            Ok(wifi) => (
                "Wi-Fi · WSC".to_owned(),
                format!(
                    "{} · {}",
                    wifi_authentication_label(wifi.authentication),
                    wifi_encryption_label(wifi.encryption)
                ),
            ),
            Err(error) => ("Wi-Fi · WSC (invalid)".to_owned(), error.to_string()),
        };
    }
    let kind = match record.tnf() {
        Tnf::Empty => "Empty".to_owned(),
        Tnf::WellKnown => format!(
            "Well-known · {}",
            String::from_utf8_lossy(record.record_type())
        ),
        Tnf::Media => format!("Media · {}", String::from_utf8_lossy(record.record_type())),
        Tnf::AbsoluteUri => format!(
            "Absolute URI · {}",
            String::from_utf8_lossy(record.record_type())
        ),
        Tnf::External => format!(
            "External · {}",
            String::from_utf8_lossy(record.record_type())
        ),
        Tnf::Unknown => "Unknown".to_owned(),
        Tnf::Unchanged => "Chunk".to_owned(),
    };
    let payload = record.payload();
    let text = match std::str::from_utf8(payload) {
        Ok(value)
            if !value.chars().any(|character| {
                character.is_control() && character != '\n' && character != '\t'
            }) =>
        {
            let preview: String = value.chars().take(256).collect();
            let suffix = if value.chars().count() > 256 {
                "…"
            } else {
                ""
            };
            format!("{preview}{suffix}")
        }
        _ => classic::hex_preview(payload, 64),
    };
    (kind, text)
}

fn wifi_fields(record: &NdefRecord) -> Vec<(&'static str, String)> {
    if record.tnf() != Tnf::Media || record.record_type() != b"application/vnd.wfa.wsc" {
        return Vec::new();
    }
    let Ok(wifi) = WifiRecord::from_record(record) else {
        return Vec::new();
    };
    let mut fields = vec![("SSID", wsc_text(&wifi.ssid))];
    if let Some(key) = &wifi.network_key {
        fields.push(("Password", wsc_text(key)));
    }
    fields
}

fn wsc_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn wifi_authentication_label(value: WifiAuthentication) -> String {
    let mut parts = Vec::new();
    if value.contains(WifiAuthentication::OPEN) {
        parts.push("Open");
    }
    if value.contains(WifiAuthentication::WPA_PERSONAL) {
        parts.push("WPA-Personal");
    }
    if value.contains(WifiAuthentication::WPA2_PERSONAL) {
        parts.push("WPA2-Personal");
    }
    if value.contains(WifiAuthentication::SHARED) {
        parts.push("Shared");
    }
    if value.contains(WifiAuthentication::WPA_ENTERPRISE) {
        parts.push("WPA-Enterprise");
    }
    if value.contains(WifiAuthentication::WPA2_ENTERPRISE) {
        parts.push("WPA2-Enterprise");
    }
    if parts.is_empty() {
        format!("0x{:04X}", value.bits())
    } else {
        parts.join(" · ")
    }
}

fn wifi_encryption_label(value: WifiEncryption) -> String {
    let mut parts = Vec::new();
    if value.contains(WifiEncryption::NONE) {
        parts.push("None");
    }
    if value.contains(WifiEncryption::WEP) {
        parts.push("WEP");
    }
    if value.contains(WifiEncryption::TKIP) {
        parts.push("TKIP");
    }
    if value.contains(WifiEncryption::AES) {
        parts.push("AES");
    }
    if parts.is_empty() {
        format!("0x{:04X}", value.bits())
    } else {
        parts.join(" · ")
    }
}

fn refresh_devices(ui: &Rc<Ui>) {
    let devices = match ui.service.lock() {
        Ok(mut service) => match service.refresh_devices() {
            Ok(()) => service.devices().to_vec(),
            Err(error) => {
                toast_error(ui, &error.to_string());
                return;
            }
        },
        Err(_) => {
            toast_error(ui, "Reader service lock was poisoned");
            return;
        }
    };

    ui.live.set(false);
    reset_advanced(ui);
    render_type2(ui);
    update_inspector(ui);
    update_card(ui);
    update_restore_sensitivity(ui);
    update_ndef_actions(ui);
    if ui.dump.borrow().is_some() || ui.type2.borrow().is_some() {
        ui.status
            .set_localized("Inspection only · read the card again");
    }
    ui.devices.replace(devices.clone());
    let labels = devices
        .iter()
        .map(|device| format!("{} · {}", device.name, device.conn_string))
        .collect::<Vec<_>>();
    let labels = labels.iter().map(String::as_str).collect::<Vec<_>>();
    ui.reader_model
        .splice(0, ui.reader_model.n_items(), &labels);
    if devices.is_empty() {
        ui.reader_value.set_localized("No supported reader found");
        ui.reader_combo.set_sensitive(false);
        append_log(ui, "refresh · no supported reader");
        ui.status.set_localized("No reader found");
        render_diagnostics(ui);
        return;
    }

    ui.reader_combo.set_sensitive(!ui.busy.get());
    ui.reader_combo.set_selected(0);
    let first = &devices[0];
    append_log(
        ui,
        &format!(
            "refresh · {} reader(s) · {}",
            devices.len(),
            first.conn_string
        ),
    );
    render_diagnostics(ui);
}

fn start_read(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    reset_advanced(&ui);
    set_busy(&ui, true);
    ui.status.set_localized("Reading card…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let include_common_keys = ui.preferences.borrow().use_common_keys;
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.read_card(&keys, include_common_keys, CancelToken::new()),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_read(&ui, result),
            Err(_) => finish_read(&ui, Err(NfcError::Io("reader worker stopped".to_owned()))),
        }
    });
}

fn finish_read(ui: &Rc<Ui>, result: NfcResult<CardReadResult>) {
    set_busy(ui, false);
    match result {
        Ok(CardReadResult::Classic(read)) => {
            let warnings = read.warnings;
            let dump = read.dump;
            ui.type2.replace(None);
            ui.type2_restore.replace(None);
            ui.live.set(true);
            ui.dump.replace(Some(dump.clone()));
            populate_rows(ui, &dump);
            ui.views.set_visible_child_name("card");
            update_card(ui);
            update_ndef(ui);
            update_restore_sensitivity(ui);
            render_type2(ui);
            let readable = dump.readability().readable_blocks;
            ui.status
                .set_localized(&format!("Read complete · {readable} blocks"));
            append_log(
                ui,
                &format!(
                    "read · {} · {}/{} blocks",
                    classic::display_layout(dump.layout),
                    readable,
                    dump.layout.block_count()
                ),
            );
            for warning in &warnings {
                append_log(ui, &format!("warning · {warning}"));
            }
            if warnings.is_empty() {
                toast_info(ui, "Card read complete");
            } else {
                toast_info(
                    ui,
                    &format!(
                        "Card read with {} warning(s); see the activity log",
                        warnings.len()
                    ),
                );
            }
        }
        Ok(CardReadResult::Type2(read)) => {
            let session = Type2Session::new(read.card, read.image, read.warnings);
            let known = session.image.known_pages();
            let total = session.image.layout.page_count();
            let product = session.image.layout.product.name();
            let ndef = session
                .ndef()
                .map(|ndef| type2_ui::ndef_state_label(&ndef.state))
                .unwrap_or_else(|error| error.to_owned());
            let warnings = session.warnings.clone();
            show_type2_session(ui, session, true);
            ui.status
                .set_localized(&format!("Read complete · {known}/{total} pages"));
            append_log(
                ui,
                &format!("read · {product} · {known}/{total} pages · NDEF {ndef}"),
            );
            for warning in &warnings {
                append_log(ui, &format!("warning · {warning}"));
            }
            if warnings.is_empty() {
                toast_info(ui, "Type 2 tag read complete");
            } else {
                toast_info(
                    ui,
                    &format!(
                        "Type 2 read with {} warning(s); see the activity log",
                        warnings.len()
                    ),
                );
            }
        }
        Ok(_) => {
            ui.status.set_localized("Read failed");
            toast_error(ui, "unsupported read result");
        }
        Err(error) => {
            ui.live.set(false);
            update_inspector(ui);
            update_type2_inspector(ui);
            update_card(ui);
            update_ndef_actions(ui);
            update_type2_actions(ui);
            ui.status.set_localized("Read failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn populate_rows(ui: &Ui, dump: &Dump) {
    ui.rows.remove_all();
    for row in classic::rows_for(dump) {
        ui.rows.append(&glib::BoxedAnyObject::new(row));
    }
    if dump.blocks.is_empty() {
        ui.table_stack.set_visible_child_name("empty");
    } else {
        ui.table_stack.set_visible_child_name("table");
        ui.selection.set_selected(0);
    }
}

fn replace_row(ui: &Ui, block: usize) {
    let row = {
        let dump = ui.dump.borrow();
        dump.as_ref().and_then(|dump| {
            dump.blocks
                .get(block)
                .map(|block| BlockRow::from_block(block, dump.layout))
        })
    };
    if let Some(row) = row {
        ui.rows
            .splice(block as u32, 1, &[glib::BoxedAnyObject::new(row)]);
    }
}

fn start_read_block(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        return;
    }
    let Some(row) = selected_row(&ui) else {
        return;
    };
    let block = row.number;
    set_busy(&ui, true);
    ui.status
        .set_localized(&format!("Reading block {block:03}…"));
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.classic_read_block(block, &keys, true, CancelToken::new()),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_read_block(&ui, block, result),
            Err(_) => finish_read_block(
                &ui,
                block,
                Err(NfcError::Io("reader worker stopped".to_owned())),
            ),
        }
    });
}

fn finish_read_block(ui: &Rc<Ui>, block: usize, result: NfcResult<[u8; 16]>) {
    set_busy(ui, false);
    match result {
        Ok(data) => {
            {
                let mut dump = ui.dump.borrow_mut();
                if let Some(dump) = dump.as_mut()
                    && let Some(block) = dump.blocks.get_mut(block)
                {
                    block.set_read(data);
                }
            }
            replace_row(ui, block);
            update_inspector(ui);
            update_card(ui);
            ui.status.set_localized(&format!("Block {block:03} read"));
            append_log(ui, &format!("read block · {block:03}"));
        }
        Err(error) => {
            ui.status.set_localized("Block read failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn confirm_write(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        return;
    }
    let Some(row) = selected_row(&ui) else {
        return;
    };
    if row.number == 0 {
        toast_error(&ui, "Block 0 is read-only");
        return;
    }
    let text = ui.inspector.hex.text();
    let data = match classic::parse_hex_16(text.as_str()) {
        Ok(data) => data,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let current = {
        let dump = ui.dump.borrow();
        dump.as_ref()
            .and_then(|dump| dump.blocks.get(row.number))
            .map(|block| block.data)
    };
    let Some(current) = current else {
        toast_error(&ui, "The selected block is not in the current dump");
        return;
    };
    if current == data {
        toast_info(&ui, "The block already contains these bytes");
        return;
    }

    let heading = format!("Write block {:03}?", row.number);
    let role = if row.trailer {
        "sector trailer"
    } else {
        "data block"
    };
    let body = format!(
        "Sector {:02} · {role}\n\nOld  {}\nNew  {}\n\nA project backup is written before the write, and the block is read back afterwards.",
        row.sector,
        classic::hex_with_mask(&current, u16::MAX),
        classic::hex_with_mask(&data, u16::MAX)
    );
    let dialog = adw::AlertDialog::builder()
        .heading(&heading)
        .body(&body)
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(
        &dialog,
        "write",
        if row.trailer {
            "Write trailer"
        } else {
            "Write block"
        },
    );
    dialog.set_response_appearance(
        "write",
        if row.trailer {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let trailer = row.trailer;
    let block = row.number;
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_write_dialog.replace(None);
        if response == "write" {
            start_write(ui2.clone(), block, data, trailer);
        }
    });
    ui.pending_write_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_write(ui: Rc<Ui>, block: usize, data: [u8; 16], trailer: bool) {
    set_busy(&ui, true);
    ui.status
        .set_localized(&format!("Writing block {block:03}…"));
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.borrow().clone();
    let expected_card = ui.dump.borrow().as_ref().and_then(|dump| dump.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = ClassicBlockWrite::new(block, data, &keys, &backup);
                if trailer {
                    request = request.confirm_trailer(true);
                }
                if let Some(card) = expected_card {
                    request = request.expected_card(card);
                }
                service.classic_write_block(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_write(&ui, block, data, result),
            Err(_) => finish_write(
                &ui,
                block,
                data,
                Err(NfcError::Io("reader worker stopped".to_owned())),
            ),
        }
    });
}

fn finish_write(ui: &Rc<Ui>, block: usize, data: [u8; 16], result: NfcResult<ClassicWriteResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            if result.changed {
                {
                    let mut dump = ui.dump.borrow_mut();
                    if let Some(dump) = dump.as_mut()
                        && let Some(block) = dump.blocks.get_mut(block)
                    {
                        block.set_read(data);
                    }
                }
                replace_row(ui, block);
                update_inspector(ui);
                update_card(ui);
                update_ndef(ui);
                let backup = result
                    .backup
                    .as_ref()
                    .map(|backup| backup.path.display().to_string())
                    .unwrap_or_else(|| "no backup".to_owned());
                ui.status
                    .set_localized(&format!("Block {block:03} written"));
                append_log(ui, &format!("write · block {block:03} · {backup}"));
                toast_info(ui, &format!("Block {block:03} written and read back"));
            } else {
                ui.status.set_localized("No change");
                toast_info(ui, "The block already contained these bytes");
            }
        }
        Err(error) => {
            ui.status.set_localized("Write failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn choose_ndef_raw(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Load raw NDEF message"))
        .build();
    let ui2 = ui.clone();
    dialog.open(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        load_ndef_raw(&ui2, &path);
    });
}

fn load_ndef_raw(ui: &Rc<Ui>, path: &Path) {
    let loaded = ui.ndef_view.draft.borrow_mut().load_raw(path);
    match loaded {
        Ok(()) => {
            ui.ndef_view
                .raw_label
                .set_localized(&path.display().to_string());
            ui.ndef_view.source.set_selected(NdefSource::Raw as u32);
            update_ndef_actions(ui);
            toast_info(ui, "Raw NDEF message loaded");
        }
        Err(error) => toast_error(ui, &error),
    }
}

fn ndef_state_summary(info: &ClassicNdefInfo) -> String {
    match &info.ndef.state {
        ClassicNdefState::Unformatted => "Current NDEF state: unformatted".to_owned(),
        ClassicNdefState::Empty => "Current NDEF state: empty message".to_owned(),
        ClassicNdefState::Message(message) => format!(
            "Current NDEF state: {} record(s), {} bytes",
            message.records().len(),
            info.ndef.message_bytes
        ),
    }
}

fn stored_ndef_message(ui: &Ui) -> Option<NdefMessage> {
    if let Some(session) = ui.type2.borrow().as_ref() {
        return session.ndef().ok().and_then(Type2Ndef::message).cloned();
    }
    ui.ndef
        .borrow()
        .as_ref()
        .and_then(|info| info.ndef.message().cloned())
}

fn export_ndef_message(ui: &Rc<Ui>) {
    let Some(message) = stored_ndef_message(ui) else {
        toast_error(ui, "No NDEF message to export");
        return;
    };
    let bytes = match message.encode() {
        Ok(bytes) => bytes,
        Err(error) => {
            toast_error(ui, &format!("Cannot encode the message: {error}"));
            return;
        }
    };
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Export NDEF message"))
        .initial_name("message.ndef")
        .build();
    let ui2 = ui.clone();
    dialog.save(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        match std::fs::write(&path, &bytes) {
            Ok(()) => {
                ui2.status
                    .set_localized(&format!("Saved {}", path.display()));
                append_log(
                    &ui2,
                    &format!("ndef export · {} · {} bytes", path.display(), bytes.len()),
                );
                toast_info(&ui2, "NDEF message exported");
            }
            Err(error) => {
                toast_error(
                    &ui2,
                    &format!("Could not write {}: {error}", path.display()),
                );
            }
        }
    });
}

fn confirm_ndef_write(ui: Rc<Ui>) {
    if ui.type2.borrow().is_some() {
        confirm_type2_ndef_write(ui);
        return;
    }
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "NDEF writes need a live selected card");
        return;
    }
    let draft = ui.ndef_view.draft.borrow().clone();
    let message = match draft.message() {
        Ok(message) => message,
        Err(error) => {
            toast_error(&ui, &format!("Cannot encode the message: {error}"));
            return;
        }
    };
    let bytes = match message.encode() {
        Ok(bytes) => bytes,
        Err(error) => {
            toast_error(&ui, &format!("Cannot encode the message: {error}"));
            return;
        }
    };
    if !ui.dump.borrow().as_ref().is_some_and(Dump::complete) {
        toast_error(&ui, "NDEF writes need a complete card read");
        return;
    }
    let Some(info) = ui.ndef.borrow().clone() else {
        toast_error(&ui, "The NDEF state is unavailable; read the card again");
        return;
    };
    let Some(layout) = info.layout.as_ref() else {
        toast_error(&ui, "Not NDEF formatted; format the NDEF area first");
        return;
    };
    if layout.sectors.is_empty() {
        toast_error(
            &ui,
            "The MAD has no NFC sectors; format the NDEF area first",
        );
        return;
    }
    if layout.read_only || !layout.writable {
        toast_error(&ui, "The NDEF area is read-only; writing is disabled");
        return;
    }
    if !matches!(
        info.ndef.state,
        ClassicNdefState::Empty | ClassicNdefState::Message(_)
    ) {
        toast_error(&ui, "Not NDEF formatted; format the NDEF area first");
        return;
    }
    if let Some(error) = classic::ndef_capacity_error(bytes.len(), info.ndef.capacity) {
        toast_error(&ui, &error);
        return;
    }
    if info.ndef.message() == Some(&message) {
        toast_error(&ui, "The NDEF area already stores this message");
        return;
    }

    let body = format!(
        "{}\nMessage {} bytes · capacity {} bytes\nNew bytes  {}\n\n{}\n\nA project backup is written before the write, and the NDEF Message TLV is read back afterwards.",
        draft.summary(),
        bytes.len(),
        info.ndef.capacity,
        classic::hex_preview(&bytes, 48),
        ndef_state_summary(&info)
    );
    let dialog = adw::AlertDialog::builder()
        .heading("Write NDEF message?")
        .body(&body)
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "write", "Write NDEF");
    dialog.set_response_appearance("write", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_ndef_write_dialog.replace(None);
        if response == "write" {
            start_ndef_write(ui2.clone(), message.clone());
        }
    });
    ui.pending_ndef_write_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_ndef_write(ui: Rc<Ui>, message: NdefMessage) {
    set_busy(&ui, true);
    ui.status.set_localized("Writing NDEF message…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.borrow().clone();
    let expected_card = ui.dump.borrow().as_ref().and_then(|dump| dump.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = ClassicNdefWrite::new(message, &keys, &backup);
                if let Some(card) = expected_card {
                    request = request.expected_card(card);
                }
                service.classic_ndef_write(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_ndef_write(&ui, result),
            Err(_) => finish_ndef_write(&ui, Err(NfcError::Io("reader worker stopped".to_owned()))),
        }
    });
}

fn finish_ndef_write(ui: &Rc<Ui>, result: NfcResult<ClassicNdefWriteResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            apply_ndef_result(ui, &result.dump, &result.info);
            let backup = result.backup.path.display().to_string();
            ui.status.set_localized(&format!(
                "NDEF written · {} block(s)",
                result.written_blocks
            ));
            append_log(
                ui,
                &format!(
                    "ndef write · {} block(s), {} changed · {backup}",
                    result.written_blocks, result.changed_blocks
                ),
            );
            toast_info(ui, "NDEF written and read back");
        }
        Err(error) => {
            ui.status.set_localized("NDEF write failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn confirm_ndef_format(ui: Rc<Ui>) {
    if ui.type2.borrow().is_some() {
        confirm_type2_ndef_format(ui);
        return;
    }
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "NDEF format needs a live selected card");
        return;
    }
    let Some(dump) = ui.dump.borrow().clone() else {
        toast_error(&ui, "Read a card before formatting its NDEF area");
        return;
    };
    if !dump.complete() {
        toast_error(&ui, "NDEF format needs a complete card read");
        return;
    }
    let read_only = ui.ndef_view.read_only.is_active();
    let area = match ui
        .ndef
        .borrow()
        .as_ref()
        .and_then(|info| info.layout.as_ref())
    {
        Some(layout) => {
            let sectors = layout
                .sectors
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "NFC sectors {} · {} data blocks · {} bytes",
                sectors,
                layout.data_blocks.len(),
                layout.data_len()
            )
        }
        None => "the MAD and every NFC sector".to_owned(),
    };
    let body = format!(
        "{area}\n\nFormat replaces the MAD, the NDEF Message TLV, sector keys, and access bits on every NFC sector.\nRead-only after format: {}\n\nA project backup is written before the format, and every written block is read back afterwards.",
        if read_only { "yes" } else { "no" }
    );
    let dialog = adw::AlertDialog::builder()
        .heading("Format the NDEF area?")
        .body(&body)
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "format", "Format NDEF area");
    dialog.set_response_appearance("format", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_ndef_format_dialog.replace(None);
        if response == "format" {
            start_ndef_format(ui2.clone(), read_only);
        }
    });
    ui.pending_ndef_format_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_ndef_format(ui: Rc<Ui>, read_only: bool) {
    set_busy(&ui, true);
    ui.status.set_localized("Formatting the NDEF area…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.borrow().clone();
    let expected_card = ui.dump.borrow().as_ref().and_then(|dump| dump.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = ClassicNdefFormat::new(&keys, &backup)
                    .confirm(true)
                    .read_only(read_only);
                if let Some(card) = expected_card {
                    request = request.expected_card(card);
                }
                service.classic_ndef_format(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_ndef_format(&ui, result),
            Err(_) => {
                finish_ndef_format(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_ndef_format(ui: &Rc<Ui>, result: NfcResult<ClassicNdefFormatResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            apply_ndef_result(ui, &result.dump, &result.info);
            let backup = result.backup.path.display().to_string();
            ui.status.set_localized(&format!(
                "NDEF formatted · {} block(s), {} trailer(s)",
                result.written_blocks, result.trailer_writes
            ));
            append_log(
                ui,
                &format!(
                    "ndef format · {} block(s), {} trailer(s) · {backup}",
                    result.written_blocks, result.trailer_writes
                ),
            );
            toast_info(ui, "NDEF area formatted and read back");
        }
        Err(error) => {
            ui.status.set_localized("NDEF format failed");
            toast_error(ui, &error.to_string());
        }
    }
}

/// Replaces the live image with the service result and refreshes every surface
/// that depends on it. No redundant card read is performed.
fn apply_ndef_result(ui: &Rc<Ui>, dump: &Dump, info: &ClassicNdefInfo) {
    populate_rows(ui, dump);
    ui.dump.replace(Some(dump.clone()));
    ui.ndef.replace(Some(info.clone()));
    update_card(ui);
    render_ndef(ui, Some(info), None);
    update_restore_sensitivity(ui);
}

fn typed_dialog(heading: &str, body: &str, confirm_label: &str) -> TypedDialog {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(ADVANCED_PHRASE));
    let hint = gtk::Label::new(None);
    hint.set_localized(&format!("Type {ADVANCED_PHRASE} to enable the action."));
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    hint.add_css_class("caption");
    let extra = gtk::Box::new(Orientation::Vertical, 6);
    extra.append(&hint);
    extra.append(&entry);
    dialog.set_extra_child(Some(&extra));
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "confirm", confirm_label);
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("confirm", false);
    let dialog2 = dialog.clone();
    entry.connect_changed(move |entry| {
        dialog2.set_response_enabled("confirm", entry.text() == ADVANCED_PHRASE);
    });
    TypedDialog { dialog, entry }
}

fn reset_advanced(ui: &Ui) {
    ui.type2_view.advanced.set_active(false);
    ui.type2_restore.replace(None);
    ui.type2_view
        .restore_label
        .set_localized("No restore image loaded");
    update_type2_restore_sensitivity(ui);
}

fn render_type2(ui: &Ui) {
    let session = ui.type2.borrow().clone();
    let view = &ui.type2_view;
    let Some(session) = session else {
        view.summary
            .set_localized("Read a Type 2 tag to inspect its pages.");
        view.warnings.set_visible(false);
        view.rows.remove_all();
        view.table_stack.set_visible_child_name("empty");
        view.configuration.set_localized("Read a Type 2 tag.");
        view.advanced.set_sensitive(false);
        view.editor.title.set_localized("No page");
        view.editor
            .meta
            .set_localized("Read a Type 2 tag to inspect a page.");
        view.editor.details.set_localized("");
        view.editor.hex.set_text("");
        view.editor.hex.set_sensitive(false);
        view.editor.ascii.set_localized("");
        view.editor.feedback.set_localized("");
        view.editor.write.set_sensitive(false);
        update_type2_restore_sensitivity(ui);
        update_type2_actions(ui);
        return;
    };
    let ndef = session
        .ndef()
        .map(|ndef| type2_ui::ndef_state_label(&ndef.state))
        .unwrap_or_else(|error| format!("invalid ({error})"));
    view.summary.set_localized(&format!(
        "{} · UID {}\n{}/{} pages readable · CC {} · NDEF {}",
        session.image.layout.product.name(),
        session.card.uid_hex(),
        session.image.known_pages(),
        session.image.layout.page_count(),
        type2_ui::hex_bytes(&session.image.cc.to_bytes()),
        ndef
    ));
    if session.warnings.is_empty() {
        view.warnings.set_visible(false);
    } else {
        view.warnings.set_localized(&session.warnings.join("\n"));
        view.warnings.set_visible(true);
    }
    view.advanced.set_sensitive(!ui.busy.get());
    view.configuration
        .set_localized(&type2_ui::configuration_text(&session));
    update_type2_page_table(ui);
    update_type2_inspector(ui);
    update_type2_restore_sensitivity(ui);
    update_type2_actions(ui);
}

fn update_type2_page_table(ui: &Ui) {
    let selected = selected_type2_page(ui);
    let advanced = ui.type2_view.advanced.is_active();
    let rows = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| type2_ui::page_rows(session, advanced))
        .unwrap_or_default();
    ui.type2_view.rows.remove_all();
    for row in &rows {
        ui.type2_view
            .rows
            .append(&glib::BoxedAnyObject::new(row.clone()));
    }
    if rows.is_empty() {
        ui.type2_view.table_stack.set_visible_child_name("empty");
        return;
    }
    ui.type2_view.table_stack.set_visible_child_name("table");
    let index = selected
        .and_then(|page| rows.iter().position(|row| row.page == page))
        .unwrap_or(0);
    ui.type2_view.selection.set_selected(index as u32);
}

fn selected_type2_row(ui: &Ui) -> Option<type2_ui::PageRow> {
    ui.type2_view
        .selection
        .selected_item()
        .and_downcast::<glib::BoxedAnyObject>()
        .map(|object| object.borrow::<type2_ui::PageRow>().clone())
}

fn selected_type2_page(ui: &Ui) -> Option<u8> {
    selected_type2_row(ui).map(|row| row.page)
}

fn select_type2_page(ui: &Ui, page: u8) {
    for index in 0..ui.type2_view.rows.n_items() {
        let Some(object) = ui
            .type2_view
            .rows
            .item(index)
            .and_downcast::<glib::BoxedAnyObject>()
        else {
            continue;
        };
        if object.borrow::<type2_ui::PageRow>().page == page {
            ui.type2_view.selection.set_selected(index);
            return;
        }
    }
}

fn update_type2_inspector(ui: &Ui) {
    let session = ui.type2.borrow();
    let Some(session) = session.as_ref() else {
        ui.type2_view.editor.title.set_localized("No page");
        ui.type2_view
            .editor
            .meta
            .set_localized("Read a Type 2 tag to inspect a page.");
        ui.type2_view.editor.details.set_localized("");
        ui.type2_view.editor.hex.set_text("");
        ui.type2_view.editor.ascii.set_localized("");
        ui.type2_view.editor.feedback.set_localized("");
        ui.type2_view.editor.write.set_sensitive(false);
        return;
    };
    let Some(row) = selected_type2_row(ui) else {
        ui.type2_view.editor.title.set_localized("No page");
        ui.type2_view
            .editor
            .meta
            .set_localized("Select a page in the table.");
        ui.type2_view.editor.details.set_localized("");
        ui.type2_view.editor.hex.set_text("");
        ui.type2_view.editor.ascii.set_localized("");
        ui.type2_view.editor.feedback.set_localized("");
        ui.type2_view.editor.write.set_sensitive(false);
        return;
    };
    let advanced = ui.type2_view.advanced.is_active();
    let Some(insight) = type2_ui::inspect(session, row.page, advanced) else {
        return;
    };
    let editor = &ui.type2_view.editor;
    editor.title.set_localized(&insight.title);
    editor.meta.set_localized(&insight.meta);
    editor.details.set_localized(&insight.details);
    editor.hex.set_text(&insight.hex);
    editor.ascii.set_localized(&insight.ascii);
    ui.selected_label.set_localized(&format!(
        "Page {:03} · {}\n{}",
        row.page, row.role, row.lock
    ));
    update_type2_write_sensitivity(ui);
}

fn update_type2_write_sensitivity(ui: &Ui) {
    let session = ui.type2.borrow();
    let Some(session) = session.as_ref() else {
        ui.type2_view.editor.write.set_sensitive(false);
        return;
    };
    let Some(row) = selected_type2_row(ui) else {
        ui.type2_view.editor.write.set_sensitive(false);
        return;
    };
    let advanced = ui.type2_view.advanced.is_active();
    let gate = type2_ui::write_gate(session, row.page, advanced);
    let ready = ui.live.get() && !ui.busy.get() && row.complete;
    let editor = &ui.type2_view.editor;
    let mut text = String::new();
    let mut error = false;
    let mut sensitive = false;
    let mut write = false;
    match &gate {
        Err(reason) => {
            text.clone_from(reason);
            error = true;
        }
        Ok(()) if !row.complete => {
            text = "This page was not readable; read the tag again.".to_owned();
            error = true;
        }
        Ok(()) if !ready => {
            text = "Read a live card to write this page.".to_owned();
        }
        Ok(()) => match type2_ui::parse_hex4(&editor.hex.text()) {
            Err(reason) => {
                text = reason;
                error = true;
            }
            Ok(data) => {
                sensitive = true;
                let same = session
                    .image
                    .page(row.page)
                    .is_some_and(|page| *page == data);
                if same {
                    text = "The page already contains these bytes.".to_owned();
                } else {
                    text = format!("Ready to write page {}.", row.page);
                    write = true;
                }
            }
        },
    }
    editor.hex.set_sensitive(sensitive);
    editor.write.set_sensitive(write);
    editor.feedback.set_localized(&text);
    if error {
        editor.feedback.add_css_class("error");
    } else {
        editor.feedback.remove_css_class("error");
    }
}

fn update_type2_actions(ui: &Ui) {
    let session = ui.type2.borrow();
    let view = &ui.type2_view;
    let busy = ui.busy.get();
    let live = ui.live.get();
    let advanced = view.advanced.is_active();
    let has_session = session.is_some();
    let has_configuration = session
        .as_ref()
        .is_some_and(|session| session.configuration().is_some());
    view.save.set_sensitive(has_session && !busy);
    view.load.set_sensitive(!busy);
    view.advanced.set_sensitive(has_session && !busy);
    view.restore_load.set_sensitive(has_session && !busy);
    view.password_apply
        .set_sensitive(live && !busy && advanced && has_configuration);
    view.password_note.set_localized(if !has_session {
        "Read a Type 2 tag to inspect its password configuration."
    } else if !has_configuration {
        "This tag has no readable NTAG configuration pages."
    } else if advanced {
        "PWD and PACK are write-only; the service verifies PACK with PWD_AUTH after the write."
    } else {
        "Enable Advanced mode to change locks, configuration, or the password."
    });

    let mut lock_ready = false;
    let mut lock_text = "Enable Advanced mode to change lock bits.".to_owned();
    if advanced {
        if live && !busy {
            if let Some(session) = session.as_ref() {
                match type2_ui::parse_page_list(&view.lock_pages.text()) {
                    Err(error) => {
                        lock_text = error;
                    }
                    Ok(pages) => match type2_ui::lock_preview(session, &pages) {
                        Err(error) => lock_text = error,
                        Ok(preview) if preview.update.is_empty() => {
                            lock_text = "The requested pages are already locked.".to_owned();
                        }
                        Ok(preview) => {
                            let protected = session.configuration().is_some_and(|config| {
                                config.requires_password(2)
                                    || session
                                        .image
                                        .layout
                                        .dynamic_lock
                                        .is_some_and(|page| config.requires_password(page))
                            });
                            if protected {
                                lock_text = "Password protection covers the lock pages; PWD_AUTH is required first.".to_owned();
                            } else {
                                lock_ready = true;
                                lock_text = format!(
                                    "Pages {}\nStatic lock {} → {}\nDynamic lock {} → {}\nOne-way: set bits cannot be cleared. The service backs up the image and reads every lock page back.",
                                    pages
                                        .iter()
                                        .map(u8::to_string)
                                        .collect::<Vec<_>>()
                                        .join(", "),
                                    type2_ui::hex_bytes(&preview.static_before),
                                    type2_ui::hex_bytes(&preview.static_after),
                                    type2_ui::hex_bytes(&preview.dynamic_before),
                                    type2_ui::hex_bytes(&preview.dynamic_after)
                                );
                            }
                        }
                    },
                }
            }
        } else {
            lock_text = "Read a live card to change lock bits.".to_owned();
        }
    }
    view.lock_preview.set_localized(&lock_text);
    view.lock_apply.set_sensitive(lock_ready && live && !busy);
    update_type2_restore_sensitivity(ui);
    update_type2_write_sensitivity(ui);
}

fn type2_restore_compatible(ui: &Ui) -> bool {
    let source = ui.type2_restore.borrow();
    let session = ui.type2.borrow();
    match (source.as_ref(), session.as_ref()) {
        (Some((_, source)), Some(session)) => {
            source.layout == session.image.layout
                && source.cc.to_bytes() == session.image.cc.to_bytes()
        }
        _ => false,
    }
}

fn update_type2_restore_sensitivity(ui: &Ui) {
    let view = &ui.type2_view;
    let has_source = ui.type2_restore.borrow().is_some();
    let any = view.include_memory.is_active()
        || view.include_configuration.is_active()
        || view.include_cc.is_active()
        || view.include_locks.is_active();
    view.restore.set_sensitive(
        ui.live.get() && !ui.busy.get() && has_source && any && type2_restore_compatible(ui),
    );
}

fn populate_type2_forms(ui: &Ui) {
    let session = ui.type2.borrow();
    let Some(session) = session.as_ref() else {
        return;
    };
    let view = &ui.type2_view;
    match session.configuration() {
        Some(config) => {
            view.auth0.set_text(&config.auth0.to_string());
            view.access.set_text(&format!("{:02X}", config.access));
        }
        None => {
            view.auth0.set_text("");
            view.access.set_text("");
        }
    }
    view.pwd.set_text("");
    view.pack.set_text("");
    view.lock_pages.set_text("");
    view.configuration
        .set_localized(&type2_ui::configuration_text(session));
}

fn show_type2_session(ui: &Rc<Ui>, session: Type2Session, live: bool) {
    ui.live.set(false);
    ui.dump.replace(None);
    ui.ndef.replace(None);
    ui.rows.remove_all();
    ui.table_stack.set_visible_child_name("empty");
    ui.type2.replace(Some(session));
    ui.type2_view.advanced.set_active(false);
    ui.type2_restore.replace(None);
    ui.type2_view
        .restore_label
        .set_localized("No restore image loaded");
    ui.live.set(live);
    ui.type2_view.tabs.set_visible_child_name("memory");
    ui.views.set_visible_child_name("type2");
    populate_type2_forms(ui);
    render_type2(ui);
    update_card(ui);
    update_ndef(ui);
    update_restore_sensitivity(ui);
}

fn apply_type2_image(ui: &Rc<Ui>, image: Type2Image, warnings: Vec<String>) {
    let card = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.clone());
    let Some(card) = card else {
        return;
    };
    ui.type2
        .replace(Some(Type2Session::new(card, image, warnings)));
    render_type2(ui);
    update_card(ui);
    update_ndef(ui);
}

fn save_type2_dump(ui: &Rc<Ui>) {
    if ui.type2.borrow().is_none() {
        toast_error(ui, "No Type 2 image to save");
        return;
    }
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Save Type 2 project image"))
        .initial_name("tag.nfc-foundry.json")
        .build();
    let ui2 = ui.clone();
    dialog.save(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        if let Err(error) = write_type2_dump(&ui2, &path) {
            toast_error(&ui2, &error);
        }
    });
}

fn write_type2_dump(ui: &Rc<Ui>, path: &Path) -> Result<(), String> {
    let image = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.image.clone())
        .ok_or_else(|| "No Type 2 image to save".to_owned())?;
    let bytes = type2_exchange::export(&image).map_err(|error| error.to_string())?;
    std::fs::write(path, &bytes)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    ui.status
        .set_localized(&format!("Saved {}", path.display()));
    append_log(
        ui,
        &format!(
            "type2 dump · {} · {}/{} pages · {}",
            path.display(),
            image.known_pages(),
            image.layout.page_count(),
            image.layout.product.name()
        ),
    );
    toast_info(ui, &format!("Saved {}", path.display()));
    Ok(())
}

fn choose_type2_restore(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Choose Type 2 project image"))
        .build();
    let ui2 = ui.clone();
    dialog.open(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        if let Err(error) = load_type2_restore_file(&ui2, &path) {
            toast_error(&ui2, &error);
        }
    });
}

fn load_type2_restore_file(ui: &Rc<Ui>, path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let image = type2_exchange::load(&bytes).map_err(|error| error.to_string())?;
    let summary = format!(
        "{} · UID {} · {}/{} pages",
        path.display(),
        image.uid_hex(),
        image.known_pages(),
        image.layout.page_count()
    );
    ui.type2_restore.replace(Some((path.to_path_buf(), image)));
    ui.type2_view.restore_label.set_localized(&summary);
    ui.type2_view.restore_status.set_localized(
        "The live card is backed up before the restore and every written page is read back.",
    );
    append_log(ui, &format!("type2 restore source · {summary}"));
    update_type2_restore_sensitivity(ui);
    Ok(())
}

fn type2_restore_options(view: &Type2View) -> RestoreOptions {
    RestoreOptions {
        restore_user_memory: view.include_memory.is_active(),
        restore_capability_container: view.include_cc.is_active(),
        restore_configuration: view.include_configuration.is_active(),
        restore_lock_bytes: view.include_locks.is_active(),
    }
}

fn confirm_type2_restore(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "Restoring needs a live selected card");
        return;
    }
    let Some(session) = ui.type2.borrow().clone() else {
        toast_error(&ui, "Read a Type 2 tag before restoring");
        return;
    };
    let Some((path, source)) = ui.type2_restore.borrow().clone() else {
        toast_error(&ui, "Choose a Type 2 project image first");
        return;
    };
    if source.layout != session.image.layout {
        toast_error(
            &ui,
            &format!(
                "The project image is for {} but the card is {}",
                source.layout.product.name(),
                session.image.layout.product.name()
            ),
        );
        return;
    }
    if source.cc.to_bytes() != session.image.cc.to_bytes() {
        toast_error(
            &ui,
            "The project image capability container does not match the card",
        );
        return;
    }
    let options = type2_restore_options(&ui.type2_view);
    if !(options.restore_user_memory
        || options.restore_capability_container
        || options.restore_configuration
        || options.restore_lock_bytes)
    {
        toast_error(&ui, "Select at least one page class to restore");
        return;
    }
    let mut classes = Vec::new();
    if options.restore_user_memory {
        classes.push("user memory");
    }
    if options.restore_configuration {
        classes.push("configuration pages");
    }
    if options.restore_capability_container {
        classes.push("capability container");
    }
    if options.restore_lock_bytes {
        classes.push("lock bytes");
    }
    let body = format!(
        "Target {} · UID {}\nSource {} · UID {}\nRestore {}\n\nA project backup of the live tag is written before the restore, and every written page is read back. Manufacturer and UID pages are never written.",
        session.image.layout.product.name(),
        session.card.uid_hex(),
        path.display(),
        source.uid_hex(),
        classes.join(", ")
    );
    let typed = typed_dialog("Restore the Type 2 image?", &body, "Restore image");
    let ui2 = ui.clone();
    let typed2 = typed.clone();
    typed.dialog.connect_response(None, move |_, response| {
        if response != "confirm" || typed2.entry.text() != ADVANCED_PHRASE {
            return;
        }
        ui2.pending_type2_restore_dialog.replace(None);
        start_type2_restore(ui2.clone(), source.clone(), options);
    });
    ui.pending_type2_restore_dialog.replace(Some(typed.clone()));
    present_alert(&typed.dialog, &ui.window);
}

fn start_type2_restore(ui: Rc<Ui>, source: Type2Image, options: RestoreOptions) {
    set_busy(&ui, true);
    ui.status.set_localized("Restoring the Type 2 image…");
    let service = ui.service.clone();
    let backup = ui.backup.borrow().clone();
    let expected = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = Type2Restore::new(source, &backup)
                    .options(options)
                    .confirm(true);
                if let Some(card) = expected {
                    request = request.expected_card(card);
                }
                service.type2_restore(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_type2_restore(&ui, result),
            Err(_) => {
                finish_type2_restore(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_type2_restore(ui: &Rc<Ui>, result: NfcResult<Type2MutationResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let backup = result.backup.path.display().to_string();
            apply_type2_image(ui, result.image, result.warnings);
            ui.status.set_localized(&format!(
                "Restore complete · {} written, {} unchanged",
                result.summary.written_pages, result.summary.unchanged_pages
            ));
            append_log(
                ui,
                &format!(
                    "type2 restore · {} written, {} unchanged · {backup}",
                    result.summary.written_pages, result.summary.unchanged_pages
                ),
            );
            toast_info(ui, "Type 2 image restored and read back");
            ui.type2_restore.replace(None);
            ui.type2_view
                .restore_label
                .set_localized("No restore image loaded");
            update_type2_restore_sensitivity(ui);
        }
        Err(error) => {
            ui.status.set_localized("Type 2 restore failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn confirm_type2_page_write(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "Type 2 writes need a live selected card");
        return;
    }
    let Some(session) = ui.type2.borrow().clone() else {
        toast_error(&ui, "Read a Type 2 tag first");
        return;
    };
    let Some(row) = selected_type2_row(&ui) else {
        toast_error(&ui, "Select a page first");
        return;
    };
    let advanced_page = row.advanced;
    let advanced = ui.type2_view.advanced.is_active();
    if let Err(error) = type2_ui::write_gate(&session, row.page, advanced) {
        toast_error(&ui, &error);
        return;
    }
    let data = match type2_ui::parse_hex4(&ui.type2_view.editor.hex.text()) {
        Ok(data) => data,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let Some(current) = session.image.page(row.page).copied() else {
        toast_error(&ui, "The selected page is not readable");
        return;
    };
    if current == data {
        toast_info(&ui, "The page already contains these bytes");
        return;
    }
    let body = format!(
        "{} · UID {}\nPage {:03} · {}\n\nOld  {}\nNew  {}\n\nA project backup is written before the write, and the page is read back afterwards.",
        session.image.layout.product.name(),
        session.card.uid_hex(),
        row.page,
        session.image.role(row.page).label(),
        type2_ui::hex4(&current),
        type2_ui::hex4(&data)
    );
    if advanced_page {
        let typed = typed_dialog("Write an Advanced Type 2 page?", &body, "Write page");
        let ui2 = ui.clone();
        let typed2 = typed.clone();
        typed.dialog.connect_response(None, move |_, response| {
            if response != "confirm" || typed2.entry.text() != ADVANCED_PHRASE {
                return;
            }
            ui2.pending_type2_advanced_dialog.replace(None);
            start_type2_page_write(ui2.clone(), row.page, data, true);
        });
        ui.pending_type2_advanced_dialog
            .replace(Some(typed.clone()));
        present_alert(&typed.dialog, &ui.window);
    } else {
        let dialog = adw::AlertDialog::builder()
            .heading(format!("Write page {:03}?", row.page))
            .body(&body)
            .build();
        add_response(&dialog, "cancel", "Cancel");
        add_response(&dialog, "write", "Write page");
        dialog.set_response_appearance("write", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let ui2 = ui.clone();
        dialog.connect_response(None, move |_, response| {
            ui2.pending_type2_page_dialog.replace(None);
            if response == "write" {
                start_type2_page_write(ui2.clone(), row.page, data, false);
            }
        });
        ui.pending_type2_page_dialog.replace(Some(dialog.clone()));
        present_alert(&dialog, &ui.window);
    }
}

fn start_type2_page_write(ui: Rc<Ui>, page: u8, data: [u8; 4], advanced: bool) {
    set_busy(&ui, true);
    ui.status.set_localized(&format!("Writing page {page:03}…"));
    let service = ui.service.clone();
    let backup = ui.backup.borrow().clone();
    let expected = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = Type2PageWrite::new(page, data, &backup)
                    .advanced(advanced)
                    .confirm(advanced);
                if let Some(card) = expected {
                    request = request.expected_card(card);
                }
                service.type2_write_page(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_type2_page_write(&ui, page, result),
            Err(_) => finish_type2_page_write(
                &ui,
                page,
                Err(NfcError::Io("reader worker stopped".to_owned())),
            ),
        }
    });
}

fn finish_type2_page_write(ui: &Rc<Ui>, page: u8, result: NfcResult<Type2PageResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let changed = result.changed;
            let backup = result
                .backup
                .as_ref()
                .map(|backup| backup.path.display().to_string())
                .unwrap_or_else(|| "no backup".to_owned());
            apply_type2_image(ui, result.image, result.warnings);
            select_type2_page(ui, page);
            update_type2_inspector(ui);
            if changed {
                ui.status
                    .set_localized(&format!("Type 2 page {page:03} written"));
                append_log(ui, &format!("type2 write · page {page:03} · {backup}"));
                toast_info(ui, &format!("Page {page:03} written and read back"));
            } else {
                ui.status.set_localized("No change");
                toast_info(ui, "The page already contained these bytes");
            }
        }
        Err(error) => {
            ui.status.set_localized("Type 2 write failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn confirm_type2_lock(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "Locking needs a live selected card");
        return;
    }
    if !ui.type2_view.advanced.is_active() {
        toast_error(&ui, "Enable Advanced mode before locking pages");
        return;
    }
    let Some(session) = ui.type2.borrow().clone() else {
        toast_error(&ui, "Read a Type 2 tag first");
        return;
    };
    let pages = match type2_ui::parse_page_list(&ui.type2_view.lock_pages.text()) {
        Ok(pages) => pages,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let preview = match type2_ui::lock_preview(&session, &pages) {
        Ok(preview) => preview,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    if preview.update.is_empty() {
        toast_info(&ui, "The requested pages are already locked");
        return;
    }
    let body = format!(
        "{} · UID {}\nPages {}\n\nStatic lock {} → {}\nDynamic lock {} → {}\n\nLock bits are one-way; set bits can never be cleared. A project backup is written first, and the lock pages are read back.",
        session.image.layout.product.name(),
        session.card.uid_hex(),
        pages
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        type2_ui::hex_bytes(&preview.static_before),
        type2_ui::hex_bytes(&preview.static_after),
        type2_ui::hex_bytes(&preview.dynamic_before),
        type2_ui::hex_bytes(&preview.dynamic_after)
    );
    let typed = typed_dialog("Apply one-way lock bits?", &body, "Apply lock bits");
    let ui2 = ui.clone();
    let typed2 = typed.clone();
    typed.dialog.connect_response(None, move |_, response| {
        if response != "confirm" || typed2.entry.text() != ADVANCED_PHRASE {
            return;
        }
        ui2.pending_type2_lock_dialog.replace(None);
        start_type2_lock(ui2.clone(), pages.clone());
    });
    ui.pending_type2_lock_dialog.replace(Some(typed.clone()));
    present_alert(&typed.dialog, &ui.window);
}

fn start_type2_lock(ui: Rc<Ui>, pages: Vec<u8>) {
    set_busy(&ui, true);
    ui.status.set_localized("Writing Type 2 lock bits…");
    let service = ui.service.clone();
    let backup = ui.backup.borrow().clone();
    let expected = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = Type2Lock::new(pages, &backup).confirm(true);
                if let Some(card) = expected {
                    request = request.expected_card(card);
                }
                service.type2_lock(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_type2_lock(&ui, result),
            Err(_) => finish_type2_lock(&ui, Err(NfcError::Io("reader worker stopped".to_owned()))),
        }
    });
}

fn finish_type2_lock(ui: &Rc<Ui>, result: NfcResult<Type2LockResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let empty = result.update.is_empty();
            let backup = result
                .backup
                .as_ref()
                .map(|backup| backup.path.display().to_string())
                .unwrap_or_else(|| "no backup".to_owned());
            apply_type2_image(ui, result.image, result.warnings);
            ui.type2_view.lock_pages.set_text("");
            if empty {
                ui.status.set_localized("Lock bits already set");
                toast_info(ui, "The requested pages are already locked");
            } else {
                ui.status.set_localized("Lock bits written");
                append_log(ui, &format!("type2 lock · {backup}"));
                toast_info(ui, "Lock bits written and read back");
            }
        }
        Err(error) => {
            ui.status.set_localized("Type 2 lock failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn confirm_type2_password(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "Password configuration needs a live selected card");
        return;
    }
    if !ui.type2_view.advanced.is_active() {
        toast_error(&ui, "Enable Advanced mode before changing the password");
        return;
    }
    let Some(session) = ui.type2.borrow().clone() else {
        toast_error(&ui, "Read a Type 2 tag first");
        return;
    };
    let auth0 = match type2_ui::parse_u8(&ui.type2_view.auth0.text()) {
        Ok(auth0) => auth0,
        Err(error) => {
            toast_error(&ui, &format!("AUTH0: {error}"));
            return;
        }
    };
    let access = match type2_ui::parse_hex_bytes::<1>(&ui.type2_view.access.text(), "ACCESS") {
        Ok(access) => access[0],
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let password = match type2_ui::parse_hex_bytes::<4>(&ui.type2_view.pwd.text(), "PWD") {
        Ok(password) => password,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let pack = match type2_ui::parse_hex_bytes::<2>(&ui.type2_view.pack.text(), "PACK") {
        Ok(pack) => pack,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let old = session
        .configuration()
        .map(|config| format!("AUTH0 {} · ACCESS {:02X}", config.auth0, config.access))
        .unwrap_or_else(|| "configuration unavailable".to_owned());
    let body = format!(
        "{} · UID {}\n\nOld  {}\nNew  AUTH0 {auth0} · ACCESS {access:02X}\nPWD  {}\nPACK {}\n\nPWD and PACK are write-only. A project backup is written first; the service writes PWD, PACK, ACCESS, and AUTH0, then verifies PACK with a fresh PWD_AUTH.",
        session.image.layout.product.name(),
        session.card.uid_hex(),
        old,
        type2_ui::hex_bytes(&password),
        type2_ui::hex_bytes(&pack)
    );
    let typed = typed_dialog(
        "Change the Type 2 password configuration?",
        &body,
        "Apply configuration",
    );
    let ui2 = ui.clone();
    let typed2 = typed.clone();
    typed.dialog.connect_response(None, move |_, response| {
        if response != "confirm" || typed2.entry.text() != ADVANCED_PHRASE {
            return;
        }
        ui2.pending_type2_password_dialog.replace(None);
        start_type2_password(ui2.clone(), auth0, access, password, pack);
    });
    ui.pending_type2_password_dialog
        .replace(Some(typed.clone()));
    present_alert(&typed.dialog, &ui.window);
}

fn start_type2_password(ui: Rc<Ui>, auth0: u8, access: u8, password: [u8; 4], pack: [u8; 2]) {
    set_busy(&ui, true);
    ui.status.set_localized("Writing password configuration…");
    let service = ui.service.clone();
    let backup = ui.backup.borrow().clone();
    let expected = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request =
                    Type2Password::new(auth0, access, password, pack, &backup).confirm(true);
                if let Some(card) = expected {
                    request = request.expected_card(card);
                }
                service.type2_password(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_type2_password(&ui, result),
            Err(_) => {
                finish_type2_password(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_type2_password(ui: &Rc<Ui>, result: NfcResult<Type2MutationResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let backup = result.backup.path.display().to_string();
            apply_type2_image(ui, result.image, result.warnings);
            populate_type2_forms(ui);
            ui.status
                .set_localized("Password configuration written and verified");
            append_log(ui, &format!("type2 password · {backup}"));
            toast_info(ui, "Password configuration written and verified");
        }
        Err(error) => {
            ui.status.set_localized("Type 2 password failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn render_type2_ndef(ui: &Ui, session: &Type2Session) {
    let view = &ui.ndef_view;
    while let Some(child) = view.info.first_child() {
        view.info.remove(&child);
    }
    view.read_only.set_visible(false);
    view.read_only.set_active(false);
    match session.ndef() {
        Err(error) => {
            view.state.set_localized("NDEF unavailable");
            let label = ndef_note(error);
            label.add_css_class("error");
            view.info.append(&label);
            update_ndef_actions(ui);
            return;
        }
        Ok(ndef) => {
            view.state.set_localized(match &ndef.state {
                NdefState::Unformatted => "Not NDEF formatted",
                NdefState::Empty => "NDEF formatted · empty message",
                NdefState::Message(message) => {
                    if message.records().len() == 1 {
                        "NDEF · 1 record"
                    } else {
                        "NDEF · multiple records"
                    }
                }
            });
            let mut facts = vec![
                format!(
                    "{} · UID {}",
                    session.image.layout.product.name(),
                    session.card.uid_hex()
                ),
                format!(
                    "CC {} · data area {} bytes · capacity {} bytes",
                    type2_ui::hex_bytes(&session.image.cc.to_bytes()),
                    session.image.cc.size,
                    ndef.capacity
                ),
                format!(
                    "{} · locked pages: {}",
                    if ndef.read_only {
                        "capability container read-only"
                    } else {
                        "capability container writable"
                    },
                    type2_ui::ndef_locked_pages(session)
                        .map(|pages| if pages.is_empty() {
                            "none".to_owned()
                        } else {
                            pages
                                .iter()
                                .map(u8::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_else(|error| error.to_string())
                ),
            ];
            if let Some(page) = type2_ui::ndef_password_page(session) {
                facts.push(format!(
                    "Password protection covers the data area from page {page}; PWD_AUTH is required before writing."
                ));
            }
            let facts = ndef_note(&facts.join("\n"));
            facts.add_css_class("caption");
            view.info.append(&facts);
            match &ndef.state {
                NdefState::Unformatted => {
                    view.info.append(&ndef_note(
                        "Format the NDEF data area before writing; Format creates an empty NDEF Message TLV.",
                    ));
                }
                NdefState::Empty => {
                    view.info.append(&ndef_note(&format!(
                        "Capacity {} bytes · message empty",
                        ndef.capacity
                    )));
                }
                NdefState::Message(message) => {
                    let capacity = ndef_note(&format!(
                        "Capacity {} bytes · message {} bytes",
                        ndef.capacity, ndef.message_bytes
                    ));
                    capacity.add_css_class("caption");
                    view.info.append(&capacity);
                    for (index, record) in message.records().iter().enumerate() {
                        view.info.append(&record_card(index, record));
                    }
                }
            }
            for warning in &session.warnings {
                let label = ndef_note(warning);
                label.add_css_class("caption");
                view.info.append(&label);
            }
        }
    }
    update_ndef_actions(ui);
}

fn encoded_draft(draft: &NdefDraft) -> std::result::Result<(NdefMessage, Vec<u8>), String> {
    let message = draft
        .message()
        .map_err(|error| format!("Cannot encode the message: {error}"))?;
    let bytes = message
        .encode()
        .map_err(|error| format!("Cannot encode the message: {error}"))?;
    Ok((message, bytes))
}

fn ndef_preview(
    draft: &NdefDraft,
    encoded: &std::result::Result<(NdefMessage, Vec<u8>), String>,
    capacity: Option<usize>,
) -> String {
    let mut lines = vec![draft.summary()];
    match encoded {
        Ok((_, bytes)) => {
            lines.push(match capacity {
                Some(capacity) => {
                    format!(
                        "Message {} bytes · capacity {} bytes",
                        bytes.len(),
                        capacity
                    )
                }
                None => format!("Message {} bytes", bytes.len()),
            });
            if !bytes.is_empty() {
                lines.push(classic::hex_preview(bytes, 48));
            }
        }
        Err(error) => lines.push(error.clone()),
    }
    lines.join("\n")
}

fn type2_format_gate(session: &Type2Session) -> std::result::Result<(), String> {
    if session.image.layout.config.is_some() && session.configuration().is_none() {
        return Err(
            "The NTAG configuration pages are not readable; protection state is unknown."
                .to_owned(),
        );
    }
    if session.image.ndef_area().is_none() {
        return Err("The NDEF data area is not completely readable.".to_owned());
    }
    if !session.image.cc.writable() {
        return Err("The capability container marks the NDEF data area read-only.".to_owned());
    }
    let locked = type2_ui::ndef_locked_pages(session)?;
    if !locked.is_empty() {
        return Err(format!(
            "Pages {} are locked read-only.",
            locked
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(page) = type2_ui::ndef_password_page(session) {
        return Err(format!(
            "Password protection covers the data area from page {page}; PWD_AUTH is required first."
        ));
    }
    Ok(())
}

fn type2_ndef_gate(session: &Type2Session) -> std::result::Result<(), String> {
    type2_format_gate(session)?;
    let ndef = session.ndef().map_err(str::to_owned)?;
    if ndef.read_only {
        return Err("The capability container marks the NDEF data area read-only.".to_owned());
    }
    if !matches!(ndef.state, NdefState::Empty | NdefState::Message(_)) {
        return Err("Not NDEF formatted · Format is required before writing.".to_owned());
    }
    Ok(())
}

fn update_type2_composer(ui: &Ui, session: &Type2Session) {
    let view = &ui.ndef_view;
    view.read_only.set_visible(false);
    let draft = view.draft.borrow();
    let encoded = encoded_draft(&draft);
    let ndef = session.ndef().ok();
    let capacity = ndef.map(|ndef| ndef.capacity);
    let write_gate = type2_ndef_gate(session);
    let format_ready = ui.live.get() && !ui.busy.get() && type2_format_gate(session).is_ok();
    let same = ndef
        .and_then(Type2Ndef::message)
        .zip(encoded.as_ref().ok())
        .is_some_and(|(stored, (message, _))| stored == message);

    let mut ready = false;
    let mut feedback_error = false;
    let feedback = if !ui.live.get() {
        "Read a live Type 2 tag to write or format.".to_owned()
    } else if ui.busy.get() {
        "An operation is running.".to_owned()
    } else if let Err(error) = &write_gate {
        feedback_error = !error.contains("Format is required");
        error.clone()
    } else {
        match &encoded {
            Err(error) => {
                feedback_error = true;
                error.clone()
            }
            Ok((_, bytes)) => match capacity {
                Some(capacity) => match classic::ndef_capacity_error(bytes.len(), capacity) {
                    Some(error) => {
                        feedback_error = true;
                        error
                    }
                    None if same => "The NDEF area already stores this message.".to_owned(),
                    None => {
                        ready = true;
                        format!("Ready to write {} bytes.", bytes.len())
                    }
                },
                None => "Capacity is unavailable; read the tag again.".to_owned(),
            },
        }
    };
    view.preview
        .set_localized(&ndef_preview(&draft, &encoded, capacity));
    view.feedback.set_localized(&feedback);
    if feedback_error {
        view.feedback.add_css_class("error");
    } else {
        view.feedback.remove_css_class("error");
    }
    view.write.set_sensitive(ready);
    view.format.set_sensitive(format_ready);
    view.export
        .set_sensitive(session.ndef().ok().and_then(Type2Ndef::message).is_some());
}

fn confirm_type2_ndef_write(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "NDEF writes need a live selected card");
        return;
    }
    let Some(session) = ui.type2.borrow().clone() else {
        toast_error(&ui, "Read a Type 2 tag first");
        return;
    };
    if let Err(error) = type2_ndef_gate(&session) {
        toast_error(&ui, &error);
        return;
    }
    let Some(ndef) = session.ndef().ok() else {
        toast_error(&ui, "The NDEF state is unavailable");
        return;
    };
    if !matches!(ndef.state, NdefState::Empty | NdefState::Message(_)) {
        toast_error(&ui, "Not NDEF formatted; format the data area first");
        return;
    }
    let draft = ui.ndef_view.draft.borrow().clone();
    let (message, bytes) = match encoded_draft(&draft) {
        Ok(encoded) => encoded,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    if let Some(error) = classic::ndef_capacity_error(bytes.len(), ndef.capacity) {
        toast_error(&ui, &error);
        return;
    }
    if ndef.message() == Some(&message) {
        toast_error(&ui, "The NDEF area already stores this message");
        return;
    }
    let body = format!(
        "{} · UID {}\n{}\nMessage {} bytes · capacity {} bytes\nNew bytes  {}\n\nA project backup is written before the write, and the NDEF Message TLV is read back afterwards.",
        session.image.layout.product.name(),
        session.card.uid_hex(),
        draft.summary(),
        bytes.len(),
        ndef.capacity,
        classic::hex_preview(&bytes, 48)
    );
    let dialog = adw::AlertDialog::builder()
        .heading("Write NDEF message?")
        .body(&body)
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "write", "Write NDEF");
    dialog.set_response_appearance("write", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_ndef_write_dialog.replace(None);
        if response == "write" {
            start_type2_ndef_write(ui2.clone(), message.clone());
        }
    });
    ui.pending_ndef_write_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_type2_ndef_write(ui: Rc<Ui>, message: NdefMessage) {
    set_busy(&ui, true);
    ui.status.set_localized("Writing NDEF message…");
    let service = ui.service.clone();
    let backup = ui.backup.borrow().clone();
    let expected = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = Type2NdefWrite::new(message, &backup);
                if let Some(card) = expected {
                    request = request.expected_card(card);
                }
                service.type2_ndef_write(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_type2_ndef_write(&ui, result),
            Err(_) => {
                finish_type2_ndef_write(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_type2_ndef_write(ui: &Rc<Ui>, result: NfcResult<Type2MutationResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let backup = result.backup.path.display().to_string();
            apply_type2_image(ui, result.image, result.warnings);
            ui.status.set_localized(&format!(
                "NDEF written · {} page(s)",
                result.summary.written_pages
            ));
            append_log(
                ui,
                &format!(
                    "type2 ndef write · {} page(s), {} unchanged · {backup}",
                    result.summary.written_pages, result.summary.unchanged_pages
                ),
            );
            toast_info(ui, "NDEF written and read back");
        }
        Err(error) => {
            ui.status.set_localized("NDEF write failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn confirm_type2_ndef_format(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "NDEF format needs a live selected card");
        return;
    }
    let Some(session) = ui.type2.borrow().clone() else {
        toast_error(&ui, "Read a Type 2 tag first");
        return;
    };
    if let Err(error) = type2_format_gate(&session) {
        toast_error(&ui, &error);
        return;
    }
    let pages = type2_ui::ndef_area_pages(&session.image);
    let page_range = if pages.is_empty() {
        "none".to_owned()
    } else {
        format!("{}..={}", pages[0], pages[pages.len() - 1])
    };
    let area_size = format!("{} bytes", session.image.cc.size);
    let body = format!(
        "{} · UID {}\n\nPages {page_range} ({area_size})\n\nThe NDEF data area is replaced with an empty NDEF Message TLV; proprietary TLVs are removed. A project backup is written first, and every written page is read back.",
        session.image.layout.product.name(),
        session.card.uid_hex(),
    );
    let typed = typed_dialog("Format the NDEF data area?", &body, "Format NDEF area");
    let ui2 = ui.clone();
    let typed2 = typed.clone();
    typed.dialog.connect_response(None, move |_, response| {
        if response != "confirm" || typed2.entry.text() != ADVANCED_PHRASE {
            return;
        }
        ui2.pending_type2_format_dialog.replace(None);
        start_type2_ndef_format(ui2.clone());
    });
    ui.pending_type2_format_dialog.replace(Some(typed.clone()));
    present_alert(&typed.dialog, &ui.window);
}

fn start_type2_ndef_format(ui: Rc<Ui>) {
    set_busy(&ui, true);
    ui.status.set_localized("Formatting the NDEF data area…");
    let service = ui.service.clone();
    let backup = ui.backup.borrow().clone();
    let expected = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.clone());
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = Type2Format::new(&backup).confirm(true);
                if let Some(card) = expected {
                    request = request.expected_card(card);
                }
                service.type2_ndef_format(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_type2_ndef_format(&ui, result),
            Err(_) => {
                finish_type2_ndef_format(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_type2_ndef_format(ui: &Rc<Ui>, result: NfcResult<Type2MutationResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let backup = result.backup.path.display().to_string();
            apply_type2_image(ui, result.image, result.warnings);
            ui.status.set_localized(&format!(
                "NDEF formatted · {} page(s)",
                result.summary.written_pages
            ));
            append_log(
                ui,
                &format!(
                    "type2 ndef format · {} page(s), {} unchanged · {backup}",
                    result.summary.written_pages, result.summary.unchanged_pages
                ),
            );
            toast_info(ui, "NDEF data area formatted and read back");
        }
        Err(error) => {
            ui.status.set_localized("NDEF format failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn refresh_keys(ui: &Ui) {
    let store = ui.keys.borrow();
    ui.keys_view.rows.remove_all();
    for row in key_rows(&store) {
        ui.keys_view.rows.append(&glib::BoxedAnyObject::new(row));
    }
    let keys_path = ui.keys_path.borrow();
    let location = keys_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "no store path".to_owned());
    ui.keys_view
        .status
        .set_localized(&format!("{} key(s) · {}", store.len(), location));
    ui.keys_view.save.set_sensitive(keys_path.is_some());
    let common = if ui.preferences.borrow().use_common_keys {
        format!("{} common", nfc_app::classic::COMMON_KEYS.len())
    } else {
        "common keys off".to_owned()
    };
    ui.key_value.set_localized(&format!(
        "{} saved · {common}\nA + B candidates",
        store.len()
    ));
}

fn save_keys(ui: &Rc<Ui>, notify: bool) {
    let Some(path) = ui.keys_path.borrow().clone() else {
        toast_error(ui, "No key store path; set XDG_CONFIG_HOME or HOME");
        return;
    };
    let result = ui.keys.borrow().save(&path);
    match result {
        Ok(()) => {
            refresh_keys(ui);
            append_log(ui, &format!("keys · saved {}", path.display()));
            if notify {
                toast_info(ui, &format!("Saved {}", path.display()));
            }
        }
        Err(error) => toast_error(ui, &error.to_string()),
    }
}

fn add_key(ui: &Rc<Ui>) {
    let key = match Key::from_hex(&ui.keys_view.hex.text()) {
        Ok(key) => key,
        Err(error) => {
            toast_error(ui, &error.to_string());
            return;
        }
    };
    let sector_text = ui.keys_view.sector.text();
    let sector = if sector_text.trim().is_empty() {
        None
    } else {
        match sector_text.trim().parse::<usize>() {
            Ok(sector) => Some(sector),
            Err(_) => {
                toast_error(ui, "Sector must be a number or empty for all sectors");
                return;
            }
        }
    };
    let kind = match ui.keys_view.kind.selected() {
        1 => KeyKind::A,
        2 => KeyKind::B,
        _ => KeyKind::Any,
    };
    let added = ui
        .keys
        .borrow_mut()
        .add(KeyEntry::new(sector, kind, key, KeySource::Manual));
    match added {
        Ok(true) => {
            ui.keys_view.hex.set_text("");
            append_log(ui, &format!("keys · added {} key", kind.label()));
            save_keys(ui, false);
        }
        Ok(false) => toast_info(ui, "That key entry is already present"),
        Err(error) => toast_error(ui, &error.to_string()),
    }
}

fn import_keys(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Import Classic keys"))
        .build();
    let ui2 = ui.clone();
    dialog.open(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                toast_error(&ui2, &format!("Could not read {}: {error}", path.display()));
                return;
            }
        };
        let added = ui2.keys.borrow_mut().import_text_str(&text);
        match added {
            Ok(added) => {
                append_log(
                    &ui2,
                    &format!("keys · imported {added} key(s) from {}", path.display()),
                );
                save_keys(&ui2, false);
                toast_info(&ui2, &format!("Imported {added} key(s)"));
            }
            Err(error) => toast_error(&ui2, &error.to_string()),
        }
    });
}

fn export_keys(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Export Proxmark3 key dictionary"))
        .initial_name("classic-keys.dic")
        .build();
    let ui2 = ui.clone();
    dialog.save(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        let text = ui2.keys.borrow().export_dic();
        match std::fs::write(&path, text.as_bytes()) {
            Ok(()) => {
                append_log(&ui2, &format!("keys · exported .dic to {}", path.display()));
                toast_info(
                    &ui2,
                    "Exported unique keys; sector, A/B, and provenance are not stored in .dic",
                );
            }
            Err(error) => {
                toast_error(
                    &ui2,
                    &format!("Could not write {}: {error}", path.display()),
                );
            }
        }
    });
}

#[derive(Clone, Copy)]
enum DiffSide {
    Before,
    After,
}

fn choose_diff_dump(ui: &Rc<Ui>, side: DiffSide) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Load Classic dump for diff"))
        .build();
    let ui2 = ui.clone();
    dialog.open(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                toast_error(&ui2, &format!("Could not read {}: {error}", path.display()));
                return;
            }
        };
        match exchange::load(&bytes) {
            Ok(load) => {
                let label = format!(
                    "{}\n{} · {}/{} blocks readable{}",
                    path.display(),
                    load.format.name(),
                    load.dump.readability().readable_blocks,
                    load.dump.layout.block_count(),
                    if load.warnings.is_empty() {
                        String::new()
                    } else {
                        format!(" · {} warning(s)", load.warnings.len())
                    }
                );
                let (target, target_label) = match side {
                    DiffSide::Before => (&ui2.diff_before, &ui2.diff_view.before_label),
                    DiffSide::After => (&ui2.diff_after, &ui2.diff_view.after_label),
                };
                target.replace(Some(load.clone()));
                target_label.set_localized(&label);
                for warning in &load.warnings {
                    append_log(&ui2, &format!("warning · {warning}"));
                }
                ui2.diff_view.compare.set_sensitive(
                    ui2.diff_before.borrow().is_some() && ui2.diff_after.borrow().is_some(),
                );
                append_log(
                    &ui2,
                    &format!("diff · loaded {} · {}", path.display(), load.format.name()),
                );
            }
            Err(error) => toast_error(&ui2, &error.to_string()),
        }
    });
}

fn compare_dumps(ui: &Rc<Ui>) {
    let before = ui
        .diff_before
        .borrow()
        .as_ref()
        .map(|load| load.dump.clone());
    let after = ui
        .diff_after
        .borrow()
        .as_ref()
        .map(|load| load.dump.clone());
    let (Some(before), Some(after)) = (before, after) else {
        toast_error(ui, "Load both images before comparing");
        return;
    };
    match diff::diff(&before, &after) {
        Ok(result) => {
            ui.diff_view.rows.remove_all();
            for row in classic::diff_rows(&result) {
                ui.diff_view.rows.append(&glib::BoxedAnyObject::new(row));
            }
            let mut summary = format!(
                "{} · {} changed · {} equal · {} unknown · {} trailer change(s)",
                classic::display_layout(result.layout),
                result.changed_blocks,
                result.equal_blocks,
                result.unknown_blocks,
                result.trailer_changes
            );
            if result.identity_changed {
                summary.push_str(" · card identity changed");
            }
            ui.diff_view.summary.set_localized(&summary);
            append_log(ui, &format!("diff · {summary}"));
        }
        Err(error) => {
            ui.diff_view.summary.set_localized(&error.to_string());
            toast_error(ui, &error.to_string());
        }
    }
}

fn load_restore_dump(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Load Classic dump to restore"))
        .build();
    let ui2 = ui.clone();
    dialog.open(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                toast_error(&ui2, &format!("Could not read {}: {error}", path.display()));
                return;
            }
        };
        match exchange::load(&bytes) {
            Ok(load) => {
                let readability = load.dump.readability();
                let uid = load
                    .dump
                    .card
                    .as_ref()
                    .map(|card| card.uid_hex())
                    .unwrap_or_else(|| "unknown".to_owned());
                ui2.restore_view
                    .file
                    .set_localized(&path.display().to_string());
                ui2.restore_view.summary.set_localized(&format!(
                    "{} · {} · UID {}\n{}/{} blocks known · {} of {} sectors readable{}",
                    classic::display_layout(load.dump.layout),
                    load.format.name(),
                    uid,
                    readability.readable_blocks,
                    load.dump.layout.block_count(),
                    readability.readable_sectors,
                    readability.total_sectors,
                    if load.warnings.is_empty() {
                        String::new()
                    } else {
                        format!(" · {} warning(s)", load.warnings.len())
                    }
                ));
                ui2.restore_view.result.set_localized("");
                ui2.restore_view.rows.remove_all();
                for warning in &load.warnings {
                    append_log(&ui2, &format!("warning · {warning}"));
                }
                append_log(
                    &ui2,
                    &format!(
                        "restore · loaded {} · {}",
                        path.display(),
                        load.format.name()
                    ),
                );
                ui2.restore_load.replace(Some((path, load)));
                update_restore_sensitivity(&ui2);
            }
            Err(error) => toast_error(&ui2, &error.to_string()),
        }
    });
}

fn update_restore_sensitivity(ui: &Ui) {
    ui.restore_view
        .restore
        .set_sensitive(classic::restore_ready(
            ui.live.get(),
            ui.busy.get(),
            ui.restore_load.borrow().is_some(),
        ));
}

fn confirm_restore(ui: &Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(ui, "Restore needs a live selected card");
        return;
    }
    let source = ui.restore_load.borrow().clone();
    let Some((path, load)) = source else {
        toast_error(ui, "Load a dump before restoring");
        return;
    };
    let known = load
        .dump
        .blocks
        .iter()
        .filter(|block| block.complete())
        .count();
    let body = format!(
        "Load: {}\nImage: {} · {} of {} blocks known\n\nA project backup is written before the write. Data blocks are written before sector trailers, and every block is read back. Block 0 is never written.",
        path.display(),
        classic::display_layout(load.dump.layout),
        known,
        load.dump.layout.block_count()
    );
    let dialog = adw::AlertDialog::builder()
        .heading("Restore dump to the live card?")
        .body(body)
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "restore", "Restore");
    dialog.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_restore_dialog.replace(None);
        if response == "restore" {
            start_restore(&ui2, path.clone(), load.dump.clone());
        }
    });
    ui.pending_restore_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_restore(ui: &Rc<Ui>, path: PathBuf, dump: Dump) {
    let ui = ui.clone();
    set_busy(&ui, true);
    ui.status.set_localized("Restoring dump…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.borrow().clone();
    let continue_on_failure = ui.restore_view.continue_on_error.is_active();
    let expected_card = dump.card.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = ClassicRestore::new(dump, &keys, &backup)
                    .confirmed(true)
                    .continue_on_failure(continue_on_failure);
                if let Some(card) = expected_card {
                    request = request.expected_card(card);
                }
                service.classic_restore(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_restore(&ui, path, result),
            Err(_) => finish_restore(
                &ui,
                path,
                Err(NfcError::Io("reader worker stopped".to_owned())),
            ),
        }
    });
}

fn finish_restore(ui: &Rc<Ui>, path: PathBuf, result: NfcResult<ClassicRestoreResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let backup = result
                .backup
                .as_ref()
                .map(|backup| backup.path.display().to_string())
                .unwrap_or_else(|| "no backup needed".to_owned());
            let mut text = format!(
                "Restored {} block(s), {} trailer(s); {} unchanged, {} unknown skipped.\nBackup: {}",
                result.written_blocks,
                result.trailer_writes,
                result.unchanged_blocks,
                result.skipped_unknown_blocks,
                backup
            );
            for warning in &result.warnings {
                text.push_str("\nWarning: ");
                text.push_str(warning);
                append_log(ui, &format!("warning · {warning}"));
            }
            for failure in &result.failures {
                text.push_str("\nFailure: ");
                text.push_str(failure);
                append_log(ui, &format!("failure · {failure}"));
            }
            ui.restore_view.rows.remove_all();
            for row in restore_rows(&result) {
                ui.restore_view.rows.append(&glib::BoxedAnyObject::new(row));
            }
            ui.restore_view.result.set_localized(&text);
            ui.status
                .set_localized(&format!("Restore complete · {}", path.display()));
            append_log(
                ui,
                &format!(
                    "restore · {} · {} block(s) written",
                    path.display(),
                    result.written_blocks
                ),
            );
            if result.failures.is_empty() {
                toast_info(ui, "Restore complete and read back");
            } else {
                toast_info(ui, "Restore finished with failures; see the Restore screen");
            }
            ui.live.set(false);
            update_inspector(ui);
            update_restore_sensitivity(ui);
        }
        Err(error) => {
            ui.status.set_localized("Restore failed");
            ui.restore_view.rows.remove_all();
            ui.restore_view.result.set_localized(&error.to_string());
            toast_error(ui, &error.to_string());
            update_restore_sensitivity(ui);
        }
    }
}

fn save_dump(ui: &Rc<Ui>) {
    let dump = ui.dump.borrow().clone();
    let Some(dump) = dump else {
        if ui.type2.borrow().is_some() {
            save_type2_dump(ui);
        } else {
            toast_error(ui, "No dump to save");
        }
        return;
    };
    let options = classic::save_format_options(&dump);
    let default_format = ui
        .preferences
        .borrow()
        .dump_format()
        .unwrap_or(DumpFormat::Auto);
    let body = if dump.complete() {
        "Complete image. The configured default is preselected; every format can represent it."
    } else {
        "Partial image. Only the NFC Foundry project format can preserve unknown bytes; raw, Proxmark3 JSON, and MCT are disabled."
    };
    let dialog = adw::AlertDialog::builder()
        .heading("Save Classic dump")
        .body(body)
        .build();
    let list = gtk::Box::new(Orientation::Vertical, 4);
    let mut group: Option<gtk::CheckButton> = None;
    let mut buttons: Vec<(DumpFormat, gtk::CheckButton)> = Vec::new();
    let mut preselected = false;
    for option in options {
        let button = gtk::CheckButton::with_label(option.label);
        if let Some(group) = &group {
            button.set_group(Some(group));
        } else {
            group = Some(button.clone());
        }
        button.set_sensitive(option.enabled);
        if !option.enabled {
            button.set_tooltip_text(Some(
                "A complete image is required; unknown bytes cannot be represented.",
            ));
        }
        if option.format == default_format && option.enabled {
            button.set_active(true);
            preselected = true;
        }
        list.append(&button);
        buttons.push((option.format, button));
    }
    if !preselected
        && let Some((_, button)) = buttons
            .iter()
            .find(|(format, _)| *format == DumpFormat::Auto)
    {
        button.set_active(true);
    }
    dialog.set_extra_child(Some(&list));
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "save", "Choose file…");
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let format = buttons
            .iter()
            .find(|(_, button)| button.is_active())
            .map(|(format, _)| *format)
            .unwrap_or(DumpFormat::Auto);
        choose_save_path(&ui2, &dump, format);
    });
    present_alert(&dialog, &ui.window);
}

fn choose_save_path(ui: &Rc<Ui>, dump: &Dump, format: DumpFormat) {
    let chosen = format.resolve(dump);
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Save Classic dump"))
        .initial_name(chosen.default_file_name())
        .build();
    let ui2 = ui.clone();
    let dump = dump.clone();
    dialog.save(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        match exchange::export(&dump, format) {
            Ok(bytes) => match std::fs::write(&path, &bytes) {
                Ok(()) => {
                    append_log(
                        &ui2,
                        &format!("save · {} · {} format", path.display(), chosen.name()),
                    );
                    toast_info(
                        &ui2,
                        &format!("Saved {} ({})", path.display(), chosen.name()),
                    );
                }
                Err(error) => {
                    toast_error(
                        &ui2,
                        &format!("Could not write {}: {error}", path.display()),
                    );
                }
            },
            Err(error) => toast_error(&ui2, &error.to_string()),
        }
    });
}

fn load_dump(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Load dump"))
        .build();
    let ui2 = ui.clone();
    dialog.open(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        if let Err(error) = load_dump_file(&ui2, &path) {
            toast_error(&ui2, &error);
        }
    });
}

/// Load a Classic or Type 2 project image. The Type 2 format is checked first
/// because both are JSON files with different markers and no shared schema.
fn load_dump_file(ui: &Rc<Ui>, path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let is_type2 = bytes
        .windows(b"nfc-foundry-type2".len())
        .any(|window| window == b"nfc-foundry-type2");
    if is_type2 {
        let image = type2_exchange::load(&bytes).map_err(|error| error.to_string())?;
        let session = Type2Session::from_image(image, Vec::new());
        show_type2_session(ui, session, false);
        ui.status.set_localized(&format!(
            "Loaded {} · Type 2 project · inspection only",
            path.display()
        ));
        append_log(
            ui,
            &format!(
                "load · {} · nfc-foundry-type2 · inspection only",
                path.display()
            ),
        );
        toast_info(
            ui,
            "Type 2 image loaded; writes are disabled without a live card",
        );
        return Ok(());
    }
    let loaded = exchange::load(&bytes).map_err(|error| error.to_string())?;
    ui.type2.replace(None);
    ui.type2_restore.replace(None);
    ui.live.set(false);
    ui.dump.replace(Some(loaded.dump.clone()));
    populate_rows(ui, &loaded.dump);
    ui.views.set_visible_child_name("card");
    update_card(ui);
    update_ndef(ui);
    update_restore_sensitivity(ui);
    render_type2(ui);
    ui.status.set_localized(&format!(
        "Loaded {} · {}",
        path.display(),
        loaded.format.name()
    ));
    append_log(
        ui,
        &format!(
            "load · {} · {} · {} warning(s)",
            path.display(),
            loaded.format.name(),
            loaded.warnings.len()
        ),
    );
    for warning in &loaded.warnings {
        append_log(ui, &format!("warning · {warning}"));
    }
    toast_info(ui, "Dump loaded; writes are disabled without a live card");
    Ok(())
}

fn key_types_from(view: &KeyscanView) -> KeyTypes {
    match view.key_types.selected() {
        1 => KeyTypes::a_only(),
        2 => KeyTypes::b_only(),
        _ => KeyTypes::ALL,
    }
}

fn key_type_from(index: u32) -> nfc_core::KeyType {
    match index {
        1 => nfc_core::KeyType::B,
        _ => nfc_core::KeyType::A,
    }
}

fn keyscan_rows(result: &ClassicKeyscanResult) -> Vec<SectorKeyRow> {
    result
        .sectors
        .iter()
        .map(|sector| SectorKeyRow {
            sector: format!("{:02}", sector.sector),
            key_a: sector
                .key_a
                .map(|key| key.to_hex())
                .unwrap_or_else(|| "—".to_owned()),
            key_b: sector
                .key_b
                .map(|key| key.to_hex())
                .unwrap_or_else(|| "—".to_owned()),
        })
        .collect()
}

fn update_keyscan_sensitivity(ui: &Ui) {
    let busy = ui.busy.get();
    let running = ui.keyscan_cancel.borrow().is_some();
    let has_result = ui.keyscan_result.borrow().is_some();
    let view = &ui.keyscan_view;
    view.start.set_sensitive(!busy);
    view.cancel.set_sensitive(running);
    view.common.set_sensitive(!busy && !running);
    view.key_types.set_sensitive(!busy && !running);
    view.dictionary.set_sensitive(!busy && !running);
    view.choose_dictionary.set_sensitive(!busy && !running);
    view.save.set_sensitive(!busy && has_result);
    view.export.set_sensitive(!busy && has_result);
}

fn choose_keyscan_dictionary(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Choose a Classic key dictionary"))
        .build();
    let ui2 = ui.clone();
    dialog.open(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        ui2.keyscan_view
            .dictionary
            .set_text(&path.display().to_string());
    });
}

fn dictionary_candidates(ui: &Ui) -> std::result::Result<Vec<Key>, String> {
    let path = ui.keyscan_view.dictionary.text();
    let path = path.trim();
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("Could not read {path}: {error}"))?;
    let mut store = KeyStore::new();
    store
        .import_text_str(&text)
        .map_err(|error| error.to_string())?;
    if store.is_empty() {
        return Err(format!("The dictionary {path} contains no keys"));
    }
    Ok(store.all_keys())
}

fn start_keyscan(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let candidates = match dictionary_candidates(&ui) {
        Ok(candidates) => candidates,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let include_common_keys = ui.keyscan_view.common.is_active();
    let key_types = key_types_from(&ui.keyscan_view);
    let token = CancelToken::new();
    let run = ui.keyscan_run.get().wrapping_add(1);
    ui.keyscan_run.set(run);
    ui.keyscan_cancel.replace(Some(token.clone()));
    ui.keyscan_result.replace(None);
    ui.keyscan_view.rows.remove_all();
    ui.keyscan_view.table_stack.set_visible_child_name("empty");
    ui.keyscan_view
        .progress
        .set_localized("Authenticating candidates…");
    ui.keyscan_view
        .summary
        .set_localized("Scanning every sector.");
    set_busy(&ui, true);
    ui.status.set_localized("Scanning keys…");

    let keys = ui.keys.borrow().clone();
    let service = ui.service.clone();
    let (result_sender, result_receiver) = async_channel::bounded(1);
    let (progress_sender, progress_receiver) = async_channel::unbounded();
    let sink: KeyscanSink = Arc::new(move |progress| {
        let _ = progress_sender.try_send(progress);
    });
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.classic_keyscan(
                ClassicKeyscan::new(&keys, candidates)
                    .include_common_keys(include_common_keys)
                    .key_types(key_types)
                    .cancel(token)
                    .progress(Some(sink)),
            ),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = result_sender.send_blocking(result);
    });

    let ui2 = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(progress) = progress_receiver.recv().await {
            if ui2.keyscan_run.get() != run {
                break;
            }
            ui2.keyscan_view
                .progress
                .set_localized(&keyscan_progress_text(progress));
        }
    });
    glib::spawn_future_local(async move {
        match result_receiver.recv().await {
            Ok(result) => finish_keyscan(&ui, result),
            Err(_) => finish_keyscan(&ui, Err(NfcError::Io("key scan worker stopped".to_owned()))),
        }
    });
}

fn keyscan_progress_text(progress: KeyscanProgress) -> String {
    format!(
        "Sector {}/{} · {} attempts · {} verified",
        progress.sector + 1,
        progress.sector_count,
        progress.attempts,
        progress.verified
    )
}

fn finish_keyscan(ui: &Rc<Ui>, result: NfcResult<ClassicKeyscanResult>) {
    ui.keyscan_cancel.replace(None);
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let rows = keyscan_rows(&result);
            for row in &rows {
                ui.keyscan_view
                    .rows
                    .append(&glib::BoxedAnyObject::new(row.clone()));
            }
            ui.keyscan_view.table_stack.set_visible_child_name("table");
            let found = result
                .sectors
                .iter()
                .filter(|sector| sector.key_a.is_some() || sector.key_b.is_some())
                .count();
            ui.keyscan_view.summary.set_localized(&format!(
                "{} sector(s) with a key · {} attempts · {} verified trailer reads",
                found, result.attempts, result.verified
            ));
            ui.keyscan_view.progress.set_localized("Scan complete.");
            ui.status
                .set_localized(&format!("Key scan complete · {found} sector(s)"));
            append_log(
                ui,
                &format!(
                    "keyscan · {found} sector(s) · {} attempts · {} verified",
                    result.attempts, result.verified
                ),
            );
            ui.keyscan_result.replace(Some(result));
            if found == 0 {
                toast_info(ui, "No candidate key authenticated a sector");
            } else {
                toast_info(ui, &format!("Found keys in {found} sector(s)"));
            }
        }
        Err(NfcError::Cancelled) => {
            ui.keyscan_view.progress.set_localized("Scan cancelled.");
            ui.keyscan_view
                .summary
                .set_localized("Cancelled before the next reader exchange.");
            ui.status.set_localized("Key scan cancelled");
            append_log(ui, "keyscan · cancelled");
        }
        Err(error) => {
            ui.keyscan_view.progress.set_localized("Scan failed.");
            ui.keyscan_view.summary.set_localized(&error.to_string());
            ui.status.set_localized("Key scan failed");
            toast_error(ui, &error.to_string());
        }
    }
    update_keyscan_sensitivity(ui);
}

fn cancel_keyscan(ui: &Ui) {
    if let Some(token) = ui.keyscan_cancel.borrow().as_ref() {
        token.cancel();
        ui.keyscan_view
            .progress
            .set_localized("Cancelling after the current exchange…");
    }
}

fn save_keyscan_keys(ui: &Rc<Ui>) {
    let Some(result) = ui.keyscan_result.borrow().clone() else {
        toast_error(ui, "Run a key scan first");
        return;
    };
    if ui.keys_path.borrow().is_none() {
        toast_error(ui, "Set a key store path in Settings first");
        return;
    }
    let added = match ui.keys.borrow_mut().merge(&result.store) {
        Ok(added) => added,
        Err(error) => {
            toast_error(ui, &error.to_string());
            return;
        }
    };
    if added == 0 {
        append_log(ui, "keyscan · no new keys to save");
        toast_info(ui, "Every discovered key is already stored");
        return;
    }
    save_keys(ui, false);
    append_log(ui, &format!("keyscan · merged {added} key(s)"));
    toast_info(ui, &format!("Merged {added} discovered key(s)"));
}

fn export_keyscan_dic(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Export scanned keys"))
        .initial_name("scanned-keys.dic")
        .build();
    let ui2 = ui.clone();
    dialog.save(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        if let Err(error) = write_keyscan_dic(&ui2, &path) {
            toast_error(&ui2, &error);
        }
    });
}

fn write_keyscan_dic(ui: &Rc<Ui>, path: &Path) -> std::result::Result<(), String> {
    let Some(result) = ui.keyscan_result.borrow().clone() else {
        return Err("Run a key scan first".to_owned());
    };
    let text = result.store.export_dic();
    let count = result.store.len();
    std::fs::write(path, text.as_bytes())
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    append_log(
        ui,
        &format!("keyscan · exported {count} key(s) to {}", path.display()),
    );
    toast_info(ui, &format!("Exported {count} key(s)"));
    Ok(())
}

fn update_recovery_sensitivity(ui: &Ui) {
    let busy = ui.busy.get();
    let running = ui.recovery_cancel.borrow().is_some();
    let has_result = ui.recovery_result.borrow().is_some();
    let view = &ui.recovery_view;
    let strategy = recovery_strategy(view.strategy.selected());
    let auto = strategy == RecoveryStrategy::Auto;
    let uses_known = !matches!(strategy, RecoveryStrategy::DarkSide) && !auto;
    let uses_tables = matches!(
        strategy,
        RecoveryStrategy::Auto | RecoveryStrategy::HardNested
    );
    let uses_candidates = !matches!(strategy, RecoveryStrategy::DarkSide);
    let editable = !busy && !running;

    view.strategy.set_sensitive(editable);
    view.advanced.set_sensitive(editable);
    view.start.set_sensitive(!busy);
    view.cancel.set_sensitive(running);
    view.save.set_sensitive(!busy && has_result);
    view.known_row.set_visible(uses_known);
    view.target_row.set_visible(!auto);
    view.known_block.set_sensitive(editable);
    view.known_type.set_sensitive(editable);
    view.target_block.set_sensitive(editable);
    view.target_type.set_sensitive(editable);
    view.tables.set_sensitive(editable && uses_tables);
    view.choose_tables.set_sensitive(editable && uses_tables);
    view.max_nonces.set_sensitive(editable && uses_tables);
    view.common.set_sensitive(editable && uses_candidates);
}

fn choose_recovery_tables(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines(
            "Choose the hardnested table directory",
        ))
        .build();
    let ui2 = ui.clone();
    dialog.select_folder(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local directory");
            return;
        };
        ui2.recovery_view
            .tables
            .set_text(&path.display().to_string());
    });
}

fn parse_block_entry(entry: &gtk::Entry, label: &str) -> std::result::Result<u8, String> {
    let text = entry.text();
    let value = text
        .trim()
        .parse::<u16>()
        .map_err(|_| format!("{label} must be a number in 0..=255"))?;
    u8::try_from(value).map_err(|_| format!("{label} must be in 0..=255"))
}

fn start_recovery(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let view = &ui.recovery_view;
    let strategy = recovery_strategy(view.strategy.selected());
    let auto = strategy == RecoveryStrategy::Auto;
    let known_block = if auto {
        4
    } else {
        match parse_block_entry(&view.known_block, "known block") {
            Ok(block) => block,
            Err(error) => {
                toast_error(&ui, &error);
                return;
            }
        }
    };
    let target_block = if auto {
        0
    } else {
        match parse_block_entry(&view.target_block, "target block") {
            Ok(block) => block,
            Err(error) => {
                toast_error(&ui, &error);
                return;
            }
        }
    };
    let known_key_type = key_type_from(view.known_type.selected());
    let target_key_type = key_type_from(view.target_type.selected());
    let max_nonces = match usize::try_from(view.max_nonces.value_as_int()) {
        Ok(value) => value,
        Err(_) => {
            toast_error(&ui, "Max nonces must be zero or positive");
            return;
        }
    };
    let tables_text = view.tables.text();
    let tables_text = tables_text.trim();
    let tables_dir = if tables_text.is_empty() {
        None
    } else {
        Some(PathBuf::from(tables_text))
    };
    let candidates = ui
        .keys
        .borrow()
        .recovery_candidates(view.common.is_active());
    let candidate_count = candidates.len();

    let token = CancelToken::new();
    let run = ui.recovery_run.get().wrapping_add(1);
    ui.recovery_run.set(run);
    ui.recovery_cancel.replace(Some(token.clone()));
    ui.recovery_result.replace(None);
    let log = &view.log;
    log.clear();
    log.append(
        "info",
        &format!(
            "Start · {} · {candidate_count} candidate key(s)",
            recovery_strategy_label(strategy)
        ),
    );
    view.elapsed.set_localized("Elapsed 0:00");
    ui.recovery_started.replace(Some(Instant::now()));
    start_recovery_timer(&ui);
    set_busy(&ui, true);
    ui.status.set_localized("Recovery running.");

    let service = ui.service.clone();
    let (result_sender, result_receiver) = async_channel::bounded(1);
    let (progress_sender, progress_receiver) = async_channel::unbounded();
    let sink: ProgressSink = Arc::new(move |progress| {
        let _ = progress_sender.try_send(progress);
    });
    std::thread::spawn(move || {
        let request = RecoveryRequest {
            known_block,
            known_key_type,
            target_block,
            target_key_type,
            candidates,
            cancel: token,
            progress: Some(sink),
            tables_dir,
            max_nonces,
        };
        let result = match service.lock() {
            Ok(mut service) => service.recover(strategy, request),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = result_sender.send_blocking(result);
    });

    let ui2 = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(progress) = progress_receiver.recv().await {
            if ui2.recovery_run.get() != run {
                break;
            }
            let text = recovery_progress_text(progress);
            if recovery_progress_is_live(progress) {
                ui2.recovery_view.log.live("info", &text);
            } else {
                ui2.recovery_view.log.append("info", &text);
            }
        }
    });
    glib::spawn_future_local(async move {
        match result_receiver.recv().await {
            Ok(result) => finish_recovery(&ui, result, strategy, target_block, target_key_type),
            Err(_) => finish_recovery(
                &ui,
                Err(NfcError::Io("recovery worker stopped".to_owned())),
                strategy,
                target_block,
                target_key_type,
            ),
        }
    });
}

fn start_recovery_timer(ui: &Rc<Ui>) {
    if let Some(timer) = ui.recovery_timer.borrow_mut().take() {
        timer.remove();
    }
    let ui2 = ui.clone();
    let source = glib::timeout_add_seconds_local(1, move || {
        if let Some(started) = *ui2.recovery_started.borrow() {
            ui2.recovery_view
                .elapsed
                .set_localized(&format!("Elapsed {}", format_elapsed(started.elapsed())));
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
    ui.recovery_timer.replace(Some(source));
}

fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn recovery_progress_text(progress: RecoveryProgress) -> String {
    match progress {
        RecoveryProgress::Dictionary {
            sector,
            sector_count,
            found,
        } => format!(
            "Dictionary · sector {}/{sector_count} · {found} key(s) verified",
            sector + 1
        ),
        RecoveryProgress::Darkside { attempts } => {
            format!("Darkside · {attempts} probes")
        }
        RecoveryProgress::Sector {
            sector,
            sector_count,
            hardnested,
        } => format!(
            "{} · sector {}/{sector_count}",
            if hardnested { "Hardnested" } else { "Nested" },
            sector + 1
        ),
        RecoveryProgress::Verify {
            sector,
            sector_count,
        } => format!("Read verification · sector {}/{sector_count}", sector + 1),
        RecoveryProgress::Acquiring {
            received,
            distinct_nonces,
            first_bytes,
            expected_states,
        } => format!(
            "Acquiring · {received} frames · {distinct_nonces} distinct nonces · {first_bytes}/256 first bytes · {expected_states:.3e} states"
        ),
        RecoveryProgress::Filtering { expected_states } => {
            format!("Filtering · {expected_states:.3e} expected states")
        }
        RecoveryProgress::BruteForce {
            tested_states,
            total_states,
            expected_states,
        } => {
            let percent = tested_states
                .saturating_mul(100)
                .checked_div(total_states)
                .unwrap_or(0);
            format!(
                "Brute force · {percent}% ({tested_states}/{total_states}) · {expected_states:.3e} expected states"
            )
        }
    }
}

fn recovery_progress_is_live(progress: RecoveryProgress) -> bool {
    matches!(
        progress,
        RecoveryProgress::Darkside { .. }
            | RecoveryProgress::Verify { .. }
            | RecoveryProgress::Acquiring { .. }
            | RecoveryProgress::Filtering { .. }
            | RecoveryProgress::BruteForce { .. }
    )
}

fn finish_recovery(
    ui: &Rc<Ui>,
    result: NfcResult<RecoveryResult>,
    strategy: RecoveryStrategy,
    target_block: u8,
    target_key_type: nfc_core::KeyType,
) {
    let elapsed = ui
        .recovery_started
        .borrow()
        .map(|started| started.elapsed())
        .unwrap_or_default();
    ui.recovery_started.replace(None);
    if let Some(timer) = ui.recovery_timer.borrow_mut().take() {
        timer.remove();
    }
    ui.recovery_cancel.replace(None);
    set_busy(ui, false);
    ui.recovery_view
        .elapsed
        .set_localized(&format!("Elapsed {}", format_elapsed(elapsed)));
    match result {
        Ok(result) => {
            let log = &ui.recovery_view.log;
            log.end_live();
            if result.sector_keys.is_empty() {
                for (key_type, key) in &result.keys {
                    log.append(
                        "success",
                        &format!(
                            "Verified {} · {} · verified",
                            key_type.label(),
                            key.to_hex()
                        ),
                    );
                }
            } else {
                for entry in &result.sector_keys {
                    log.append(
                        "success",
                        &format!(
                            "Verified sector {} · {} · {} · verified",
                            entry.sector,
                            entry.key_type.label(),
                            entry.key.to_hex()
                        ),
                    );
                }
            }
            let attempts = result
                .attempts
                .iter()
                .map(|kind| format!("{kind:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            log.append(
                "success",
                &format!(
                    "Complete · {} · {attempts} · {} key(s) verified",
                    recovery_strategy_label(strategy),
                    result.keys.len()
                ),
            );
            if let Some(profile) = &result.profile {
                log.append(
                    "info",
                    &format!(
                        "Profile · PRNG {:?} · known key {}",
                        profile.prng, profile.has_known_key
                    ),
                );
            }
            if !result.note.is_empty() {
                log.append("info", &result.note);
            }
            ui.status.set_localized(&format!(
                "Recovery complete · {} key(s) verified",
                result.keys.len()
            ));
            append_log(
                ui,
                &format!(
                    "recovery · {} · {} key(s) verified",
                    recovery_strategy_label(strategy),
                    result.keys.len()
                ),
            );
            ui.recovery_result.replace(Some(RecoveryOutcome {
                result,
                target_block,
                target_key_type,
            }));
        }
        Err(NfcError::Cancelled) => {
            ui.recovery_view
                .log
                .append("warning", "Cancelled before the next reader exchange.");
            ui.status.set_localized("Recovery cancelled");
            append_log(ui, "recovery · cancelled");
        }
        Err(error) => {
            let message = error.to_string();
            let limitation = matches!(error, NfcError::Unsupported(_))
                || message.to_ascii_lowercase().contains("hardnested table");
            let tag = if limitation { "warning" } else { "error" };
            ui.recovery_view.log.end_live();
            ui.recovery_view.log.append(tag, &message);
            if limitation {
                ui.status.set_localized("Recovery limitation");
                append_log(ui, &format!("recovery · limitation · {message}"));
            } else {
                ui.status.set_localized("Recovery failed");
                append_log(ui, &format!("recovery · failed · {message}"));
            }
        }
    }
    update_recovery_sensitivity(ui);
}

fn cancel_recovery(ui: &Ui) {
    if let Some(token) = ui.recovery_cancel.borrow().as_ref() {
        token.cancel();
        ui.recovery_view
            .log
            .append("warning", "Cancelling after the current exchange…");
    }
}

fn save_recovered_key(ui: &Rc<Ui>) {
    let Some(outcome) = ui.recovery_result.borrow().clone() else {
        toast_error(ui, "No recovered key to save");
        return;
    };
    if ui.keys_path.borrow().is_none() {
        toast_error(ui, "Set a key store path in Settings first");
        return;
    }
    let entries: Vec<(Option<usize>, nfc_core::KeyType, nfc_core::Key)> =
        if outcome.result.sector_keys.is_empty() {
            let sector = ui.dump.borrow().as_ref().and_then(|dump| {
                dump.layout
                    .sector_for_block(outcome.target_block as usize)
                    .ok()
            });
            outcome
                .result
                .keys
                .iter()
                .map(|(key_type, key)| (sector, *key_type, *key))
                .collect()
        } else {
            outcome
                .result
                .sector_keys
                .iter()
                .map(|entry| (Some(entry.sector), entry.key_type, entry.key))
                .collect()
        };
    let mut added = 0;
    for (sector, key_type, key) in entries {
        let kind = match key_type {
            nfc_core::KeyType::A => KeyKind::A,
            nfc_core::KeyType::B => KeyKind::B,
        };
        let entry = KeyEntry::new(sector, kind, key, KeySource::Recovery);
        match ui.keys.borrow_mut().add(entry) {
            Ok(true) => added += 1,
            Ok(false) => {}
            Err(error) => {
                toast_error(ui, &error.to_string());
                return;
            }
        }
    }
    if added == 0 {
        append_log(ui, "recovery · key already stored");
        toast_info(ui, "The recovered key is already in the store");
        return;
    }
    save_keys(ui, false);
    let detail = if outcome.result.sector_keys.is_empty() {
        outcome.target_key_type.label().to_string()
    } else {
        "per-sector".to_owned()
    };
    append_log(ui, &format!("recovery · saved {added} key(s) · {detail}"));
    toast_info(ui, &format!("Saved {added} recovered key(s)"));
}

fn render_diagnostics(ui: &Ui) {
    let version = env!("CARGO_PKG_VERSION");
    let view = &ui.diagnostics_view;
    view.version
        .set_localized(&format!("NFC Foundry {version} · nfctl/service {version}"));
    view.drivers.set_localized(&format!(
        "Supported readers: {}<port> · {}<bus>:<address>",
        nfc_drivers::PN532_UART_PREFIX,
        nfc_drivers::ACR122_USB_PREFIX
    ));
    let tables_dir = ui.preferences.borrow().hardnested_tables.clone();
    let tables = match nfc_recover::hardnested::tables::resolve_dir(tables_dir.as_deref()) {
        Ok(path) => format!("Hardnested tables: {}", path.display()),
        Err(error) => format!("Hardnested tables: not resolved ({error})"),
    };
    view.tables.set_localized(&tables);
    let devices = ui.devices.borrow();
    let readers = if devices.is_empty() {
        "Readers: none found".to_owned()
    } else {
        let mut text = format!("Readers: {}", devices.len());
        for device in devices.iter() {
            text.push_str(&format!("\n{}\t{}", device.name, device.conn_string));
        }
        text
    };
    view.readers.set_localized(&readers);
    view.build.set_localized(&format!(
        "Build: {} · {} · {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
}

#[derive(Clone, Copy)]
enum SettingsPath {
    Backup,
    Tables,
}

fn choose_settings_file(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Choose the Classic key store"))
        .initial_name("classic-keys.json")
        .build();
    let ui2 = ui.clone();
    dialog.save(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local file");
            return;
        };
        ui2.settings_view
            .key_store
            .set_text(&path.display().to_string());
    });
}

fn choose_settings_folder(ui: &Rc<Ui>, kind: SettingsPath) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines(match kind {
            SettingsPath::Backup => "Choose the backup directory",
            SettingsPath::Tables => "Choose the hardnested table directory",
        }))
        .build();
    let ui2 = ui.clone();
    dialog.select_folder(Some(&ui.window), None::<&gio::Cancellable>, move |result| {
        let file = match result {
            Ok(file) => file,
            Err(error) => {
                if !error.matches(gtk::DialogError::Dismissed) {
                    toast_error(&ui2, &error.to_string());
                }
                return;
            }
        };
        let Some(path) = file.path() else {
            toast_error(&ui2, "The selected location is not a local directory");
            return;
        };
        let entry = match kind {
            SettingsPath::Tables => &ui2.settings_view.tables,
            SettingsPath::Backup => &ui2.settings_view.backup_dir,
        };
        entry.set_text(&path.display().to_string());
    });
}

fn render_settings(ui: &Ui) {
    let preferences = ui.preferences.borrow().clone();
    let view = &ui.settings_view;
    view.key_store.set_text(
        &preferences
            .key_store
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    );
    view.backup_dir.set_text(
        &preferences
            .backup_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    );
    view.tables.set_text(
        &preferences
            .hardnested_tables
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    );
    view.common.set_active(preferences.use_common_keys);
    view.dump_format.set_selected(dump_format_index(
        preferences.dump_format().unwrap_or(DumpFormat::Auto),
    ));
    ui.language_updating.set(true);
    view.language
        .set_selected(language_index(preferences.language()));
    ui.language_updating.set(false);
    view.config_path.set_localized(&format!(
        "Preference file: {}",
        ui.preferences_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "unavailable (no XDG config directory)".to_owned())
    ));
    if let Some(error) = ui.settings_error.borrow().as_ref() {
        view.status.set_localized(&format!("Not loaded: {error}"));
    } else {
        view.status.set_localized("");
    }
}

fn entry_path(entry: &gtk::Entry) -> Option<PathBuf> {
    let text = entry.text();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

/// Applies the language immediately to the built window, then persists the
/// choice. Page, reader, card image, table selection, and open form values are
/// widget state and are not rebuilt.
fn switch_language(ui: &Rc<Ui>, language: Language) {
    if i18n::current_language() == language {
        return;
    }
    i18n::set_current_language(language);
    let status = {
        let mut preferences = ui.preferences.borrow_mut();
        preferences.set_language(language);
        match ui.preferences_path.as_ref() {
            Some(path) => match preferences.save(path) {
                Ok(()) => format!("Saved {}", path.display()),
                Err(error) => {
                    drop(preferences);
                    i18n::localize_widget_tree(&ui.window);
                    ui.settings_view
                        .status
                        .set_localized(&format!("Not saved: {error}"));
                    toast_error(ui, &error.to_string());
                    return;
                }
            },
            None => "No configuration path; set XDG_CONFIG_HOME or HOME".to_owned(),
        }
    };
    i18n::localize_widget_tree(&ui.window);
    ui.settings_view.status.set_localized(&status);
    if ui.preferences_path.is_none() {
        toast_error(ui, "No configuration path; set XDG_CONFIG_HOME or HOME");
    }
}

fn save_settings(ui: &Rc<Ui>) {
    let Some(path) = ui.preferences_path.clone() else {
        toast_error(ui, "No configuration path; set XDG_CONFIG_HOME or HOME");
        return;
    };
    let view = &ui.settings_view;
    let format = DUMP_FORMAT_CHOICES
        .get(view.dump_format.selected() as usize)
        .map(|(format, _)| *format)
        .unwrap_or(DumpFormat::Auto);
    let mut preferences = UiPreferences {
        key_store: entry_path(&view.key_store),
        backup_dir: entry_path(&view.backup_dir),
        hardnested_tables: entry_path(&view.tables),
        use_common_keys: view.common.is_active(),
        default_dump_format: None,
        language: None,
    };
    preferences.set_dump_format(format);
    preferences.set_language(language_at(view.language.selected()));
    if let Err(error) = preferences.validate() {
        view.status.set_localized(&format!("Not saved: {error}"));
        toast_error(ui, &error.to_string());
        return;
    }
    if let Err(error) = preferences.save(&path) {
        view.status.set_localized(&format!("Not saved: {error}"));
        toast_error(ui, &error.to_string());
        return;
    }
    let keys_path = preferences
        .key_store
        .clone()
        .or_else(KeyStore::default_path);
    let reloaded = match &keys_path {
        Some(path) => KeyStore::load_or_default(path),
        None => Ok(KeyStore::new()),
    };
    let keys = match reloaded {
        Ok(keys) => keys,
        Err(error) => {
            view.status.set_localized(&format!(
                "Saved preferences but the key store failed: {error}"
            ));
            toast_error(ui, &error.to_string());
            return;
        }
    };
    ui.keys.replace(keys);
    ui.keys_path.replace(keys_path);
    *ui.backup.borrow_mut() = preferences
        .backup_dir
        .clone()
        .map(BackupStore::new)
        .unwrap_or_default();
    ui.keyscan_view
        .common
        .set_active(preferences.use_common_keys);
    ui.recovery_view
        .common
        .set_active(preferences.use_common_keys);
    if let Some(tables) = &preferences.hardnested_tables {
        ui.recovery_view
            .tables
            .set_text(&tables.display().to_string());
    } else {
        ui.recovery_view.tables.set_text("");
    }
    ui.preferences.replace(preferences);
    ui.settings_error.replace(None);
    refresh_keys(ui);
    render_diagnostics(ui);
    view.status
        .set_localized(&format!("Saved {}", path.display()));
    ui.status.set_localized("Preferences saved");
    append_log(ui, &format!("settings · saved {}", path.display()));
    toast_info(ui, &format!("Saved {}", path.display()));
}

fn set_busy(ui: &Ui, busy: bool) {
    ui.busy.set(busy);
    if busy {
        ui.spinner.set_visible(true);
        ui.spinner.start();
    } else {
        ui.spinner.stop();
        ui.spinner.set_visible(false);
    }
    ui.read_button.set_sensitive(!busy);
    ui.reader_combo.set_sensitive(!busy);
    update_inspector(ui);
    update_restore_sensitivity(ui);
    update_ndef_actions(ui);
    update_type2_actions(ui);
    update_keyscan_sensitivity(ui);
    update_recovery_sensitivity(ui);
}

fn toast_info(ui: &Rc<Ui>, message: &str) {
    ui.toast
        .add_toast(adw::Toast::new(&i18n::translate_lines(message)));
}

fn toast_error(ui: &Rc<Ui>, message: &str) {
    append_log(ui, &format!("error · {message}"));
    ui.toast
        .add_toast(adw::Toast::new(&i18n::translate_lines(message)));
}

fn add_response(dialog: &adw::AlertDialog, id: &str, label: &str) {
    dialog.add_response(id, &i18n::translate_lines(label));
}

fn present_alert(dialog: &adw::AlertDialog, window: &adw::ApplicationWindow) {
    i18n::localize_alert(dialog);
    dialog.present(Some(window));
}

fn append_log(ui: &Ui, message: &str) {
    let time = activity_time();
    let (operation, result, detail) = split_log_message(message);
    let row = ActivityRow {
        time: time.clone(),
        operation,
        result,
        detail,
    };
    ui.activity_view
        .rows
        .append(&glib::BoxedAnyObject::new(row.clone()));
    ui.activity.borrow_mut().push(row);
}

fn split_log_message(message: &str) -> (String, String, String) {
    let mut fields = message.splitn(3, " · ");
    let operation = fields.next().unwrap_or("event").trim().to_owned();
    let second = fields.next().map(str::trim);
    let third = fields.next().map(str::trim);
    match (second, third) {
        (None, _) => (operation, String::new(), String::new()),
        (Some(value), None) if matches!(operation.as_str(), "warning" | "error") => {
            let kind = operation.clone();
            (operation, kind, value.to_owned())
        }
        (Some(value), None) => (operation, value.to_owned(), String::new()),
        (Some(value), Some(rest)) => (operation, value.to_owned(), rest.to_owned()),
    }
}

fn activity_time() -> String {
    glib::DateTime::now_local()
        .and_then(|date| date.format("%H:%M:%S"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "—".to_owned())
}

fn clear_activity(ui: &Rc<Ui>) {
    ui.activity.borrow_mut().clear();
    ui.activity_view.rows.remove_all();
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nfc_classic::{Layout, bcc_for_uid};
    use nfc_classic_ndef::{
        MAD_ACCESS_BITS, MAD_KEY_A, MAD1_GPB, Mad, MadVersion, NFC_GPB_RO, NFC_GPB_RW, NFC_KEY_A,
        NFC_RO_ACCESS_BITS, NFC_RW_ACCESS_BITS, format_ndef_area,
    };
    use nfc_core::{CardInfo, CardType};
    use nfc_transport::mock::{MockHandle, MockTransport};

    use super::*;

    const TEST_UID: [u8; 4] = [0x04, 0xA2, 0x24, 0x8A];
    const TYPE2_UID: [u8; 7] = [0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80];

    fn test_card() -> CardInfo {
        CardInfo::new(
            TEST_UID.to_vec(),
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )
    }

    fn type2_card(uid: u8) -> CardInfo {
        let mut bytes = TYPE2_UID.to_vec();
        bytes[6] = uid;
        CardInfo::new(bytes, [0x00, 0x44], 0x00, CardType::Type2)
    }

    fn other_type2_card() -> CardInfo {
        type2_card(0x81)
    }

    /// A complete NTAG213 image with an empty formatted NDEF area.
    fn ntag_pages(read_only: bool, locked: bool, formatted: bool) -> Vec<[u8; 4]> {
        let uid = TYPE2_UID;
        let mut pages = vec![[0u8; 4]; 45];
        pages[0] = [uid[0], uid[1], uid[2], uid[0] ^ uid[1] ^ uid[2] ^ 0x88];
        pages[1] = [uid[3], uid[4], uid[5], uid[6]];
        pages[2] = [
            uid[3] ^ uid[4] ^ uid[5] ^ uid[6],
            0x48,
            if locked { 0x10 } else { 0x00 },
            0x00,
        ];
        pages[3] = [0xE1, 0x10, 0x12, if read_only { 0xF0 } else { 0x00 }];
        let mut area = [0u8; 144];
        if formatted {
            area[..3].copy_from_slice(&[0x03, 0x00, 0xFE]);
        }
        for index in 0..36 {
            pages[4 + index].copy_from_slice(&area[index * 4..index * 4 + 4]);
        }
        pages[40] = [0x00, 0x00, 0x00, 0xBD];
        pages[41] = [0x00, 0x00, 0x00, 0xFF];
        pages[42] = [0x00, 0x00, 0x00, 0x00];
        pages
    }

    fn install_ntag(handle: &MockHandle, pages: &[[u8; 4]]) {
        handle.set_card(Some(type2_card(0x80)));
        handle.clear_pages();
        for (page, data) in pages.iter().enumerate() {
            handle.set_page(page as u8, *data);
        }
    }

    fn choose_reader(harness: &Harness, id: &str) {
        harness
            .ui
            .service
            .lock()
            .expect("service")
            .select_device(id)
            .expect("select reader");
        let index = harness
            .ui
            .devices
            .borrow()
            .iter()
            .position(|device| device.conn_string == id)
            .expect("device index") as u32;
        harness.ui.reader_combo.set_selected(index);
    }

    fn read_type2(harness: &Harness, pages: &[[u8; 4]]) {
        reset(harness);
        install_ntag(&harness.ntag, pages);
        choose_reader(harness, "mock:ntag");
        start_read(harness.ui.clone());
        pump_until(&harness.ui, "Type 2 read", || {
            !harness.ui.busy.get() && harness.ui.status.label().contains("Read complete")
        });
        assert!(harness.ui.type2.borrow().is_some());
    }

    fn typed_pending(slot: &RefCell<Option<TypedDialog>>) -> TypedDialog {
        slot.borrow().clone().expect("typed dialog")
    }

    fn other_card() -> CardInfo {
        CardInfo::new(vec![0x55; 4], [0x00, 0x04], 0x08, CardType::MifareClassic1K)
    }

    fn image() -> Vec<[u8; 16]> {
        let mut blocks = vec![[0u8; 16]; 64];
        for (number, block) in blocks.iter_mut().enumerate() {
            *block = [number as u8; 16];
        }
        blocks[0] = [0u8; 16];
        blocks[0][0..4].copy_from_slice(&TEST_UID);
        blocks[0][4] = bcc_for_uid(&TEST_UID).expect("bcc");
        blocks[0][5] = 0x08;
        blocks[0][6..8].copy_from_slice(&[0x00, 0x04]);
        for sector in 0..16 {
            let mut trailer = [0xFF; 16];
            trailer[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
            trailer[9] = 0x69;
            blocks[sector * 4 + 3] = trailer;
        }
        blocks
    }

    fn fixture_dump() -> Dump {
        dump_for(&image())
    }

    fn dump_for(blocks: &[[u8; 16]]) -> Dump {
        let mut raw = Vec::new();
        for block in blocks {
            raw.extend_from_slice(block);
        }
        Dump::from_raw(&raw)
            .expect("fixture image")
            .with_card(test_card())
    }

    const NDEF_SECTORS: [u8; 2] = [1, 2];

    /// A 1K Classic image with a MAD1 directory and NDEF sectors 1 and 2. The
    /// data area is formatted with an empty mandatory NDEF Message TLV.
    fn formatted_image(read_only: bool) -> Vec<[u8; 16]> {
        let layout = Layout::Classic1K;
        let mut blocks = image();

        let mad = Mad::blank(layout, MadVersion::Mad1, &NDEF_SECTORS).expect("blank MAD");
        for (block, data) in mad.encode(layout).expect("encode MAD") {
            blocks[block] = data;
        }
        let mut mad_trailer = [0xFF; 16];
        mad_trailer[..6].copy_from_slice(&MAD_KEY_A);
        mad_trailer[6..9].copy_from_slice(&MAD_ACCESS_BITS);
        mad_trailer[9] = MAD1_GPB;
        blocks[3] = mad_trailer;

        let area = format_ndef_area(NDEF_SECTORS.len() * 3 * 16).expect("NDEF area");
        let mut offset = 0;
        for sector in NDEF_SECTORS {
            let first = usize::from(sector) * 4;
            for (index, block) in blocks[first..first + 3].iter_mut().enumerate() {
                let start = offset + index * 16;
                block.copy_from_slice(&area[start..start + 16]);
            }
            offset += 3 * 16;
            let mut trailer = [0xFF; 16];
            trailer[..6].copy_from_slice(&NFC_KEY_A);
            trailer[6..9].copy_from_slice(if read_only {
                &NFC_RO_ACCESS_BITS
            } else {
                &NFC_RW_ACCESS_BITS
            });
            trailer[9] = if read_only { NFC_GPB_RO } else { NFC_GPB_RW };
            blocks[first + 3] = trailer;
        }
        blocks
    }

    fn install_image(harness: &Harness, blocks: &[[u8; 16]]) {
        harness.handle.set_card(Some(test_card()));
        for (block, data) in blocks.iter().enumerate() {
            harness.handle.set_block(block as u8, *data);
        }
    }

    fn show_image(ui: &Rc<Ui>, dump: &Dump) {
        ui.dump.replace(Some(dump.clone()));
        populate_rows(ui, dump);
        ui.live.set(true);
        update_inspector(ui);
        update_ndef(ui);
        update_restore_sensitivity(ui);
    }

    struct Harness {
        app: adw::Application,
        ui: Rc<Ui>,
        handle: MockHandle,
        ntag: MockHandle,
        keys_dir: tempfile::TempDir,
        backups: tempfile::TempDir,
    }

    fn harness() -> Harness {
        gtk::init().expect("GTK display");
        adw::init().expect("libadwaita");
        let app = adw::Application::builder()
            .application_id("io.github.nfc_foundry.NfcFoundry.GuiTest")
            .build();
        app.register(None::<&gio::Cancellable>)
            .expect("register app");

        let transport = MockTransport::new();
        let handle = transport.add(
            DeviceInfo::new("Mock Classic", "mock:classic"),
            Some(test_card()),
        );
        for (block, data) in image().into_iter().enumerate() {
            handle.set_block(block as u8, data);
        }
        let ntag = transport.add(
            DeviceInfo::new("Mock NTAG", "mock:ntag"),
            Some(type2_card(0x80)),
        );
        install_ntag(&ntag, &ntag_pages(false, false, true));
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("enumerate");
        service.select_device("mock:classic").expect("select");

        let keys_dir = tempfile::tempdir().expect("keys dir");
        let backups = tempfile::tempdir().expect("backup dir");
        let keys_path = keys_dir.path().join("classic-keys.json");
        let preferences_path = keys_dir.path().join("ui.json");
        let mut preferences = UiPreferences::default();
        preferences.set_language(Language::En);
        let ui = build_ui_with(
            &app,
            Workbench {
                service: Arc::new(Mutex::new(service)),
                keys: KeyStore::new(),
                keys_path: Some(keys_path),
                backup: BackupStore::new(backups.path()),
                key_error: None,
                preferences,
                preferences_path: Some(preferences_path),
                settings_error: None,
            },
        );
        Harness {
            app,
            ui,
            handle,
            ntag,
            keys_dir,
            backups,
        }
    }

    fn close_alert(slot: &RefCell<Option<adw::AlertDialog>>) {
        if let Some(dialog) = slot.borrow_mut().take() {
            dialog.force_close();
        }
    }

    fn close_dialogs(widget: &impl IsA<gtk::Widget>) {
        let widget = widget.as_ref();
        if let Some(dialog) = widget.downcast_ref::<adw::AlertDialog>() {
            dialog.force_close();
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            close_dialogs(&current);
            child = current.next_sibling();
        }
    }

    fn close_typed(slot: &RefCell<Option<TypedDialog>>) {
        if let Some(typed) = slot.borrow_mut().take() {
            typed.dialog.force_close();
        }
    }

    fn reset(harness: &Harness) {
        harness.handle.set_card(Some(test_card()));
        for (block, data) in image().into_iter().enumerate() {
            harness.handle.set_block(block as u8, data);
        }
        install_ntag(&harness.ntag, &ntag_pages(false, false, true));
        harness
            .ui
            .service
            .lock()
            .expect("service")
            .select_device("mock:classic")
            .expect("select classic");
        harness.ui.reader_combo.set_selected(0);
        let ui = &harness.ui;
        ui.live.set(false);
        ui.busy.set(false);
        ui.dump.replace(None);
        ui.type2.replace(None);
        ui.type2_restore.replace(None);
        close_alert(&ui.pending_type2_page_dialog);
        close_typed(&ui.pending_type2_advanced_dialog);
        close_typed(&ui.pending_type2_lock_dialog);
        close_typed(&ui.pending_type2_password_dialog);
        close_typed(&ui.pending_type2_restore_dialog);
        close_typed(&ui.pending_type2_format_dialog);
        ui.type2_view.advanced.set_active(false);
        ui.type2_view.rows.remove_all();
        ui.type2_view.table_stack.set_visible_child_name("empty");
        ui.type2_view.lock_pages.set_text("");
        ui.type2_view.auth0.set_text("");
        ui.type2_view.access.set_text("");
        ui.type2_view.pwd.set_text("");
        ui.type2_view.pack.set_text("");
        ui.type2_view.include_memory.set_active(true);
        ui.type2_view.include_configuration.set_active(false);
        ui.type2_view.include_cc.set_active(false);
        ui.type2_view.include_locks.set_active(false);
        render_type2(ui);
        ui.restore_load.replace(None);
        ui.diff_before.replace(None);
        ui.diff_after.replace(None);
        close_alert(&ui.pending_restore_dialog);
        close_alert(&ui.pending_write_dialog);
        close_alert(&ui.pending_ndef_write_dialog);
        close_alert(&ui.pending_ndef_format_dialog);
        ui.ndef.replace(None);
        ui.ndef_view.draft.replace(NdefDraft::default());
        ui.ndef_view.source.set_selected(0);
        ui.ndef_view
            .input_stack
            .set_visible_child_name(NdefSource::Text.stack_name());
        ui.ndef_view.language.set_text("en");
        ui.ndef_view.text.buffer().set_text("");
        ui.ndef_view.uri.set_text("");
        ui.ndef_view.raw_label.set_localized("No file chosen");
        ui.ndef_view.read_only.set_active(false);
        update_ndef_actions(ui);
        ui.rows.remove_all();
        ui.diff_view.rows.remove_all();
        ui.restore_view.rows.remove_all();
        ui.restore_view.result.set_localized("");
        ui.diff_view
            .summary
            .set_localized("Load both images, then compare.");
        ui.status.set_localized("Ready");
        ui.keys.borrow_mut().clear();
        ui.keys_path
            .replace(Some(harness.keys_dir.path().join("classic-keys.json")));
        *ui.backup.borrow_mut() = BackupStore::new(harness.backups.path());
        let mut preferences = UiPreferences::default();
        preferences.set_language(i18n::current_language());
        ui.preferences.replace(preferences);
        ui.settings_error.replace(None);
        let _ = std::fs::remove_file(harness.keys_dir.path().join("ui.json"));
        ui.keyscan_cancel.replace(None);
        ui.keyscan_result.replace(None);
        ui.keyscan_run.set(0);
        ui.keyscan_view.rows.remove_all();
        ui.keyscan_view.table_stack.set_visible_child_name("empty");
        ui.keyscan_view.dictionary.set_text("");
        ui.keyscan_view.common.set_active(true);
        ui.keyscan_view.key_types.set_selected(0);
        ui.keyscan_view.progress.set_localized("No scan running.");
        ui.keyscan_view
            .summary
            .set_localized("Scan a MIFARE Classic card to list per-sector keys.");
        ui.recovery_cancel.replace(None);
        ui.recovery_result.replace(None);
        ui.recovery_started.replace(None);
        if let Some(timer) = ui.recovery_timer.borrow_mut().take() {
            timer.remove();
        }
        ui.recovery_run.set(0);
        ui.recovery_view.strategy.set_selected(0);
        ui.recovery_view.known_block.set_text("4");
        ui.recovery_view.known_type.set_selected(0);
        ui.recovery_view.target_block.set_text("0");
        ui.recovery_view.target_type.set_selected(0);
        ui.recovery_view.tables.set_text("");
        ui.recovery_view.max_nonces.set_value(0.0);
        ui.recovery_view.common.set_active(true);
        ui.recovery_view.log.clear();
        ui.recovery_view.elapsed.set_localized("Elapsed —");
        ui.activity.borrow_mut().clear();
        ui.activity_view.rows.remove_all();
        ui.settings_view.status.set_localized("");
        render_settings(ui);
        render_diagnostics(ui);
        refresh_keys(ui);
        update_keyscan_sensitivity(ui);
        update_recovery_sensitivity(ui);
        for entry in std::fs::read_dir(harness.backups.path()).expect("backup dir") {
            let entry = entry.expect("backup entry");
            std::fs::remove_file(entry.path()).expect("remove backup");
        }
    }

    fn show_dump(ui: &Rc<Ui>) {
        let dump = fixture_dump();
        ui.dump.replace(Some(dump.clone()));
        populate_rows(ui, &dump);
        ui.live.set(true);
        update_inspector(ui);
        update_restore_sensitivity(ui);
    }

    fn changed_block() -> [u8; 16] {
        [0xAA; 16]
    }

    fn pump_until(ui: &Ui, label: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let context = glib::MainContext::default();
        while !condition() {
            while context.iteration(false) {}
            if Instant::now() >= deadline {
                panic!(
                    "timed out waiting for {label}; status {:?}",
                    ui.status.label()
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn emit_response(dialog: &adw::AlertDialog, response: &str) {
        dialog.emit_by_name::<()>("response", &[&response]);
    }

    #[test]
    fn wifi_wsc_records_decode_and_binary_media_uses_hex() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&[0x10, 0x0E, 0x00, 0x36, 0x10, 0x26, 0x00, 0x01, 0x01]);
        payload.extend_from_slice(&[0x10, 0x45, 0x00, 0x07]);
        payload.extend_from_slice(b"TESTNET");
        payload.extend_from_slice(&[0x10, 0x03, 0x00, 0x02, 0x00, 0x20]);
        payload.extend_from_slice(&[0x10, 0x0F, 0x00, 0x02, 0x00, 0x0C]);
        payload.extend_from_slice(&[0x10, 0x27, 0x00, 0x0C]);
        payload.extend_from_slice(b"testpass1234");
        payload.extend_from_slice(&[0x10, 0x20, 0x00, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let record =
            NdefRecord::media(b"application/vnd.wfa.wsc".to_vec(), payload).expect("wsc record");
        let (kind, content) = record_summary(&record);
        assert!(kind.contains("Wi-Fi"), "{kind}");
        assert!(content.contains("WPA2-Personal"), "{content}");
        assert!(content.contains("AES"), "{content}");
        assert_eq!(
            wifi_fields(&record),
            vec![
                ("SSID", "TESTNET".to_owned()),
                ("Password", "testpass1234".to_owned())
            ]
        );

        let binary =
            NdefRecord::media(b"application/octet-stream".to_vec(), vec![0x00, 0xFF, 0x10])
                .expect("binary record");
        let (_, preview) = record_summary(&binary);
        assert_eq!(preview, "00 FF 10");
        assert!(!preview.contains('\u{FFFD}'));
    }

    #[test]
    fn gtk_workbench_scenarios() {
        if std::env::var_os("NFC_FOUNDRY_GUI_TESTS").is_none() {
            eprintln!("skipping GTK UI tests; set NFC_FOUNDRY_GUI_TESTS=1 under Xvfb");
            return;
        }
        let harness = harness();
        selection_updates_inspector(&harness);
        write_gates_follow_state(&harness);
        reader_refresh_invalidates(&harness);
        write_confirmation_cancel_and_success(&harness);
        identity_refusal_is_visible(&harness);
        restore_cancel_then_confirm(&harness);
        diff_screen_classifies_blocks(&harness);
        keys_screen_adds_and_saves(&harness);
        keyscan_finds_and_saves(&harness);
        recovery_limitation_and_saved_key(&harness);
        diagnostics_and_activity(&harness);
        settings_persist_and_reject(&harness);
        ndef_composer_sources(&harness);
        ndef_write_confirmation_cancel_and_success(&harness);
        ndef_write_gates(&harness);
        ndef_format_cancel_and_success(&harness);
        type2_read_pages_and_advanced(&harness);
        type2_user_page_write(&harness);
        type2_advanced_page_write(&harness);
        type2_lock_and_password(&harness);
        type2_ndef_write_and_format(&harness);
        type2_partial_ndef(&harness);
        type2_dump_and_restore(&harness);
        navigation_busy_and_focus(&harness);
        language_switch_preserves_state(&harness);
        harness.ui.window.close();
        drop(harness.app);
    }

    fn navigation_busy_and_focus(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        for page in [
            "card",
            "type2",
            "ndef",
            "keys",
            "keyscan",
            "recovery",
            "diff",
            "restore",
            "diagnostics",
            "activity",
            "settings",
        ] {
            ui.views.set_visible_child_name(page);
            assert_eq!(ui.views.visible_child_name().as_deref(), Some(page));
        }
        ui.views.set_visible_child_name("card");
        assert_eq!(
            ui.table_stack.visible_child_name().as_deref(),
            Some("empty")
        );
        ui.selection.set_selected(gtk::INVALID_LIST_POSITION);
        update_inspector(ui);
        assert_eq!(ui.inspector.title.label(), "No block");
        assert_eq!(ui.status.label(), "Ready");

        set_busy(ui, true);
        assert!(ui.spinner.is_visible());
        assert!(!ui.read_button.is_sensitive());
        set_busy(ui, false);
        assert!(!ui.spinner.is_visible());
        assert!(ui.read_button.is_sensitive());

        let context = glib::MainContext::default();
        while context.iteration(false) {}
        assert!(ui.read_button.is_focusable());
        assert!(ui.window.child_focus(gtk::DirectionType::TabForward));
    }

    fn is_technical(text: &str) -> bool {
        const TOKENS: [&str; 24] = [
            "MIFARE",
            "NTAG",
            "NDEF",
            "Type 2",
            "NFC",
            "UID",
            "BCC",
            "MAD",
            "CC ",
            "Key A",
            "Key B",
            "GPB",
            "PWD",
            "PACK",
            "AUTH0",
            "ACCESS",
            "CFGLCK",
            "PROT",
            "MIRROR",
            "AUTH_LIM",
            "C1C2C3",
            "PRNG",
            "Crypto1",
            "FM11RF08S",
        ];
        TOKENS.iter().any(|token| text.contains(token))
            || [
                "nfc-foundry",
                ".json",
                ".bin",
                ".mct",
                ".dic",
                "NFC Foundry",
                "nfctl",
                "mock:",
                "Mock ",
                "/",
                "—",
                "(None)",
                "Search…",
                "English",
                "简体中文",
                "Darkside",
                "Nested",
                "Hardnested",
                "unsupported:",
                "I_OWN_THIS_CARD",
                "URI",
            ]
            .iter()
            .any(|token| text.contains(token))
            || text.split_whitespace().all(|word| {
                word.len() < 3
                    || word.chars().all(|character| {
                        character.is_ascii_digit() || character.is_ascii_hexdigit()
                    })
            })
    }

    fn english_chrome(texts: &[String]) -> Vec<String> {
        texts
            .iter()
            .flat_map(|text| text.split('\n'))
            .filter(|text| {
                !text.trim().is_empty()
                    && !text.chars().any(
                        |character| matches!(character as u32, 0x3400..=0x9FFF | 0xF900..=0xFAFF),
                    )
                    && text.split_whitespace().any(|word| {
                        word.len() >= 3 && word.chars().any(|c| c.is_ascii_alphabetic())
                    })
                    && !is_technical(text)
            })
            .map(str::to_owned)
            .collect()
    }

    fn language_switch_preserves_state(harness: &Harness) {
        close_dialogs(&harness.ui.window);
        let context = glib::MainContext::default();
        while context.iteration(false) {}
        reset(harness);
        show_dump(&harness.ui);
        harness.ui.selection.set_selected(4);
        harness.ui.views.set_visible_child_name("card");
        let uid = harness
            .ui
            .dump
            .borrow()
            .as_ref()
            .and_then(|dump| dump.card.as_ref().map(|card| card.uid_hex()))
            .expect("card identity");
        let reader = harness.ui.reader_combo.selected();
        harness.ui.settings_view.language.set_selected(1);
        assert_eq!(i18n::current_language(), Language::ZhCn);
        assert_eq!(harness.ui.read_button.label().as_deref(), Some("读取卡片"));
        assert_eq!(harness.ui.status.label(), "就绪");
        assert!(
            harness
                .ui
                .views
                .pages()
                .item(0)
                .and_downcast::<gtk::StackPage>()
                .and_then(|page| page.title())
                .is_some_and(|title| title == "卡片")
        );
        assert_eq!(
            harness.ui.views.visible_child_name().as_deref(),
            Some("card")
        );
        assert_eq!(harness.ui.selection.selected(), 4);
        assert_eq!(harness.ui.reader_combo.selected(), reader);
        assert_eq!(
            harness
                .ui
                .dump
                .borrow()
                .as_ref()
                .and_then(|dump| dump.card.as_ref().map(|card| card.uid_hex())),
            Some(uid)
        );
        let preferences =
            UiPreferences::load(&harness.keys_dir.path().join("ui.json")).expect("preferences");
        assert_eq!(preferences.language.as_deref(), Some("zh-CN"));

        let texts = i18n::visible_texts(&harness.ui.window);
        let leftovers = english_chrome(&texts);
        assert!(leftovers.is_empty(), "untranslated chrome: {leftovers:#?}");

        start_read(harness.ui.clone());
        pump_until(&harness.ui, "Chinese Classic read", || {
            !harness.ui.busy.get() && harness.ui.status.label().starts_with("读取完成")
        });
        assert!(
            harness.ui.status.label().contains("读取完成 · 64 个数据块"),
            "{}",
            harness.ui.status.label()
        );

        harness.ui.selection.set_selected(4);
        update_inspector(&harness.ui);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        assert!(harness.ui.inspector.write.is_sensitive());
        confirm_write(harness.ui.clone());
        let dialog = harness
            .ui
            .pending_write_dialog
            .borrow()
            .clone()
            .expect("write dialog");
        assert_eq!(dialog.heading().as_deref(), Some("写入数据块 004？"));
        assert_eq!(dialog.response_label("write"), "写入数据块");
        assert_eq!(dialog.response_label("cancel"), "取消");
        assert_eq!(dialog.close_response(), "cancel");
        assert_eq!(dialog.default_response().as_deref(), Some("cancel"));
        assert!(dialog.body().contains("扇区 01 · 数据块"));
        let dialog_texts = i18n::visible_alert_texts(&dialog);
        let dialog_leftovers = english_chrome(&dialog_texts);
        assert!(
            dialog_leftovers.is_empty(),
            "untranslated dialog: {dialog_leftovers:#?}"
        );
        emit_response(&dialog, "cancel");
        harness.ui.pending_write_dialog.replace(None);

        choose_reader(harness, "mock:ntag");
        start_read(harness.ui.clone());
        pump_until(&harness.ui, "Chinese Type 2 read", || {
            !harness.ui.busy.get() && harness.ui.status.label().starts_with("读取完成")
        });
        assert_eq!(
            harness.ui.views.visible_child_name().as_deref(),
            Some("type2")
        );
        let texts = i18n::visible_texts(&harness.ui.window);
        let leftovers = english_chrome(&texts);
        assert!(leftovers.is_empty(), "untranslated Type 2: {leftovers:#?}");

        harness.ui.settings_view.language.set_selected(0);
        assert_eq!(i18n::current_language(), Language::En);
        assert_eq!(harness.ui.read_button.label().as_deref(), Some("Read card"));
    }

    fn selection_updates_inspector(harness: &Harness) {
        show_dump(&harness.ui);
        harness.ui.selection.set_selected(4);
        update_inspector(&harness.ui);
        assert_eq!(harness.ui.inspector.title.label(), "Block 004");
        assert!(
            harness
                .ui
                .inspector
                .details
                .label()
                .contains("Access group")
        );
        assert!(harness.ui.inspector.details.label().contains("read A|B"));

        harness.ui.selection.set_selected(7);
        update_inspector(&harness.ui);
        assert_eq!(harness.ui.inspector.title.label(), "Block 007");
        assert!(harness.ui.inspector.details.label().contains("trailer"));
        assert!(harness.ui.inspector.details.label().contains("Group 0"));

        harness.ui.selection.set_selected(0);
        update_inspector(&harness.ui);
        assert!(harness.ui.inspector.details.label().contains("BCC"));
        assert!(!harness.ui.inspector.write.is_sensitive());
    }

    fn write_gates_follow_state(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        harness.ui.selection.set_selected(4);
        update_inspector(&harness.ui);
        let unchanged = classic::hex_with_mask(&image()[4], u16::MAX);
        harness.ui.inspector.hex.set_text(&unchanged);
        update_write_sensitivity(&harness.ui);
        assert!(!harness.ui.inspector.write.is_sensitive());
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        assert!(harness.ui.inspector.write.is_sensitive());
        harness.ui.live.set(false);
        update_inspector(&harness.ui);
        assert!(!harness.ui.inspector.write.is_sensitive());

        harness.ui.selection.set_selected(0);
        update_inspector(&harness.ui);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        assert!(!harness.ui.inspector.write.is_sensitive());
    }

    fn reader_refresh_invalidates(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        harness.ui.selection.set_selected(4);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        assert!(harness.ui.inspector.write.is_sensitive());
        refresh_devices(&harness.ui);
        assert!(!harness.ui.live.get());
        assert!(!harness.ui.inspector.write.is_sensitive());
        assert!(harness.ui.status.label().contains("Inspection only"));
    }

    fn write_confirmation_cancel_and_success(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        harness.ui.selection.set_selected(4);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        confirm_write(harness.ui.clone());
        let dialog = harness
            .ui
            .pending_write_dialog
            .borrow()
            .clone()
            .expect("write dialog");
        emit_response(&dialog, "cancel");
        assert!(harness.ui.pending_write_dialog.borrow().is_none());
        assert_eq!(harness.handle.block(4), Some(image()[4]));

        confirm_write(harness.ui.clone());
        let dialog = harness
            .ui
            .pending_write_dialog
            .borrow()
            .clone()
            .expect("write dialog");
        emit_response(&dialog, "write");
        pump_until(&harness.ui, "block write", || {
            !harness.ui.busy.get() && harness.ui.status.label().contains("written")
        });
        assert_eq!(harness.handle.block(4), Some(changed_block()));

        let mut trailer = image()[7];
        trailer[15] = 0xFE;
        harness.ui.selection.set_selected(7);
        update_inspector(&harness.ui);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&trailer, u16::MAX));
        update_write_sensitivity(&harness.ui);
        assert!(
            harness.ui.inspector.write.is_sensitive(),
            "a valid changed trailer is writable after confirmation"
        );
        confirm_write(harness.ui.clone());
        let dialog = harness
            .ui
            .pending_write_dialog
            .borrow()
            .clone()
            .expect("trailer dialog");
        emit_response(&dialog, "cancel");
        assert_eq!(harness.handle.block(7), Some(image()[7]));
    }

    fn identity_refusal_is_visible(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        harness.handle.set_card(Some(other_card()));
        start_write(harness.ui.clone(), 4, changed_block(), false);
        pump_until(&harness.ui, "identity refusal", || {
            harness.ui.status.label() == "Write failed"
        });
        assert_eq!(harness.handle.block(4), Some(image()[4]));
        let log = activity_text(&harness.ui);
        assert!(
            log.contains("the card identity changed"),
            "activity did not report the identity refusal: {log}"
        );
    }

    fn restore_cancel_then_confirm(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        let mut source = fixture_dump();
        source.blocks[4].set_read([0xCC; 16]);
        harness.ui.restore_load.replace(Some((
            PathBuf::from("test-restore.bin"),
            DumpLoad {
                format: DumpFormat::Raw,
                dump: source,
                warnings: Vec::new(),
            },
        )));
        update_restore_sensitivity(&harness.ui);
        assert!(harness.ui.restore_view.restore.is_sensitive());

        confirm_restore(&harness.ui);
        let dialog = harness
            .ui
            .pending_restore_dialog
            .borrow()
            .clone()
            .expect("restore dialog");
        emit_response(&dialog, "cancel");
        assert!(harness.ui.pending_restore_dialog.borrow().is_none());
        assert_eq!(harness.handle.block(4), Some(image()[4]));

        confirm_restore(&harness.ui);
        let dialog = harness
            .ui
            .pending_restore_dialog
            .borrow()
            .clone()
            .expect("restore dialog");
        emit_response(&dialog, "restore");
        pump_until(&harness.ui, "restore completion", || {
            !harness.ui.busy.get() && harness.ui.status.label().contains("Restore complete")
        });
        assert_eq!(harness.handle.block(4), Some([0xCC; 16]));
        assert!(
            harness
                .ui
                .restore_view
                .result
                .label()
                .contains("Restored 1 block")
        );
        assert_eq!(harness.ui.restore_view.rows.n_items(), 64);
        let written = harness
            .ui
            .restore_view
            .rows
            .item(4)
            .and_downcast::<glib::BoxedAnyObject>()
            .expect("restore row")
            .borrow::<RestoreRow>()
            .clone();
        assert_eq!(written.state, "written");
        let manufacturer = harness
            .ui
            .restore_view
            .rows
            .item(0)
            .and_downcast::<glib::BoxedAnyObject>()
            .expect("restore row")
            .borrow::<RestoreRow>()
            .clone();
        assert_eq!(manufacturer.state, "unchanged");
        assert_eq!(
            std::fs::read_dir(harness.backups.path())
                .expect("backup dir")
                .count(),
            1
        );
        assert!(!harness.ui.live.get());
    }

    fn diff_screen_classifies_blocks(harness: &Harness) {
        reset(harness);
        let before = fixture_dump();
        let mut after = before.clone();
        after.blocks[4].set_read([0xAA; 16]);
        after.blocks[5].known_mask = 0;
        harness.ui.diff_before.replace(Some(DumpLoad {
            format: DumpFormat::Project,
            dump: before,
            warnings: Vec::new(),
        }));
        harness.ui.diff_after.replace(Some(DumpLoad {
            format: DumpFormat::Project,
            dump: after,
            warnings: Vec::new(),
        }));
        compare_dumps(&harness.ui);
        let summary = harness.ui.diff_view.summary.label();
        assert!(summary.contains("1 changed"), "{summary}");
        assert!(summary.contains("1 unknown"), "{summary}");
        assert_eq!(harness.ui.diff_view.rows.n_items(), 2);
        let detail = harness
            .ui
            .diff_view
            .rows
            .item(1)
            .and_downcast::<glib::BoxedAnyObject>()
            .expect("row")
            .borrow::<classic::DiffRow>()
            .detail
            .clone();
        assert!(detail.contains("unknown bytes"), "{detail}");
    }

    fn keys_screen_adds_and_saves(harness: &Harness) {
        reset(harness);
        harness.ui.keys_view.hex.set_text("A0A1A2A3A4A5");
        harness.ui.keys_view.sector.set_text("4");
        harness.ui.keys_view.kind.set_selected(1);
        add_key(&harness.ui);
        assert_eq!(harness.ui.keys.borrow().len(), 1);
        assert_eq!(harness.ui.keys_view.rows.n_items(), 1);
        assert!(harness.ui.key_value.label().contains("1 saved"));
        let path = harness.keys_dir.path().join("classic-keys.json");
        assert!(path.exists());

        harness.ui.keys_view.hex.set_text("A0A1A2A3A4A5");
        harness.ui.keys_view.sector.set_text("4");
        add_key(&harness.ui);
        assert_eq!(harness.ui.keys.borrow().len(), 1);

        harness.ui.keys_view.hex.set_text("not-hex");
        add_key(&harness.ui);
        assert_eq!(harness.ui.keys.borrow().len(), 1);
    }

    fn activity_text(ui: &Ui) -> String {
        ui.activity
            .borrow()
            .iter()
            .map(|row| {
                format!(
                    "{} · {} · {} · {}",
                    row.time, row.operation, row.result, row.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn keyscan_finds_and_saves(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        let dictionary = harness.keys_dir.path().join("scan.dic");
        std::fs::write(&dictionary, "A0A1A2A3A4A5\n").expect("dictionary");
        ui.keyscan_view
            .dictionary
            .set_text(&dictionary.display().to_string());
        ui.keyscan_view.common.set_active(false);
        ui.keyscan_view.key_types.set_selected(0);
        start_keyscan(ui.clone());
        pump_until(ui, "key scan", || !ui.busy.get());
        let result = ui.keyscan_result.borrow().clone().expect("scan result");
        assert_eq!(result.sectors.len(), 16);
        assert_eq!(result.verified, 32);
        assert_eq!(ui.keyscan_view.rows.n_items(), 16);
        assert!(ui.keyscan_view.summary.label().contains("16 sector"));
        assert!(
            !activity_text(ui).contains("A0A1A2A3A4A5"),
            "activity leaked key material"
        );

        save_keyscan_keys(ui);
        let path = harness.keys_dir.path().join("classic-keys.json");
        let text = std::fs::read_to_string(&path).expect("key store");
        assert!(text.contains("A0A1A2A3A4A5"));
        let dic = harness.keys_dir.path().join("scan-export.dic");
        write_keyscan_dic(ui, &dic).expect("export dic");
        assert_eq!(
            std::fs::read_to_string(&dic).expect("dic text"),
            "A0A1A2A3A4A5\n"
        );

        finish_keyscan(ui, Err(NfcError::Cancelled));
        assert!(ui.keyscan_view.summary.label().contains("Cancelled"));
        assert!(!ui.busy.get());
        assert!(activity_text(ui).contains("keyscan · cancelled"));
    }

    fn recovery_limitation_and_saved_key(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        ui.recovery_view.strategy.set_selected(1);
        update_recovery_sensitivity(ui);
        start_recovery(ui.clone());
        pump_until(ui, "darkside recovery", || !ui.busy.get());
        let log = ui.recovery_view.log.text();
        assert!(log.contains("unsupported"), "{log}");
        assert!(activity_text(ui).contains("recovery · limitation"));
        assert!(!activity_text(ui).contains("recovery · failed"));

        let key = Key::from_hex("B0B1B2B3B4B5").expect("key");
        let result = RecoveryResult {
            keys: vec![(nfc_core::KeyType::A, key)],
            sector_keys: Vec::new(),
            attempts: vec![nfc_recover::AttackKind::Dictionary],
            profile: None,
            note: "dictionary hit".to_owned(),
        };
        finish_recovery(
            ui,
            Ok(result),
            RecoveryStrategy::Auto,
            0,
            nfc_core::KeyType::A,
        );
        let log = ui.recovery_view.log.text();
        assert!(log.contains("B0B1B2B3B4B5"), "{log}");
        assert!(log.contains("verified"), "{log}");
        assert!(log.contains("Complete"), "{log}");
        assert!(ui.recovery_view.save.is_sensitive());
        assert!(
            !activity_text(ui).contains("B0B1B2B3B4B5"),
            "activity leaked a recovered key"
        );
        save_recovered_key(ui);
        let path = harness.keys_dir.path().join("classic-keys.json");
        let text = std::fs::read_to_string(&path).expect("key store");
        assert!(text.contains("B0B1B2B3B4B5"));
        let stored = KeyStore::load(&path).expect("load recovered store");
        assert!(
            stored
                .entries()
                .iter()
                .any(|entry| entry.source == KeySource::Recovery)
        );

        // A long result stays inside the scrollable log and the save action
        // remains reachable at the bottom of the page.
        let sector_keys: Vec<nfc_recover::RecoveredKey> = (0..16)
            .flat_map(|sector| {
                [nfc_core::KeyType::A, nfc_core::KeyType::B]
                    .into_iter()
                    .map(move |key_type| nfc_recover::RecoveredKey {
                        sector,
                        key_type,
                        key: Key::new([sector as u8; 6]),
                        method: nfc_recover::AttackKind::Dictionary,
                    })
            })
            .collect();
        let result = RecoveryResult {
            keys: sector_keys
                .iter()
                .map(|entry| (entry.key_type, entry.key))
                .collect(),
            sector_keys,
            attempts: vec![nfc_recover::AttackKind::Dictionary],
            profile: None,
            note: "sector keys 32/32 · read verification 16/16 sectors".to_owned(),
        };
        finish_recovery(
            ui,
            Ok(result),
            RecoveryStrategy::Auto,
            0,
            nfc_core::KeyType::A,
        );
        ui.views.set_visible_child_name("recovery");
        pump_until(ui, "recovery page", || {
            ui.recovery_view.log.text().contains("sector 15")
        });
        let bounds = ui
            .recovery_view
            .save
            .compute_bounds(&ui.window)
            .expect("save bounds");
        assert!(
            bounds.y() + bounds.height() <= ui.window.height() as f32,
            "save action is outside the window: {bounds:?} in {}px",
            ui.window.height()
        );
        ui.recovery_view.log.clear();
        ui.recovery_view.log.live("info", "probe 1");
        ui.recovery_view.log.live("info", "probe 2");
        assert_eq!(ui.recovery_view.log.text(), "probe 2");
        ui.recovery_view.log.append("info", "done");
        assert_eq!(ui.recovery_view.log.text(), "probe 2\ndone\n");

        let progress = recovery_progress_text(RecoveryProgress::BruteForce {
            tested_states: 5,
            total_states: 10,
            expected_states: 10.0,
        });
        assert!(progress.contains("50%"), "{progress}");
        assert_eq!(format_elapsed(Duration::from_secs(125)), "2:05");
        assert_eq!(format_elapsed(Duration::from_secs(3725)), "1:02:05");
    }

    fn diagnostics_and_activity(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        render_diagnostics(ui);
        assert!(
            ui.diagnostics_view
                .version
                .label()
                .contains(env!("CARGO_PKG_VERSION"))
        );
        assert!(ui.diagnostics_view.drivers.label().contains("pn532_uart"));
        assert!(ui.diagnostics_view.drivers.label().contains("acr122_usb"));
        assert!(ui.diagnostics_view.readers.label().contains("mock:classic"));
        assert!(
            ui.diagnostics_view
                .tables
                .label()
                .starts_with("Hardnested tables:")
        );
        assert!(ui.diagnostics_view.build.label().contains("Build:"));

        ui.keys_view.hex.set_text("A0A1A2A3A4A5");
        ui.keys_view.sector.set_text("3");
        ui.keys_view.kind.set_selected(1);
        add_key(ui);
        assert_eq!(ui.keys.borrow().len(), 1);
        let text = activity_text(ui);
        assert!(!text.is_empty());
        assert!(
            !text.contains("A0A1A2A3A4A5"),
            "activity leaked key material: {text}"
        );
        assert!(ui.activity.borrow().iter().all(|row| !row.time.is_empty()));

        clear_activity(ui);
        assert!(ui.activity.borrow().is_empty());
        assert_eq!(ui.activity_view.rows.n_items(), 0);
    }

    fn settings_persist_and_reject(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        let new_store = harness.keys_dir.path().join("settings-keys.json");
        let new_backup = harness.keys_dir.path().join("settings-backups");
        ui.settings_view
            .key_store
            .set_text(&new_store.display().to_string());
        ui.settings_view
            .backup_dir
            .set_text(&new_backup.display().to_string());
        ui.settings_view.tables.set_text("");
        ui.settings_view.common.set_active(false);
        ui.settings_view
            .dump_format
            .set_selected(dump_format_index(DumpFormat::Mct));
        save_settings(ui);

        let config = harness.keys_dir.path().join("ui.json");
        assert!(config.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&config)
                .expect("config metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let preferences = UiPreferences::load(&config).expect("load preferences");
        assert_eq!(preferences.key_store.as_deref(), Some(new_store.as_path()));
        assert_eq!(
            preferences.backup_dir.as_deref(),
            Some(new_backup.as_path())
        );
        assert!(!preferences.use_common_keys);
        assert_eq!(preferences.dump_format().expect("format"), DumpFormat::Mct);
        assert_eq!(ui.keys_path.borrow().as_deref(), Some(new_store.as_path()));
        assert_eq!(ui.backup.borrow().directory(), new_backup.as_path());
        assert!(!ui.preferences.borrow().use_common_keys);
        assert!(!ui.keyscan_view.common.is_active());
        assert!(!ui.recovery_view.common.is_active());

        ui.settings_view
            .key_store
            .set_text(&harness.keys_dir.path().display().to_string());
        save_settings(ui);
        assert!(
            ui.settings_view.status.label().starts_with("Not saved:"),
            "{}",
            ui.settings_view.status.label()
        );
        assert!(activity_text(ui).contains("error"));

        let broken = harness.keys_dir.path().join("broken-ui.json");
        std::fs::write(&broken, "not json").expect("write");
        assert!(UiPreferences::load(&broken).is_err());
    }

    fn ndef_log(ui: &Ui) -> String {
        activity_text(ui)
    }

    fn ndef_composer_sources(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;

        show_image(ui, &fixture_dump());
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.format.is_sensitive());
        assert!(
            ui.ndef_view.feedback.label().contains("Format is required"),
            "{}",
            ui.ndef_view.feedback.label()
        );

        let blocks = formatted_image(false);
        install_image(harness, &blocks);
        show_image(ui, &dump_for(&blocks));
        assert!(ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("Ready to write"));

        ui.ndef_view.text.buffer().set_text("Hello");
        assert!(
            ui.ndef_view.preview.label().contains("Text · language en"),
            "{}",
            ui.ndef_view.preview.label()
        );
        assert!(ui.ndef_view.preview.label().contains("D1 01"));
        assert!(ui.ndef_view.write.is_sensitive());

        ui.ndef_view.source.set_selected(1);
        ui.ndef_view.uri.set_text("https://example.com/a");
        assert!(
            ui.ndef_view
                .preview
                .label()
                .contains("URI · https://example.com/a")
        );
        assert!(ui.ndef_view.write.is_sensitive());
        ui.ndef_view.uri.set_text("");
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("must not be empty"));

        ui.ndef_view.source.set_selected(0);
        ui.ndef_view.language.set_text("");
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("language code"));
        ui.ndef_view.language.set_text("en");
        ui.ndef_view.text.buffer().set_text(&"x".repeat(120));
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(
            ui.ndef_view.feedback.label().contains("at most 93"),
            "{}",
            ui.ndef_view.feedback.label()
        );

        let dir = tempfile::tempdir().expect("raw NDEF dir");
        let path = dir.path().join("message.ndef");
        let message = NdefMessage::single(
            TextRecord::new("raw", "en")
                .expect("text")
                .to_record()
                .expect("record"),
        )
        .expect("message");
        std::fs::write(&path, message.encode().expect("encode")).expect("raw file");
        load_ndef_raw(ui, &path);
        assert_eq!(ui.ndef_view.source.selected(), NdefSource::Raw as u32);
        assert!(ui.ndef_view.raw_label.label().contains("message.ndef"));
        assert!(
            ui.ndef_view
                .preview
                .label()
                .contains("Raw NDEF · 1 record(s)")
        );
        assert!(ui.ndef_view.write.is_sensitive());

        let bad = dir.path().join("bad.ndef");
        std::fs::write(&bad, [0xD1, 0x01]).expect("bad raw file");
        load_ndef_raw(ui, &bad);
        assert!(ndef_log(ui).contains("not a valid NDEF message"));
        assert!(
            ui.ndef_view.write.is_sensitive(),
            "a malformed file keeps the previous composed source"
        );

        ui.live.set(false);
        update_ndef_actions(ui);
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(!ui.ndef_view.format.is_sensitive());
    }

    fn ndef_write_confirmation_cancel_and_success(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        let blocks = formatted_image(false);
        install_image(harness, &blocks);
        show_image(ui, &dump_for(&blocks));
        ui.ndef_view.text.buffer().set_text("Hello");
        assert!(ui.ndef_view.write.is_sensitive());

        confirm_ndef_write(ui.clone());
        let dialog = ui
            .pending_ndef_write_dialog
            .borrow()
            .clone()
            .expect("NDEF write dialog");
        emit_response(&dialog, "cancel");
        assert!(ui.pending_ndef_write_dialog.borrow().is_none());
        assert_eq!(harness.handle.block(4), Some(blocks[4]));

        confirm_ndef_write(ui.clone());
        let dialog = ui
            .pending_ndef_write_dialog
            .borrow()
            .clone()
            .expect("NDEF write dialog");
        emit_response(&dialog, "write");
        pump_until(ui, "NDEF write", || {
            !ui.busy.get() && ui.status.label().contains("NDEF written")
        });

        let block = harness.handle.block(4).expect("written block");
        assert_eq!(block[0], 0x03, "mandatory NDEF Message TLV");
        assert_ne!(block[1], 0);
        let info = ui.ndef.borrow().clone().expect("NDEF info");
        match info.ndef.state {
            ClassicNdefState::Message(message) => {
                assert_eq!(message.records().len(), 1);
                let record = TextRecord::from_record(&message.records()[0]).expect("text record");
                assert_eq!(record.text, "Hello");
            }
            other => panic!("unexpected NDEF state: {other:?}"),
        }
        assert!(
            ui.dump
                .borrow()
                .as_ref()
                .is_some_and(|dump| dump.blocks[4].data[0] == 0x03),
            "the refreshed image carries the written TLV"
        );
        assert_eq!(
            std::fs::read_dir(harness.backups.path())
                .expect("backup dir")
                .count(),
            1
        );
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(
            ui.ndef_view
                .feedback
                .label()
                .contains("already stores this message")
        );
        assert!(ndef_log(ui).contains("ndef write"));
        assert!(
            !activity_text(ui).contains("Hello"),
            "activity leaked the NDEF payload"
        );
        assert!(!ndef_log(ui).contains("Hello"));
        assert!(ui.ndef_view.export.is_sensitive());
        let message = stored_ndef_message(ui).expect("stored NDEF message");
        assert!(!message.encode().expect("encode").is_empty());
    }

    fn ndef_write_gates(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;

        show_image(ui, &fixture_dump());
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.format.is_sensitive());
        ui.ndef_view.text.buffer().set_text("Hello");
        confirm_ndef_write(ui.clone());
        assert!(ui.pending_ndef_write_dialog.borrow().is_none());
        assert!(ui.ndef_view.feedback.label().contains("Format is required"));

        let read_only = formatted_image(true);
        install_image(harness, &read_only);
        show_image(ui, &dump_for(&read_only));
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(
            ui.ndef_view.feedback.label().contains("read-only"),
            "{}",
            ui.ndef_view.feedback.label()
        );

        let mut partial = dump_for(&read_only);
        partial.blocks[4].known_mask = 0;
        show_image(ui, &partial);
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(!ui.ndef_view.format.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("partial"));

        let blocks = formatted_image(false);
        install_image(harness, &blocks);
        show_image(ui, &dump_for(&blocks));
        ui.live.set(false);
        update_ndef_actions(ui);
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(!ui.ndef_view.format.is_sensitive());

        ui.live.set(true);
        update_ndef_actions(ui);
        harness.handle.set_card(Some(other_card()));
        ui.ndef_view.text.buffer().set_text("Hello");
        confirm_ndef_write(ui.clone());
        let dialog = ui
            .pending_ndef_write_dialog
            .borrow()
            .clone()
            .expect("NDEF write dialog");
        emit_response(&dialog, "write");
        pump_until(ui, "NDEF identity refusal", || {
            !ui.busy.get() && ui.status.label() == "NDEF write failed"
        });
        assert_eq!(harness.handle.block(4), Some(blocks[4]));
        assert!(ndef_log(ui).contains("the card identity changed"));
    }

    fn ndef_format_cancel_and_success(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        show_image(ui, &fixture_dump());
        assert!(ui.ndef_view.format.is_sensitive());
        assert!(!ui.ndef_view.write.is_sensitive());

        confirm_ndef_format(ui.clone());
        let dialog = ui
            .pending_ndef_format_dialog
            .borrow()
            .clone()
            .expect("NDEF format dialog");
        emit_response(&dialog, "cancel");
        assert!(ui.pending_ndef_format_dialog.borrow().is_none());
        assert_eq!(harness.handle.block(3), Some(image()[3]));

        ui.ndef_view.read_only.set_active(true);
        confirm_ndef_format(ui.clone());
        let dialog = ui
            .pending_ndef_format_dialog
            .borrow()
            .clone()
            .expect("NDEF format dialog");
        emit_response(&dialog, "format");
        pump_until(ui, "NDEF format", || {
            !ui.busy.get() && ui.status.label().contains("NDEF formatted")
        });

        let trailer = harness.handle.block(3).expect("MAD trailer");
        assert_eq!(&trailer[..6], &MAD_KEY_A);
        assert_eq!(trailer[9], MAD1_GPB);
        let info = ui.ndef.borrow().clone().expect("NDEF info");
        assert_eq!(info.ndef.state, ClassicNdefState::Empty);
        let layout = info.layout.as_ref().expect("NDEF layout");
        assert_eq!(layout.sectors, (1..=15).collect::<Vec<u8>>());
        assert!(layout.read_only);
        let sector_one = harness.handle.block(7).expect("sector 1 trailer");
        assert_eq!(sector_one[9], NFC_GPB_RO);
        assert_eq!(
            std::fs::read_dir(harness.backups.path())
                .expect("backup dir")
                .count(),
            1
        );
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ndef_log(ui).contains("ndef format"));
    }

    fn type2_read_pages_and_advanced(harness: &Harness) {
        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        assert!(ui.live.get());
        assert_eq!(ui.type2_view.rows.n_items(), 38);
        let summary = ui.type2_view.summary.label();
        assert!(summary.contains("NTAG213"), "{summary}");
        assert!(summary.contains("04 5B 1C 2A 3B 4C 80"), "{summary}");

        ui.type2_view.advanced.set_active(true);
        assert_eq!(ui.type2_view.rows.n_items(), 45);
        select_type2_page(ui, 3);
        update_type2_inspector(ui);
        assert_eq!(ui.type2_view.editor.title.label(), "Page 003");
        assert!(
            ui.type2_view
                .editor
                .details
                .label()
                .contains("Data area 144 bytes"),
            "{}",
            ui.type2_view.editor.details.label()
        );
        assert!(ui.type2_view.editor.hex.is_sensitive());

        select_type2_page(ui, 43);
        update_type2_inspector(ui);
        assert!(
            ui.type2_view.editor.details.label().contains("write-only"),
            "{}",
            ui.type2_view.editor.details.label()
        );

        select_type2_page(ui, 0);
        update_type2_inspector(ui);
        assert!(!ui.type2_view.editor.write.is_sensitive());
        assert!(
            ui.type2_view.editor.feedback.label().contains("read-only"),
            "{}",
            ui.type2_view.editor.feedback.label()
        );

        ui.type2_view.advanced.set_active(false);
        assert_eq!(ui.type2_view.rows.n_items(), 38);
        update_ndef(ui);
        assert_eq!(ui.ndef_view.state.label(), "NDEF formatted · empty message");
        assert!(ui.ndef_view.format.is_sensitive());
    }

    fn type2_user_page_write(harness: &Harness) {
        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        select_type2_page(ui, 5);
        update_type2_inspector(ui);
        ui.type2_view.editor.hex.set_text("AA BB CC DD");
        update_type2_write_sensitivity(ui);
        assert!(
            ui.type2_view.editor.write.is_sensitive(),
            "title {:?} feedback {:?} live {} busy {} hex {:?}",
            ui.type2_view.editor.title.label(),
            ui.type2_view.editor.feedback.label(),
            ui.live.get(),
            ui.busy.get(),
            ui.type2_view.editor.hex.text()
        );

        confirm_type2_page_write(ui.clone());
        let dialog = ui
            .pending_type2_page_dialog
            .borrow()
            .clone()
            .expect("page dialog");
        emit_response(&dialog, "cancel");
        assert!(ui.pending_type2_page_dialog.borrow().is_none());
        assert_eq!(harness.ntag.page(5), Some([0; 4]));

        confirm_type2_page_write(ui.clone());
        let dialog = ui
            .pending_type2_page_dialog
            .borrow()
            .clone()
            .expect("page dialog");
        emit_response(&dialog, "write");
        pump_until(ui, "Type 2 page write", || {
            !ui.busy.get() && ui.status.label().contains("Type 2 page 005 written")
        });
        assert_eq!(harness.ntag.page(5), Some([0xAA, 0xBB, 0xCC, 0xDD]));
        assert_eq!(
            ui.type2
                .borrow()
                .as_ref()
                .and_then(|session| session.image.page(5).copied()),
            Some([0xAA, 0xBB, 0xCC, 0xDD])
        );
        assert_eq!(
            std::fs::read_dir(harness.backups.path())
                .expect("backup dir")
                .count(),
            1
        );

        read_type2(harness, &ntag_pages(false, true, true));
        let ui = &harness.ui;
        select_type2_page(ui, 4);
        update_type2_inspector(ui);
        assert!(!ui.type2_view.editor.write.is_sensitive());
        assert!(
            ui.type2_view.editor.feedback.label().contains("locked"),
            "{}",
            ui.type2_view.editor.feedback.label()
        );
    }

    fn type2_advanced_page_write(harness: &Harness) {
        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        assert_eq!(ui.type2_view.rows.n_items(), 38);
        ui.type2_view.advanced.set_active(true);
        select_type2_page(ui, 3);
        update_type2_inspector(ui);
        ui.type2_view.editor.hex.set_text("E1 10 12 F0");
        update_type2_write_sensitivity(ui);
        assert!(ui.type2_view.editor.write.is_sensitive());

        confirm_type2_page_write(ui.clone());
        let typed = typed_pending(&ui.pending_type2_advanced_dialog);
        let body = typed.dialog.body();
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        assert!(body.contains("Old  E1 10 12 00"), "{body}");
        assert!(body.contains("New  E1 10 12 F0"), "{body}");
        assert!(body.contains("backup"), "{body}");
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        assert!(
            ui.pending_type2_advanced_dialog.borrow().is_some(),
            "the typed phrase is required"
        );
        assert_eq!(harness.ntag.page(3), Some([0xE1, 0x10, 0x12, 0x00]));
        typed.entry.set_text(ADVANCED_PHRASE);
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        pump_until(ui, "Advanced page write", || {
            !ui.busy.get() && ui.status.label().contains("Type 2 page 003 written")
        });
        assert_eq!(harness.ntag.page(3), Some([0xE1, 0x10, 0x12, 0xF0]));
        assert!(
            !ui.type2
                .borrow()
                .as_ref()
                .expect("session")
                .image
                .cc
                .writable()
        );
        update_ndef(ui);
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(!ui.ndef_view.format.is_sensitive());
    }

    fn type2_lock_and_password(harness: &Harness) {
        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        ui.type2_view.advanced.set_active(true);
        ui.type2_view.lock_pages.set_text("4-5");
        update_type2_actions(ui);
        assert!(ui.type2_view.lock_apply.is_sensitive());
        assert!(
            ui.type2_view.lock_preview.label().contains("30 00"),
            "{}",
            ui.type2_view.lock_preview.label()
        );

        confirm_type2_lock(ui.clone());
        let typed = typed_pending(&ui.pending_type2_lock_dialog);
        let body = typed.dialog.body();
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        assert!(body.contains("Static lock 00 00 → 30 00"), "{body}");
        assert!(body.contains("one-way"), "{body}");
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        assert!(ui.pending_type2_lock_dialog.borrow().is_some());
        typed.entry.set_text(ADVANCED_PHRASE);
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        pump_until(ui, "Type 2 lock", || {
            !ui.busy.get() && ui.status.label().contains("Lock bits written")
        });
        let page2 = harness.ntag.page(2).expect("page 2");
        assert_eq!(page2[2], 0x30);
        assert_eq!(&page2[..2], &ntag_pages(false, false, true)[2][..2]);
        select_type2_page(ui, 4);
        update_type2_inspector(ui);
        assert!(!ui.type2_view.editor.write.is_sensitive());

        ui.type2_view.lock_pages.set_text("4-5");
        update_type2_actions(ui);
        assert!(!ui.type2_view.lock_apply.is_sensitive());
        assert!(
            ui.type2_view
                .lock_preview
                .label()
                .contains("already locked")
        );

        harness
            .ntag
            .set_password([0x11, 0x22, 0x33, 0x44], [0xAA, 0xBB]);
        ui.type2_view.auth0.set_text("32");
        ui.type2_view.access.set_text("00");
        ui.type2_view.pwd.set_text("11223344");
        ui.type2_view.pack.set_text("AABB");
        update_type2_actions(ui);
        assert!(ui.type2_view.password_apply.is_sensitive());
        confirm_type2_password(ui.clone());
        let typed = typed_pending(&ui.pending_type2_password_dialog);
        let body = typed.dialog.body();
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        assert!(body.contains("AUTH0 32"), "{body}");
        assert!(body.contains("PWD  11 22 33 44"), "{body}");
        typed.entry.set_text(ADVANCED_PHRASE);
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        pump_until(ui, "Type 2 password", || {
            !ui.busy.get() && ui.status.label().contains("Password configuration written")
        });
        let config = harness.ntag.page(41).expect("cfg0");
        assert_eq!(config[3], 0x20);
        assert_eq!(harness.ntag.page(43), Some([0x11, 0x22, 0x33, 0x44]));
        assert!(
            ui.type2_view.configuration.label().contains("AUTH0 32"),
            "{}",
            ui.type2_view.configuration.label()
        );
        assert_eq!(
            std::fs::read_dir(harness.backups.path())
                .expect("backup dir")
                .count(),
            2
        );
    }

    fn type2_ndef_write_and_format(harness: &Harness) {
        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        ui.ndef_view.text.buffer().set_text("Hello");
        assert!(ui.ndef_view.write.is_sensitive());
        assert!(
            ui.ndef_view.preview.label().contains("Text · language en"),
            "{}",
            ui.ndef_view.preview.label()
        );

        confirm_ndef_write(ui.clone());
        let dialog = ui
            .pending_ndef_write_dialog
            .borrow()
            .clone()
            .expect("NDEF write dialog");
        emit_response(&dialog, "cancel");
        assert_eq!(harness.ntag.page(4), Some([0x03, 0x00, 0xFE, 0x00]));

        confirm_ndef_write(ui.clone());
        let dialog = ui
            .pending_ndef_write_dialog
            .borrow()
            .clone()
            .expect("NDEF write dialog");
        emit_response(&dialog, "write");
        pump_until(ui, "Type 2 NDEF write", || {
            !ui.busy.get() && ui.status.label().contains("NDEF written")
        });
        let page4 = harness.ntag.page(4).expect("page 4");
        assert_eq!(page4[0], 0x03);
        assert_ne!(page4[1], 0x00);
        assert!(matches!(
            ui.type2
                .borrow()
                .as_ref()
                .expect("session")
                .ndef()
                .expect("NDEF")
                .state,
            NdefState::Message(_)
        ));
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(
            ui.ndef_view
                .feedback
                .label()
                .contains("already stores this message")
        );

        confirm_ndef_format(ui.clone());
        let typed = typed_pending(&ui.pending_type2_format_dialog);
        let body = typed.dialog.body();
        assert!(body.contains("NDEF data area"), "{body}");
        assert!(body.contains("read back"), "{body}");
        typed.entry.set_text(ADVANCED_PHRASE);
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        pump_until(ui, "Type 2 NDEF format", || {
            !ui.busy.get() && ui.status.label().contains("NDEF formatted")
        });
        assert_eq!(harness.ntag.page(4), Some([0x03, 0x00, 0xFE, 0x00]));
        assert!(matches!(
            ui.type2
                .borrow()
                .as_ref()
                .expect("session")
                .ndef()
                .expect("NDEF")
                .state,
            NdefState::Empty
        ));

        read_type2(harness, &ntag_pages(true, false, true));
        let ui = &harness.ui;
        ui.ndef_view.text.buffer().set_text("Hello");
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(
            ui.ndef_view.feedback.label().contains("read-only"),
            "feedback {:?} state {:?}",
            ui.ndef_view.feedback.label(),
            ui.ndef_view.state.label()
        );
        assert!(!ui.ndef_view.format.is_sensitive());

        read_type2(harness, &ntag_pages(false, false, false));
        let ui = &harness.ui;
        update_ndef(ui);
        assert_eq!(ui.ndef_view.state.label(), "Not NDEF formatted");
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.format.is_sensitive());

        read_type2(harness, &ntag_pages(false, true, true));
        let ui = &harness.ui;
        ui.ndef_view.text.buffer().set_text("Hello");
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("locked"));
        assert!(!ui.ndef_view.format.is_sensitive());

        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        harness.ntag.set_card(Some(other_type2_card()));
        ui.ndef_view.text.buffer().set_text("Hello");
        confirm_ndef_write(ui.clone());
        let dialog = ui
            .pending_ndef_write_dialog
            .borrow()
            .clone()
            .expect("NDEF write dialog");
        emit_response(&dialog, "write");
        pump_until(ui, "Type 2 NDEF identity refusal", || {
            !ui.busy.get() && ui.status.label() == "NDEF write failed"
        });
        assert_eq!(harness.ntag.page(4), Some([0x03, 0x00, 0xFE, 0x00]));
        assert!(ndef_log(ui).contains("the card identity changed"));
    }

    fn type2_partial_ndef(harness: &Harness) {
        reset(harness);
        let pages = ntag_pages(false, false, true);
        harness.ntag.clear_pages();
        for (page, data) in pages.iter().enumerate() {
            if page != 20 {
                harness.ntag.set_page(page as u8, *data);
            }
        }
        choose_reader(harness, "mock:ntag");
        start_read(harness.ui.clone());
        pump_until(&harness.ui, "partial Type 2 read", || {
            !harness.ui.busy.get() && harness.ui.status.label().contains("Read complete")
        });
        let ui = &harness.ui;
        assert!(
            !ui.type2
                .borrow()
                .as_ref()
                .expect("session")
                .warnings
                .is_empty()
        );
        update_ndef(ui);
        assert_eq!(ui.ndef_view.state.label(), "NDEF unavailable");
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(!ui.ndef_view.format.is_sensitive());
    }

    fn type2_dump_and_restore(harness: &Harness) {
        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        let dir = tempfile::tempdir().expect("type 2 dump dir");
        let dump_path = dir.path().join("tag.nfc-foundry.json");
        write_type2_dump(ui, &dump_path).expect("save Type 2 dump");
        assert!(dump_path.exists());
        load_dump_file(ui, &dump_path).expect("load Type 2 dump");
        assert!(!ui.live.get());
        assert!(ui.type2.borrow().is_some());
        assert!(ui.status.label().contains("inspection only"));

        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        let mut source = ui.type2.borrow().as_ref().expect("session").image.clone();
        source.set_page(6, [0xA6; 4]).expect("source page");
        let source_path = dir.path().join("restore.nfc-foundry.json");
        std::fs::write(
            &source_path,
            type2_exchange::export(&source).expect("export"),
        )
        .expect("source file");
        load_type2_restore_file(ui, &source_path).expect("load restore source");
        assert!(ui.type2_view.restore.is_sensitive());

        confirm_type2_restore(ui.clone());
        let typed = typed_pending(&ui.pending_type2_restore_dialog);
        emit_response(&typed.dialog, "cancel");
        assert_eq!(harness.ntag.page(6), Some([0; 4]));

        confirm_type2_restore(ui.clone());
        let typed = typed_pending(&ui.pending_type2_restore_dialog);
        let body = typed.dialog.body();
        assert!(body.contains("Target NTAG213"), "{body}");
        assert!(body.contains("Restore user memory"), "{body}");
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        typed.entry.set_text(ADVANCED_PHRASE);
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        pump_until(ui, "Type 2 restore", || {
            !ui.busy.get() && ui.status.label().contains("Restore complete")
        });
        assert_eq!(harness.ntag.page(6), Some([0xA6; 4]));
        assert_eq!(
            std::fs::read_dir(harness.backups.path())
                .expect("backup dir")
                .count(),
            1
        );

        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        load_type2_restore_file(ui, &source_path).expect("load restore source");
        harness.ntag.set_card(Some(other_type2_card()));
        confirm_type2_restore(ui.clone());
        let typed = typed_pending(&ui.pending_type2_restore_dialog);
        typed.entry.set_text(ADVANCED_PHRASE);
        typed.dialog.emit_by_name::<()>("response", &[&"confirm"]);
        pump_until(ui, "Type 2 restore identity refusal", || {
            !ui.busy.get() && ui.status.label() == "Type 2 restore failed"
        });
        assert_eq!(harness.ntag.page(6), Some([0; 4]));
        assert!(ndef_log(ui).contains("the card identity changed"));

        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        load_type2_restore_file(ui, &source_path).expect("load restore source");
        ui.type2_view.include_memory.set_active(false);
        update_type2_restore_sensitivity(ui);
        assert!(!ui.type2_view.restore.is_sensitive());
    }
}
