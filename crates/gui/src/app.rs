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
    AuthenticatedKey, BackupStore, ClassicBlockReadResult, ClassicKeyscan, ClassicKeyscanResult,
    ClassicNdefFormat, ClassicNdefFormatResult, ClassicNdefWrite, ClassicNdefWriteResult,
    ClassicRestoreResult, ClassicUidExecution, ClassicUidPlan, ClassicUidPreflight,
    ClassicWriteExecution, ClassicWritePlan, ClassicWritePreflight, KeyEntry, KeyFile, KeyKind,
    KeySource, KeyStore, KeyTypes, KeyscanProgress, KeyscanSink, RestoreSelection, RestoreUidState,
    WorkbenchSnapshot, common_keys, common_only_keys, default_extended_keys, default_keys,
    ev1_signature_keys, format_uid, parse_block_list, parse_uid,
};
use nfc_app::desfire::{
    ApplicationKeyType, AuthKeyType, CommunicationMode, DesfireAppCreateRequest,
    DesfireAppDeleteRequest, DesfireFileCreateRequest, DesfireFileDeleteRequest,
    DesfireFileSettingsRequest, DesfireFileSpec, DesfireFreeMemoryRequest,
    DesfireNdefDeleteRequest, DesfireNdefFormatRequest, DesfireReadRequest, DesfireReadResult,
    DesfireRecordOperation, DesfireRecordRequest, DesfireUidRequest, DesfireValueOperation,
    DesfireValueRequest, DesfireVerifyKeyRequest, DesfireWriteRequest, NDEF_APPLICATION_AID,
    NDEF_CC_FILE, NDEF_CC_SIZE, NDEF_DATA_FILE, NDEF_DATA_SIZE, app_create_text, app_delete_text,
    card_uid_text, file_create_text, file_delete_text, file_settings_text, free_memory_text,
    key_verification_text, read_protection_text, record_text, value_text, write_text,
};
use nfc_app::type2::{
    Type2Auth, Type2Format, Type2Lock, Type2LockResult, Type2MutationResult, Type2NdefWrite,
    Type2PageResult, Type2PageWrite, Type2Password, Type2ReadRequest, Type2ReadResult,
    Type2Restore, Type2UidExecution, Type2UidPlan, Type2UidResult, parse_uid_hex,
};
use nfc_app::{
    CardReadResult, ClassicNdefInfo, ClassicNdefState, DriverInfo, ReaderDevice, ReaderEvent,
    ReaderSnapshot, ReaderStatus, ReaderUpdate, SelfCheck, Service, classic_health,
};
use nfc_classic::{Dump, DumpFormat, DumpLoad, diff, exchange};
use nfc_core::{CancelToken, CardInfo, CardType, Key, NfcError, Result as NfcResult};
use nfc_drivers::DriverRegistry;
use nfc_ndef::{LeAddressType, LeRole, NdefMessage, SmartPosterAction, TextEncoding, TextRecord};
use nfc_recover::auto::RecoveryStrategy;
use nfc_recover::{ProgressSink, RecoveryProgress, RecoveryRequest, RecoveryResult};
use nfc_transport::{DeviceInfo, ReaderCapabilities};
use nfc_type2::{
    NdefState, RestoreOptions, Type2Image, Type2Ndef, Type2Product, exchange as type2_exchange,
};

use crate::classic::{self, BlockRow};
use crate::i18n::{self, Language, Localized};
use crate::ndef::{
    self as ndef_editor, BluetoothDraft, DppCarrier, DppDraft, MediaDraft, NdefEditor, RawDraft,
    RecordDraft, RecordKind, RecordRow, SmartPosterDraft, TextDraft, UriDraft, VCardDraft,
    WifiDraft,
};
use crate::settings::{UiPreferences, preferences_path};
use crate::type2::{self as type2_ui, Session as Type2Session};

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
    /// Run the background card monitor. Tests step the monitor explicitly.
    pub auto_monitor: bool,
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
            auto_monitor: true,
        }
    }
}

pub fn build_ui(app: &adw::Application) {
    gtk::Window::set_default_icon_name("nfc-foundry");
    build_ui_with(app, Workbench::hardware());
}

fn build_ui_with(app: &adw::Application, workbench: Workbench) -> Rc<Ui> {
    install_css();
    #[cfg(not(test))]
    nfc_app::paths::ensure_default_dirs();
    let Workbench {
        service,
        keys,
        keys_path,
        backup,
        key_error,
        preferences,
        preferences_path,
        settings_error,
        auto_monitor,
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
        row.display_status().to_owned()
    }));
    let data_column = copy_text_column::<BlockRow>("Data", 320, true, |row| row.data.clone());
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

    let details_view = build_details_view();
    let keys_view = build_keys_view();
    let keyscan_view = build_keyscan_view(&preferences);
    let recovery_view = build_recovery_view(&preferences);
    let diff_view = build_diff_view();
    let restore_view = build_restore_view();
    let type2_view = build_type2_view();
    let desfire_view = build_desfire_view();
    let diagnostics_view = build_diagnostics_view();
    let activity_view = build_activity_view();
    let settings_view = build_settings_view(&preferences);

    let keys_tabs = gtk::Stack::new();
    keys_tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    keys_tabs.set_vexpand(true);
    keys_tabs.add_titled(&keys_view.root, Some("keys"), "Key store");
    keys_tabs.add_titled(&keyscan_view.root, Some("keyscan"), "Key scan");
    let (keys_group, keys_switcher) = grouped_page(&keys_tabs);

    let dumps_tabs = gtk::Stack::new();
    dumps_tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    dumps_tabs.set_vexpand(true);
    dumps_tabs.add_titled(&diff_view.root, Some("diff"), "Diff");
    dumps_tabs.add_titled(&restore_view.root, Some("restore"), "Restore");
    let (dumps_group, dumps_switcher) = grouped_page(&dumps_tabs);

    let system_tabs = gtk::Stack::new();
    system_tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    system_tabs.set_vexpand(true);
    system_tabs.add_titled(&diagnostics_view.root, Some("diagnostics"), "Diagnostics");
    system_tabs.add_titled(&activity_view.root, Some("activity"), "Activity");
    system_tabs.add_titled(&settings_view.root, Some("settings"), "Settings");
    let (system_group, system_switcher) = grouped_page(&system_tabs);

    let classic_ndef_slot = gtk::Box::new(Orientation::Vertical, 0);
    classic_ndef_slot.set_vexpand(true);
    classic_ndef_slot.append(&ndef_view.root);

    let classic_save_dump = gtk::Button::with_label("Save dump…");
    let classic_load_dump = gtk::Button::with_label("Load dump…");
    let classic_toolbar = gtk::Box::new(Orientation::Horizontal, 8);
    classic_toolbar.append(&classic_save_dump);
    classic_toolbar.append(&classic_load_dump);
    let classic_dumps = gtk::Box::new(Orientation::Vertical, 8);
    classic_dumps.set_vexpand(true);
    classic_dumps.append(&classic_toolbar);
    classic_dumps.append(&dumps_group);

    let classic_tabs = gtk::Stack::new();
    classic_tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    classic_tabs.set_vexpand(true);
    classic_tabs.add_titled(&card_page, Some("blocks"), "Blocks");
    classic_tabs.add_titled(&details_view.root, Some("details"), "Details");
    classic_tabs.add_titled(&classic_ndef_slot, Some("ndef"), "NDEF");
    classic_tabs.add_titled(&keys_group, Some("keys"), "Keys");
    classic_tabs.add_titled(&recovery_view.root, Some("recovery"), "Recovery");
    classic_tabs.add_titled(&classic_dumps, Some("dumps"), "Dumps");
    let (classic_root, classic_switcher) = grouped_page(&classic_tabs);

    let views = gtk::Stack::new();
    views.set_transition_type(gtk::StackTransitionType::Crossfade);
    views.set_vexpand(true);
    views.add_named(&classic_root, Some("classic"));
    views.add_named(&type2_view.root, Some("type2"));
    views.add_named(&desfire_view.root, Some("desfire"));
    views.add_named(&system_group, Some("system"));
    views.set_visible_child_name("classic");
    let center = gtk::Box::new(Orientation::Vertical, 0);
    center.set_hexpand(true);
    center.set_vexpand(true);
    center.set_halign(gtk::Align::Fill);
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
    let reader_state = gtk::Label::new(Some("Connecting…"));
    reader_state.set_xalign(0.0);
    reader_state.set_wrap(true);
    reader_state.add_css_class("caption");
    let reader_card_info = gtk::Label::new(Some("Card: none"));
    reader_card_info.set_xalign(0.0);
    reader_card_info.set_wrap(true);
    reader_card_info.set_selectable(true);
    reader_card_info.add_css_class("caption");
    reader_card_info.add_css_class("dim-label");
    let reader_caps = gtk::Label::new(Some("Capabilities: not opened"));
    reader_caps.set_xalign(0.0);
    reader_caps.set_wrap(true);
    reader_caps.set_selectable(true);
    reader_caps.add_css_class("caption");
    reader_caps.add_css_class("dim-label");
    reader_body.append(&reader_combo);
    reader_body.append(&reader_value);
    reader_body.append(&reader_state);
    reader_body.append(&reader_card_info);
    reader_body.append(&reader_caps);

    let (card_card, _, card_body) = rail_card("CARD");
    let card_title = gtk::Label::new(Some("No card"));
    card_title.set_xalign(0.0);
    card_title.add_css_class("title-3");
    let card_detail = gtk::Label::new(Some("Select a reader and read a card."));
    card_detail.set_xalign(0.0);
    card_detail.set_wrap(true);
    card_detail.set_selectable(true);
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
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some(&i18n::translate_lines("Back")));
    back.add_css_class("flat");
    back.set_visible(false);
    header.pack_start(&back);
    let workspace_button = gtk::MenuButton::new();
    workspace_button.set_label("MIFARE Classic");
    workspace_button.add_css_class("flat");
    header.pack_start(&workspace_button);
    let read_button = gtk::Button::with_label("Read card");
    read_button.add_css_class("suggested-action");
    header.pack_end(&read_button);
    let actions_button = gtk::MenuButton::new();
    actions_button.set_icon_name("open-menu-symbolic");
    actions_button.set_tooltip_text(Some(&i18n::translate_lines("Main menu")));
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
        #[cfg(test)]
        toasts: RefCell::new(Vec::new()),
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
        reader_state,
        reader_card_info,
        reader_caps,
        reader_snapshot: RefCell::new(ReaderSnapshot {
            status: ReaderStatus::Connecting,
            selected: None,
            card: None,
            capabilities: ReaderCapabilities::default(),
            firmware: None,
            error: None,
        }),
        card_title,
        card_detail,
        key_value,
        selected_label,
        ndef_view,
        inspector,
        details_view,
        keys_view,
        dump_source: RefCell::new(None),
        keyscan_view,
        recovery_view,
        diff_view,
        restore_view,
        type2_view,
        desfire_view,
        diagnostics_view,
        activity_view,
        settings_view,
        activity: RefCell::new(Vec::new()),
        views: views.clone(),
        workspace: Cell::new(Workspace::Classic),
        workspace_button: workspace_button.clone(),
        classic_tabs,
        classic_switcher,
        classic_ndef_slot,
        classic_save_dump,
        classic_load_dump,
        back,
        #[cfg(test)]
        menu: actions_button.clone(),
        last_primary: RefCell::new("classic".to_owned()),
        keys_tabs,
        keys_switcher,
        dumps_tabs,
        dumps_switcher,
        system_tabs,
        system_switcher,
        type2: RefCell::new(None),
        type2_restore: RefCell::new(None),
        diff_before: RefCell::new(None),
        diff_after: RefCell::new(None),
        restore_load: RefCell::new(None),
        dirty_blocks: RefCell::new(Vec::new()),
        undo_depth: Cell::new(0),
        pending_plan: RefCell::new(None),
        pending_plan_dialog: RefCell::new(None),
        pending_uid_plan: RefCell::new(None),
        pending_uid_dialog: RefCell::new(None),
        pending_ndef_write_dialog: RefCell::new(None),
        pending_ndef_format_dialog: RefCell::new(None),
        pending_type2_page_dialog: RefCell::new(None),
        pending_type2_advanced_dialog: RefCell::new(None),
        pending_type2_lock_dialog: RefCell::new(None),
        pending_type2_password_dialog: RefCell::new(None),
        pending_type2_uid_plan: RefCell::new(None),
        pending_type2_uid_dialog: RefCell::new(None),
        pending_type2_restore_dialog: RefCell::new(None),
        pending_type2_format_dialog: RefCell::new(None),
        pending_desfire_dialog: RefCell::new(None),
        devices: RefCell::new(Vec::new()),
        keyscan_cancel: RefCell::new(None),
        keyscan_result: RefCell::new(None),
        keyscan_run: Cell::new(0),
        recovery_cancel: RefCell::new(None),
        pending_recovery_dialog: RefCell::new(None),
        recovery_result: RefCell::new(None),
        recovery_started: RefCell::new(None),
        recovery_timer: RefCell::new(None),
        recovery_run: Cell::new(0),
        language_updating: Cell::new(false),
    });

    connect_signals(&ui);
    build_workspace_menu(&ui, &workspace_button);
    build_actions_menu(&ui, &actions_button);
    update_card(&ui);
    update_ndef(&ui);
    render_type2(&ui);
    refresh_keys(&ui);
    render_settings(&ui);
    update_keyscan_sensitivity(&ui);
    update_recovery_sensitivity(&ui);
    update_restore_sensitivity(&ui);
    update_desfire_actions(&ui);
    refresh_devices(&ui);
    if auto_monitor {
        start_reader_monitor(ui.clone());
    }
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
    commit: gtk::Button,
    undo: gtk::Button,
    revert: gtk::Button,
    uid_advanced: gtk::Switch,
    uid: gtk::Entry,
    uid_write: gtk::Button,
}

struct KeysView {
    root: gtk::Box,
    rows: gio::ListStore,
    selection: gtk::SingleSelection,
    status: gtk::Label,
    report: gtk::Label,
    hex: gtk::Entry,
    sector: gtk::Entry,
    kind: gtk::DropDown,
    add: gtk::Button,
    delete: gtk::Button,
    import: gtk::Button,
    save: gtk::Button,
    export: gtk::Button,
}

struct DetailsView {
    root: gtk::Box,
    body: gtk::Label,
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
    mode: gtk::DropDown,
    blocks: gtk::Entry,
    continue_on_error: gtk::CheckButton,
    uid: gtk::Entry,
    uid_advanced: gtk::Switch,
    restore: gtk::Button,
    rows: gio::ListStore,
    result: gtk::Label,
}

#[derive(Clone)]
struct PendingPlan {
    plan: ClassicWritePlan,
    source: Dump,
    restore_path: Option<PathBuf>,
    advanced: bool,
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
    key_types: gtk::DropDown,
    dictionary: gtk::Entry,
    choose_dictionary: gtk::Button,
    select: gtk::Button,
    use_defaults: gtk::Button,
    selection_label: gtk::Label,
    selected: RefCell<Vec<Key>>,
    dictionary_keys: RefCell<Vec<Key>>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workspace {
    Classic,
    Type2,
    Desfire,
}

impl Workspace {
    const fn stack_name(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Type2 => "type2",
            Self::Desfire => "desfire",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Classic => "MIFARE Classic",
            Self::Type2 => "NFC Forum Type 2",
            Self::Desfire => "DESFire",
        }
    }
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
    platform: gtk::Label,
    drivers: gtk::Label,
    firmware: gtk::Label,
    capabilities: gtk::Label,
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
    #[cfg(test)]
    toasts: RefCell<Vec<adw::Toast>>,
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
    reader_state: gtk::Label,
    reader_card_info: gtk::Label,
    reader_caps: gtk::Label,
    reader_snapshot: RefCell<ReaderSnapshot>,
    card_title: gtk::Label,
    card_detail: gtk::Label,
    key_value: gtk::Label,
    selected_label: gtk::Label,
    ndef_view: NdefView,
    inspector: Inspector,
    details_view: DetailsView,
    keys_view: KeysView,
    dump_source: RefCell<Option<String>>,
    keyscan_view: KeyscanView,
    recovery_view: RecoveryView,
    diff_view: DiffView,
    restore_view: RestoreView,
    type2_view: Type2View,
    desfire_view: DesfireView,
    diagnostics_view: DiagnosticsView,
    activity_view: ActivityView,
    settings_view: SettingsView,
    activity: RefCell<Vec<ActivityRow>>,
    views: gtk::Stack,
    workspace: Cell<Workspace>,
    workspace_button: gtk::MenuButton,
    classic_tabs: gtk::Stack,
    classic_switcher: gtk::StackSwitcher,
    classic_ndef_slot: gtk::Box,
    classic_save_dump: gtk::Button,
    classic_load_dump: gtk::Button,
    back: gtk::Button,
    #[cfg(test)]
    menu: gtk::MenuButton,
    last_primary: RefCell<String>,
    keys_tabs: gtk::Stack,
    keys_switcher: gtk::StackSwitcher,
    dumps_tabs: gtk::Stack,
    dumps_switcher: gtk::StackSwitcher,
    system_tabs: gtk::Stack,
    system_switcher: gtk::StackSwitcher,
    type2: RefCell<Option<Type2Session>>,
    type2_restore: RefCell<Option<(PathBuf, Type2Image)>>,
    diff_before: RefCell<Option<DumpLoad>>,
    diff_after: RefCell<Option<DumpLoad>>,
    restore_load: RefCell<Option<(PathBuf, DumpLoad)>>,
    dirty_blocks: RefCell<Vec<usize>>,
    undo_depth: Cell<usize>,
    pending_plan: RefCell<Option<PendingPlan>>,
    pending_plan_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_uid_plan: RefCell<Option<ClassicUidPlan>>,
    pending_uid_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_ndef_write_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_ndef_format_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_page_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_advanced_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_lock_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_password_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_uid_plan: RefCell<Option<Type2UidPlan>>,
    pending_type2_uid_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_restore_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_type2_format_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_desfire_dialog: RefCell<Option<adw::AlertDialog>>,
    devices: RefCell<Vec<DeviceInfo>>,
    keyscan_cancel: RefCell<Option<CancelToken>>,
    keyscan_result: RefCell<Option<ClassicKeyscanResult>>,
    keyscan_run: Cell<u64>,
    recovery_cancel: RefCell<Option<CancelToken>>,
    pending_recovery_dialog: RefCell<Option<adw::AlertDialog>>,
    recovery_result: RefCell<Option<RecoveryOutcome>>,
    recovery_started: RefCell<Option<Instant>>,
    recovery_timer: RefCell<Option<glib::SourceId>>,
    recovery_run: Cell<u64>,
    language_updating: Cell<bool>,
}

struct Type2View {
    root: gtk::Box,
    tabs: gtk::Stack,
    switcher: gtk::StackSwitcher,
    ndef_slot: gtk::Box,
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
    auth_pwd: gtk::Entry,
    auth_pack: gtk::Entry,
    pwd: gtk::Entry,
    pack: gtk::Entry,
    password_apply: gtk::Button,
    password_note: gtk::Label,
    uid_target: gtk::Entry,
    uid_apply: gtk::Button,
    uid_note: gtk::Label,
}

struct DesfireView {
    root: gtk::Box,
    tabs: gtk::Stack,
    switcher: gtk::StackSwitcher,
    summary: gtk::Label,
    refresh: gtk::Button,
    aid: gtk::Entry,
    file: gtk::Entry,
    key: gtk::PasswordEntry,
    read_key_type: gtk::DropDown,
    key_number: gtk::Entry,
    offset: gtk::Entry,
    length: gtk::Entry,
    start_record: gtk::Entry,
    record_count: gtk::Entry,
    read: gtk::Button,
    output: gtk::Label,
    status: gtk::Label,
    write_aid: gtk::Entry,
    write_file: gtk::Entry,
    write_key: gtk::PasswordEntry,
    write_key_type: gtk::DropDown,
    write_key_number: gtk::Entry,
    write_offset: gtk::Entry,
    write_data: gtk::Entry,
    write_encoding: gtk::DropDown,
    write: gtk::Button,
    write_output: gtk::Label,
    write_status: gtk::Label,
    verify_aid: gtk::Entry,
    verify_key: gtk::PasswordEntry,
    verify_key_type: gtk::DropDown,
    verify_key_number: gtk::Entry,
    verify: gtk::Button,
    read_uid: gtk::Button,
    verify_output: gtk::Label,
    verify_status: gtk::Label,
    advanced: gtk::Switch,
    manage_aid: gtk::Entry,
    manage_key: gtk::PasswordEntry,
    manage_key_type: gtk::DropDown,
    manage_key_number: gtk::Entry,
    manage_app_keys: gtk::Entry,
    manage_app_key_type: gtk::DropDown,
    create_app: gtk::Button,
    delete_app: gtk::Button,
    manage_file: gtk::Entry,
    manage_iso_file: gtk::Entry,
    manage_file_type: gtk::DropDown,
    manage_communication: gtk::DropDown,
    manage_access_rights: gtk::Entry,
    manage_size: gtk::Entry,
    manage_lower_limit: gtk::Entry,
    manage_upper_limit: gtk::Entry,
    manage_value: gtk::Entry,
    manage_flags: gtk::Entry,
    manage_record_size: gtk::Entry,
    manage_max_records: gtk::Entry,
    manage_tmackey: gtk::PasswordEntry,
    manage_tmackey_version: gtk::Entry,
    create_file: gtk::Button,
    delete_file: gtk::Button,
    change_file_settings: gtk::Button,
    read_memory: gtk::Button,
    manage_output: gtk::Label,
    manage_status: gtk::Label,
    mutation_aid: gtk::Entry,
    mutation_file: gtk::Entry,
    mutation_key: gtk::PasswordEntry,
    mutation_key_type: gtk::DropDown,
    mutation_key_number: gtk::Entry,
    mutation_tmac_key: gtk::PasswordEntry,
    mutation_operation: gtk::DropDown,
    mutation_amount: gtk::Entry,
    mutation_record: gtk::Entry,
    mutation_offset: gtk::Entry,
    mutation_data: gtk::Entry,
    mutation_run: gtk::Button,
    mutation_output: gtk::Label,
    mutation_status: gtk::Label,
    ndef_key: gtk::PasswordEntry,
    ndef_key_type: gtk::DropDown,
    ndef_key_number: gtk::Entry,
    read_ndef: gtk::Button,
    format_ndef: gtk::Button,
    delete_ndef: gtk::Button,
    ndef_status: gtk::Label,
    ndef_output: gtk::Label,
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
    edit_state: gtk::Label,
    facts: gtk::Label,
    capacity_used: gtk::Label,
    capacity_meta: gtk::Label,
    capacity_meter: gtk::ProgressBar,
    capacity_state: gtk::Label,
    rows: gio::ListStore,
    selection: gtk::SingleSelection,
    rebuilding: Cell<bool>,
    inspector_title: gtk::Label,
    inspector_kind: gtk::Label,
    inspector_fields: gtk::Label,
    inspector_payload: gtk::Label,
    inspector_valid: gtk::Label,
    edit: gtk::Button,
    remove: gtk::Button,
    move_up: gtk::Button,
    move_down: gtk::Button,
    add: gtk::Button,
    load: gtk::Button,
    export: gtk::Button,
    reload: gtk::Button,
    clear: gtk::Button,
    validate: gtk::Button,
    preflight_tlv: gtk::Label,
    preflight_fit: gtk::Label,
    preflight_protected: gtk::Label,
    raw_head: gtk::Label,
    raw_hex: gtk::Label,
    feedback: gtk::Label,
    write: gtk::Button,
    format: gtk::Button,
    read_only: gtk::CheckButton,
    editor: RefCell<NdefEditor>,
    editor_key: RefCell<Option<String>>,
    force_editor_reset: Cell<bool>,
    pending_record_dialog: RefCell<Option<gtk::Window>>,
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
    let write = gtk::Button::with_label("Stage edit");
    write.add_css_class("suggested-action");
    let commit = gtk::Button::with_label("Write changes…");
    commit.add_css_class("destructive-action");
    let undo = gtk::Button::with_label("Undo edit");
    let revert = gtk::Button::with_label("Revert edits");
    read.set_sensitive(false);
    write.set_sensitive(false);
    commit.set_sensitive(false);
    undo.set_sensitive(false);
    revert.set_sensitive(false);
    let actions = gtk::Box::new(Orientation::Horizontal, 6);
    actions.append(&read);
    actions.append(&write);
    let history = gtk::Box::new(Orientation::Horizontal, 6);
    history.append(&undo);
    history.append(&revert);
    let write_actions = gtk::Box::new(Orientation::Horizontal, 6);
    write_actions.append(&commit);

    let note = gtk::Label::new(Some(
        "Edits change the working image only. Write changes confirms a preflight plan, \
         backs up the card, and reads every block back. Block 0 stays read-only.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let uid_advanced_label = gtk::Label::new(Some("Advanced mode"));
    uid_advanced_label.set_xalign(0.0);
    uid_advanced_label.set_hexpand(true);
    uid_advanced_label.add_css_class("caption");
    let uid_advanced = gtk::Switch::new();
    uid_advanced.set_valign(gtk::Align::Center);
    uid_advanced.set_sensitive(false);
    let uid_advanced_row = gtk::Box::new(Orientation::Horizontal, 6);
    uid_advanced_row.append(&uid_advanced_label);
    uid_advanced_row.append(&uid_advanced);

    let uid = gtk::Entry::new();
    uid.set_placeholder_text(Some("Target UID, e.g. 04A2248A"));
    uid.add_css_class("mono");
    uid.set_hexpand(true);
    uid.set_sensitive(false);
    let uid_write = gtk::Button::with_label("Write UID…");
    uid_write.add_css_class("destructive-action");
    uid_write.set_sensitive(false);
    let uid_actions = gtk::Box::new(Orientation::Horizontal, 6);
    uid_actions.append(&uid);
    uid_actions.append(&uid_write);
    let uid_note = gtk::Label::new(Some(
        "UID writes need Advanced mode, a magic CUID/Gen2 Classic 1K card, a sector-0 backup, \
         and a confirmation dialog. An ordinary card refuses the write unchanged.",
    ));
    uid_note.set_xalign(0.0);
    uid_note.set_wrap(true);
    uid_note.add_css_class("caption");
    uid_note.add_css_class("dim-label");

    root.append(&kicker);
    root.append(&title);
    root.append(&meta);
    root.append(&error);
    root.append(&details);
    root.append(&gtk::Separator::new(Orientation::Horizontal));
    root.append(&hex);
    root.append(&actions);
    root.append(&history);
    root.append(&write_actions);
    root.append(&note);
    root.append(&gtk::Separator::new(Orientation::Horizontal));
    root.append(&uid_advanced_row);
    root.append(&uid_actions);
    root.append(&uid_note);

    Inspector {
        root,
        title,
        meta,
        error,
        details,
        hex,
        read,
        write,
        commit,
        undo,
        revert,
        uid_advanced,
        uid,
        uid_write,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyRow {
    sector: String,
    kind: String,
    key: String,
    source: String,
    /// Index in the stored entry list; `None` for a built-in common key.
    stored: Option<usize>,
}

fn key_rows(store: &KeyStore) -> Vec<KeyRow> {
    let mut rows: Vec<KeyRow> = store
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| KeyRow {
            sector: match entry.sector {
                Some(sector) => i18n::translate_lines(&format!("Sector {sector}")),
                None => i18n::translate_lines("all sectors"),
            },
            kind: i18n::translate_lines(entry.kind.label()),
            key: entry.key.to_hex(),
            source: i18n::translate_lines(entry.source.label()),
            stored: Some(index),
        })
        .collect();
    for key in common_keys() {
        if store.entries().iter().any(|entry| entry.key == key) {
            continue;
        }
        rows.push(KeyRow {
            sector: i18n::translate_lines("all sectors"),
            kind: i18n::translate_lines("A+B"),
            key: key.to_hex(),
            source: i18n::translate_lines("built-in"),
            stored: None,
        });
    }
    rows
}

fn page_box() -> gtk::Box {
    let page = gtk::Box::new(Orientation::Vertical, 10);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page
}

/// A primary page that owns a nested `Stack` and its switcher. Used for the
/// grouped Keys, Dumps, and System sections.
fn grouped_page(stack: &gtk::Stack) -> (gtk::Box, gtk::StackSwitcher) {
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(stack));
    switcher.set_halign(gtk::Align::Start);
    let row = gtk::Box::new(Orientation::Horizontal, 0);
    row.set_margin_top(6);
    row.set_margin_bottom(6);
    row.set_margin_start(12);
    row.set_margin_end(12);
    row.append(&switcher);
    let root = gtk::Box::new(Orientation::Vertical, 0);
    root.set_vexpand(true);
    root.append(&row);
    root.append(stack);
    (root, switcher)
}

fn build_ndef_view() -> NdefView {
    let root = page_box();
    let state = gtk::Label::new(Some("No card loaded"));
    state.set_xalign(0.0);
    state.set_hexpand(true);
    state.add_css_class("title-3");
    let edit_state = gtk::Label::new(Some("No unsaved changes"));
    edit_state.set_xalign(1.0);
    edit_state.add_css_class("caption");
    edit_state.add_css_class("dim-label");
    let state_row = gtk::Box::new(Orientation::Horizontal, 8);
    state_row.append(&state);
    state_row.append(&edit_state);

    let facts = gtk::Label::new(Some("Read a card to inspect its NDEF message."));
    facts.set_xalign(0.0);
    facts.set_wrap(true);
    facts.set_selectable(true);
    facts.add_css_class("caption");
    facts.add_css_class("dim-label");

    let capacity = gtk::Box::new(Orientation::Vertical, 6);
    capacity.add_css_class("card");
    let capacity_head = gtk::Box::new(Orientation::Horizontal, 8);
    let capacity_title = gtk::Label::new(Some("NDEF capacity"));
    capacity_title.set_xalign(0.0);
    capacity_title.set_hexpand(true);
    capacity_title.add_css_class("rail-title");
    let capacity_state = gtk::Label::new(Some("No message"));
    capacity_state.set_xalign(1.0);
    capacity_state.add_css_class("title-4");
    capacity_head.append(&capacity_title);
    capacity_head.append(&capacity_state);
    let capacity_numbers = gtk::Box::new(Orientation::Horizontal, 8);
    let capacity_used = gtk::Label::new(Some("0 B"));
    capacity_used.set_xalign(0.0);
    capacity_used.add_css_class("title-2");
    let capacity_meta = gtk::Label::new(Some("0 records · capacity unavailable"));
    capacity_meta.set_xalign(0.0);
    capacity_meta.set_hexpand(true);
    capacity_meta.set_wrap(true);
    capacity_meta.add_css_class("caption");
    capacity_meta.add_css_class("dim-label");
    capacity_numbers.append(&capacity_used);
    capacity_numbers.append(&capacity_meta);
    let capacity_meter = gtk::ProgressBar::new();
    capacity_meter.set_show_text(false);
    capacity_meter.set_hexpand(true);
    capacity.append(&capacity_head);
    capacity.append(&capacity_numbers);
    capacity.append(&capacity_meter);

    let records = gtk::Box::new(Orientation::Vertical, 6);
    records.add_css_class("card");
    let records_head = gtk::Box::new(Orientation::Horizontal, 8);
    let records_title = gtk::Label::new(Some("Message records"));
    records_title.set_xalign(0.0);
    records_title.set_hexpand(true);
    records_title.add_css_class("title-4");
    let load = gtk::Button::with_label("Load .ndef…");
    let reload = gtk::Button::with_label("Reload from card");
    let clear = gtk::Button::with_label("New message");
    let add = gtk::Button::with_label("Add record…");
    add.add_css_class("suggested-action");
    records_head.append(&records_title);
    records_head.append(&load);
    records_head.append(&reload);
    records_head.append(&clear);
    records_head.append(&add);

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    let table = gtk::ColumnView::new(Some(selection.clone()));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.set_vexpand(true);
    table.append_column(&text_column::<RecordRow>("#", 40, false, |row| {
        format!("{:02}", row.index + 1)
    }));
    table.append_column(&text_column::<RecordRow>("Type", 150, false, |row| {
        row.kind.clone()
    }));
    let preview_column = text_column::<RecordRow>("Preview", 260, false, |row| row.preview.clone());
    preview_column.set_expand(true);
    table.append_column(&preview_column);
    table.append_column(&text_column::<RecordRow>("Size", 64, true, |row| {
        row.size.clone()
    }));
    let table_scroll = gtk::ScrolledWindow::builder().child(&table).build();
    table_scroll.set_has_frame(true);
    table_scroll.set_overlay_scrolling(false);
    table_scroll.set_min_content_height(180);
    table_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    table_scroll.set_vexpand(true);
    table_scroll.set_hexpand(true);
    records.append(&records_head);
    records.append(&table_scroll);

    let preflight = gtk::Box::new(Orientation::Vertical, 6);
    preflight.add_css_class("card");
    let preflight_head = gtk::Box::new(Orientation::Horizontal, 8);
    let preflight_title = gtk::Label::new(Some("Preflight"));
    preflight_title.set_xalign(0.0);
    preflight_title.set_hexpand(true);
    preflight_title.add_css_class("title-4");
    let validate = gtk::Button::with_label("Validate");
    preflight_head.append(&preflight_title);
    preflight_head.append(&validate);
    let preflight_tlv = gtk::Label::new(Some("TLV pending"));
    let preflight_fit = gtk::Label::new(Some("Capacity pending"));
    let preflight_protected =
        gtk::Label::new(Some("Protected areas are never written by an NDEF write."));
    preflight.append(&preflight_head);
    for label in [&preflight_tlv, &preflight_fit, &preflight_protected] {
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("caption");
        preflight.append(label);
    }

    let raw = gtk::Box::new(Orientation::Vertical, 6);
    raw.add_css_class("card");
    let raw_head = gtk::Label::new(Some("Raw message"));
    raw_head.set_xalign(0.0);
    raw_head.add_css_class("title-4");
    let raw_hex = gtk::Label::new(Some("No message composed"));
    raw_hex.set_xalign(0.0);
    raw_hex.set_wrap(true);
    raw_hex.set_selectable(true);
    raw_hex.add_css_class("mono");
    raw.append(&raw_head);
    raw.append(&raw_hex);

    let left = gtk::Box::new(Orientation::Vertical, 10);
    left.append(&capacity);
    left.append(&records);
    left.append(&preflight);
    left.append(&raw);
    let left_scroll = gtk::ScrolledWindow::builder().child(&left).build();
    left_scroll.set_has_frame(false);
    left_scroll.set_overlay_scrolling(false);
    left_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    left_scroll.set_vexpand(true);
    left_scroll.set_hexpand(true);

    let inspector = gtk::Box::new(Orientation::Vertical, 8);
    inspector.add_css_class("card");
    inspector.set_width_request(320);
    inspector.set_hexpand(false);
    inspector.set_valign(gtk::Align::Start);
    let inspector_kicker = gtk::Label::new(Some("RECORD INSPECTOR"));
    inspector_kicker.set_xalign(0.0);
    inspector_kicker.add_css_class("rail-title");
    let inspector_title = gtk::Label::new(Some("No record selected"));
    inspector_title.set_xalign(0.0);
    inspector_title.set_wrap(true);
    inspector_title.add_css_class("title-4");
    let inspector_kind = gtk::Label::new(Some("Add or select a record."));
    inspector_kind.set_xalign(0.0);
    inspector_kind.set_wrap(true);
    let inspector_fields = gtk::Label::new(None);
    inspector_fields.set_xalign(0.0);
    inspector_fields.set_wrap(true);
    inspector_fields.set_selectable(true);
    inspector_fields.add_css_class("mono");
    let inspector_payload = gtk::Label::new(None);
    inspector_payload.set_xalign(0.0);
    inspector_payload.set_wrap(true);
    inspector_payload.set_selectable(true);
    inspector_payload.add_css_class("mono");
    let inspector_valid = gtk::Label::new(None);
    inspector_valid.set_xalign(0.0);
    inspector_valid.set_wrap(true);
    inspector_valid.add_css_class("caption");
    inspector_valid.add_css_class("dim-label");
    let edit = gtk::Button::with_label("Edit record…");
    edit.add_css_class("suggested-action");
    let remove = gtk::Button::with_label("Remove");
    remove.add_css_class("destructive-action");
    let move_up = gtk::Button::from_icon_name("go-up-symbolic");
    move_up.set_tooltip_text(Some("Move record up"));
    let move_down = gtk::Button::from_icon_name("go-down-symbolic");
    move_down.set_tooltip_text(Some("Move record down"));
    move_up.add_css_class("flat");
    move_down.add_css_class("flat");
    let inspector_actions = gtk::Box::new(Orientation::Horizontal, 6);
    inspector_actions.append(&edit);
    inspector_actions.append(&remove);
    inspector_actions.append(&move_up);
    inspector_actions.append(&move_down);
    inspector.append(&inspector_kicker);
    inspector.append(&inspector_title);
    inspector.append(&inspector_kind);
    inspector.append(&inspector_fields);
    inspector.append(&inspector_payload);
    inspector.append(&inspector_valid);
    inspector.append(&inspector_actions);

    let content = gtk::Box::new(Orientation::Horizontal, 12);
    content.set_vexpand(true);
    content.append(&left_scroll);
    content.append(&inspector);

    let feedback = gtk::Label::new(None);
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    feedback.set_hexpand(true);
    feedback.add_css_class("caption");
    feedback.add_css_class("dim-label");
    let export = gtk::Button::with_label("Save .ndef…");
    let format = gtk::Button::with_label("Format NDEF area…");
    format.add_css_class("destructive-action");
    format.set_sensitive(false);
    let read_only = gtk::CheckButton::with_label("Leave the NDEF area read-only");
    let write = gtk::Button::with_label("Write NDEF");
    write.add_css_class("suggested-action");
    write.set_sensitive(false);
    let actions = gtk::Box::new(Orientation::Horizontal, 8);
    actions.append(&feedback);
    actions.append(&export);
    actions.append(&format);
    actions.append(&read_only);
    actions.append(&write);

    root.append(&state_row);
    root.append(&facts);
    root.append(&content);
    root.append(&actions);

    NdefView {
        root,
        state,
        edit_state,
        facts,
        capacity_used,
        capacity_meta,
        capacity_meter,
        capacity_state,
        rows,
        selection,
        rebuilding: Cell::new(false),
        inspector_title,
        inspector_kind,
        inspector_fields,
        inspector_payload,
        inspector_valid,
        edit,
        remove,
        move_up,
        move_down,
        add,
        load,
        export,
        reload,
        clear,
        validate,
        preflight_tlv,
        preflight_fit,
        preflight_protected,
        raw_head,
        raw_hex,
        feedback,
        write,
        format,
        read_only,
        editor: RefCell::new(NdefEditor::default()),
        editor_key: RefCell::new(None),
        force_editor_reset: Cell::new(false),
        pending_record_dialog: RefCell::new(None),
    }
}

fn build_desfire_view() -> DesfireView {
    let inventory_page = page_box();
    let refresh = gtk::Button::with_label("Refresh card");
    refresh.add_css_class("suggested-action");
    let summary = gtk::Label::new(Some("Read the card to list DESFire applications."));
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.set_selectable(true);
    let summary_scroll = gtk::ScrolledWindow::builder()
        .child(&summary)
        .vexpand(true)
        .build();
    inventory_page.append(&refresh);
    inventory_page.append(&summary_scroll);

    let read_page = page_box();
    let form = gtk::Grid::new();
    form.set_row_spacing(8);
    form.set_column_spacing(12);

    let mut row = 0;
    let mut add_field = |label: &str, widget: &gtk::Widget| {
        let caption = gtk::Label::new(Some(label));
        caption.set_xalign(0.0);
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        form.attach(&caption, 0, row, 1, 1);
        form.attach(widget, 1, row, 1, 1);
        widget.set_hexpand(true);
        row += 1;
    };

    let aid = gtk::Entry::new();
    aid.set_placeholder_text(Some("A1B2C3"));
    aid.set_max_width_chars(12);
    add_field("Application ID", aid.upcast_ref());

    let file = gtk::Entry::new();
    file.set_placeholder_text(Some("01"));
    file.set_max_width_chars(4);
    add_field("File ID", file.upcast_ref());

    let key = gtk::PasswordEntry::new();
    key.set_show_peek_icon(true);
    key.set_placeholder_text(Some("16 or 32 hex digits; leave empty for free access"));
    add_field("Legacy key (optional)", key.upcast_ref());

    let read_key_type =
        gtk::DropDown::from_strings(&["Auto", "DES", "2K3DES", "3K3DES", "AES", "AES EV2"]);
    read_key_type.set_selected(0);
    add_field("Key type", read_key_type.upcast_ref());

    let key_number = gtk::Entry::new();
    key_number.set_text("0");
    key_number.set_max_width_chars(4);
    add_field("Key number", key_number.upcast_ref());

    let offset = gtk::Entry::new();
    offset.set_text("0");
    offset.set_max_width_chars(10);
    add_field("Offset", offset.upcast_ref());

    let length = gtk::Entry::new();
    length.set_placeholder_text(Some("rest of file"));
    length.set_max_width_chars(10);
    add_field("Length (optional)", length.upcast_ref());

    let start_record = gtk::Entry::new();
    start_record.set_text("0");
    start_record.set_max_width_chars(10);
    add_field("Start record (0 = newest)", start_record.upcast_ref());

    let record_count = gtk::Entry::new();
    record_count.set_placeholder_text(Some("rest of file"));
    record_count.set_max_width_chars(10);
    add_field("Record count (optional)", record_count.upcast_ref());

    let read = gtk::Button::with_label("Read file");
    read.add_css_class("suggested-action");
    let hint = gtk::Label::new(Some(
        "Read the card, then enter an application ID and file ID. The file type selects data, value, or record reading.",
    ));
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    hint.add_css_class("caption");
    hint.add_css_class("dim-label");

    let output = gtk::Label::new(None);
    output.set_xalign(0.0);
    output.set_wrap(true);
    output.set_selectable(true);
    output.add_css_class("monospace");
    let output_scroll = gtk::ScrolledWindow::builder()
        .child(&output)
        .vexpand(true)
        .build();
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("caption");
    status.add_css_class("dim-label");

    read_page.append(&form);
    read_page.append(&hint);
    read_page.append(&read);
    read_page.append(&output_scroll);
    read_page.append(&status);

    let write_page = page_box();
    let write_form = gtk::Grid::new();
    write_form.set_row_spacing(8);
    write_form.set_column_spacing(12);
    let mut write_row = 0;
    let mut add_write_field = |label: &str, widget: &gtk::Widget| {
        let caption = gtk::Label::new(Some(label));
        caption.set_xalign(0.0);
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        write_form.attach(&caption, 0, write_row, 1, 1);
        write_form.attach(widget, 1, write_row, 1, 1);
        widget.set_hexpand(true);
        write_row += 1;
    };

    let write_aid = gtk::Entry::new();
    write_aid.set_placeholder_text(Some("A1B2C3"));
    write_aid.set_max_width_chars(12);
    add_write_field("Application ID", write_aid.upcast_ref());

    let write_file = gtk::Entry::new();
    write_file.set_placeholder_text(Some("01"));
    write_file.set_max_width_chars(4);
    add_write_field("File ID", write_file.upcast_ref());

    let write_key = gtk::PasswordEntry::new();
    write_key.set_show_peek_icon(true);
    write_key.set_placeholder_text(Some("optional; required unless the file is free"));
    add_write_field("Key (optional)", write_key.upcast_ref());

    let write_key_type =
        gtk::DropDown::from_strings(&["Auto", "DES", "2K3DES", "3K3DES", "AES", "AES EV2"]);
    write_key_type.set_selected(0);
    add_write_field("Key type", write_key_type.upcast_ref());

    let write_key_number = gtk::Entry::new();
    write_key_number.set_text("0");
    write_key_number.set_max_width_chars(4);
    add_write_field("Key number", write_key_number.upcast_ref());

    let write_offset = gtk::Entry::new();
    write_offset.set_text("0");
    write_offset.set_max_width_chars(10);
    add_write_field("Offset", write_offset.upcast_ref());

    let write_encoding = gtk::DropDown::from_strings(&["Hex", "Text"]);
    write_encoding.set_selected(0);
    add_write_field("Payload encoding", write_encoding.upcast_ref());

    let write_data = gtk::Entry::new();
    write_data.set_placeholder_text(Some("AA BB CC or UTF-8 text"));
    add_write_field("Data", write_data.upcast_ref());

    let write = gtk::Button::with_label("Write file");
    write.add_css_class("suggested-action");
    let write_hint = gtk::Label::new(Some(
        "The current file content is exported before the write, the card identity is \
         checked, and the written range is read back. A file this session cannot read \
         back is refused because the write cannot be verified.",
    ));
    write_hint.set_xalign(0.0);
    write_hint.set_wrap(true);
    write_hint.add_css_class("caption");
    write_hint.add_css_class("dim-label");
    let write_output = gtk::Label::new(None);
    write_output.set_xalign(0.0);
    write_output.set_wrap(true);
    write_output.set_selectable(true);
    write_output.add_css_class("monospace");
    let write_output_scroll = gtk::ScrolledWindow::builder()
        .child(&write_output)
        .vexpand(true)
        .build();
    let write_status = gtk::Label::new(None);
    write_status.set_xalign(0.0);
    write_status.add_css_class("caption");
    write_status.add_css_class("dim-label");
    write_page.append(&write_form);
    write_page.append(&write_hint);
    write_page.append(&write);
    write_page.append(&write_output_scroll);
    write_page.append(&write_status);

    let verify_page = page_box();
    let verify_form = gtk::Grid::new();
    verify_form.set_row_spacing(8);
    verify_form.set_column_spacing(12);
    let mut verify_row = 0;
    let mut add_verify_field = |label: &str, widget: &gtk::Widget| {
        let caption = gtk::Label::new(Some(label));
        caption.set_xalign(0.0);
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        verify_form.attach(&caption, 0, verify_row, 1, 1);
        verify_form.attach(widget, 1, verify_row, 1, 1);
        widget.set_hexpand(true);
        verify_row += 1;
    };
    let verify_aid = gtk::Entry::new();
    verify_aid.set_placeholder_text(Some("A1B2C3"));
    verify_aid.set_max_width_chars(12);
    add_verify_field("Application ID", verify_aid.upcast_ref());
    let verify_key = gtk::PasswordEntry::new();
    verify_key.set_show_peek_icon(true);
    verify_key.set_placeholder_text(Some("DES 16 hex digits; AES/2K3DES 32; 3K3DES 48"));
    add_verify_field("Key", verify_key.upcast_ref());
    let verify_key_type =
        gtk::DropDown::from_strings(&["Auto", "DES", "2K3DES", "3K3DES", "AES", "AES EV2"]);
    verify_key_type.set_selected(0);
    add_verify_field("Key type", verify_key_type.upcast_ref());
    let verify_key_number = gtk::Entry::new();
    verify_key_number.set_text("0");
    verify_key_number.set_max_width_chars(4);
    add_verify_field("Key number", verify_key_number.upcast_ref());

    let verify_hint = gtk::Label::new(Some(
        "Authenticate an application key without reading file data. The card UID is \
         returned only after authentication, and any response check must verify.",
    ));
    verify_hint.set_xalign(0.0);
    verify_hint.set_wrap(true);
    verify_hint.add_css_class("caption");
    verify_hint.add_css_class("dim-label");

    let verify_buttons = gtk::Box::new(Orientation::Horizontal, 8);
    let verify = gtk::Button::with_label("Verify key");
    verify.add_css_class("suggested-action");
    let read_uid = gtk::Button::with_label("Read UID");
    verify_buttons.append(&verify);
    verify_buttons.append(&read_uid);

    let verify_output = gtk::Label::new(None);
    verify_output.set_xalign(0.0);
    verify_output.set_wrap(true);
    verify_output.set_selectable(true);
    verify_output.add_css_class("monospace");
    let verify_output_scroll = gtk::ScrolledWindow::builder()
        .child(&verify_output)
        .vexpand(true)
        .build();
    let verify_status = gtk::Label::new(None);
    verify_status.set_xalign(0.0);
    verify_status.add_css_class("caption");
    verify_status.add_css_class("dim-label");
    verify_page.append(&verify_form);
    verify_page.append(&verify_hint);
    verify_page.append(&verify_buttons);
    verify_page.append(&verify_output_scroll);
    verify_page.append(&verify_status);

    let manage_page = page_box();
    let advanced_row = gtk::Box::new(Orientation::Horizontal, 8);
    let advanced_label = gtk::Label::new(Some("Advanced mode"));
    advanced_label.set_xalign(0.0);
    advanced_label.set_hexpand(true);
    let advanced = gtk::Switch::new();
    advanced_row.append(&advanced_label);
    advanced_row.append(&advanced);

    let manage_hint = gtk::Label::new(Some(
        "Create and delete applications and all six file types, change file settings, \
         and read free EEPROM bytes. Creation checks the free space and reads the new \
         settings back. Deletion requires Advanced mode, writes a manifest first, and is \
         not a byte-for-byte restore.",
    ));
    manage_hint.set_xalign(0.0);
    manage_hint.set_wrap(true);
    manage_hint.add_css_class("caption");
    manage_hint.add_css_class("dim-label");

    let manage_form = gtk::Grid::new();
    manage_form.set_row_spacing(8);
    manage_form.set_column_spacing(12);
    let mut manage_row = 0;
    let mut add_manage_field = |label: &str, widget: &gtk::Widget| {
        let caption = gtk::Label::new(Some(label));
        caption.set_xalign(0.0);
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        manage_form.attach(&caption, 0, manage_row, 1, 1);
        manage_form.attach(widget, 1, manage_row, 1, 1);
        widget.set_hexpand(true);
        manage_row += 1;
    };

    let manage_aid = gtk::Entry::new();
    manage_aid.set_placeholder_text(Some("A1B2C3"));
    manage_aid.set_max_width_chars(12);
    add_manage_field("Application ID", manage_aid.upcast_ref());

    let manage_key = gtk::PasswordEntry::new();
    manage_key.set_show_peek_icon(true);
    manage_key.set_placeholder_text(Some("PICC or application key; required by the card"));
    add_manage_field("Key (optional)", manage_key.upcast_ref());

    let manage_key_type =
        gtk::DropDown::from_strings(&["Auto", "DES", "2K3DES", "3K3DES", "AES", "AES EV2"]);
    manage_key_type.set_selected(0);
    add_manage_field("Key type", manage_key_type.upcast_ref());

    let manage_key_number = gtk::Entry::new();
    manage_key_number.set_text("0");
    manage_key_number.set_max_width_chars(4);
    add_manage_field("Key number", manage_key_number.upcast_ref());

    let manage_app_keys = gtk::Entry::new();
    manage_app_keys.set_text("1");
    manage_app_keys.set_max_width_chars(4);
    add_manage_field("Application key count", manage_app_keys.upcast_ref());

    let manage_app_key_type = gtk::DropDown::from_strings(&["2K3DES", "3K3DES", "AES"]);
    manage_app_key_type.set_selected(0);
    add_manage_field("Application key type", manage_app_key_type.upcast_ref());

    let manage_file = gtk::Entry::new();
    manage_file.set_placeholder_text(Some("01"));
    manage_file.set_max_width_chars(4);
    add_manage_field("File ID", manage_file.upcast_ref());

    let manage_iso_file = gtk::Entry::new();
    manage_iso_file.set_placeholder_text(Some("ISO file ID, optional"));
    manage_iso_file.set_max_width_chars(6);
    add_manage_field("ISO file ID (optional)", manage_iso_file.upcast_ref());

    let manage_file_type = gtk::DropDown::from_strings(&[
        "Standard",
        "Backup",
        "Value",
        "Linear record",
        "Cyclic record",
        "Transaction MAC",
    ]);
    manage_file_type.set_selected(0);
    add_manage_field("File type", manage_file_type.upcast_ref());

    let manage_communication = gtk::DropDown::from_strings(&["Plain", "MACed", "Enciphered"]);
    manage_communication.set_selected(0);
    add_manage_field("Communication", manage_communication.upcast_ref());

    let manage_access_rights = gtk::Entry::new();
    manage_access_rights.set_text("0000");
    manage_access_rights.set_max_width_chars(6);
    manage_access_rights.set_placeholder_text(Some("EEEE free, 0000 key 0"));
    add_manage_field("Access rights (hex)", manage_access_rights.upcast_ref());

    let manage_size = gtk::Entry::new();
    manage_size.set_placeholder_text(Some("standard/backup bytes"));
    manage_size.set_max_width_chars(8);
    add_manage_field("Size", manage_size.upcast_ref());

    let manage_lower_limit = gtk::Entry::new();
    manage_lower_limit.set_placeholder_text(Some("value lower limit"));
    add_manage_field("Lower limit", manage_lower_limit.upcast_ref());

    let manage_upper_limit = gtk::Entry::new();
    manage_upper_limit.set_placeholder_text(Some("value upper limit"));
    add_manage_field("Upper limit", manage_upper_limit.upcast_ref());

    let manage_value = gtk::Entry::new();
    manage_value.set_placeholder_text(Some("initial value"));
    add_manage_field("Value", manage_value.upcast_ref());

    let manage_flags = gtk::Entry::new();
    manage_flags.set_text("00");
    manage_flags.set_max_width_chars(4);
    manage_flags.set_placeholder_text(Some("01 limited credit, 02 free GetValue"));
    add_manage_field("Value flags", manage_flags.upcast_ref());

    let manage_record_size = gtk::Entry::new();
    manage_record_size.set_placeholder_text(Some("record bytes"));
    manage_record_size.set_max_width_chars(8);
    add_manage_field("Record size", manage_record_size.upcast_ref());

    let manage_max_records = gtk::Entry::new();
    manage_max_records.set_placeholder_text(Some("maximum records"));
    manage_max_records.set_max_width_chars(8);
    add_manage_field("Max records", manage_max_records.upcast_ref());

    let manage_tmackey = gtk::PasswordEntry::new();
    manage_tmackey.set_show_peek_icon(true);
    manage_tmackey.set_placeholder_text(Some("Transaction MAC AES key, 32 hex digits"));
    add_manage_field("Transaction MAC key", manage_tmackey.upcast_ref());

    let manage_tmackey_version = gtk::Entry::new();
    manage_tmackey_version.set_text("0");
    manage_tmackey_version.set_max_width_chars(4);
    add_manage_field(
        "Transaction MAC key version",
        manage_tmackey_version.upcast_ref(),
    );

    let app_buttons = gtk::Box::new(Orientation::Horizontal, 8);
    let create_app = gtk::Button::with_label("Create application");
    create_app.add_css_class("suggested-action");
    let delete_app = gtk::Button::with_label("Delete application");
    delete_app.add_css_class("destructive-action");
    let read_memory = gtk::Button::with_label("Read free memory");
    app_buttons.append(&create_app);
    app_buttons.append(&read_memory);
    app_buttons.append(&delete_app);

    let file_buttons = gtk::Box::new(Orientation::Horizontal, 8);
    let create_file = gtk::Button::with_label("Create file");
    create_file.add_css_class("suggested-action");
    let change_file_settings = gtk::Button::with_label("Change file settings");
    let delete_file = gtk::Button::with_label("Delete file");
    delete_file.add_css_class("destructive-action");
    file_buttons.append(&create_file);
    file_buttons.append(&change_file_settings);
    file_buttons.append(&delete_file);

    let manage_output = gtk::Label::new(None);
    manage_output.set_xalign(0.0);
    manage_output.set_wrap(true);
    manage_output.set_selectable(true);
    manage_output.add_css_class("monospace");
    let manage_output_scroll = gtk::ScrolledWindow::builder()
        .child(&manage_output)
        .vexpand(true)
        .build();
    let manage_status = gtk::Label::new(None);
    manage_status.set_xalign(0.0);
    manage_status.set_wrap(true);
    manage_status.set_selectable(true);
    manage_status.add_css_class("caption");
    manage_status.add_css_class("dim-label");
    let manage_form_scroll = gtk::ScrolledWindow::builder()
        .child(&manage_form)
        .vexpand(true)
        .build();
    manage_page.append(&advanced_row);
    manage_page.append(&manage_hint);
    manage_page.append(&manage_form_scroll);
    manage_page.append(&app_buttons);
    manage_page.append(&file_buttons);
    manage_page.append(&manage_output_scroll);
    manage_page.append(&manage_status);

    let ndef_page = page_box();

    let ndef_hint = gtk::Label::new(Some(
        "Read the NDEF Tag Application without authenticating. Formatting creates the \
         application D27600 with the CC and NDEF files; no full-card backup exists, and \
         deleting the application is the undo path. Both actions need Advanced mode, \
         which is on the Manage tab.",
    ));
    ndef_hint.set_xalign(0.0);
    ndef_hint.set_wrap(true);
    ndef_hint.add_css_class("caption");
    ndef_hint.add_css_class("dim-label");

    let ndef_form = gtk::Grid::new();
    ndef_form.set_row_spacing(8);
    ndef_form.set_column_spacing(12);
    let ndef_key_caption = gtk::Label::new(Some("PICC master key"));
    ndef_key_caption.set_xalign(0.0);
    ndef_key_caption.add_css_class("caption");
    ndef_key_caption.add_css_class("dim-label");
    let ndef_key = gtk::PasswordEntry::new();
    ndef_key.set_show_peek_icon(true);
    ndef_key.set_placeholder_text(Some("16 or 32 hex digits"));
    ndef_form.attach(&ndef_key_caption, 0, 0, 1, 1);
    ndef_form.attach(&ndef_key, 1, 0, 1, 1);
    ndef_key.set_hexpand(true);
    let ndef_key_type_caption = gtk::Label::new(Some("Key type"));
    ndef_key_type_caption.set_xalign(0.0);
    ndef_key_type_caption.add_css_class("caption");
    ndef_key_type_caption.add_css_class("dim-label");
    let ndef_key_type =
        gtk::DropDown::from_strings(&["Auto", "DES", "2K3DES", "3K3DES", "AES", "AES EV2"]);
    ndef_key_type.set_selected(0);
    ndef_form.attach(&ndef_key_type_caption, 0, 1, 1, 1);
    ndef_form.attach(&ndef_key_type, 1, 1, 1, 1);
    let ndef_key_number_caption = gtk::Label::new(Some("PICC key number"));
    ndef_key_number_caption.set_xalign(0.0);
    ndef_key_number_caption.add_css_class("caption");
    ndef_key_number_caption.add_css_class("dim-label");
    let ndef_key_number = gtk::Entry::new();
    ndef_key_number.set_text("0");
    ndef_key_number.set_max_width_chars(4);
    ndef_form.attach(&ndef_key_number_caption, 0, 2, 1, 1);
    ndef_form.attach(&ndef_key_number, 1, 2, 1, 1);

    let read_ndef = gtk::Button::with_label("Read NDEF");
    read_ndef.add_css_class("suggested-action");
    let ndef_output = gtk::Label::new(None);
    ndef_output.set_xalign(0.0);
    ndef_output.set_wrap(true);
    ndef_output.set_selectable(true);
    ndef_output.add_css_class("monospace");
    let ndef_output_scroll = gtk::ScrolledWindow::builder()
        .child(&ndef_output)
        .vexpand(true)
        .build();

    let format_ndef = gtk::Button::with_label("Format NDEF application");
    format_ndef.add_css_class("destructive-action");
    let delete_ndef = gtk::Button::with_label("Delete NDEF application");
    delete_ndef.add_css_class("destructive-action");
    let ndef_buttons = gtk::Box::new(Orientation::Horizontal, 8);
    ndef_buttons.append(&format_ndef);
    ndef_buttons.append(&delete_ndef);
    let ndef_status = gtk::Label::new(None);
    ndef_status.set_xalign(0.0);
    ndef_status.set_wrap(true);
    ndef_status.set_selectable(true);
    ndef_status.add_css_class("caption");
    ndef_status.add_css_class("dim-label");
    ndef_page.append(&ndef_hint);
    ndef_page.append(&ndef_form);
    ndef_page.append(&read_ndef);
    ndef_page.append(&ndef_buttons);
    ndef_page.append(&ndef_status);
    ndef_page.append(&ndef_output_scroll);

    let mutation_page = page_box();
    let mutation_form = gtk::Grid::new();
    mutation_form.set_row_spacing(8);
    mutation_form.set_column_spacing(12);
    let mut mutation_row = 0;
    let mut add_mutation_field = |label: &str, widget: &gtk::Widget| {
        let caption = gtk::Label::new(Some(label));
        caption.set_xalign(0.0);
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        mutation_form.attach(&caption, 0, mutation_row, 1, 1);
        mutation_form.attach(widget, 1, mutation_row, 1, 1);
        widget.set_hexpand(true);
        mutation_row += 1;
    };

    let mutation_aid = gtk::Entry::new();
    mutation_aid.set_placeholder_text(Some("A1B2C3"));
    mutation_aid.set_max_width_chars(12);
    add_mutation_field("Application ID", mutation_aid.upcast_ref());

    let mutation_file = gtk::Entry::new();
    mutation_file.set_placeholder_text(Some("01"));
    mutation_file.set_max_width_chars(4);
    add_mutation_field("File ID", mutation_file.upcast_ref());

    let mutation_key = gtk::PasswordEntry::new();
    mutation_key.set_show_peek_icon(true);
    mutation_key.set_placeholder_text(Some("optional; required unless the file is free"));
    add_mutation_field("Key (optional)", mutation_key.upcast_ref());

    let mutation_key_type =
        gtk::DropDown::from_strings(&["Auto", "DES", "2K3DES", "3K3DES", "AES", "AES EV2"]);
    mutation_key_type.set_selected(0);
    add_mutation_field("Key type", mutation_key_type.upcast_ref());

    let mutation_key_number = gtk::Entry::new();
    mutation_key_number.set_text("0");
    mutation_key_number.set_max_width_chars(4);
    add_mutation_field("Key number", mutation_key_number.upcast_ref());

    let mutation_tmac_key = gtk::PasswordEntry::new();
    mutation_tmac_key.set_show_peek_icon(true);
    mutation_tmac_key.set_placeholder_text(Some("AppTransactionMACKey, 32 hex digits"));
    add_mutation_field(
        "Transaction MAC key (optional)",
        mutation_tmac_key.upcast_ref(),
    );

    let mutation_operation = gtk::DropDown::from_strings(&[
        "Value credit",
        "Value debit",
        "Value limited credit",
        "Record append",
        "Record update",
        "Record clear",
    ]);
    mutation_operation.set_selected(0);
    add_mutation_field("Operation", mutation_operation.upcast_ref());

    let mutation_amount = gtk::Entry::new();
    mutation_amount.set_placeholder_text(Some("credit/debit/limited-credit amount"));
    mutation_amount.set_max_width_chars(12);
    add_mutation_field("Amount", mutation_amount.upcast_ref());

    let mutation_record = gtk::Entry::new();
    mutation_record.set_text("0");
    mutation_record.set_placeholder_text(Some("0 = newest record"));
    mutation_record.set_max_width_chars(10);
    add_mutation_field("Record number (update)", mutation_record.upcast_ref());

    let mutation_offset = gtk::Entry::new();
    mutation_offset.set_text("0");
    mutation_offset.set_max_width_chars(10);
    add_mutation_field("Record offset (update)", mutation_offset.upcast_ref());

    let mutation_data = gtk::Entry::new();
    mutation_data.set_placeholder_text(Some("record bytes as hex"));
    add_mutation_field("Record data (append/update)", mutation_data.upcast_ref());

    let mutation_run = gtk::Button::with_label("Apply value or record change");
    mutation_run.add_css_class("suggested-action");
    let mutation_hint = gtk::Label::new(Some(
        "Value changes check the file limits before the command, commit the transaction,          and read the value back. Record changes read every record first, check the record          bounds, commit, and read every record back. A Transaction MAC is shown as verified          only when the supplied AppTransactionMACKey reproduces it.",
    ));
    mutation_hint.set_xalign(0.0);
    mutation_hint.set_wrap(true);
    mutation_hint.add_css_class("caption");
    mutation_hint.add_css_class("dim-label");
    let mutation_output = gtk::Label::new(None);
    mutation_output.set_xalign(0.0);
    mutation_output.set_wrap(true);
    mutation_output.set_selectable(true);
    mutation_output.add_css_class("monospace");
    let mutation_output_scroll = gtk::ScrolledWindow::builder()
        .child(&mutation_output)
        .vexpand(true)
        .build();
    let mutation_status = gtk::Label::new(None);
    mutation_status.set_xalign(0.0);
    mutation_status.set_wrap(true);
    mutation_status.set_selectable(true);
    mutation_status.add_css_class("caption");
    mutation_status.add_css_class("dim-label");
    let mutation_form_scroll = gtk::ScrolledWindow::builder()
        .child(&mutation_form)
        .vexpand(true)
        .build();
    mutation_page.append(&mutation_hint);
    mutation_page.append(&mutation_form_scroll);
    mutation_page.append(&mutation_run);
    mutation_page.append(&mutation_output_scroll);
    mutation_page.append(&mutation_status);

    let tabs = gtk::Stack::new();
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.set_vexpand(true);
    tabs.add_titled(&inventory_page, Some("inventory"), "Inventory");
    tabs.add_titled(&read_page, Some("read"), "Read");
    tabs.add_titled(&write_page, Some("write"), "Write");
    tabs.add_titled(&verify_page, Some("verify"), "Verify");
    tabs.add_titled(&manage_page, Some("manage"), "Manage");
    tabs.add_titled(&mutation_page, Some("mutation"), "Values & records");
    tabs.add_titled(&ndef_page, Some("ndef"), "NDEF");
    let (root, switcher) = grouped_page(&tabs);
    DesfireView {
        root,
        tabs,
        switcher,
        summary,
        refresh,
        aid,
        file,
        key,
        read_key_type,
        key_number,
        offset,
        length,
        start_record,
        record_count,
        read,
        output,
        status,
        write_aid,
        write_file,
        write_key,
        write_key_type,
        write_key_number,
        write_offset,
        write_data,
        write_encoding,
        write,
        write_output,
        write_status,
        verify_aid,
        verify_key,
        verify_key_type,
        verify_key_number,
        verify,
        read_uid,
        verify_output,
        verify_status,
        advanced,
        manage_aid,
        manage_key,
        manage_key_type,
        manage_key_number,
        manage_app_keys,
        manage_app_key_type,
        create_app,
        delete_app,
        manage_file,
        manage_iso_file,
        manage_file_type,
        manage_communication,
        manage_access_rights,
        manage_size,
        manage_lower_limit,
        manage_upper_limit,
        manage_value,
        manage_flags,
        manage_record_size,
        manage_max_records,
        manage_tmackey,
        manage_tmackey_version,
        create_file,
        delete_file,
        change_file_settings,
        read_memory,
        manage_output,
        manage_status,
        mutation_aid,
        mutation_file,
        mutation_key,
        mutation_key_type,
        mutation_key_number,
        mutation_tmac_key,
        mutation_operation,
        mutation_amount,
        mutation_record,
        mutation_offset,
        mutation_data,
        mutation_run,
        mutation_output,
        mutation_status,
        ndef_key,
        ndef_key_type,
        ndef_key_number,
        read_ndef,
        format_ndef,
        delete_ndef,
        ndef_status,
        ndef_output,
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
    summary.set_selectable(true);
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
    let data_column =
        copy_text_column::<type2_ui::PageRow>("Data", 180, true, |row| row.data.clone());
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
        "Restore writes only the selected page classes after the confirmation dialog.",
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
    tabs.add_titled(&memory, Some("pages"), "Pages");

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

    let access_card = gtk::Box::new(Orientation::Vertical, 6);
    access_card.add_css_class("card");
    let access_title = gtk::Label::new(Some("Protected access"));
    access_title.set_xalign(0.0);
    access_title.add_css_class("title-4");
    let auth_pwd = secret_field_row("Access PWD", "8 hex digits");
    let auth_pack = secret_field_row("Access PACK", "4 hex digits");
    let access_note = gtk::Label::new(Some(
        "Used for reads and writes on a tag protected by AUTH0, and as the current credentials when changing the configuration. Cleared after each operation and never saved.",
    ));
    access_note.set_xalign(0.0);
    access_note.set_wrap(true);
    access_note.add_css_class("caption");
    access_note.add_css_class("dim-label");
    access_card.append(&access_title);
    access_card.append(&auth_pwd.0);
    access_card.append(&auth_pack.0);
    access_card.append(&access_note);

    let password_card = gtk::Box::new(Orientation::Vertical, 6);
    password_card.add_css_class("card");
    let password_title = gtk::Label::new(Some("Password and access"));
    password_title.set_xalign(0.0);
    password_title.add_css_class("title-4");
    let auth0 = field_row("AUTH0", "255");
    let access = field_row("ACCESS", "00");
    let pwd = secret_field_row("PWD", "8 hex digits");
    let pack = secret_field_row("PACK", "4 hex digits");
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

    let uid_card = gtk::Box::new(Orientation::Vertical, 6);
    uid_card.add_css_class("card");
    let uid_title = gtk::Label::new(Some("UID (magic clone)"));
    uid_title.set_xalign(0.0);
    uid_title.add_css_class("title-4");
    let uid_row = field_row("Target UID", "14 hex digits");
    let uid_target = uid_row.1;
    let uid_apply = gtk::Button::with_label("Change UID…");
    uid_apply.add_css_class("destructive-action");
    uid_apply.set_sensitive(false);
    let uid_note = gtk::Label::new(Some(
        "Only a UID-writable NTAG213 clone accepts this. A full backup is written first; page 0/1/2 are read back and reselected, and the original UID is restored on failure.",
    ));
    uid_note.set_xalign(0.0);
    uid_note.set_wrap(true);
    uid_note.add_css_class("caption");
    uid_note.add_css_class("dim-label");
    uid_card.append(&uid_title);
    uid_card.append(&uid_row.0);
    uid_card.append(&uid_apply);
    uid_card.append(&uid_note);

    protection_root.append(&config_card);
    protection_root.append(&access_card);
    protection_root.append(&lock_card);
    protection_root.append(&password_card);
    protection_root.append(&uid_card);
    tabs.add_titled(&protection_scroll, Some("protection"), "Protection");

    let ndef_slot = gtk::Box::new(Orientation::Vertical, 0);
    ndef_slot.set_vexpand(true);
    tabs.add_titled(&ndef_slot, Some("ndef"), "NDEF");

    let type2_dumps = gtk::Box::new(Orientation::Vertical, 10);
    type2_dumps.set_vexpand(true);
    type2_dumps.append(&toolbar);
    type2_dumps.append(&restore_card);
    tabs.add_titled(&type2_dumps, Some("dumps"), "Dumps");

    root.append(&header);
    root.append(&warnings);
    root.append(&switcher_row);
    root.append(&tabs);

    Type2View {
        root,
        tabs,
        switcher,
        ndef_slot,
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
        auth_pwd: auth_pwd.1,
        auth_pack: auth_pack.1,
        pwd: pwd.1,
        pack: pack.1,
        password_apply,
        password_note,
        uid_target,
        uid_apply,
        uid_note,
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

fn secret_field_row(label: &str, placeholder: &str) -> (gtk::Box, gtk::Entry) {
    let (row, entry) = field_row(label, placeholder);
    entry.set_visibility(false);
    entry.set_input_purpose(gtk::InputPurpose::Password);
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
        "User pages are backed up and read back. Advanced pages need Advanced mode and a confirmation dialog.",
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

fn build_details_view() -> DetailsView {
    let root = page_box();
    let title = gtk::Label::new(Some("Card details and health"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Identity, image completeness, validation findings, and verified key coverage for the \
         loaded image. Values come from the same dump and key store the block table uses; no \
         card is read and no key material is shown.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");
    let body = gtk::Label::new(Some("No card image loaded"));
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_selectable(true);
    body.add_css_class("mono");
    root.append(&title);
    root.append(&note);
    root.append(&body);
    DetailsView { root, body }
}

fn build_keys_view() -> KeysView {
    let root = page_box();
    let title = gtk::Label::new(Some("Sector keys"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Per-card Proxmark3 .dic files are the primary key format. This JSON store is legacy: it \
         keeps sector, A/B, and provenance for imported keys. Built-in keys are candidates and are \
         not stored.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let rows = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(rows.clone()));
    let table = gtk::ColumnView::new(Some(selection.clone()));
    table.set_show_column_separators(true);
    table.set_show_row_separators(true);
    table.append_column(&text_column::<KeyRow>("Sector", 120, false, |row| {
        row.sector.clone()
    }));
    table.append_column(&text_column::<KeyRow>("Type", 70, false, |row| {
        row.kind.to_owned()
    }));
    table.append_column(&copy_text_column::<KeyRow>("Key", 160, true, |row| {
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
    let delete = gtk::Button::with_label("Delete selected");
    delete.set_sensitive(false);
    let save = gtk::Button::with_label("Save store");
    let export = gtk::Button::with_label("Export .dic…");
    let actions = gtk::Box::new(Orientation::Horizontal, 6);
    actions.append(&delete);
    actions.append(&import);
    actions.append(&save);
    actions.append(&export);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("caption");
    status.add_css_class("dim-label");
    let report = gtk::Label::new(None);
    report.set_xalign(0.0);
    report.set_wrap(true);
    report.set_selectable(true);
    report.add_css_class("caption");
    report.add_css_class("dim-label");

    root.append(&title);
    root.append(&note);
    root.append(&scroll);
    root.append(&add_row);
    root.append(&actions);
    root.append(&status);
    root.append(&report);

    KeysView {
        root,
        rows,
        selection,
        status,
        report,
        hex,
        sector,
        kind,
        add,
        delete,
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

    let mode =
        gtk::DropDown::from_strings(&["All known blocks", "Changes only", "Selected blocks"]);
    mode.set_selected(0);
    let blocks = gtk::Entry::new();
    blocks.set_placeholder_text(Some("Blocks, e.g. 1,4,5-7"));
    blocks.set_hexpand(true);
    blocks.set_sensitive(false);
    let selection_row = gtk::Box::new(Orientation::Horizontal, 8);
    selection_row.append(&mode);
    selection_row.append(&blocks);
    let continue_on_error = gtk::CheckButton::with_label("Continue after a failed block");
    continue_on_error.set_active(false);
    let uid_label = gtk::Label::new(Some("Target UID after restore (optional)"));
    uid_label.set_xalign(0.0);
    uid_label.set_hexpand(true);
    uid_label.add_css_class("caption");
    let uid_advanced_label = gtk::Label::new(Some("Advanced mode"));
    uid_advanced_label.set_xalign(0.0);
    uid_advanced_label.add_css_class("caption");
    let uid_advanced = gtk::Switch::new();
    uid_advanced.set_valign(gtk::Align::Center);
    let uid = gtk::Entry::new();
    uid.set_placeholder_text(Some("Target UID, e.g. DEADBEEF"));
    uid.add_css_class("mono");
    uid.set_hexpand(true);
    let uid_row = gtk::Box::new(Orientation::Horizontal, 8);
    uid_row.append(&uid);
    let advanced_row = gtk::Box::new(Orientation::Horizontal, 8);
    advanced_row.append(&uid_label);
    advanced_row.append(&uid_advanced_label);
    advanced_row.append(&uid_advanced);
    let restore = gtk::Button::with_label("Restore to card");
    restore.add_css_class("destructive-action");
    restore.set_sensitive(false);
    let note = gtk::Label::new(Some(
        "Restore backs up the current card first, writes data blocks before sector trailers, \
         and reads every block back. Block 0 is written only when a target UID is supplied, \
         after the restore is verified. A card with a different identity from the image is \
         refused before the backup.",
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
    root.append(&selection_row);
    root.append(&continue_on_error);
    root.append(&uid_row);
    root.append(&advanced_row);
    root.append(&restore);
    root.append(&note);
    root.append(&result);
    root.append(&scroll);

    RestoreView {
        root,
        load,
        file,
        summary,
        mode,
        blocks,
        continue_on_error,
        uid,
        uid_advanced,
        restore,
        rows,
        result,
    }
}

fn build_keyscan_view(preferences: &UiPreferences) -> KeyscanView {
    let root = page_box();
    let title = gtk::Label::new(Some("Key scan"));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    let note = gtk::Label::new(Some(
        "Authenticate the selected keys per sector and verify a hit by reading the sector trailer. \
         The scan never writes the card. First run checks the three standard keys; the selection \
         is remembered. Discovered keys are saved as a per-card Proxmark3 .dic file.",
    ));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let selected = match &preferences.keyscan_keys {
        Some(values) => values
            .iter()
            .filter_map(|hex| Key::from_hex(hex).ok())
            .collect::<Vec<_>>(),
        None => default_keys(),
    };
    let selection_label = gtk::Label::new(None);
    selection_label.set_xalign(0.0);
    selection_label.set_localized(&format!("{} key(s) selected", selected.len()));
    selection_label.add_css_class("caption");
    let select = gtk::Button::with_label("Select keys…");
    let use_defaults = gtk::Button::with_label("Use defaults");
    let selection_row = gtk::Box::new(Orientation::Horizontal, 6);
    selection_row.append(&selection_label);
    selection_row.append(&select);
    selection_row.append(&use_defaults);

    let key_types = gtk::DropDown::from_strings(&["A and B", "A only", "B only"]);
    let dictionary = gtk::Entry::new();
    dictionary.set_placeholder_text(Some("Optional Proxmark3 .dic dictionary"));
    dictionary.set_hexpand(true);
    let choose_dictionary = gtk::Button::with_label("Choose…");
    let dictionary_row = gtk::Box::new(Orientation::Horizontal, 6);
    dictionary_row.append(&dictionary);
    dictionary_row.append(&choose_dictionary);
    let options = gtk::Box::new(Orientation::Horizontal, 8);
    options.append(&selection_row);
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
    table.append_column(&copy_text_column::<SectorKeyRow>(
        "Key A",
        180,
        true,
        |row| row.key_a.clone(),
    ));
    table.append_column(&copy_text_column::<SectorKeyRow>(
        "Key B",
        180,
        true,
        |row| row.key_b.clone(),
    ));
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
        key_types,
        dictionary,
        choose_dictionary,
        select,
        use_defaults,
        selection_label,
        selected: RefCell::new(selected),
        dictionary_keys: RefCell::new(Vec::new()),
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
    let platform = diagnostic_label();
    let drivers = diagnostic_label();
    let firmware = diagnostic_label();
    let capabilities = diagnostic_label();
    let tables = diagnostic_label();
    let readers = diagnostic_label();
    let build = diagnostic_label();
    let refresh = gtk::Button::with_label("Refresh readers");

    root.append(&title);
    root.append(&note);
    root.append(&version);
    root.append(&platform);
    root.append(&drivers);
    root.append(&firmware);
    root.append(&capabilities);
    root.append(&tables);
    root.append(&readers);
    root.append(&build);
    root.append(&refresh);

    DiagnosticsView {
        root,
        version,
        platform,
        drivers,
        firmware,
        capabilities,
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

    let copy_rows = rows.clone();
    table.connect_activate(move |table, position| {
        let Some(object) = copy_rows
            .item(position)
            .and_downcast::<glib::BoxedAnyObject>()
        else {
            return;
        };
        let activity = object.borrow::<ActivityRow>();
        let text = format!(
            "{}\t{}\t{}\t{}",
            activity.time, activity.operation, activity.result, activity.detail
        );
        drop(activity);
        table.clipboard().set_text(&text);
    });
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
    root.append(&setting_row("Use default keys", &common));
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

fn clear_label_selections(widget: &impl IsA<gtk::Widget>) {
    let widget = widget.as_ref();
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.is_selectable()
    {
        label.select_region(0, 0);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        clear_label_selections(&widget);
        child = widget.next_sibling();
    }
}

fn wire_group_focus(stack: &gtk::Stack, switcher: &gtk::StackSwitcher) {
    let signal_stack = stack.clone();
    let signal_switcher = switcher.clone();
    stack.connect_visible_child_name_notify(move |_| {
        if let Some(page) = signal_stack.visible_child() {
            clear_label_selections(&page);
        }
        let idle_stack = signal_stack.clone();
        let idle_switcher = signal_switcher.clone();
        glib::idle_add_local_once(move || {
            if let Some(page) = idle_stack.visible_child() {
                clear_label_selections(&page);
            }
            idle_switcher.grab_focus();
        });
    });
}

fn connect_signals(ui: &Rc<Ui>) {
    let ui2 = ui.clone();
    ui.back.connect_clicked(move |_| {
        let target = ui2.last_primary.borrow().clone();
        match target.as_str() {
            "classic" => switch_workspace(&ui2, Workspace::Classic),
            "type2" => switch_workspace(&ui2, Workspace::Type2),
            _ => switch_workspace(&ui2, Workspace::Classic),
        }
    });
    let ui2 = ui.clone();
    ui.classic_save_dump
        .connect_clicked(move |_| save_dump(&ui2));
    let ui2 = ui.clone();
    ui.classic_load_dump
        .connect_clicked(move |_| load_dump(&ui2));
    let ui2 = ui.clone();
    ui.views.connect_visible_child_name_notify(move |views| {
        let name = views
            .visible_child_name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        if !name.is_empty() && name != "system" {
            ui2.last_primary.replace(name.clone());
        }
        ui2.back.set_visible(name == "system");
        if let Some(page) = views.visible_child() {
            clear_label_selections(&page);
        }
        let ui3 = ui2.clone();
        glib::idle_add_local_once(move || {
            if let Some(page) = ui3.views.visible_child() {
                clear_label_selections(&page);
            }
            if ui3.views.visible_child_name().as_deref() == Some("system") {
                ui3.back.grab_focus();
            } else {
                active_switcher(&ui3).grab_focus();
            }
        });
    });
    wire_group_focus(&ui.keys_tabs, &ui.keys_switcher);
    wire_group_focus(&ui.dumps_tabs, &ui.dumps_switcher);
    wire_group_focus(&ui.system_tabs, &ui.system_switcher);
    let ui2 = ui.clone();
    ui.type2_view
        .tabs
        .connect_visible_child_name_notify(move |_| {
            clear_label_selections(&ui2.type2_view.root);
            let ui3 = ui2.clone();
            glib::idle_add_local_once(move || {
                clear_label_selections(&ui3.type2_view.root);
                ui3.type2_view.switcher.grab_focus();
            });
        });
    let ui2 = ui.clone();
    ui.selection
        .connect_selected_notify(move |_| update_inspector(&ui2));

    let ui2 = ui.clone();
    ui.read_button
        .connect_clicked(move |_| start_read(ui2.clone()));

    let ui2 = ui.clone();
    ui.desfire_view
        .refresh
        .connect_clicked(move |_| start_desfire_inventory(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .read
        .connect_clicked(move |_| start_desfire_read(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .write
        .connect_clicked(move |_| start_desfire_write(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .verify
        .connect_clicked(move |_| start_desfire_verify_key(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .read_uid
        .connect_clicked(move |_| start_desfire_uid(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .read_ndef
        .connect_clicked(move |_| start_desfire_ndef_read(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .format_ndef
        .connect_clicked(move |_| start_desfire_ndef_format(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .delete_ndef
        .connect_clicked(move |_| start_desfire_ndef_delete(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .create_app
        .connect_clicked(move |_| start_desfire_app_create(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .delete_app
        .connect_clicked(move |_| start_desfire_app_delete(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .create_file
        .connect_clicked(move |_| start_desfire_file_create(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .delete_file
        .connect_clicked(move |_| start_desfire_file_delete(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .change_file_settings
        .connect_clicked(move |_| start_desfire_file_settings(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .read_memory
        .connect_clicked(move |_| start_desfire_memory(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .mutation_run
        .connect_clicked(move |_| start_desfire_mutation(ui2.clone()));
    let ui2 = ui.clone();
    ui.desfire_view
        .advanced
        .connect_active_notify(move |_| update_desfire_actions(&ui2));
    wire_group_focus(&ui.desfire_view.tabs, &ui.desfire_view.switcher);

    let ui2 = ui.clone();
    ui.inspector
        .read
        .connect_clicked(move |_| start_read_block(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .write
        .connect_clicked(move |_| stage_edit(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .commit
        .connect_clicked(move |_| confirm_workbench_write(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .undo
        .connect_clicked(move |_| undo_edit(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .revert
        .connect_clicked(move |_| revert_edits(ui2.clone()));

    let ui2 = ui.clone();
    ui.inspector
        .hex
        .connect_changed(move |_| update_write_sensitivity(&ui2));

    let ui2 = ui.clone();
    ui.inspector
        .uid_advanced
        .connect_active_notify(move |_| update_classic_uid_actions(&ui2));

    let ui2 = ui.clone();
    ui.inspector
        .uid_write
        .connect_clicked(move |_| confirm_uid_write(ui2.clone()));

    let ui2 = ui.clone();
    ui.ndef_view
        .selection
        .connect_selected_notify(move |selection| {
            if ui2.ndef_view.rebuilding.get() {
                return;
            }
            let selected = selection.selected();
            ui2.ndef_view
                .editor
                .borrow_mut()
                .select((selected != gtk::INVALID_LIST_POSITION).then_some(selected as usize));
            update_ndef_inspector(&ui2);
            update_ndef_actions(&ui2);
        });
    let ui2 = ui.clone();
    ui.ndef_view
        .add
        .connect_clicked(move |_| open_ndef_record_dialog(ui2.clone(), None));
    let ui2 = ui.clone();
    ui.ndef_view.edit.connect_clicked(move |_| {
        let selected = ui2.ndef_view.editor.borrow().selected();
        open_ndef_record_dialog(ui2.clone(), selected);
    });
    let ui2 = ui.clone();
    ui.ndef_view
        .remove
        .connect_clicked(move |_| remove_selected_ndef_record(&ui2));
    let ui2 = ui.clone();
    ui.ndef_view
        .move_up
        .connect_clicked(move |_| move_selected_ndef_record(&ui2, true));
    let ui2 = ui.clone();
    ui.ndef_view
        .move_down
        .connect_clicked(move |_| move_selected_ndef_record(&ui2, false));
    let ui2 = ui.clone();
    ui.ndef_view
        .load
        .connect_clicked(move |_| choose_ndef_file(&ui2));
    let ui2 = ui.clone();
    ui.ndef_view
        .reload
        .connect_clicked(move |_| reload_ndef_from_card(&ui2));
    let ui2 = ui.clone();
    ui.ndef_view
        .clear
        .connect_clicked(move |_| clear_ndef_message(&ui2));
    let ui2 = ui.clone();
    ui.ndef_view
        .validate
        .connect_clicked(move |_| validate_ndef_message(&ui2));
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
    ui.ndef_view
        .read_only
        .connect_toggled(move |_| update_ndef_actions(&ui2));

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
        &ui.type2_view.auth_pwd,
        &ui.type2_view.auth_pack,
        &ui.type2_view.pwd,
        &ui.type2_view.pack,
    ] {
        let ui2 = ui.clone();
        entry.connect_changed(move |_| {
            update_type2_actions(&ui2);
            update_ndef_actions(&ui2);
        });
    }
    let ui2 = ui.clone();
    ui.type2_view
        .password_apply
        .connect_clicked(move |_| confirm_type2_password(ui2.clone()));
    let ui2 = ui.clone();
    ui.type2_view
        .uid_target
        .connect_changed(move |_| update_type2_actions(&ui2));
    let ui2 = ui.clone();
    ui.type2_view
        .uid_apply
        .connect_clicked(move |_| confirm_type2_uid(ui2.clone()));

    let ui2 = ui.clone();
    ui.keys_view.add.connect_clicked(move |_| add_key(&ui2));
    let ui2 = ui.clone();
    ui.keys_view
        .delete
        .connect_clicked(move |_| delete_key(&ui2));
    let ui2 = ui.clone();
    ui.keys_view
        .selection
        .connect_selected_notify(move |_| update_key_delete(&ui2));
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
        .select
        .connect_clicked(move |_| show_keyscan_selection(&ui2));
    let ui2 = ui.clone();
    ui.keyscan_view.use_defaults.connect_clicked(move |_| {
        *ui2.keyscan_view.selected.borrow_mut() = default_keys();
        update_keyscan_selection_label(&ui2);
        save_keyscan_selection(&ui2);
    });
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
    ui.system_tabs
        .connect_visible_child_name_notify(move |tabs| {
            if tabs.visible_child_name().as_deref() == Some("diagnostics") {
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
    ui.restore_view
        .mode
        .connect_selected_notify(move |dropdown| {
            ui2.restore_view
                .blocks
                .set_sensitive(dropdown.selected() == 2);
        });

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

    menu.append(&menu_section("Reader"));
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
    menu.append(&menu_section("System"));
    for (label, page) in [
        ("Diagnostics", "diagnostics"),
        ("Activity", "activity"),
        ("Settings", "settings"),
    ] {
        let item = menu_button(label);
        let ui = ui.clone();
        let popover = popover.clone();
        item.connect_clicked(move |_| {
            popover.popdown();
            show_system_page(&ui, page);
        });
        menu.append(&item);
    }

    menu.append(&gtk::Separator::new(Orientation::Horizontal));
    let about = menu_button("About NFC Foundry");
    {
        let ui = ui.clone();
        let popover = popover.clone();
        about.connect_clicked(move |_| {
            popover.popdown();
            show_about(&ui);
        });
    }
    menu.append(&about);

    popover.set_child(Some(&menu));
    button.set_popover(Some(&popover));
}

fn menu_section(title: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_localized(title);
    label.set_xalign(0.0);
    label.set_margin_top(6);
    label.set_margin_start(8);
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label
}

fn active_switcher(ui: &Ui) -> gtk::StackSwitcher {
    match ui.workspace.get() {
        Workspace::Classic => ui.classic_switcher.clone(),
        Workspace::Type2 => ui.type2_view.switcher.clone(),
        Workspace::Desfire => ui.desfire_view.switcher.clone(),
    }
}

fn build_workspace_menu(ui: &Rc<Ui>, button: &gtk::MenuButton) {
    let popover = gtk::Popover::new();
    let menu = gtk::Box::new(Orientation::Vertical, 2);
    for workspace in [Workspace::Classic, Workspace::Type2, Workspace::Desfire] {
        let item = menu_button(workspace.label());
        let ui = ui.clone();
        let popover = popover.clone();
        item.connect_clicked(move |_| {
            popover.popdown();
            switch_workspace(&ui, workspace);
        });
        menu.append(&item);
    }
    popover.set_child(Some(&menu));
    button.set_popover(Some(&popover));
}

fn switch_workspace(ui: &Rc<Ui>, workspace: Workspace) {
    ui.workspace.set(workspace);
    ui.workspace_button.set_localized(workspace.label());
    ui.views.set_visible_child_name(workspace.stack_name());
    if workspace == Workspace::Desfire {
        ui.read_button.set_localized("Refresh card");
    } else {
        ui.read_button.set_localized("Read card");
    }
    let ndef = ui.ndef_view.root.upcast_ref::<gtk::Widget>();
    ndef.unparent();
    match workspace {
        Workspace::Classic => ui.classic_ndef_slot.append(&ui.ndef_view.root),
        Workspace::Type2 => ui.type2_view.ndef_slot.append(&ui.ndef_view.root),
        Workspace::Desfire => {}
    }
    update_ndef(ui);
    update_ndef_actions(ui);
    update_card(ui);
}

fn show_system_page(ui: &Rc<Ui>, page: &str) {
    ui.views.set_visible_child_name("system");
    ui.system_tabs.set_visible_child_name(page);
    if page == "diagnostics" {
        render_diagnostics(ui);
    }
}

fn show_about(ui: &Rc<Ui>) {
    let dialog = adw::AboutDialog::builder()
        .application_name("NFC Foundry")
        .application_icon("nfc-foundry")
        .developer_name("NFC Foundry contributors")
        .version(env!("CARGO_PKG_VERSION"))
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/YangtseSu/nfc-foundry-rs")
        .build();
    dialog.present(Some(&ui.window));
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
    text_column_with(title, width, mono, getter, false)
}

fn copy_text_column<T: 'static>(
    title: &str,
    width: i32,
    mono: bool,
    getter: fn(&T) -> String,
) -> gtk::ColumnViewColumn {
    text_column_with(title, width, mono, getter, true)
}

fn text_column_with<T: 'static>(
    title: &str,
    width: i32,
    mono: bool,
    getter: fn(&T) -> String,
    copy_on_double_click: bool,
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
        if copy_on_double_click {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let target = label.clone();
            gesture.connect_pressed(move |_, presses, _, _| {
                if presses == 2 {
                    target.clipboard().set_text(&target.label());
                }
            });
            label.add_controller(gesture);
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
        ui.inspector.commit.set_sensitive(false);
        ui.inspector.undo.set_sensitive(false);
        ui.inspector.revert.set_sensitive(false);
        ui.selected_label.set_localized("No block selected");
        update_classic_uid_actions(ui);
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
        row.sector,
        row.display_status()
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
    let can_edit = !ui.busy.get() && row.complete && row.number != 0;
    ui.inspector.hex.set_sensitive(can_edit);
    ui.inspector
        .read
        .set_sensitive(ui.live.get() && !ui.busy.get());
    ui.selected_label.set_localized(&format!(
        "Block {:03} · sector {:02}\n{} · {}",
        row.number,
        row.sector,
        role,
        row.display_status()
    ));
    update_write_sensitivity(ui);
    update_workbench_actions(ui);
    update_classic_uid_actions(ui);
}

fn update_write_sensitivity(ui: &Ui) {
    let Some(row) = selected_row(ui) else {
        ui.inspector.write.set_sensitive(false);
        return;
    };
    if ui.busy.get() || !row.complete || row.number == 0 {
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
        .is_some_and(|block| block.complete() && block.data == data);
    ui.inspector.write.set_sensitive(!same);
}

fn update_workbench_actions(ui: &Ui) {
    let has_image = ui.dump.borrow().is_some();
    let dirty = !ui.dirty_blocks.borrow().is_empty();
    ui.inspector
        .commit
        .set_sensitive(has_image && ui.live.get() && !ui.busy.get() && dirty);
    ui.inspector
        .undo
        .set_sensitive(ui.undo_depth.get() > 0 && !ui.busy.get());
    ui.inspector.revert.set_sensitive(dirty && !ui.busy.get());
}

fn update_card(ui: &Ui) {
    if ui.workspace.get() == Workspace::Desfire {
        ui.card_title.set_localized("DESFire");
        ui.card_detail
            .set_localized("Read the card to list applications and files.");
        ui.detail.set_localized("DESFire");
        return;
    }
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
    render_details(ui);
}

/// Renders the shared health model for the loaded image. Counts and findings
/// come from `nfc_app::classic_health`, the same model `nfctl card health`
/// uses; the view adds only source and live-state labels.
fn render_details(ui: &Ui) {
    let dump = ui.dump.borrow().clone();
    let Some(dump) = dump else {
        ui.details_view.body.set_localized("No card image loaded.");
        return;
    };
    let keys = ui.keys.borrow();
    let dirty = ui.dirty_blocks.borrow();
    let health = classic_health(&dump, &keys, &dirty);
    drop(dirty);
    drop(keys);

    let mut lines: Vec<String> = Vec::new();
    lines.push("Identity".to_owned());
    match &health.identity {
        Some(identity) => {
            lines.push(format!(
                "Card: {} · UID {} ({} bytes)",
                identity.card_type.label(),
                identity.uid,
                identity.uid_length
            ));
            lines.push(format!(
                "ATQA {:02X}{:02X} · SAK {:02X} · inferred {}",
                identity.atqa[0],
                identity.atqa[1],
                identity.sak,
                identity.card_type.label()
            ));
        }
        None => lines.push("Card identity: not recorded".to_owned()),
    }
    lines.push(String::new());
    lines.push("Image".to_owned());
    lines.push(format!(
        "Layout: {}",
        health
            .layout
            .map(classic::display_layout)
            .unwrap_or("unknown")
    ));
    lines.push(format!(
        "Source: {}",
        ui.dump_source.borrow().as_deref().unwrap_or("not recorded")
    ));
    lines.push(format!(
        "State: {}",
        if ui.live.get() {
            "live · writes enabled"
        } else {
            "inspection only · read again to write"
        }
    ));
    if let Some(device) = &health.device {
        lines.push(format!("Reader: {device}"));
    }
    match (health.started_at, health.finished_at) {
        (Some(started), Some(finished)) => {
            lines.push(format!("Read at: unix {started}..={finished}"));
        }
        _ => lines.push("Read at: not recorded".to_owned()),
    }
    lines.push(String::new());
    lines.push("Completeness".to_owned());
    lines.push(format!(
        "Image: {}",
        if health.complete {
            "complete"
        } else {
            "incomplete"
        }
    ));
    lines.push(format!(
        "Blocks: {}/{} readable · sectors: {}/{}",
        health.readable_blocks, health.total_blocks, health.readable_sectors, health.total_sectors
    ));
    if !health.skipped_sectors.is_empty() {
        lines.push(format!(
            "Skipped sectors: {}",
            health
                .skipped_sectors
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    lines.push(String::new());
    lines.push("Keys".to_owned());
    lines.push(format!(
        "Verified: {}/{} sectors · A {} · B {} · both {}",
        health.key_coverage.any_sectors,
        health.key_coverage.sectors,
        health.key_coverage.key_a_sectors,
        health.key_coverage.key_b_sectors,
        health.key_coverage.both_sectors
    ));
    lines.push(format!("Dump keys: {}", health.dump_keys));
    lines.push(String::new());
    lines.push("Workbench".to_owned());
    if health.dirty_blocks.is_empty() {
        lines.push("Dirty: none".to_owned());
    } else {
        lines.push(format!(
            "Dirty: {} block(s) · {}",
            health.dirty_blocks.len(),
            health
                .dirty_blocks
                .iter()
                .map(|block| format!("{block:03}"))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    lines.push(String::new());
    lines.push("Validation".to_owned());
    if health.errors.is_empty() && health.warnings.is_empty() {
        lines.push("Validation: ok".to_owned());
    }
    for error in &health.errors {
        lines.push(format!("Error: {error}"));
    }
    for warning in &health.warnings {
        lines.push(format!("Warning: {warning}"));
    }
    ui.details_view.body.set_localized(&lines.join("\n"));
}

fn update_ndef(ui: &Ui) {
    if ui.workspace.get() == Workspace::Desfire {
        ui.ndef.replace(None);
        return;
    }
    if ui.workspace.get() == Workspace::Type2 {
        let type2 = ui.type2.borrow().clone();
        if let Some(session) = type2 {
            ui.ndef.replace(None);
            render_type2_ndef(ui, &session);
            return;
        }
        ui.ndef.replace(None);
        render_ndef(ui, None, None);
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

/// Renders NDEF facts supplied by the last read or service result. The editor
/// is reloaded only when the card source changed or a read explicitly reset it,
/// so an unsaved draft is not overwritten by unrelated refreshes.
fn render_ndef(ui: &Ui, info: Option<&ClassicNdefInfo>, error: Option<&str>) {
    let view = &ui.ndef_view;
    view.read_only.set_visible(true);

    if ui.dump.borrow().is_none() {
        view.state.set_localized("No card loaded");
        view.facts
            .set_localized("Read a card to inspect its MAD mapping and NDEF message.");
        view.facts.remove_css_class("error");
        refresh_ndef_editor(ui);
        return;
    }
    if let Some(error) = error {
        view.state.set_localized("NDEF unavailable");
        view.facts.set_localized(error);
        view.facts.add_css_class("error");
        refresh_ndef_editor(ui);
        return;
    }
    let Some(info) = info else {
        view.state.set_localized("NDEF unavailable");
        view.facts.set_localized("The NDEF facts are unavailable.");
        view.facts.remove_css_class("error");
        refresh_ndef_editor(ui);
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

    let mut facts = Vec::new();
    if let Some(layout) = &info.layout {
        let sectors = layout
            .sectors
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        facts.push(format!(
            "MAD sectors: {} · mapping version {}.{} · {}",
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
        ));
    } else {
        facts.push("No MAD or NFC sector mapping was found on this card.".to_owned());
    }
    match &info.ndef.state {
        ClassicNdefState::Unformatted => facts.push(
            "Format the NDEF area before writing; Format creates the MAD and NFC sectors."
                .to_owned(),
        ),
        ClassicNdefState::Empty => facts.push(format!(
            "Capacity {} bytes · message empty",
            info.ndef.capacity
        )),
        ClassicNdefState::Message(message) => facts.push(format!(
            "Capacity {} bytes · message {} bytes · {} record(s)",
            info.ndef.capacity,
            info.ndef.message_bytes,
            message.records().len()
        )),
    }
    facts.extend(info.warnings.iter().cloned());
    view.facts.set_localized(&facts.join("\n"));
    view.facts.remove_css_class("error");

    sync_ndef_editor(ui, info.ndef.message(), classic_ndef_key(ui));
    refresh_ndef_editor(ui);
}

fn classic_ndef_key(ui: &Ui) -> String {
    let uid = ui
        .dump
        .borrow()
        .as_ref()
        .and_then(|dump| dump.card.as_ref().map(CardInfo::uid_hex))
        .unwrap_or_default();
    let source = ui.dump_source.borrow().clone().unwrap_or_default();
    let live = if ui.live.get() { "live" } else { "inspect" };
    format!("classic:{live}:{uid}:{source}")
}

/// Load the card message into the editor unless the user has unsaved edits.
/// `force_editor_reset` is set by an explicit read so the old draft cannot
/// outlive a fresh card image.
fn invalidate_ndef_editor(ui: &Ui) {
    ui.ndef_view.editor_key.replace(None);
    ui.ndef_view.force_editor_reset.set(true);
}

fn sync_ndef_editor(ui: &Ui, message: Option<&NdefMessage>, key: String) {
    let force = ui.ndef_view.force_editor_reset.replace(false);
    let current = ui.ndef_view.editor_key.borrow().clone();
    if current.as_deref() == Some(key.as_str()) && !force {
        return;
    }
    if ui.ndef_view.editor.borrow().is_dirty() && !force {
        return;
    }
    ui.ndef_view.editor.borrow_mut().load_message(message);
    ui.ndef_view.editor_key.replace(Some(key));
    populate_ndef_rows(ui);
    update_ndef_inspector(ui);
}

/// Active target facts and the service-level gates for the shared NDEF page.
struct NdefTarget {
    capacity: Option<usize>,
    stored: Option<NdefMessage>,
    write: std::result::Result<(), String>,
    format: std::result::Result<(), String>,
    protected: &'static str,
    loaded: bool,
}

fn ndef_target(ui: &Ui) -> NdefTarget {
    let busy = ui.busy.get();
    let live = ui.live.get();
    if ui.workspace.get() == Workspace::Type2 {
        let session = ui.type2.borrow().clone();
        let Some(session) = session else {
            return NdefTarget {
                capacity: None,
                stored: None,
                write: Err("Read a Type 2 tag first".to_owned()),
                format: Err("Read a Type 2 tag first".to_owned()),
                protected: "Configuration, lock, and password pages are not changed by an NDEF write.",
                loaded: false,
            };
        };
        let auth_ready = type2_auth_ready(ui);
        let ndef = session.ndef().ok();
        return NdefTarget {
            capacity: ndef.as_ref().map(|ndef| ndef.capacity),
            stored: ndef.as_ref().and_then(|ndef| ndef.message()).cloned(),
            write: if busy {
                Err("An operation is running.".to_owned())
            } else if !live {
                Err("Read a live Type 2 tag to write or format.".to_owned())
            } else {
                type2_ndef_gate(&session, auth_ready)
            },
            format: if busy {
                Err("An operation is running.".to_owned())
            } else if !live {
                Err("Read a live Type 2 tag to write or format.".to_owned())
            } else {
                type2_format_gate(&session, auth_ready)
            },
            protected: "Configuration, lock, and password pages are not changed by an NDEF write.",
            loaded: true,
        };
    }

    let dump = ui.dump.borrow().clone();
    let info = ui.ndef.borrow().clone();
    let complete = dump.as_ref().is_some_and(Dump::complete);
    let layout = info.as_ref().and_then(|info| info.layout.as_ref());
    let format = if busy {
        Err("An operation is running.".to_owned())
    } else if !live {
        Err("Read a live card to write or format.".to_owned())
    } else if dump.is_none() {
        Err("Read a card to inspect its MAD mapping and NDEF message.".to_owned())
    } else if !complete {
        Err("The image is partial; read the whole card first.".to_owned())
    } else if info.is_none() {
        Err("NDEF state is unavailable.".to_owned())
    } else {
        Ok(())
    };
    let write = if format.is_err() {
        format.clone()
    } else if !matches!(
        info.as_ref().map(|info| &info.ndef.state),
        Some(ClassicNdefState::Empty | ClassicNdefState::Message(_))
    ) {
        Err("Not NDEF formatted · Format is required before writing.".to_owned())
    } else if !layout
        .is_some_and(|layout| !layout.sectors.is_empty() && layout.writable && !layout.read_only)
    {
        if layout.is_some_and(|layout| layout.read_only) {
            Err("The NDEF area is read-only; writing is disabled.".to_owned())
        } else {
            Err("The NFC mapping is not writable; format the card.".to_owned())
        }
    } else {
        Ok(())
    };
    NdefTarget {
        capacity: info.as_ref().map(|info| info.ndef.capacity),
        stored: info.as_ref().and_then(|info| info.ndef.message().cloned()),
        write,
        format,
        protected: "Sector trailers, keys, and access bits are not changed by an NDEF write.",
        loaded: dump.is_some(),
    }
}

fn refresh_ndef_editor(ui: &Ui) {
    populate_ndef_rows(ui);
    update_ndef_inspector(ui);
    let view = &ui.ndef_view;
    let target = ndef_target(ui);
    let editor = view.editor.borrow();
    let encoded = editor.message().and_then(|message| {
        message
            .encode()
            .map(|bytes| (message, bytes))
            .map_err(|error| error.to_string())
    });
    let used = encoded.as_ref().ok().map(|(_, bytes)| bytes.len());
    let records = editor.records().len();

    match (used, target.capacity) {
        (Some(used), Some(capacity)) => {
            view.capacity_used
                .set_localized(&format!("{used} / {capacity} B"));
            view.capacity_meta.set_localized(&format!(
                "{records} record(s) · {} B free",
                capacity.saturating_sub(used)
            ));
            view.capacity_meter.set_fraction(if capacity == 0 {
                0.0
            } else {
                (used as f64 / capacity as f64).min(1.0)
            });
        }
        (Some(used), None) => {
            view.capacity_used.set_localized(&format!("{used} B"));
            view.capacity_meta
                .set_localized(&format!("{records} record(s) · capacity unavailable"));
            view.capacity_meter.set_fraction(0.0);
        }
        (None, capacity) => {
            view.capacity_used.set_localized(
                &capacity
                    .map(|capacity| format!("— / {capacity} B"))
                    .unwrap_or_else(|| "—".to_owned()),
            );
            view.capacity_meta
                .set_localized(&format!("{records} record(s) · message not encodable"));
            view.capacity_meter.set_fraction(0.0);
        }
    }

    let (capacity_state, state_error) = if records == 0 {
        ("No records".to_owned(), false)
    } else {
        match &encoded {
            Err(_) => ("Invalid message".to_owned(), true),
            Ok((_, bytes)) => match target.capacity {
                Some(capacity) if bytes.len() > capacity => ("Over capacity".to_owned(), true),
                Some(_) => ("Valid NDEF".to_owned(), false),
                None => ("Capacity unavailable".to_owned(), false),
            },
        }
    };
    view.capacity_state.set_localized(&capacity_state);
    if state_error {
        view.capacity_state.add_css_class("error");
    } else {
        view.capacity_state.remove_css_class("error");
    }

    let mut feedback_error = false;
    let feedback = if let Err(error) = &target.write {
        if error.contains("Format is required") {
            error.clone()
        } else {
            feedback_error = true;
            error.clone()
        }
    } else if records == 0 {
        "Add at least one record to write.".to_owned()
    } else {
        match &encoded {
            Err(error) => {
                feedback_error = true;
                error.clone()
            }
            Ok((message, bytes)) => match target.capacity {
                Some(capacity) => match classic::ndef_capacity_error(bytes.len(), capacity) {
                    Some(error) => {
                        feedback_error = true;
                        error
                    }
                    None if target.stored.as_ref() == Some(message) => {
                        "The NDEF area already stores this message.".to_owned()
                    }
                    None => format!("Ready to write {} bytes.", bytes.len()),
                },
                None => "Capacity is unavailable; read the card again.".to_owned(),
            },
        }
    };
    let fits = match (&encoded, target.capacity) {
        (Ok((_, bytes)), Some(capacity)) => bytes.len() <= capacity,
        _ => false,
    };
    let different = match &encoded {
        Ok((message, _)) => target.stored.as_ref() != Some(message),
        Err(_) => false,
    };
    let ready = !feedback_error && records > 0 && target.write.is_ok() && fits && different;

    view.edit_state.set_localized(if editor.is_dirty() {
        "Unsaved changes"
    } else {
        "No unsaved changes"
    });
    view.feedback.set_localized(&feedback);
    if feedback_error {
        view.feedback.add_css_class("error");
    } else {
        view.feedback.remove_css_class("error");
    }

    // A live gate failure is already reported in `feedback`; only report TLV
    // and capacity facts here so the same error is not repeated three times.
    let tlv = match &encoded {
        Ok((_, bytes)) => format!("TLV valid · {} B · {} record(s)", bytes.len(), records),
        Err(error) => error.clone(),
    };
    view.preflight_tlv.set_localized(&tlv);
    let fit = match (used, target.capacity) {
        (Some(used), Some(capacity)) if used <= capacity => format!(
            "Fits user memory · {} B free of {capacity} B",
            capacity - used
        ),
        (Some(used), Some(capacity)) => {
            format!("Does not fit · needs {used} B, capacity {capacity} B")
        }
        (_, _) => "Capacity is unavailable; read the card again.".to_owned(),
    };
    view.preflight_fit.set_localized(&fit);
    view.preflight_protected.set_localized(target.protected);

    match editor.tlv_preview(96) {
        Ok((len, preview)) => {
            view.raw_head
                .set_localized(&format!("Raw message · TLV 0x03 · {len} bytes"));
            view.raw_hex.set_localized(&preview);
        }
        Err(error) => {
            view.raw_head.set_localized("Raw message");
            view.raw_hex.set_localized(&error);
        }
    }

    let busy = ui.busy.get();
    let selected = editor.selected();
    view.read_only
        .set_visible(ui.workspace.get() == Workspace::Classic);
    if ui.workspace.get() != Workspace::Classic {
        view.read_only.set_active(false);
    }
    view.write.set_sensitive(ready && !busy);
    view.format.set_sensitive(target.format.is_ok() && !busy);
    view.export.set_sensitive(encoded.is_ok() && !busy);
    view.add.set_sensitive(!busy);
    view.load.set_sensitive(!busy);
    view.reload.set_sensitive(target.loaded && !busy);
    view.clear
        .set_sensitive(!editor.records().is_empty() && !busy);
    view.validate.set_sensitive(!busy);
    view.edit.set_sensitive(selected.is_some() && !busy);
    view.remove.set_sensitive(selected.is_some() && !busy);
    view.move_up
        .set_sensitive(selected.is_some_and(|index| index > 0) && !busy);
    view.move_down
        .set_sensitive(selected.is_some_and(|index| index + 1 < editor.records().len()) && !busy);
}

fn populate_ndef_rows(ui: &Ui) {
    let view = &ui.ndef_view;
    let (rows, selected) = {
        let editor = view.editor.borrow();
        (editor.rows(), editor.selected())
    };
    view.rebuilding.set(true);
    view.rows.remove_all();
    for row in rows {
        view.rows.append(&glib::BoxedAnyObject::new(row));
    }
    view.selection.set_selected(
        selected
            .map(|index| index as u32)
            .unwrap_or(gtk::INVALID_LIST_POSITION),
    );
    view.rebuilding.set(false);
}

fn update_ndef_inspector(ui: &Ui) {
    let view = &ui.ndef_view;
    let editor = view.editor.borrow();
    let Some(index) = editor.selected() else {
        view.inspector_title.set_localized("No record selected");
        view.inspector_kind
            .set_localized("Add or select a record to inspect it.");
        view.inspector_fields.set_localized("");
        view.inspector_payload.set_localized("");
        view.inspector_valid.set_localized("");
        view.inspector_valid.remove_css_class("error");
        view.edit.set_sensitive(false);
        view.remove.set_sensitive(false);
        view.move_up.set_sensitive(false);
        view.move_down.set_sensitive(false);
        return;
    };
    let total = editor.records().len();
    let Some(draft) = editor.records().get(index) else {
        return;
    };
    view.inspector_title
        .set_localized(&format!("Record {} of {total}", index + 1));
    view.inspector_kind.set_localized(&draft.label());
    let decoded = draft.to_record();
    match decoded {
        Ok(record) => {
            let record_type = ndef_editor::text_or_hex(record.record_type());
            let record_type = if record.record_type().len() == 1 {
                format!("{record_type} (0x{:02X})", record.record_type()[0])
            } else {
                record_type
            };
            let id = if record.id().is_empty() {
                "—".to_owned()
            } else {
                ndef_editor::format_hex(record.id(), " ")
            };
            let size = NdefMessage::single(record.clone())
                .and_then(|message| message.encoded_len())
                .map(|bytes| format!("{bytes} bytes"))
                .unwrap_or_else(|error| error.to_string());
            view.inspector_fields.set_localized(&format!(
                "TNF   {}\nTYPE  {}\nID    {}\nSize  {size}",
                ndef_editor::tnf_label(record.tnf()),
                record_type,
                id,
            ));
            let payload = record.payload();
            let payload = if payload.is_empty() {
                "—".to_owned()
            } else {
                ndef_editor::hex_preview(payload, 64)
            };
            view.inspector_payload.set_localized(&payload);
            view.inspector_valid.set_localized("Valid NDEF record");
            view.inspector_valid.remove_css_class("error");
        }
        Err(error) => {
            view.inspector_fields.set_localized("—");
            view.inspector_payload.set_localized(&draft.preview());
            view.inspector_valid.set_localized(&error.to_string());
            view.inspector_valid.add_css_class("error");
        }
    }
    view.edit.set_sensitive(!ui.busy.get());
    view.remove.set_sensitive(!ui.busy.get());
    view.move_up.set_sensitive(index > 0 && !ui.busy.get());
    view.move_down
        .set_sensitive(index + 1 < total && !ui.busy.get());
}

fn after_ndef_editor_change(ui: &Ui) {
    populate_ndef_rows(ui);
    update_ndef_inspector(ui);
    update_ndef_actions(ui);
}

fn remove_selected_ndef_record(ui: &Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let selected = ui.ndef_view.editor.borrow().selected();
    let Some(index) = selected else {
        return;
    };
    if ui.ndef_view.editor.borrow_mut().remove(index) {
        after_ndef_editor_change(ui);
    }
}

fn move_selected_ndef_record(ui: &Rc<Ui>, up: bool) {
    if ui.busy.get() {
        return;
    }
    let selected = ui.ndef_view.editor.borrow().selected();
    let Some(index) = selected else {
        return;
    };
    let moved = {
        let mut editor = ui.ndef_view.editor.borrow_mut();
        if up {
            editor.move_up(index)
        } else {
            editor.move_down(index)
        }
    };
    if moved {
        after_ndef_editor_change(ui);
    }
}

fn clear_ndef_message(ui: &Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    ui.ndef_view.editor.borrow_mut().clear();
    ui.ndef_view.editor_key.replace(Some(current_ndef_key(ui)));
    after_ndef_editor_change(ui);
}

fn current_ndef_key(ui: &Ui) -> String {
    if ui.workspace.get() == Workspace::Type2 {
        type2_ndef_key(ui)
    } else {
        classic_ndef_key(ui)
    }
}

fn type2_ndef_key(ui: &Ui) -> String {
    let live = if ui.live.get() { "live" } else { "inspect" };
    let uid = ui
        .type2
        .borrow()
        .as_ref()
        .map(|session| session.card.uid_hex())
        .unwrap_or_default();
    format!("type2:{live}:{uid}")
}

fn reload_ndef_from_card(ui: &Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let message = if ui.workspace.get() == Workspace::Type2 {
        let session = ui.type2.borrow().clone();
        let Some(session) = session else {
            toast_error(ui, "Read a Type 2 tag first");
            return;
        };
        session.ndef().ok().and_then(Type2Ndef::message).cloned()
    } else {
        let info = ui.ndef.borrow().clone();
        let Some(info) = info else {
            toast_error(ui, "Read a card to inspect its NDEF message");
            return;
        };
        info.ndef.message().cloned()
    };
    ui.ndef_view
        .editor
        .borrow_mut()
        .load_message(message.as_ref());
    ui.ndef_view.editor_key.replace(Some(current_ndef_key(ui)));
    after_ndef_editor_change(ui);
    toast_info(ui, "Reloaded the NDEF message from the card");
}

fn validate_ndef_message(ui: &Rc<Ui>) {
    let target = ndef_target(ui);
    let editor = ui.ndef_view.editor.borrow();
    let result = editor.message().and_then(|message| {
        message
            .encode()
            .map(|bytes| (bytes.len(), message.records().len()))
            .map_err(|error| error.to_string())
    });
    drop(editor);
    match result {
        Err(error) => toast_error(ui, &error),
        Ok((bytes, records)) => match target.capacity {
            Some(capacity) if bytes > capacity => toast_error(
                ui,
                &format!("The message needs {bytes} bytes; capacity is {capacity} bytes."),
            ),
            Some(capacity) => {
                ui.status.set_localized(&format!(
                    "NDEF valid · {records} record(s) · {bytes}/{capacity} B"
                ));
                toast_info(ui, "NDEF message is valid");
            }
            None => toast_error(ui, "Capacity is unavailable; read the card again."),
        },
    }
}

/// Alias kept for call sites that only need to refresh NDEF action state.
fn update_ndef_actions(ui: &Ui) {
    refresh_ndef_editor(ui);
}

const WIFI_AUTH_PRESETS: [(&str, u16); 6] = [
    ("Open", 0x0001),
    ("WPA-Personal", 0x0002),
    ("WPA2-Personal", 0x0020),
    ("WPA-Enterprise", 0x0008),
    ("WPA2-Enterprise", 0x0010),
    ("Shared", 0x0004),
];
const WIFI_ENCRYPTION_PRESETS: [(&str, u16); 4] = [
    ("None", 0x0001),
    ("WEP", 0x0002),
    ("TKIP", 0x0004),
    ("AES", 0x0008),
];
const BLE_ROLES: [LeRole; 4] = [
    LeRole::PeripheralOnly,
    LeRole::CentralOnly,
    LeRole::PeripheralPreferred,
    LeRole::CentralPreferred,
];
const POSTER_ACTIONS: [SmartPosterAction; 3] = [
    SmartPosterAction::Execute,
    SmartPosterAction::Save,
    SmartPosterAction::Edit,
];

struct RecordDialog {
    content: gtk::Box,
    save: gtk::Button,
    kind: gtk::DropDown,
    stack: gtk::Stack,
    feedback: gtk::Label,
    original: RecordDraft,
    text_language: gtk::Entry,
    text_encoding: gtk::DropDown,
    text_body: gtk::TextView,
    uri: gtk::Entry,
    media_type: gtk::Entry,
    media_payload: gtk::TextView,
    media_hex: gtk::CheckButton,
    wifi_ssid: gtk::Entry,
    wifi_ssid_hex: gtk::CheckButton,
    wifi_auth: gtk::DropDown,
    wifi_auth_bits: gtk::Entry,
    wifi_encryption: gtk::DropDown,
    wifi_encryption_bits: gtk::Entry,
    wifi_key: gtk::Entry,
    wifi_key_hex: gtk::CheckButton,
    wifi_mac: gtk::Entry,
    vcard_name: gtk::Entry,
    vcard_org: gtk::Entry,
    vcard_phone: gtk::Entry,
    vcard_email: gtk::Entry,
    vcard_url: gtk::Entry,
    ble_address: gtk::Entry,
    ble_address_type: gtk::DropDown,
    ble_role: gtk::DropDown,
    ble_name: gtk::Entry,
    ble_appearance: gtk::Entry,
    ble_flags: gtk::Entry,
    dpp_uri: gtk::Entry,
    dpp_carrier: gtk::DropDown,
    poster_uri: gtk::Entry,
    poster_title: gtk::Entry,
    poster_language: gtk::Entry,
    poster_action: gtk::DropDown,
    raw_tnf: gtk::DropDown,
    raw_type: gtk::Entry,
    raw_type_hex: gtk::CheckButton,
    raw_id: gtk::Entry,
    raw_payload: gtk::TextView,
}

fn form_caption(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label
}

fn form_row(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Horizontal, 8);
    let caption = gtk::Label::new(Some(label));
    caption.set_xalign(0.0);
    caption.set_width_chars(14);
    row.append(&caption);
    widget.set_hexpand(true);
    row.append(widget);
    row
}

fn form_entry(label: &str, placeholder: &str) -> (gtk::Box, gtk::Entry) {
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .hexpand(true)
        .build();
    (form_row(label, &entry), entry)
}

fn form_dropdown(label: &str, items: &[&str]) -> (gtk::Box, gtk::DropDown) {
    let model = gtk::StringList::new(items);
    let dropdown = gtk::DropDown::new(Some(model), None::<gtk::Expression>);
    (form_row(label, &dropdown), dropdown)
}

fn form_text(label: &str, height: i32) -> (gtk::Box, gtk::TextView) {
    let container = gtk::Box::new(Orientation::Vertical, 4);
    container.append(&form_caption(label));
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_accepts_tab(false);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(height)
        .has_frame(true)
        .build();
    container.append(&scroll);
    (container, view)
}

fn form_hex_toggle() -> gtk::CheckButton {
    gtk::CheckButton::with_label("Hexadecimal")
}

fn text_view_set(view: &gtk::TextView, text: &str) {
    view.buffer().set_text(text);
}

fn text_view_get(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn set_entry_bytes(entry: &gtk::Entry, check: &gtk::CheckButton, bytes: &[u8]) {
    if bytes.is_empty() {
        entry.set_text("");
        check.set_active(false);
        return;
    }
    match std::str::from_utf8(bytes) {
        Ok(text)
            if !text.is_empty()
                && !text.chars().any(|character| {
                    character.is_control() && character != '\n' && character != '\t'
                }) =>
        {
            entry.set_text(text);
            check.set_active(false);
        }
        _ => {
            entry.set_text(&ndef_editor::format_hex(bytes, " "));
            check.set_active(true);
        }
    }
}

fn set_optional_entry(entry: &gtk::Entry, value: Option<&String>) {
    entry.set_text(value.map(String::as_str).unwrap_or_default());
}

fn entry_bytes(entry: &gtk::Entry, hex: &gtk::CheckButton, field: &str) -> Result<Vec<u8>, String> {
    if hex.is_active() {
        ndef_editor::parse_hex(entry.text().as_str(), field)
    } else {
        Ok(entry.text().as_bytes().to_vec())
    }
}

fn optional_entry_bytes(
    entry: &gtk::Entry,
    hex: &gtk::CheckButton,
    field: &str,
) -> Result<Option<Vec<u8>>, String> {
    if entry.text().trim().is_empty() {
        Ok(None)
    } else {
        entry_bytes(entry, hex, field).map(Some)
    }
}

fn optional_entry(entry: &gtk::Entry) -> Option<String> {
    let text = entry.text().to_string();
    (!text.trim().is_empty()).then_some(text)
}

fn preset_index(presets: &[(&str, u16)], bits: u16) -> (u32, bool) {
    match presets.iter().position(|(_, preset)| *preset == bits) {
        Some(index) => (index as u32, false),
        None => (presets.len() as u32, true),
    }
}

fn build_record_form(original: &RecordDraft) -> RecordDialog {
    let kind_items = RecordKind::ALL
        .iter()
        .map(|kind| kind.label())
        .collect::<Vec<_>>();
    let kind_model = gtk::StringList::new(&kind_items);
    let kind = gtk::DropDown::new(Some(kind_model), None::<gtk::Expression>);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vexpand(true);

    let text_page = gtk::Box::new(Orientation::Vertical, 8);
    let text_meta = gtk::Box::new(Orientation::Horizontal, 8);
    let (language_row, text_language) = form_entry("Language", "en");
    let (encoding_row, text_encoding) = form_dropdown("Encoding", &["UTF-8", "UTF-16"]);
    text_meta.append(&language_row);
    text_meta.append(&encoding_row);
    let (text_box, text_body) = form_text("Text", 150);
    text_page.append(&text_meta);
    text_page.append(&text_box);
    stack.add_named(&text_page, Some(RecordKind::Text.stack_name()));

    let uri_page = gtk::Box::new(Orientation::Vertical, 8);
    let (uri_row, uri) = form_entry("URI", "https://example.com/");
    uri_page.append(&uri_row);
    stack.add_named(&uri_page, Some(RecordKind::Uri.stack_name()));

    let media_page = gtk::Box::new(Orientation::Vertical, 8);
    let (media_type_row, media_type) = form_entry("Media type", "application/json");
    let (media_box, media_payload) = form_text("Payload", 150);
    let media_hex = form_hex_toggle();
    media_page.append(&media_type_row);
    media_page.append(&media_box);
    media_page.append(&media_hex);
    stack.add_named(&media_page, Some(RecordKind::Media.stack_name()));

    let wifi_page = gtk::Box::new(Orientation::Vertical, 8);
    let (wifi_ssid_row, wifi_ssid) = form_entry("SSID", "Network name");
    let wifi_ssid_hex = form_hex_toggle();
    let wifi_ssid_box = gtk::Box::new(Orientation::Horizontal, 8);
    wifi_ssid_box.append(&wifi_ssid_row);
    wifi_ssid_box.append(&wifi_ssid_hex);
    let mut auth_items = WIFI_AUTH_PRESETS
        .iter()
        .map(|(label, _)| *label)
        .collect::<Vec<_>>();
    auth_items.push("Custom bits");
    let (wifi_auth_row, wifi_auth) = form_dropdown("Authentication", &auth_items);
    let (wifi_auth_bits_row, wifi_auth_bits) = form_entry("Auth bits", "0x0020");
    let mut encryption_items = WIFI_ENCRYPTION_PRESETS
        .iter()
        .map(|(label, _)| *label)
        .collect::<Vec<_>>();
    encryption_items.push("Custom bits");
    let (wifi_encryption_row, wifi_encryption) = form_dropdown("Encryption", &encryption_items);
    let (wifi_encryption_bits_row, wifi_encryption_bits) = form_entry("Encryption bits", "0x0008");
    let (wifi_key_row, wifi_key) = form_entry("Network key", "Passphrase");
    let wifi_key_hex = form_hex_toggle();
    let wifi_key_box = gtk::Box::new(Orientation::Horizontal, 8);
    wifi_key_box.append(&wifi_key_row);
    wifi_key_box.append(&wifi_key_hex);
    let (wifi_mac_row, wifi_mac) = form_entry("MAC address", "00:11:22:33:44:55");
    for row in [
        &wifi_ssid_box,
        &wifi_auth_row,
        &wifi_auth_bits_row,
        &wifi_encryption_row,
        &wifi_encryption_bits_row,
        &wifi_key_box,
        &wifi_mac_row,
    ] {
        wifi_page.append(row);
    }
    stack.add_named(&wifi_page, Some(RecordKind::Wifi.stack_name()));

    let vcard_page = gtk::Box::new(Orientation::Vertical, 8);
    let (vcard_name_row, vcard_name) = form_entry("Formatted name", "Ada Lovelace");
    let (vcard_org_row, vcard_org) = form_entry("Organization", "NFC Foundry");
    let (vcard_phone_row, vcard_phone) = form_entry("Phone", "+1 555 0100");
    let (vcard_email_row, vcard_email) = form_entry("Email", "ada@example.com");
    let (vcard_url_row, vcard_url) = form_entry("URL", "https://example.com");
    for row in [
        &vcard_name_row,
        &vcard_org_row,
        &vcard_phone_row,
        &vcard_email_row,
        &vcard_url_row,
    ] {
        vcard_page.append(row);
    }
    stack.add_named(&vcard_page, Some(RecordKind::VCard.stack_name()));

    let ble_page = gtk::Box::new(Orientation::Vertical, 8);
    let (ble_address_row, ble_address) = form_entry("Device address", "AA:BB:CC:DD:EE:FF");
    let (ble_address_type_row, ble_address_type) =
        form_dropdown("Address type", &["Public", "Random"]);
    let (ble_role_row, ble_role) = form_dropdown(
        "Role",
        &[
            "None",
            "Peripheral only",
            "Central only",
            "Peripheral preferred",
            "Central preferred",
        ],
    );
    let (ble_name_row, ble_name) = form_entry("Name", "NFC Foundry");
    let (ble_appearance_row, ble_appearance) = form_entry("Appearance", "0x0341");
    let (ble_flags_row, ble_flags) = form_entry("Flags", "0x06");
    for row in [
        &ble_address_row,
        &ble_address_type_row,
        &ble_role_row,
        &ble_name_row,
        &ble_appearance_row,
        &ble_flags_row,
    ] {
        ble_page.append(row);
    }
    stack.add_named(&ble_page, Some(RecordKind::BluetoothLe.stack_name()));

    let dpp_page = gtk::Box::new(Orientation::Vertical, 8);
    let (dpp_uri_row, dpp_uri) = form_entry("Bootstrapping URI", "DPP:C:81/1;M:...;;");
    let (dpp_carrier_row, dpp_carrier) = form_dropdown(
        "Carrier",
        &[DppCarrier::Uri.label(), DppCarrier::Media.label()],
    );
    dpp_page.append(&dpp_uri_row);
    dpp_page.append(&dpp_carrier_row);
    stack.add_named(&dpp_page, Some(RecordKind::Dpp.stack_name()));

    let poster_page = gtk::Box::new(Orientation::Vertical, 8);
    let (poster_uri_row, poster_uri) = form_entry("URI", "https://example.com/");
    let (poster_title_row, poster_title) = form_entry("Title", "Title");
    let (poster_language_row, poster_language) = form_entry("Language", "en");
    poster_language.set_text("en");
    let (poster_action_row, poster_action) =
        form_dropdown("Action", &["No action", "Execute", "Save", "Edit"]);
    for row in [
        &poster_uri_row,
        &poster_title_row,
        &poster_language_row,
        &poster_action_row,
    ] {
        poster_page.append(row);
    }
    stack.add_named(&poster_page, Some(RecordKind::SmartPoster.stack_name()));

    let raw_page = gtk::Box::new(Orientation::Vertical, 8);
    let tnf_items = ndef_editor::RAW_TNFS
        .iter()
        .map(|tnf| ndef_editor::tnf_label(*tnf))
        .collect::<Vec<_>>();
    let (raw_tnf_row, raw_tnf) = form_dropdown("TNF", &tnf_items);
    let (raw_type_row, raw_type) = form_entry("TYPE", "U");
    let raw_type_hex = form_hex_toggle();
    let raw_type_box = gtk::Box::new(Orientation::Horizontal, 8);
    raw_type_box.append(&raw_type_row);
    raw_type_box.append(&raw_type_hex);
    let (raw_id_row, raw_id) = form_entry("ID", "hex");
    let (raw_payload_box, raw_payload) = form_text("Payload (hexadecimal)", 150);
    for row in [&raw_tnf_row, &raw_type_box, &raw_id_row] {
        raw_page.append(row);
    }
    raw_page.append(&raw_payload_box);
    stack.add_named(&raw_page, Some(RecordKind::Raw.stack_name()));

    let feedback = gtk::Label::new(None);
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    feedback.add_css_class("caption");
    feedback.add_css_class("dim-label");

    let content = gtk::Box::new(Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&form_row("Record type", &kind));
    let scroller = gtk::ScrolledWindow::builder()
        .child(&stack)
        .vexpand(true)
        .build();
    content.append(&scroller);
    content.append(&feedback);

    let save = gtk::Button::with_label("Save record");
    save.add_css_class("suggested-action");

    let dialog = RecordDialog {
        content,
        save,
        kind,
        stack,
        feedback,
        original: original.clone(),
        text_language,
        text_encoding,
        text_body,
        uri,
        media_type,
        media_payload,
        media_hex,
        wifi_ssid,
        wifi_ssid_hex,
        wifi_auth,
        wifi_auth_bits,
        wifi_encryption,
        wifi_encryption_bits,
        wifi_key,
        wifi_key_hex,
        wifi_mac,
        vcard_name,
        vcard_org,
        vcard_phone,
        vcard_email,
        vcard_url,
        ble_address,
        ble_address_type,
        ble_role,
        ble_name,
        ble_appearance,
        ble_flags,
        dpp_uri,
        dpp_carrier,
        poster_uri,
        poster_title,
        poster_language,
        poster_action,
        raw_tnf,
        raw_type,
        raw_type_hex,
        raw_id,
        raw_payload,
    };
    populate_record_form(&dialog, original);
    dialog
}

fn populate_record_form(dialog: &RecordDialog, original: &RecordDraft) {
    let kind = original.kind();
    if let Some(index) = RecordKind::ALL
        .iter()
        .position(|candidate| *candidate == kind)
    {
        dialog.kind.set_selected(index as u32);
    }
    dialog.stack.set_visible_child_name(kind.stack_name());
    match original {
        RecordDraft::Text(draft) => {
            dialog.text_language.set_text(&draft.language);
            dialog
                .text_encoding
                .set_selected(if draft.encoding == TextEncoding::Utf16 {
                    1
                } else {
                    0
                });
            text_view_set(&dialog.text_body, &draft.text);
        }
        RecordDraft::Uri(draft) => dialog.uri.set_text(&draft.uri),
        RecordDraft::Media(draft) => {
            dialog
                .media_type
                .set_text(&String::from_utf8_lossy(&draft.media_type));
            match std::str::from_utf8(&draft.payload) {
                Ok(text)
                    if !text.chars().any(|character| {
                        character.is_control() && character != '\n' && character != '\t'
                    }) =>
                {
                    text_view_set(&dialog.media_payload, text);
                    dialog.media_hex.set_active(false);
                }
                _ => {
                    text_view_set(
                        &dialog.media_payload,
                        &ndef_editor::format_hex(&draft.payload, " "),
                    );
                    dialog.media_hex.set_active(true);
                }
            }
        }
        RecordDraft::Wifi(draft) => {
            set_entry_bytes(&dialog.wifi_ssid, &dialog.wifi_ssid_hex, &draft.ssid);
            let (index, custom) = preset_index(&WIFI_AUTH_PRESETS, draft.authentication);
            dialog.wifi_auth.set_selected(index);
            if custom {
                dialog
                    .wifi_auth_bits
                    .set_text(&format!("{:04X}", draft.authentication));
            }
            let (index, custom) = preset_index(&WIFI_ENCRYPTION_PRESETS, draft.encryption);
            dialog.wifi_encryption.set_selected(index);
            if custom {
                dialog
                    .wifi_encryption_bits
                    .set_text(&format!("{:04X}", draft.encryption));
            }
            if let Some(key) = &draft.network_key {
                set_entry_bytes(&dialog.wifi_key, &dialog.wifi_key_hex, key);
            }
            if let Some(address) = draft.mac_address {
                dialog
                    .wifi_mac
                    .set_text(&ndef_editor::format_hex(&address, ":"));
            }
        }
        RecordDraft::VCard(draft) => {
            dialog.vcard_name.set_text(&draft.formatted_name);
            set_optional_entry(&dialog.vcard_org, draft.organization.as_ref());
            set_optional_entry(&dialog.vcard_phone, draft.phone.as_ref());
            set_optional_entry(&dialog.vcard_email, draft.email.as_ref());
            set_optional_entry(&dialog.vcard_url, draft.url.as_ref());
        }
        RecordDraft::BluetoothLe(draft) => {
            if let Some((address, address_type)) = draft.address {
                dialog
                    .ble_address
                    .set_text(&ndef_editor::format_hex(&address, ":"));
                dialog.ble_address_type.set_selected(match address_type {
                    LeAddressType::Public => 0,
                    LeAddressType::Random => 1,
                });
            }
            dialog.ble_role.set_selected(
                draft
                    .role
                    .and_then(|role| BLE_ROLES.iter().position(|candidate| *candidate == role))
                    .map(|index| index as u32 + 1)
                    .unwrap_or(0),
            );
            set_optional_entry(&dialog.ble_name, draft.name.as_ref());
            if let Some(appearance) = draft.appearance {
                dialog.ble_appearance.set_text(&format!("{appearance:04X}"));
            }
            if let Some(flags) = draft.flags {
                dialog.ble_flags.set_text(&format!("{flags:02X}"));
            }
        }
        RecordDraft::Dpp(draft) => {
            dialog.dpp_uri.set_text(&draft.uri);
            dialog.dpp_carrier.set_selected(match draft.carrier {
                DppCarrier::Uri => 0,
                DppCarrier::Media => 1,
            });
        }
        RecordDraft::SmartPoster(draft) => {
            dialog.poster_uri.set_text(&draft.uri);
            if let Some(title) = draft.titles.first() {
                dialog.poster_title.set_text(&title.text);
                dialog.poster_language.set_text(&title.language);
            }
            dialog.poster_action.set_selected(
                draft
                    .action
                    .and_then(|action| {
                        POSTER_ACTIONS
                            .iter()
                            .position(|candidate| *candidate == action)
                    })
                    .map(|index| index as u32 + 1)
                    .unwrap_or(0),
            );
        }
        RecordDraft::Raw(draft) => {
            dialog.raw_tnf.set_selected(
                ndef_editor::RAW_TNFS
                    .iter()
                    .position(|tnf| *tnf == draft.tnf)
                    .unwrap_or(0) as u32,
            );
            match std::str::from_utf8(&draft.record_type) {
                Ok(text) if !text.is_empty() => {
                    dialog.raw_type.set_text(text);
                    dialog.raw_type_hex.set_active(false);
                }
                _ => {
                    dialog
                        .raw_type
                        .set_text(&ndef_editor::format_hex(&draft.record_type, " "));
                    dialog
                        .raw_type_hex
                        .set_active(!draft.record_type.is_empty());
                }
            }
            dialog
                .raw_id
                .set_text(&ndef_editor::format_hex(&draft.id, " "));
            text_view_set(
                &dialog.raw_payload,
                &ndef_editor::format_hex(&draft.payload, " "),
            );
        }
    }
}

fn collect_record_form(dialog: &RecordDialog) -> Result<RecordDraft, String> {
    let kind = RecordKind::ALL
        .get(dialog.kind.selected() as usize)
        .copied()
        .ok_or_else(|| "Choose a record type.".to_owned())?;
    let draft = match kind {
        RecordKind::Text => {
            let encoding = if dialog.text_encoding.selected() == 1 {
                TextEncoding::Utf16
            } else {
                TextEncoding::Utf8
            };
            RecordDraft::Text(TextDraft {
                text: text_view_get(&dialog.text_body),
                language: dialog.text_language.text().to_string(),
                encoding,
            })
        }
        RecordKind::Uri => RecordDraft::Uri(UriDraft {
            uri: dialog.uri.text().to_string(),
        }),
        RecordKind::Media => RecordDraft::Media(MediaDraft {
            media_type: dialog.media_type.text().as_bytes().to_vec(),
            payload: if dialog.media_hex.is_active() {
                ndef_editor::parse_hex(&text_view_get(&dialog.media_payload), "Payload")?
            } else {
                text_view_get(&dialog.media_payload).as_bytes().to_vec()
            },
        }),
        RecordKind::Wifi => {
            let auth_index = dialog.wifi_auth.selected() as usize;
            let authentication = match WIFI_AUTH_PRESETS.get(auth_index) {
                Some((_, bits)) => *bits,
                None => ndef_editor::parse_u16_hex(
                    dialog.wifi_auth_bits.text().as_str(),
                    "Authentication bits",
                )?,
            };
            let encryption_index = dialog.wifi_encryption.selected() as usize;
            let encryption = match WIFI_ENCRYPTION_PRESETS.get(encryption_index) {
                Some((_, bits)) => *bits,
                None => ndef_editor::parse_u16_hex(
                    dialog.wifi_encryption_bits.text().as_str(),
                    "Encryption bits",
                )?,
            };
            let mac = if dialog.wifi_mac.text().trim().is_empty() {
                None
            } else {
                let bytes = ndef_editor::parse_hex(dialog.wifi_mac.text().as_str(), "MAC address")?;
                match bytes.as_slice() {
                    [a, b, c, d, e, f] => Some([*a, *b, *c, *d, *e, *f]),
                    _ => return Err("MAC address must be six bytes".to_owned()),
                }
            };
            let (credential_attributes, outer_attributes) = match &dialog.original {
                RecordDraft::Wifi(original) => (
                    original.credential_attributes.clone(),
                    original.outer_attributes.clone(),
                ),
                _ => (Vec::new(), Vec::new()),
            };
            RecordDraft::Wifi(WifiDraft {
                ssid: entry_bytes(&dialog.wifi_ssid, &dialog.wifi_ssid_hex, "SSID")?,
                authentication,
                encryption,
                network_key: optional_entry_bytes(
                    &dialog.wifi_key,
                    &dialog.wifi_key_hex,
                    "Network key",
                )?,
                mac_address: mac,
                credential_attributes,
                outer_attributes,
            })
        }
        RecordKind::VCard => RecordDraft::VCard(VCardDraft {
            formatted_name: dialog.vcard_name.text().to_string(),
            organization: optional_entry(&dialog.vcard_org),
            phone: optional_entry(&dialog.vcard_phone),
            email: optional_entry(&dialog.vcard_email),
            url: optional_entry(&dialog.vcard_url),
        }),
        RecordKind::BluetoothLe => {
            let address = if dialog.ble_address.text().trim().is_empty() {
                None
            } else {
                let bytes =
                    ndef_editor::parse_hex(dialog.ble_address.text().as_str(), "Device address")?;
                match bytes.as_slice() {
                    [a, b, c, d, e, f] => Some((
                        [*a, *b, *c, *d, *e, *f],
                        if dialog.ble_address_type.selected() == 1 {
                            LeAddressType::Random
                        } else {
                            LeAddressType::Public
                        },
                    )),
                    _ => return Err("Device address must be six bytes".to_owned()),
                }
            };
            let role = BLE_ROLES
                .get(dialog.ble_role.selected().saturating_sub(1) as usize)
                .copied()
                .filter(|_| dialog.ble_role.selected() > 0);
            let extra = match &dialog.original {
                RecordDraft::BluetoothLe(original) => original.extra.clone(),
                _ => Vec::new(),
            };
            RecordDraft::BluetoothLe(BluetoothDraft {
                address,
                role,
                name: optional_entry(&dialog.ble_name),
                appearance: optional_u16(&dialog.ble_appearance, "Appearance")?,
                flags: optional_u8(&dialog.ble_flags, "Flags")?,
                extra,
            })
        }
        RecordKind::Dpp => RecordDraft::Dpp(DppDraft {
            uri: dialog.dpp_uri.text().to_string(),
            carrier: if dialog.dpp_carrier.selected() == 1 {
                DppCarrier::Media
            } else {
                DppCarrier::Uri
            },
        }),
        RecordKind::SmartPoster => {
            let (mut titles, additional) = match &dialog.original {
                RecordDraft::SmartPoster(original) => {
                    (original.titles.clone(), original.additional.clone())
                }
                _ => (Vec::new(), Vec::new()),
            };
            let title = dialog.poster_title.text().to_string();
            if title.is_empty() {
                titles.clear();
            } else {
                let language = dialog.poster_language.text().to_string();
                let title = TextRecord::new(title, language).map_err(|error| error.to_string())?;
                if let Some(first) = titles.first_mut() {
                    *first = title;
                } else {
                    titles.push(title);
                }
            }
            RecordDraft::SmartPoster(SmartPosterDraft {
                uri: dialog.poster_uri.text().to_string(),
                titles,
                action: POSTER_ACTIONS
                    .get(dialog.poster_action.selected().saturating_sub(1) as usize)
                    .copied()
                    .filter(|_| dialog.poster_action.selected() > 0),
                additional,
            })
        }
        RecordKind::Raw => RecordDraft::Raw(RawDraft {
            tnf: ndef_editor::RAW_TNFS
                .get(dialog.raw_tnf.selected() as usize)
                .copied()
                .ok_or_else(|| "Choose a TNF value.".to_owned())?,
            record_type: if dialog.raw_type_hex.is_active() {
                ndef_editor::parse_hex(dialog.raw_type.text().as_str(), "TYPE")?
            } else {
                dialog.raw_type.text().as_bytes().to_vec()
            },
            id: ndef_editor::parse_hex(dialog.raw_id.text().as_str(), "ID")?,
            payload: ndef_editor::parse_hex(&text_view_get(&dialog.raw_payload), "Payload")?,
        }),
    };
    draft.to_record().map_err(|error| error.to_string())?;
    Ok(draft)
}

fn optional_u8(entry: &gtk::Entry, field: &str) -> Result<Option<u8>, String> {
    if entry.text().trim().is_empty() {
        Ok(None)
    } else {
        ndef_editor::parse_u8_hex(entry.text().as_str(), field).map(Some)
    }
}

fn optional_u16(entry: &gtk::Entry, field: &str) -> Result<Option<u16>, String> {
    if entry.text().trim().is_empty() {
        Ok(None)
    } else {
        ndef_editor::parse_u16_hex(entry.text().as_str(), field).map(Some)
    }
}

fn open_ndef_record_dialog(ui: Rc<Ui>, index: Option<usize>) {
    if ui.busy.get() || ui.ndef_view.pending_record_dialog.borrow().is_some() {
        return;
    }
    let original = index
        .and_then(|index| ui.ndef_view.editor.borrow().records().get(index).cloned())
        .unwrap_or_else(|| RecordKind::Text.default_draft());
    let editing = index.is_some();
    let dialog = Rc::new(build_record_form(&original));
    let window = gtk::Window::builder()
        .title(if editing {
            "Edit NDEF record"
        } else {
            "Add NDEF record"
        })
        .modal(true)
        .transient_for(&ui.window)
        .default_width(680)
        .default_height(620)
        .build();
    let header = gtk::HeaderBar::new();
    let cancel = gtk::Button::with_label("Cancel");
    header.pack_start(&cancel);
    header.pack_end(&dialog.save);
    window.set_titlebar(Some(&header));
    window.set_child(Some(&dialog.content));
    dialog.save.set_receives_default(true);
    window.set_default_widget(Some(&dialog.save));

    {
        let stack = dialog.stack.clone();
        dialog.kind.connect_selected_notify(move |dropdown| {
            if let Some(kind) = RecordKind::ALL.get(dropdown.selected() as usize) {
                stack.set_visible_child_name(kind.stack_name());
            }
        });
    }

    let ui2 = ui.clone();
    let window_weak = window.downgrade();
    cancel.connect_clicked(move |_| {
        ui2.ndef_view.pending_record_dialog.replace(None);
        if let Some(window) = window_weak.upgrade() {
            window.close();
        }
    });

    let ui2 = ui.clone();
    let window_weak = window.downgrade();
    let form_weak = Rc::downgrade(&dialog);
    dialog.save.connect_clicked(move |_| {
        let Some(form) = form_weak.upgrade() else {
            return;
        };
        let draft = match collect_record_form(&form) {
            Ok(draft) => draft,
            Err(error) => {
                form.feedback.set_localized(&error);
                form.feedback.add_css_class("error");
                return;
            }
        };
        {
            let mut editor = ui2.ndef_view.editor.borrow_mut();
            match index {
                Some(index) if editor.replace(index, draft.clone()) => {}
                _ => {
                    editor.add(draft.clone());
                }
            }
        }
        ui2.ndef_view
            .editor_key
            .replace(Some(current_ndef_key(&ui2)));
        ui2.ndef_view.pending_record_dialog.replace(None);
        after_ndef_editor_change(&ui2);
        if let Some(window) = window_weak.upgrade() {
            window.close();
        }
    });

    let ui2 = ui.clone();
    let form_keep = dialog.clone();
    window.connect_close_request(move |_| {
        let _ = &form_keep;
        ui2.ndef_view.pending_record_dialog.replace(None);
        glib::Propagation::Proceed
    });

    i18n::localize_widget_tree(&window);
    ui.ndef_view
        .pending_record_dialog
        .replace(Some(window.clone()));
    window.present();
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
        ui.reader_snapshot.replace(ReaderSnapshot {
            status: ReaderStatus::Connecting,
            selected: None,
            card: None,
            capabilities: ReaderCapabilities::default(),
            firmware: None,
            error: None,
        });
        render_reader_panel(ui);
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
    poll_reader_once(ui);
}

/// Poll the service monitor once from the GTK main context. Production uses
/// [`start_reader_monitor`]; tests call this directly for deterministic steps.
fn poll_reader_once(ui: &Rc<Ui>) {
    let update = match ui.service.lock() {
        Ok(mut service) => service.poll_reader(),
        Err(_) => {
            toast_error(ui, "Reader service lock was poisoned");
            return;
        }
    };
    apply_reader_update(ui, update);
}

/// Background card monitor. The service mutex serializes polling with every
/// operation, so a poll never touches the reader while an operation owns it.
fn start_reader_monitor(ui: Rc<Ui>) {
    let (sender, receiver) = async_channel::unbounded::<ReaderUpdate>();
    let service = ui.service.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let update = match service.lock() {
                Ok(mut service) => service.poll_reader(),
                Err(_) => break,
            };
            if sender.send_blocking(update).is_err() {
                break;
            }
        }
    });
    glib::spawn_future_local(async move {
        while let Ok(update) = receiver.recv().await {
            apply_reader_update(&ui, update);
        }
    });
}

fn clear_desfire_session(ui: &Rc<Ui>) {
    ui.desfire_view.key.set_text("");
    ui.desfire_view.output.set_text("");
    ui.desfire_view.status.set_text("");
    ui.desfire_view.verify_key.set_text("");
    ui.desfire_view.verify_output.set_text("");
    ui.desfire_view.verify_status.set_text("");
    ui.desfire_view.write_key.set_text("");
    ui.desfire_view.write_output.set_text("");
    ui.desfire_view.write_status.set_text("");
    ui.desfire_view.ndef_key.set_text("");
    ui.desfire_view.ndef_status.set_text("");
    ui.desfire_view.ndef_output.set_text("");
    ui.desfire_view.manage_key.set_text("");
    ui.desfire_view.mutation_key.set_text("");
    ui.desfire_view.mutation_tmac_key.set_text("");
    ui.desfire_view.mutation_output.set_text("");
    ui.desfire_view.mutation_status.set_text("");
    ui.desfire_view.manage_tmackey.set_text("");
    ui.desfire_view.manage_output.set_text("");
    ui.desfire_view.manage_status.set_text("");
    ui.desfire_view.start_record.set_text("0");
    ui.desfire_view.record_count.set_text("");
    ui.desfire_view.read_key_type.set_selected(0);
    ui.desfire_view.write_key_type.set_selected(0);
    ui.desfire_view.write_encoding.set_selected(0);
    ui.desfire_view.verify_key_type.set_selected(0);
    ui.desfire_view.ndef_key_type.set_selected(0);
    ui.desfire_view.manage_key_type.set_selected(0);
    ui.desfire_view.manage_app_key_type.set_selected(0);
    ui.desfire_view.manage_file_type.set_selected(0);
    ui.desfire_view.manage_communication.set_selected(0);
    ui.desfire_view.advanced.set_active(false);
    if let Some(dialog) = ui.pending_desfire_dialog.borrow_mut().take() {
        dialog.force_close();
    }
}

fn apply_reader_update(ui: &Rc<Ui>, update: ReaderUpdate) {
    ui.reader_snapshot.replace(update.snapshot);
    render_reader_panel(ui);
    for event in update.events {
        match event {
            ReaderEvent::CardRemoved(_) => {
                clear_desfire_session(ui);
                expire_live_image(ui, "Card removed · read the card again");
            }
            ReaderEvent::CardAppeared(card) => {
                let changed = live_card(ui).is_some_and(|current| !current.same_identity(&card));
                if changed {
                    clear_desfire_session(ui);
                    expire_live_image(ui, "A different card is present · read it again");
                }
            }
            ReaderEvent::StatusChanged(ReaderStatus::Recovering) => {
                clear_desfire_session(ui);
                expire_live_image(ui, "Reader unavailable · read the card again");
            }
            ReaderEvent::StatusChanged(ReaderStatus::Error) => {
                clear_desfire_session(ui);
                expire_live_image(ui, "Reader error · read the card again");
            }
            ReaderEvent::StatusChanged(_) => {}
        }
    }
}

/// Card identity of the image the workbench currently presents, live or not.
fn live_card(ui: &Ui) -> Option<CardInfo> {
    ui.dump
        .borrow()
        .as_ref()
        .and_then(|dump| dump.card.clone())
        .or_else(|| {
            ui.type2
                .borrow()
                .as_ref()
                .map(|session| session.card.clone())
        })
}

/// A card loss or reader failure invalidates the live image. The displayed
/// bytes stay available for inspection, but every write path requires a new
/// read, and Advanced mode resets with the card.
fn expire_live_image(ui: &Rc<Ui>, status: &str) {
    if !ui.live.get() {
        return;
    }
    ui.live.set(false);
    reset_advanced(ui);
    render_type2(ui);
    update_inspector(ui);
    update_card(ui);
    update_ndef_actions(ui);
    update_type2_actions(ui);
    update_restore_sensitivity(ui);
    ui.status.set_localized(status);
    append_log(ui, &format!("monitor · {status}"));
}

fn render_reader_panel(ui: &Ui) {
    let snapshot = ui.reader_snapshot.borrow();
    let status = match snapshot.status {
        ReaderStatus::Connecting => "Connecting…".to_owned(),
        ReaderStatus::Polling if snapshot.card.is_some() => "Polling · card present".to_owned(),
        ReaderStatus::Polling => "Polling · no card".to_owned(),
        ReaderStatus::Busy => "Busy".to_owned(),
        ReaderStatus::Recovering => match &snapshot.error {
            Some(error) => format!("Recovering: {error}"),
            None => "Recovering".to_owned(),
        },
        ReaderStatus::Error => match &snapshot.error {
            Some(error) => format!("Error: {error}"),
            None => "Error".to_owned(),
        },
    };
    ui.reader_state.set_localized(&status);
    match &snapshot.card {
        Some(card) => ui.reader_card_info.set_localized(&format!(
            "Card: {} · UID {}{}",
            card.card_type.label(),
            card.uid_hex(),
            if card.card_type == CardType::MifareMini {
                " · identity only"
            } else {
                ""
            }
        )),
        None => ui.reader_card_info.set_localized("Card: none"),
    }
    ui.reader_caps
        .set_localized(&capability_text(snapshot.capabilities));
}

fn capability_text(capabilities: ReaderCapabilities) -> String {
    let mut parts = Vec::new();
    match (capabilities.classic_read, capabilities.classic_write) {
        (true, true) => parts.push("MIFARE Classic read/write"),
        (true, false) => parts.push("MIFARE Classic read"),
        (false, true) => parts.push("MIFARE Classic write"),
        (false, false) => {}
    }
    match (capabilities.type2_read, capabilities.type2_write) {
        (true, true) => parts.push("Type 2 read/write"),
        (true, false) => parts.push("Type 2 read"),
        (false, true) => parts.push("Type 2 write"),
        (false, false) => {}
    }
    if capabilities.raw_bytes {
        parts.push("raw bytes");
    }
    if capabilities.raw_bits {
        parts.push("raw bits");
    }
    if capabilities.manufacturer_write {
        parts.push("UID write");
    }
    if capabilities.recovery_parity {
        parts.push("recovery");
    }
    if parts.is_empty() {
        "Capabilities\nnot opened".to_owned()
    } else {
        format!("Capabilities\n{}", parts.join(" · "))
    }
}

fn parse_desfire_hex(
    value: &str,
    bytes: usize,
    label: &str,
) -> std::result::Result<Vec<u8>, String> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != bytes * 2 {
        return Err(format!("{label} must contain {} hex digits", bytes * 2));
    }
    let mut output = Vec::with_capacity(bytes);
    for index in (0..compact.len()).step_by(2) {
        let byte = u8::from_str_radix(&compact[index..index + 2], 16)
            .map_err(|_| format!("{label} contains a non-hexadecimal digit"))?;
        output.push(byte);
    }
    Ok(output)
}

fn parse_desfire_key(value: &str) -> std::result::Result<Option<Vec<u8>>, String> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.is_empty() {
        return Ok(None);
    }
    if !compact.len().is_multiple_of(2) {
        return Err("the legacy key must have an even number of hex digits".to_owned());
    }
    let mut key = Vec::with_capacity(compact.len() / 2);
    for index in (0..compact.len()).step_by(2) {
        key.push(
            u8::from_str_radix(&compact[index..index + 2], 16)
                .map_err(|_| "the legacy key contains a non-hexadecimal digit".to_owned())?,
        );
    }
    if !matches!(key.len(), 8 | 16 | 24 | 32) {
        return Err(
            "a DESFire key must be 8, 16, 24, or 32 bytes (DES, 2K3DES, 3K3DES, AES)".to_owned(),
        );
    }
    Ok(Some(key))
}

fn parse_desfire_data(value: &str) -> std::result::Result<Vec<u8>, String> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.is_empty() {
        return Err("the write payload is empty".to_owned());
    }
    if !compact.len().is_multiple_of(2) {
        return Err("the write payload must have an even number of hex digits".to_owned());
    }
    let mut data = Vec::with_capacity(compact.len() / 2);
    for index in (0..compact.len()).step_by(2) {
        data.push(
            u8::from_str_radix(&compact[index..index + 2], 16)
                .map_err(|_| "the write payload contains a non-hexadecimal digit".to_owned())?,
        );
    }
    Ok(data)
}

fn desfire_ascii(bytes: &[u8]) -> String {
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

fn start_desfire_inventory(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    set_busy(&ui, true);
    ui.status.set_localized("Reading DESFire inventory…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service
                .desfire_inventory(CancelToken::new())
                .map(|inventory| {
                    let text = nfc_app::desfire::inventory_text(&inventory);
                    let first_aid = inventory
                        .applications
                        .first()
                        .map(|application| application.aid);
                    (text, first_aid)
                }),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_desfire_inventory(&ui, result),
            Err(_) => {
                finish_desfire_inventory(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_desfire_inventory(ui: &Rc<Ui>, result: NfcResult<(String, Option<[u8; 3]>)>) {
    set_busy(ui, false);
    match result {
        Ok((text, first_aid)) => {
            let view = &ui.desfire_view;
            view.summary.set_text(&text);
            if view.aid.text().trim().is_empty()
                && let Some(aid) = first_aid
            {
                view.aid.set_text(&type2_ui::hex_bytes(&aid));
            }
            view.tabs.set_visible_child_name("inventory");
            view.status.set_localized("Inventory updated");
            ui.status.set_localized("DESFire inventory updated");
            append_log(ui, "desfire · inventory");
        }
        Err(error) => {
            ui.desfire_view.status.set_localized("Inventory failed");
            ui.status.set_localized("DESFire inventory failed");
            toast_error(ui, &format!("DESFire inventory: {error}"));
        }
    }
}

fn start_desfire_read(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let view = &ui.desfire_view;
    let parsed = (|| -> std::result::Result<_, String> {
        let aid = parse_desfire_hex(&view.aid.text(), 3, "application ID")?;
        let file = parse_desfire_hex(&view.file.text(), 1, "file ID")?[0];
        let key = parse_desfire_key(&view.key.text())?;
        let key_type = desfire_key_type(&view.read_key_type);
        let key_number = view
            .key_number
            .text()
            .trim()
            .parse::<u8>()
            .map_err(|_| "key number must be 0..=14".to_owned())?;
        if key_number > 14 {
            return Err("key number must be 0..=14".to_owned());
        }
        let offset = view
            .offset
            .text()
            .trim()
            .parse::<u32>()
            .map_err(|_| "offset must be an unsigned integer".to_owned())?;
        let length = desfire_optional_u32(&view.length.text(), "length")?;
        let start_record = view
            .start_record
            .text()
            .trim()
            .parse::<u32>()
            .map_err(|_| "start record must be an unsigned integer".to_owned())?;
        let record_count = desfire_optional_u32(&view.record_count.text(), "record count")?;
        Ok((
            [aid[0], aid[1], aid[2]],
            file,
            key,
            key_type,
            key_number,
            offset,
            length,
            start_record,
            record_count,
        ))
    })();
    let (aid, file, key, key_type, key_number, offset, length, start_record, record_count) =
        match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                ui.desfire_view
                    .status
                    .set_localized("Read input is invalid");
                ui.status.set_localized("DESFire read input is invalid");
                toast_error(&ui, &error);
                return;
            }
        };
    let mut request = DesfireReadRequest::new(aid, file)
        .key_number(key_number)
        .offset(offset)
        .length(length)
        .start_record(start_record)
        .record_count(record_count);
    if let Some(key) = key {
        request = request.key(key);
    }
    if let Some(key_type) = key_type {
        request = request.key_type(key_type);
    }
    set_busy(&ui, true);
    ui.status.set_localized("Reading DESFire file…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service
                .desfire_read(CancelToken::new(), request)
                .map(|read| match read {
                    DesfireReadResult::Data(read) => {
                        let text = format!(
                            "settings: {}\nraw: {}\nprotection: {}\ndata: {}\nascii: {}",
                            read.settings.summary(),
                            type2_ui::hex_bytes(read.settings.raw()),
                            read_protection_text(&read.protection),
                            type2_ui::hex_bytes(&read.data),
                            desfire_ascii(&read.data)
                        );
                        (text, format!("{} bytes", read.data.len()))
                    }
                    DesfireReadResult::Value(read) => {
                        let text = format!(
                            "settings: {}\nprotection: {}\nvalue: {}",
                            read.settings.summary(),
                            read_protection_text(&read.protection),
                            read.value
                        );
                        (text, format!("value {}", read.value))
                    }
                    DesfireReadResult::Records(read) => {
                        let mut text = format!(
                            "settings: {}\nprotection: {}\nrecord offset: {} · {} record(s) × {} bytes",
                            read.settings.summary(),
                            read_protection_text(&read.protection),
                            read.start_record,
                            read.records.len(),
                            read.record_size
                        );
                        for (index, record) in read.records.iter().enumerate() {
                            text.push_str(&format!(
                                "\nrecord {}: {} | {}",
                                read.first_record as usize + index,
                                type2_ui::hex_bytes(record),
                                desfire_ascii(record)
                            ));
                        }
                        (text, format!("{} records", read.records.len()))
                    }
                }),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_desfire_read(&ui, result),
            Err(_) => {
                finish_desfire_read(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn start_desfire_write(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let input = match desfire_write_input(&ui.desfire_view) {
        Ok(input) => input,
        Err(error) => {
            ui.desfire_view
                .write_status
                .set_localized("Write input is invalid");
            ui.status.set_localized("DESFire write input is invalid");
            toast_error(&ui, &error);
            return;
        }
    };
    let mut request = DesfireWriteRequest::new(input.aid, input.file, input.data)
        .offset(input.offset)
        .key_number(input.key_number);
    if let Some(key) = input.key {
        request = request.key(key);
    }
    if let Some(key_type) = input.key_type {
        request = request.key_type(key_type);
    }
    set_busy(&ui, true);
    ui.status.set_localized("Writing the DESFire file…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service
                .desfire_write(CancelToken::new(), request)
                .map(|write| {
                    let detail = format!("{} bytes", write.written);
                    (write_text(&write), detail)
                }),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_desfire_write(&ui, result),
            Err(_) => {
                finish_desfire_write(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_desfire_write(ui: &Rc<Ui>, result: NfcResult<(String, String)>) {
    set_busy(ui, false);
    match result {
        Ok((text, detail)) => {
            ui.desfire_view.write_output.set_text(&text);
            ui.desfire_view.write_status.set_localized("Write verified");
            ui.status
                .set_localized(&format!("DESFire write verified · {detail}"));
            append_log(ui, &format!("desfire · write · {detail}"));
        }
        Err(error) => {
            ui.desfire_view.write_status.set_localized("Write failed");
            ui.status.set_localized("DESFire write failed");
            toast_error(ui, &format!("DESFire write: {error}"));
        }
    }
}

fn finish_desfire_read(ui: &Rc<Ui>, result: NfcResult<(String, String)>) {
    set_busy(ui, false);
    match result {
        Ok((text, detail)) => {
            ui.desfire_view.output.set_text(&text);
            ui.status.set_localized(&format!("DESFire read · {detail}"));
            append_log(ui, &format!("desfire · read · {detail}"));
        }
        Err(error) => {
            ui.desfire_view.status.set_localized("Read failed");
            ui.status.set_localized("DESFire read failed");
            toast_error(ui, &format!("DESFire read: {error}"));
        }
    }
}

struct DesfireWriteInput {
    aid: [u8; 3],
    file: u8,
    key: Option<Vec<u8>>,
    key_type: Option<AuthKeyType>,
    key_number: u8,
    offset: u32,
    data: Vec<u8>,
}

fn desfire_write_input(view: &DesfireView) -> std::result::Result<DesfireWriteInput, String> {
    let aid = parse_desfire_hex(&view.write_aid.text(), 3, "application ID")?;
    let data = match view.write_encoding.selected() {
        0 => parse_desfire_data(&view.write_data.text())?,
        _ => {
            let text = view.write_data.text().to_string();
            if text.is_empty() {
                return Err("the write payload is empty".to_owned());
            }
            text.into_bytes()
        }
    };
    Ok(DesfireWriteInput {
        aid: [aid[0], aid[1], aid[2]],
        file: parse_desfire_hex(&view.write_file.text(), 1, "file ID")?[0],
        key: parse_desfire_key(&view.write_key.text())?,
        key_type: desfire_key_type(&view.write_key_type),
        key_number: desfire_key_number(&view.write_key_number.text())?,
        offset: view
            .write_offset
            .text()
            .trim()
            .parse::<u32>()
            .map_err(|_| "offset must be an unsigned integer".to_owned())?,
        data,
    })
}

struct DesfireKeyInput {
    aid: [u8; 3],
    key: Vec<u8>,
    key_type: Option<AuthKeyType>,
    key_number: u8,
}

fn desfire_verify_input(
    aid_text: &str,
    key_text: &str,
    key_type_dropdown: &gtk::DropDown,
    key_number_text: &str,
) -> std::result::Result<DesfireKeyInput, String> {
    let aid = parse_desfire_hex(aid_text, 3, "application ID")?;
    Ok(DesfireKeyInput {
        aid: [aid[0], aid[1], aid[2]],
        key: desfire_required_application_key(key_text)?,
        key_type: desfire_key_type(key_type_dropdown),
        key_number: desfire_key_number(key_number_text)?,
    })
}

fn start_desfire_verify_key(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let view = &ui.desfire_view;
    let input = match desfire_verify_input(
        &view.verify_aid.text(),
        &view.verify_key.text(),
        &view.verify_key_type,
        &view.verify_key_number.text(),
    ) {
        Ok(parsed) => parsed,
        Err(error) => {
            ui.desfire_view
                .verify_status
                .set_localized("Verification input is invalid");
            ui.status
                .set_localized("DESFire verification input is invalid");
            toast_error(&ui, &error);
            return;
        }
    };
    let mut request =
        DesfireVerifyKeyRequest::new(input.aid, input.key).key_number(input.key_number);
    if let Some(key_type) = input.key_type {
        request = request.key_type(key_type);
    }
    set_busy(&ui, true);
    ui.status.set_localized("Verifying the DESFire key…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                service
                    .desfire_verify_key(CancelToken::new(), request)
                    .map(|verification| {
                        let detail = format!("key {}", verification.key_number);
                        (key_verification_text(&verification), detail)
                    })
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_desfire_verify(&ui, result),
            Err(_) => {
                finish_desfire_verify(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_desfire_verify(ui: &Rc<Ui>, result: NfcResult<(String, String)>) {
    set_busy(ui, false);
    match result {
        Ok((text, detail)) => {
            ui.desfire_view.verify_output.set_text(&text);
            ui.desfire_view.verify_status.set_localized("Key verified");
            ui.status
                .set_localized(&format!("DESFire key verified · {detail}"));
            append_log(ui, &format!("desfire · verify · {detail}"));
        }
        Err(error) => {
            ui.desfire_view
                .verify_status
                .set_localized("Verification failed");
            ui.status.set_localized("DESFire verification failed");
            toast_error(ui, &format!("DESFire verify: {error}"));
        }
    }
}

fn start_desfire_uid(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let view = &ui.desfire_view;
    let input = match desfire_verify_input(
        &view.verify_aid.text(),
        &view.verify_key.text(),
        &view.verify_key_type,
        &view.verify_key_number.text(),
    ) {
        Ok(parsed) => parsed,
        Err(error) => {
            ui.desfire_view
                .verify_status
                .set_localized("UID input is invalid");
            ui.status.set_localized("DESFire UID input is invalid");
            toast_error(&ui, &error);
            return;
        }
    };
    let mut request = DesfireUidRequest::new(input.aid, input.key).key_number(input.key_number);
    if let Some(key_type) = input.key_type {
        request = request.key_type(key_type);
    }
    set_busy(&ui, true);
    ui.status.set_localized("Reading the authenticated UID…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service
                .desfire_card_uid(CancelToken::new(), request)
                .map(|read| {
                    let detail = format!("UID {}", type2_ui::hex_bytes(&read.uid));
                    (card_uid_text(&read), detail)
                }),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_desfire_uid(&ui, result),
            Err(_) => {
                finish_desfire_uid(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_desfire_uid(ui: &Rc<Ui>, result: NfcResult<(String, String)>) {
    set_busy(ui, false);
    match result {
        Ok((text, detail)) => {
            ui.desfire_view.verify_output.set_text(&text);
            ui.desfire_view.verify_status.set_localized("UID read");
            ui.status
                .set_localized(&format!("DESFire UID read · {detail}"));
            append_log(ui, &format!("desfire · uid · {detail}"));
        }
        Err(error) => {
            ui.desfire_view
                .verify_status
                .set_localized("UID read failed");
            ui.status.set_localized("DESFire UID read failed");
            toast_error(ui, &format!("DESFire UID: {error}"));
        }
    }
}

fn desfire_key_type(dropdown: &gtk::DropDown) -> Option<AuthKeyType> {
    match dropdown.selected() {
        1 => Some(AuthKeyType::Des),
        2 => Some(AuthKeyType::Tdes2),
        3 => Some(AuthKeyType::Tdes3),
        4 => Some(AuthKeyType::Aes),
        5 => Some(AuthKeyType::AesEv2),
        _ => None,
    }
}

fn desfire_optional_u32(text: &str, label: &str) -> std::result::Result<Option<u32>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        trimmed
            .parse::<u32>()
            .map(Some)
            .map_err(|_| format!("{label} must be an unsigned integer"))
    }
}

fn desfire_key_number(text: &str) -> std::result::Result<u8, String> {
    let number = text
        .trim()
        .parse::<u8>()
        .map_err(|_| "key number must be 0..=14".to_owned())?;
    if number > 14 {
        return Err("key number must be 0..=14".to_owned());
    }
    Ok(number)
}

fn desfire_required_key(text: &str) -> std::result::Result<Vec<u8>, String> {
    parse_desfire_key(text)?.ok_or_else(|| "a PICC master key is required".to_owned())
}

fn desfire_required_application_key(text: &str) -> std::result::Result<Vec<u8>, String> {
    parse_desfire_key(text)?.ok_or_else(|| "a DESFire key is required".to_owned())
}

fn desfire_type4_access_label(access: u8) -> String {
    match access {
        0x00 => "free".to_owned(),
        0xFF => "denied".to_owned(),
        other => format!("proprietary {other:#04X}"),
    }
}

fn desfire_ndef_text(read: &nfc_app::type4::Type4ReadResult) -> String {
    let mut text = format!(
        "CC {} · mapping {}.{} · MLe {} · MLc {}\n",
        type2_ui::hex_bytes(read.cc.raw()),
        read.cc.major_version,
        read.cc.minor_version,
        read.cc.max_le,
        read.cc.max_lc
    );
    for tlv in read.cc.tlvs() {
        match tlv {
            nfc_type4::CcTlv::ProprietaryFile(control) => text.push_str(&format!(
                "CC TLV: proprietary file {:04X} · {} bytes · read {} · write {}\n",
                control.file_id,
                control.max_size,
                desfire_type4_access_label(control.read_access),
                desfire_type4_access_label(control.write_access)
            )),
            nfc_type4::CcTlv::Unknown { tag, value } => text.push_str(&format!(
                "CC TLV: unknown tag {tag:#04X} · {} bytes · {}\n",
                value.len(),
                ndef_editor::format_hex(value, " ")
            )),
            nfc_type4::CcTlv::NdefFile(_)
            | nfc_type4::CcTlv::Null
            | nfc_type4::CcTlv::Terminator => {}
        }
    }
    text.push_str(&format!(
        "NDEF file {:04X} · {} bytes · read {} · write {} · {}\n",
        read.ndef.file_id,
        read.ndef.max_size,
        desfire_type4_access_label(read.ndef.read_access),
        desfire_type4_access_label(read.ndef.write_access),
        nfc_app::type4::ndef_label(&read.ndef.state)
    ));
    if let Some(message) = read.ndef.message() {
        for (index, record) in message.records().iter().enumerate() {
            let draft = ndef_editor::RecordDraft::from_record(record);
            text.push_str(&format!(
                "{}. {} · {}\n",
                index + 1,
                draft.label(),
                draft.preview()
            ));
        }
    }
    if let Some(reason) = read.read_only_reason() {
        text.push_str(&format!("read-only · {reason}\n"));
    }
    text
}

fn start_desfire_ndef_read(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    set_busy(&ui, true);
    ui.status
        .set_localized("Reading the DESFire NDEF application…");
    ui.desfire_view
        .ndef_status
        .set_localized("Reading the NDEF application…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.type4_read(CancelToken::new()),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        match receiver.recv().await {
            Ok(Ok(read)) => {
                ui.desfire_view
                    .ndef_output
                    .set_text(&desfire_ndef_text(&read));
                ui.desfire_view.ndef_status.set_localized(&format!(
                    "NDEF: {}",
                    nfc_app::type4::ndef_label(&read.ndef.state)
                ));
                ui.status.set_localized("DESFire NDEF read");
                append_log(&ui, "desfire · ndef read");
            }
            Ok(Err(error)) => {
                ui.desfire_view
                    .ndef_status
                    .set_localized("NDEF read failed");
                ui.status.set_localized("DESFire NDEF read failed");
                toast_error(&ui, &format!("DESFire NDEF read: {error}"));
            }
            Err(_) => {
                ui.desfire_view
                    .ndef_status
                    .set_localized("NDEF read failed");
                toast_error(&ui, "Reader worker stopped");
            }
        }
    });
}

fn start_desfire_ndef_format(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    if !ui.desfire_view.advanced.is_active() {
        toast_error(
            &ui,
            "Enable Advanced mode before formatting the NDEF application",
        );
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let key = desfire_required_key(&ui.desfire_view.ndef_key.text())?;
        let key_type = desfire_key_type(&ui.desfire_view.ndef_key_type);
        let key_number = desfire_key_number(&ui.desfire_view.ndef_key_number.text())?;
        Ok((key, key_type, key_number))
    })();
    let (key, key_type, key_number) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };

    set_busy(&ui, true);
    ui.status.set_localized("Checking the DESFire NDEF plan…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.desfire_inventory(CancelToken::new()),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        let inventory = match receiver.recv().await {
            Ok(Ok(inventory)) => inventory,
            Ok(Err(error)) => {
                toast_error(&ui, &format!("DESFire inventory: {error}"));
                return;
            }
            Err(_) => {
                toast_error(&ui, "Reader worker stopped");
                return;
            }
        };
        if inventory
            .applications
            .iter()
            .any(|application| application.aid == NDEF_APPLICATION_AID)
        {
            toast_error(
                &ui,
                "The NDEF application D27600 already exists; delete it first",
            );
            return;
        }
        let body = format!(
            "{}\n\nCreate the NDEF Tag Application D27600 with DF name D2760000850101:\n\
             CC: DESFire file {:02X} / ISO E103, {} bytes, read free and write key 0\n\
             NDEF: DESFire file {:02X} / ISO E104, {} bytes, read and write free\n\n\
             No full-card backup exists. Delete the application with the same PICC key to undo.\n\n\
             Type FORMAT to confirm",
            inventory.version.name(),
            NDEF_CC_FILE,
            NDEF_CC_SIZE,
            NDEF_DATA_FILE,
            NDEF_DATA_SIZE,
        );
        let dialog = typed_confirmation_dialog(
            "Format the DESFire NDEF application?",
            &body,
            "Format NDEF",
            "FORMAT",
        );
        ui.pending_desfire_dialog.replace(Some(dialog.clone()));
        let ui2 = ui.clone();
        let key = RefCell::new(Some(key));
        dialog.connect_response(None, move |_, response| {
            ui2.pending_desfire_dialog.replace(None);
            if response == "confirm"
                && let Some(key) = key.borrow_mut().take()
            {
                execute_desfire_ndef_format(ui2.clone(), key, key_type, key_number);
            }
        });
        present_alert(&dialog, &ui.window);
    });
}

fn execute_desfire_ndef_format(
    ui: Rc<Ui>,
    key: Vec<u8>,
    key_type: Option<AuthKeyType>,
    key_number: u8,
) {
    set_busy(&ui, true);
    ui.status
        .set_localized("Formatting the DESFire NDEF application…");
    ui.desfire_view
        .ndef_status
        .set_localized("Formatting the NDEF application…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = DesfireNdefFormatRequest::new(key)
                    .key_number(key_number)
                    .advanced(true)
                    .confirm(true);
                if let Some(key_type) = key_type {
                    request = request.key_type(key_type);
                }
                service.desfire_ndef_format(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        match receiver.recv().await {
            Ok(Ok(result)) => {
                ui.desfire_view.ndef_status.set_localized(&format!(
                    "NDEF application formatted · CC {} · {}",
                    type2_ui::hex_bytes(result.cc.raw()),
                    nfc_app::type4::ndef_label(&result.ndef.state)
                ));
                ui.status
                    .set_localized("DESFire NDEF application formatted");
                append_log(&ui, "desfire · ndef format");
                start_desfire_inventory(ui);
            }
            Ok(Err(error)) => {
                ui.desfire_view
                    .ndef_status
                    .set_localized("NDEF formatting failed");
                ui.status.set_localized("DESFire NDEF formatting failed");
                toast_error(&ui, &format!("DESFire NDEF format: {error}"));
            }
            Err(_) => {
                ui.desfire_view
                    .ndef_status
                    .set_localized("NDEF formatting failed");
                toast_error(&ui, "Reader worker stopped");
            }
        }
    });
}

fn start_desfire_ndef_delete(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    if !ui.desfire_view.advanced.is_active() {
        toast_error(
            &ui,
            "Enable Advanced mode before deleting the NDEF application",
        );
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let key = desfire_required_key(&ui.desfire_view.ndef_key.text())?;
        let key_type = desfire_key_type(&ui.desfire_view.ndef_key_type);
        let key_number = desfire_key_number(&ui.desfire_view.ndef_key_number.text())?;
        Ok((key, key_type, key_number))
    })();
    let (key, key_type, key_number) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };

    set_busy(&ui, true);
    ui.status
        .set_localized("Checking the DESFire NDEF application…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.desfire_inventory(CancelToken::new()),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        let inventory = match receiver.recv().await {
            Ok(Ok(inventory)) => inventory,
            Ok(Err(error)) => {
                toast_error(&ui, &format!("DESFire inventory: {error}"));
                return;
            }
            Err(_) => {
                toast_error(&ui, "Reader worker stopped");
                return;
            }
        };
        if !inventory
            .applications
            .iter()
            .any(|application| application.aid == NDEF_APPLICATION_AID)
        {
            toast_error(&ui, "The NDEF application D27600 is not present");
            return;
        }
        let body = format!(
            "{}\n\nDelete the NDEF Tag Application D27600 and its CC/NDEF files.\n\
             Only the exact AN11004 layout is deleted. No byte-for-byte restore exists.\n\n\
             Type DELETE to confirm",
            inventory.version.name()
        );
        let dialog = typed_confirmation_dialog(
            "Delete the DESFire NDEF application?",
            &body,
            "Delete NDEF",
            "DELETE",
        );
        ui.pending_desfire_dialog.replace(Some(dialog.clone()));
        let ui2 = ui.clone();
        let key = RefCell::new(Some(key));
        dialog.connect_response(None, move |_, response| {
            ui2.pending_desfire_dialog.replace(None);
            if response == "confirm"
                && let Some(key) = key.borrow_mut().take()
            {
                execute_desfire_ndef_delete(ui2.clone(), key, key_type, key_number);
            }
        });
        present_alert(&dialog, &ui.window);
    });
}

fn execute_desfire_ndef_delete(
    ui: Rc<Ui>,
    key: Vec<u8>,
    key_type: Option<AuthKeyType>,
    key_number: u8,
) {
    set_busy(&ui, true);
    ui.status
        .set_localized("Deleting the DESFire NDEF application…");
    ui.desfire_view
        .ndef_status
        .set_localized("Deleting the NDEF application…");
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = DesfireNdefDeleteRequest::new(key)
                    .key_number(key_number)
                    .advanced(true)
                    .confirm(true);
                if let Some(key_type) = key_type {
                    request = request.key_type(key_type);
                }
                service.desfire_ndef_delete(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        match receiver.recv().await {
            Ok(Ok(_)) => {
                ui.desfire_view
                    .ndef_status
                    .set_localized("NDEF application deleted");
                ui.status.set_localized("DESFire NDEF application deleted");
                append_log(&ui, "desfire · ndef delete");
                start_desfire_inventory(ui);
            }
            Ok(Err(error)) => {
                ui.desfire_view
                    .ndef_status
                    .set_localized("NDEF delete failed");
                ui.status.set_localized("DESFire NDEF delete failed");
                toast_error(&ui, &format!("DESFire NDEF delete: {error}"));
            }
            Err(_) => {
                ui.desfire_view
                    .ndef_status
                    .set_localized("NDEF delete failed");
                toast_error(&ui, "Reader worker stopped");
            }
        }
    });
}

struct DesfireManageKey {
    aid: [u8; 3],
    key: Option<Vec<u8>>,
    key_type: Option<AuthKeyType>,
    key_number: u8,
}

fn desfire_manage_key(view: &DesfireView) -> std::result::Result<DesfireManageKey, String> {
    let aid = parse_desfire_hex(&view.manage_aid.text(), 3, "application ID")?;
    let key = parse_desfire_key(&view.manage_key.text())?;
    let key_type = desfire_key_type(&view.manage_key_type);
    let key_number = desfire_key_number(&view.manage_key_number.text())?;
    Ok(DesfireManageKey {
        aid: [aid[0], aid[1], aid[2]],
        key,
        key_type,
        key_number,
    })
}

fn desfire_manage_app_key_type(dropdown: &gtk::DropDown) -> ApplicationKeyType {
    match dropdown.selected() {
        1 => ApplicationKeyType::Tdes3,
        2 => ApplicationKeyType::Aes,
        _ => ApplicationKeyType::Tdes2,
    }
}

fn desfire_manage_communication(dropdown: &gtk::DropDown) -> CommunicationMode {
    match dropdown.selected() {
        1 => CommunicationMode::Maced,
        2 => CommunicationMode::Enciphered,
        _ => CommunicationMode::Plain,
    }
}

fn desfire_manage_required_u32(text: &str, label: &str) -> std::result::Result<u32, String> {
    text.trim()
        .parse::<u32>()
        .map_err(|_| format!("{label} must be an unsigned integer"))
}

fn desfire_manage_required_i32(text: &str, label: &str) -> std::result::Result<i32, String> {
    text.trim()
        .parse::<i32>()
        .map_err(|_| format!("{label} must be an integer"))
}

fn desfire_manage_access_rights(view: &DesfireView) -> std::result::Result<u16, String> {
    let raw = parse_desfire_hex(&view.manage_access_rights.text(), 2, "access rights")?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn desfire_manage_file_id(view: &DesfireView) -> std::result::Result<u8, String> {
    Ok(parse_desfire_hex(&view.manage_file.text(), 1, "file ID")?[0])
}

fn desfire_manage_file_spec(view: &DesfireView) -> std::result::Result<DesfireFileSpec, String> {
    let file_id = desfire_manage_file_id(view)?;
    let iso_file_id = match view.manage_iso_file.text().trim() {
        "" => None,
        _ => {
            let raw = parse_desfire_hex(&view.manage_iso_file.text(), 2, "ISO file ID")?;
            Some(u16::from_le_bytes([raw[0], raw[1]]))
        }
    };
    let communication = desfire_manage_communication(&view.manage_communication).raw();
    let access_rights = desfire_manage_access_rights(view)?;
    match view.manage_file_type.selected() {
        1 => Ok(DesfireFileSpec::Backup {
            file_id,
            iso_file_id: None,
            communication,
            access_rights,
            size: desfire_manage_required_u32(&view.manage_size.text(), "size")?,
        }),
        2 => Ok(DesfireFileSpec::Value {
            file_id,
            iso_file_id: None,
            communication,
            access_rights,
            lower_limit: desfire_manage_required_i32(
                &view.manage_lower_limit.text(),
                "lower limit",
            )?,
            upper_limit: desfire_manage_required_i32(
                &view.manage_upper_limit.text(),
                "upper limit",
            )?,
            value: match view.manage_value.text().trim() {
                "" => 0,
                text => text
                    .parse::<i32>()
                    .map_err(|_| "value must be an integer".to_owned())?,
            },
            flags: parse_desfire_hex(&view.manage_flags.text(), 1, "value flags")?[0],
        }),
        3 => Ok(DesfireFileSpec::LinearRecord {
            file_id,
            iso_file_id: None,
            communication,
            access_rights,
            record_size: desfire_manage_required_u32(
                &view.manage_record_size.text(),
                "record size",
            )?,
            max_records: desfire_manage_required_u32(
                &view.manage_max_records.text(),
                "maximum records",
            )?,
        }),
        4 => Ok(DesfireFileSpec::CyclicRecord {
            file_id,
            iso_file_id: None,
            communication,
            access_rights,
            record_size: desfire_manage_required_u32(
                &view.manage_record_size.text(),
                "record size",
            )?,
            max_records: desfire_manage_required_u32(
                &view.manage_max_records.text(),
                "maximum records",
            )?,
        }),
        5 => {
            let key = parse_desfire_hex(&view.manage_tmackey.text(), 16, "Transaction MAC key")?;
            let key: [u8; 16] = key
                .try_into()
                .map_err(|_| "Transaction MAC key must be 16 bytes".to_owned())?;
            let key_version = view
                .manage_tmackey_version
                .text()
                .trim()
                .parse::<u8>()
                .map_err(|_| "Transaction MAC key version must be 0..=255".to_owned())?;
            Ok(DesfireFileSpec::TransactionMac {
                file_id,
                iso_file_id,
                communication,
                access_rights,
                key,
                key_version,
            })
        }
        _ => Ok(DesfireFileSpec::Standard {
            file_id,
            iso_file_id,
            communication,
            access_rights,
            size: desfire_manage_required_u32(&view.manage_size.text(), "size")?,
        }),
    }
}

fn run_desfire_manage<F>(
    ui: Rc<Ui>,
    busy_status: &'static str,
    success_status: &'static str,
    refresh: bool,
    task: F,
) where
    F: FnOnce(&mut Service) -> NfcResult<String> + Send + 'static,
{
    set_busy(&ui, true);
    ui.status.set_localized(busy_status);
    ui.desfire_view.manage_status.set_localized(busy_status);
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => task(&mut service),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        match receiver.recv().await {
            Ok(Ok(text)) => {
                ui.desfire_view.manage_output.set_text(&text);
                ui.desfire_view.manage_status.set_localized(success_status);
                ui.status.set_localized(success_status);
                append_log(&ui, "desfire · manage");
                update_desfire_actions(&ui);
                if refresh {
                    start_desfire_inventory(ui);
                }
            }
            Ok(Err(error)) => {
                ui.desfire_view
                    .manage_status
                    .set_localized("DESFire management failed");
                ui.status.set_localized("DESFire management failed");
                toast_error(&ui, &format!("DESFire management: {error}"));
            }
            Err(_) => {
                ui.desfire_view
                    .manage_status
                    .set_localized("Reader worker stopped");
                toast_error(&ui, "Reader worker stopped");
            }
        }
    });
}

fn run_desfire_mutation<F>(
    ui: Rc<Ui>,
    busy_status: &'static str,
    success_status: &'static str,
    task: F,
) where
    F: FnOnce(&mut Service) -> NfcResult<String> + Send + 'static,
{
    set_busy(&ui, true);
    ui.status.set_localized(busy_status);
    ui.desfire_view.mutation_status.set_localized(busy_status);
    let service = ui.service.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => task(&mut service),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        match receiver.recv().await {
            Ok(Ok(text)) => {
                ui.desfire_view.mutation_output.set_text(&text);
                ui.desfire_view
                    .mutation_status
                    .set_localized(success_status);
                ui.status.set_localized(success_status);
                append_log(&ui, "desfire · value/record");
                update_desfire_actions(&ui);
                start_desfire_inventory(ui);
            }
            Ok(Err(error)) => {
                ui.desfire_view
                    .mutation_status
                    .set_localized("DESFire value or record change failed");
                ui.status
                    .set_localized("DESFire value or record change failed");
                toast_error(&ui, &format!("DESFire value/record: {error}"));
            }
            Err(_) => {
                ui.desfire_view
                    .mutation_status
                    .set_localized("Reader worker stopped");
                toast_error(&ui, "Reader worker stopped");
            }
        }
    });
}

fn parse_desfire_bytes(value: &str, label: &str) -> std::result::Result<Vec<u8>, String> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if !compact.len().is_multiple_of(2) {
        return Err(format!("{label} must have an even number of hex digits"));
    }
    let mut output = Vec::with_capacity(compact.len() / 2);
    for index in (0..compact.len()).step_by(2) {
        output.push(
            u8::from_str_radix(&compact[index..index + 2], 16)
                .map_err(|_| format!("{label} contains a non-hexadecimal digit"))?,
        );
    }
    Ok(output)
}

fn start_desfire_mutation(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let aid = parse_desfire_hex(&ui.desfire_view.mutation_aid.text(), 3, "application ID")?;
        let aid = [aid[0], aid[1], aid[2]];
        let file_id = parse_desfire_hex(&ui.desfire_view.mutation_file.text(), 1, "file ID")?[0];
        let key = parse_desfire_key(&ui.desfire_view.mutation_key.text())?;
        let key_type = desfire_key_type(&ui.desfire_view.mutation_key_type);
        let key_number = desfire_key_number(&ui.desfire_view.mutation_key_number.text())?;
        let tmac_key = match ui.desfire_view.mutation_tmac_key.text().trim() {
            "" => None,
            _ => {
                let bytes = parse_desfire_hex(
                    &ui.desfire_view.mutation_tmac_key.text(),
                    16,
                    "Transaction MAC key",
                )?;
                Some(<[u8; 16]>::try_from(bytes.as_slice()).expect("16-byte key"))
            }
        };
        let operation = ui.desfire_view.mutation_operation.selected();
        let amount = match operation {
            0..=2 => Some(desfire_manage_required_u32(
                &ui.desfire_view.mutation_amount.text(),
                "amount",
            )?),
            _ => None,
        };
        let record =
            desfire_manage_required_u32(&ui.desfire_view.mutation_record.text(), "record number")?;
        let offset =
            desfire_manage_required_u32(&ui.desfire_view.mutation_offset.text(), "record offset")?;
        let data = match operation {
            3 | 4 => Some(parse_desfire_bytes(
                &ui.desfire_view.mutation_data.text(),
                "record data",
            )?),
            _ => None,
        };
        Ok((
            aid, file_id, key, key_type, key_number, tmac_key, operation, amount, record, offset,
            data,
        ))
    })();
    let (
        aid,
        file_id,
        key,
        key_type,
        key_number,
        tmac_key,
        operation,
        amount,
        record,
        offset,
        data,
    ) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    if operation <= 2 {
        let operation = match operation {
            0 => DesfireValueOperation::Credit,
            1 => DesfireValueOperation::Debit,
            _ => DesfireValueOperation::LimitedCredit,
        };
        let mut request = DesfireValueRequest::new(aid, file_id, operation, amount.unwrap_or(0))
            .key_number(key_number);
        if let Some(key) = key {
            request = request.key(key);
        }
        if let Some(key_type) = key_type {
            request = request.key_type(key_type);
        }
        if let Some(tmac_key) = tmac_key {
            request = request.tmac_key(tmac_key);
        }
        run_desfire_mutation(
            ui,
            "Changing the DESFire value…",
            "DESFire value changed",
            move |service| {
                service
                    .desfire_value_change(request)
                    .map(|result| value_text(&result))
            },
        );
    } else {
        let operation = match operation {
            3 => DesfireRecordOperation::Append,
            4 => DesfireRecordOperation::Update {
                record_number: record,
                offset,
            },
            _ => DesfireRecordOperation::Clear,
        };
        let mut request = DesfireRecordRequest::new(aid, file_id, operation).key_number(key_number);
        if let Some(key) = key {
            request = request.key(key);
        }
        if let Some(key_type) = key_type {
            request = request.key_type(key_type);
        }
        if let Some(tmac_key) = tmac_key {
            request = request.tmac_key(tmac_key);
        }
        if let Some(data) = data {
            request = request.data(data);
        }
        run_desfire_mutation(
            ui,
            "Changing the DESFire records…",
            "DESFire records changed",
            move |service| {
                service
                    .desfire_record_change(request)
                    .map(|result| record_text(&result))
            },
        );
    }
}

fn start_desfire_app_create(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let input = desfire_manage_key(&ui.desfire_view)?;
        let keys = desfire_manage_required_u32(&ui.desfire_view.manage_app_keys.text(), "keys")?;
        let keys = u8::try_from(keys).map_err(|_| "keys must be 1..=14".to_owned())?;
        let app_key_type = desfire_manage_app_key_type(&ui.desfire_view.manage_app_key_type);
        Ok((input, keys, app_key_type))
    })();
    let (input, keys, app_key_type) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    run_desfire_manage(
        ui,
        "Creating the DESFire application…",
        "DESFire application created",
        true,
        move |service| {
            let mut request = DesfireAppCreateRequest::new(input.aid)
                .keys(keys)
                .app_key_type(app_key_type)
                .key_number(input.key_number);
            if let Some(key) = input.key {
                request = request.key(key);
            }
            if let Some(key_type) = input.key_type {
                request = request.key_type(key_type);
            }
            service
                .desfire_create_application(request)
                .map(|result| app_create_text(&result))
        },
    );
}

fn start_desfire_app_delete(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    if !ui.desfire_view.advanced.is_active() {
        toast_error(&ui, "Enable Advanced mode before deleting an application");
        return;
    }
    let input = match desfire_manage_key(&ui.desfire_view) {
        Ok(input) => input,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let body = format!(
        "Delete application {} and every file it contains.\n\
         A manifest with the file settings is written before the command.\n\
         No byte-for-byte restore exists.\n\n\
         Type DELETE to confirm",
        type2_ui::hex_bytes(&input.aid)
    );
    let dialog = typed_confirmation_dialog(
        "Delete the DESFire application?",
        &body,
        "Delete application",
        "DELETE",
    );
    ui.pending_desfire_dialog.replace(Some(dialog.clone()));
    let ui2 = ui.clone();
    let input = RefCell::new(Some(input));
    dialog.connect_response(None, move |_, response| {
        ui2.pending_desfire_dialog.replace(None);
        if response != "confirm" {
            return;
        }
        let Some(input) = input.borrow_mut().take() else {
            return;
        };
        run_desfire_manage(
            ui2.clone(),
            "Deleting the DESFire application…",
            "DESFire application deleted",
            true,
            move |service| {
                let mut request = DesfireAppDeleteRequest::new(input.aid)
                    .key_number(input.key_number)
                    .advanced(true)
                    .confirm(true);
                if let Some(key) = input.key {
                    request = request.key(key);
                }
                if let Some(key_type) = input.key_type {
                    request = request.key_type(key_type);
                }
                service
                    .desfire_delete_application(request)
                    .map(|result| app_delete_text(&result))
            },
        );
    });
    present_alert(&dialog, &ui.window);
}

fn start_desfire_file_create(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let input = desfire_manage_key(&ui.desfire_view)?;
        let spec = desfire_manage_file_spec(&ui.desfire_view)?;
        Ok((input, spec))
    })();
    let (input, spec) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    run_desfire_manage(
        ui,
        "Creating the DESFire file…",
        "DESFire file created",
        true,
        move |service| {
            let mut request =
                DesfireFileCreateRequest::new(input.aid, spec).key_number(input.key_number);
            if let Some(key) = input.key {
                request = request.key(key);
            }
            if let Some(key_type) = input.key_type {
                request = request.key_type(key_type);
            }
            service
                .desfire_create_file(request)
                .map(|result| file_create_text(&result))
        },
    );
}

fn start_desfire_file_delete(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    if !ui.desfire_view.advanced.is_active() {
        toast_error(&ui, "Enable Advanced mode before deleting a file");
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let input = desfire_manage_key(&ui.desfire_view)?;
        let file_id = desfire_manage_file_id(&ui.desfire_view)?;
        Ok((input, file_id))
    })();
    let (input, file_id) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let body = format!(
        "Delete file {file_id:02X} from application {}.\n\
         A manifest with the file settings is written before the command.\n\
         No byte-for-byte restore exists.\n\n\
         Type DELETE to confirm",
        type2_ui::hex_bytes(&input.aid)
    );
    let dialog =
        typed_confirmation_dialog("Delete the DESFire file?", &body, "Delete file", "DELETE");
    ui.pending_desfire_dialog.replace(Some(dialog.clone()));
    let ui2 = ui.clone();
    let state = RefCell::new(Some((input, file_id)));
    dialog.connect_response(None, move |_, response| {
        ui2.pending_desfire_dialog.replace(None);
        if response != "confirm" {
            return;
        }
        let Some((input, file_id)) = state.borrow_mut().take() else {
            return;
        };
        run_desfire_manage(
            ui2.clone(),
            "Deleting the DESFire file…",
            "DESFire file deleted",
            true,
            move |service| {
                let mut request = DesfireFileDeleteRequest::new(input.aid, file_id)
                    .key_number(input.key_number)
                    .advanced(true)
                    .confirm(true);
                if let Some(key) = input.key {
                    request = request.key(key);
                }
                if let Some(key_type) = input.key_type {
                    request = request.key_type(key_type);
                }
                service
                    .desfire_delete_file(request)
                    .map(|result| file_delete_text(&result))
            },
        );
    });
    present_alert(&dialog, &ui.window);
}

fn start_desfire_file_settings(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let input = desfire_manage_key(&ui.desfire_view)?;
        let file_id = desfire_manage_file_id(&ui.desfire_view)?;
        let communication = desfire_manage_communication(&ui.desfire_view.manage_communication);
        let access_rights = desfire_manage_access_rights(&ui.desfire_view)?;
        Ok((input, file_id, communication, access_rights))
    })();
    let (input, file_id, communication, access_rights) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    run_desfire_manage(
        ui,
        "Changing the DESFire file settings…",
        "DESFire file settings changed",
        true,
        move |service| {
            let mut request = DesfireFileSettingsRequest::new(input.aid, file_id)
                .key_number(input.key_number)
                .communication(communication)
                .access_rights(access_rights);
            if let Some(key) = input.key {
                request = request.key(key);
            }
            if let Some(key_type) = input.key_type {
                request = request.key_type(key_type);
            }
            service
                .desfire_change_file_settings(request)
                .map(|result| file_settings_text(&result))
        },
    );
}

fn start_desfire_memory(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    let parsed = (|| -> std::result::Result<_, String> {
        let aid = match ui.desfire_view.manage_aid.text().trim() {
            "" => None,
            _ => {
                let aid =
                    parse_desfire_hex(&ui.desfire_view.manage_aid.text(), 3, "application ID")?;
                Some([aid[0], aid[1], aid[2]])
            }
        };
        let key = parse_desfire_key(&ui.desfire_view.manage_key.text())?;
        let key_type = desfire_key_type(&ui.desfire_view.manage_key_type);
        let key_number = desfire_key_number(&ui.desfire_view.manage_key_number.text())?;
        Ok((aid, key, key_type, key_number))
    })();
    let (aid, key, key_type, key_number) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    run_desfire_manage(
        ui,
        "Reading the DESFire free memory…",
        "DESFire free memory read",
        false,
        move |service| {
            let mut request = DesfireFreeMemoryRequest::new(aid).key_number(key_number);
            if let Some(key) = key {
                request = request.key(key);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(key_type);
            }
            service
                .desfire_free_memory(CancelToken::new(), request)
                .map(|free| free_memory_text(aid, free))
        },
    );
}

fn typed_confirmation_dialog(
    heading: &str,
    body: &str,
    confirm_label: &str,
    phrase: &str,
) -> adw::AlertDialog {
    let dialog = confirmation_dialog(heading, body, confirm_label);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(&format!("Type {phrase} to confirm")));
    entry.set_activates_default(true);
    entry.connect_map(|entry| {
        entry.grab_focus();
    });
    dialog.set_extra_child(Some(&entry));
    dialog.set_response_enabled("confirm", false);
    dialog.set_default_response(Some("confirm"));
    let weak = dialog.downgrade();
    let phrase = phrase.to_owned();
    entry.connect_changed(move |entry| {
        if let Some(dialog) = weak.upgrade() {
            dialog.set_response_enabled("confirm", entry.text().trim() == phrase);
        }
    });
    dialog
}

fn update_desfire_actions(ui: &Ui) {
    let sensitive = !ui.busy.get();
    let advanced = sensitive && ui.desfire_view.advanced.is_active();
    ui.desfire_view.refresh.set_sensitive(sensitive);
    ui.desfire_view.read.set_sensitive(sensitive);
    ui.desfire_view.write.set_sensitive(sensitive);
    ui.desfire_view.verify.set_sensitive(sensitive);
    ui.desfire_view.read_uid.set_sensitive(sensitive);
    ui.desfire_view.read_ndef.set_sensitive(sensitive);
    ui.desfire_view.format_ndef.set_sensitive(advanced);
    ui.desfire_view.delete_ndef.set_sensitive(advanced);
    ui.desfire_view.ndef_key.set_sensitive(sensitive);
    ui.desfire_view.ndef_key_number.set_sensitive(sensitive);
    ui.desfire_view.create_app.set_sensitive(sensitive);
    ui.desfire_view.delete_app.set_sensitive(advanced);
    ui.desfire_view.create_file.set_sensitive(sensitive);
    ui.desfire_view
        .change_file_settings
        .set_sensitive(sensitive);
    ui.desfire_view.delete_file.set_sensitive(advanced);
    ui.desfire_view.read_memory.set_sensitive(sensitive);
}

fn start_read(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.workspace.get() == Workspace::Desfire {
        start_desfire_inventory(ui);
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    let type2_auth = match type2_auth_from_ui(&ui) {
        Ok(auth) => auth,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    reset_advanced(&ui);
    set_busy(&ui, true);
    ui.status.set_localized("Reading card…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let include_common_keys = ui.preferences.borrow().use_common_keys;
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                service.read_card(&keys, include_common_keys, type2_auth, CancelToken::new())
            }
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
            let card = read.dump.card.clone();
            let captured = read.keys.clone();
            let warnings = read.warnings;
            let identity_only = read.identity_only;
            let dump = read.dump;
            if identity_only {
                ui.type2.replace(None);
                ui.type2_restore.replace(None);
                ui.live.set(false);
                reset_workbench(ui, &dump);
                ui.dump.replace(Some(dump.clone()));
                ui.dump_source.replace(Some("identity only".to_owned()));
                populate_rows(ui, &dump);
                switch_workspace(ui, Workspace::Classic);
                ui.classic_tabs.set_visible_child_name("blocks");
                invalidate_ndef_editor(ui);
                update_card(ui);
                update_ndef(ui);
                update_restore_sensitivity(ui);
                render_type2(ui);
                let uid = card
                    .as_ref()
                    .map(CardInfo::uid_hex)
                    .unwrap_or_else(|| "unknown".to_owned());
                ui.status.set_localized(&format!(
                    "Card identified · UID {uid} · no MIFARE data readable"
                ));
                append_log(
                    ui,
                    &format!(
                        "read · identity only · UID {uid} · 0/{} blocks",
                        dump.layout.block_count()
                    ),
                );
                for warning in &warnings {
                    append_log(ui, &format!("warning · {warning}"));
                }
                toast_info(ui, "Card identified; no MIFARE Classic data readable");
                return;
            }
            ui.type2.replace(None);
            ui.type2_restore.replace(None);
            ui.live.set(true);
            reset_workbench(ui, &dump);
            ui.dump.replace(Some(dump.clone()));
            ui.dump_source.replace(Some("live card".to_owned()));
            populate_rows(ui, &dump);
            switch_workspace(ui, Workspace::Classic);
            ui.classic_tabs.set_visible_child_name("blocks");
            invalidate_ndef_editor(ui);
            update_card(ui);
            update_ndef(ui);
            update_restore_sensitivity(ui);
            render_type2(ui);
            if let Some(card) = &card {
                record_captured_keys(ui, card, &captured);
            }
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
            clear_type2_auth(ui);
            update_type2_actions(ui);
            update_ndef_actions(ui);
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
        Ok(CardReadResult::IdentityOnly(card)) => {
            ui.type2.replace(None);
            ui.type2_restore.replace(None);
            ui.live.set(false);
            ui.dump.replace(None);
            ui.dump_source.replace(None);
            ui.rows.remove_all();
            ui.table_stack.set_visible_child_name("empty");
            reset_advanced(ui);
            update_card(ui);
            update_ndef(ui);
            update_restore_sensitivity(ui);
            render_type2(ui);
            let uid = card.uid_hex();
            ui.status.set_localized(&format!(
                "Card identified · UID {uid} · no supported read path"
            ));
            append_log(
                ui,
                &format!(
                    "read · identity only · {} · UID {uid}",
                    card.card_type.label()
                ),
            );
            toast_info(ui, "Card identified; no supported read path for its data");
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
    let dirty = ui.dirty_blocks.borrow().clone();
    for row in classic::rows_for_dirty(dump, &dirty) {
        ui.rows.append(&glib::BoxedAnyObject::new(row));
    }
    if dump.blocks.is_empty() {
        ui.table_stack.set_visible_child_name("empty");
    } else {
        ui.table_stack.set_visible_child_name("table");
        ui.selection.set_selected(0);
    }
}

fn workbench_snapshot(ui: &Ui) -> Option<WorkbenchSnapshot> {
    ui.service
        .lock()
        .ok()
        .and_then(|service| service.workbench_snapshot())
}

fn apply_workbench_snapshot(ui: &Ui, snapshot: &WorkbenchSnapshot) {
    let selected = ui.selection.selected();
    ui.dirty_blocks.replace(snapshot.dirty_blocks.clone());
    ui.undo_depth.set(snapshot.undo_depth);
    ui.dump.replace(Some(snapshot.dump.clone()));
    populate_rows(ui, &snapshot.dump);
    if selected != gtk::INVALID_LIST_POSITION {
        ui.selection.set_selected(selected);
    }
    update_inspector(ui);
    update_card(ui);
    update_ndef(ui);
    update_restore_sensitivity(ui);
}

fn refresh_workbench(ui: &Ui) {
    if let Some(snapshot) = workbench_snapshot(ui) {
        apply_workbench_snapshot(ui, &snapshot);
    }
}

fn load_workbench(ui: &Ui, dump: &Dump) {
    if let Ok(mut service) = ui.service.lock()
        && let Err(error) = service.workbench_load(dump.clone())
    {
        append_log(ui, &format!("workbench · load failed · {error}"));
    }
}

fn reset_workbench(ui: &Ui, dump: &Dump) {
    load_workbench(ui, dump);
    ui.dirty_blocks.replace(Vec::new());
    ui.undo_depth.set(0);
}

fn stage_edit(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let Some(row) = selected_row(&ui) else {
        return;
    };
    if row.number == 0 {
        toast_error(&ui, "Block 0 is read-only");
        return;
    }
    let data = match classic::parse_hex_16(ui.inspector.hex.text().as_str()) {
        Ok(data) => data,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    let result = match ui.service.lock() {
        Ok(mut service) => service.workbench_edit(row.number, data),
        Err(_) => {
            toast_error(&ui, "reader service lock was poisoned");
            return;
        }
    };
    match result {
        Ok(snapshot) => {
            apply_workbench_snapshot(&ui, &snapshot);
            ui.status
                .set_localized(&format!("Block {:03} staged", row.number));
        }
        Err(error) => toast_error(&ui, &error.to_string()),
    }
}

fn undo_edit(ui: Rc<Ui>) {
    let result = match ui.service.lock() {
        Ok(mut service) => service.workbench_undo(),
        Err(_) => {
            toast_error(&ui, "reader service lock was poisoned");
            return;
        }
    };
    match result {
        Ok(snapshot) => {
            apply_workbench_snapshot(&ui, &snapshot);
            ui.status.set_localized("Edit undone");
        }
        Err(error) => toast_error(&ui, &error.to_string()),
    }
}

fn revert_edits(ui: Rc<Ui>) {
    let result = match ui.service.lock() {
        Ok(mut service) => service.workbench_revert(),
        Err(_) => {
            toast_error(&ui, "reader service lock was poisoned");
            return;
        }
    };
    match result {
        Ok(snapshot) => {
            apply_workbench_snapshot(&ui, &snapshot);
            ui.status.set_localized("Edits reverted");
        }
        Err(error) => toast_error(&ui, &error.to_string()),
    }
}

fn replace_row(ui: &Ui, block: usize) {
    let row = {
        let dump = ui.dump.borrow();
        let dirty = ui.dirty_blocks.borrow();
        dump.as_ref().and_then(|dump| {
            dump.blocks.get(block).map(|block| {
                let mut row = BlockRow::from_block(block, dump.layout);
                row.dirty = dirty.binary_search(&block.number).is_ok();
                row
            })
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

fn finish_read_block(ui: &Rc<Ui>, block: usize, result: NfcResult<ClassicBlockReadResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let merged = ui
                .service
                .lock()
                .ok()
                .map(|mut service| service.workbench_merge_block(block, result.data));
            match merged {
                Some(Ok(report)) => {
                    if !report.conflicts.is_empty() {
                        append_log(
                            ui,
                            &format!(
                                "merge · block {block:03} conflicts with an edit and was kept"
                            ),
                        );
                    }
                    refresh_workbench(ui);
                }
                Some(Err(error)) => {
                    append_log(ui, &format!("merge · block {block:03} failed · {error}"));
                    replace_row(ui, block);
                    update_inspector(ui);
                }
                None => {
                    if let Some(dump) = ui.dump.borrow_mut().as_mut()
                        && let Some(target) = dump.blocks.get_mut(block)
                    {
                        target.set_read(result.data);
                    }
                    replace_row(ui, block);
                    update_inspector(ui);
                }
            }
            update_card(ui);
            record_captured_keys(ui, &result.card, &[result.key]);
            ui.status.set_localized(&format!("Block {block:03} read"));
            append_log(ui, &format!("read block · {block:03}"));
        }
        Err(error) => {
            ui.status.set_localized("Block read failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn confirm_workbench_write(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "Writing changes needs a live selected card");
        return;
    }
    let Some(source) = ui.dump.borrow().clone() else {
        toast_error(&ui, "No workbench image is loaded");
        return;
    };
    if ui.dirty_blocks.borrow().is_empty() {
        toast_info(&ui, "There are no edits to write");
        return;
    }
    start_preflight(
        &ui,
        source,
        RestoreSelection::ChangesOnly,
        None,
        ui.restore_view.continue_on_error.is_active(),
        None,
        false,
    );
}

#[allow(clippy::too_many_arguments)]
fn start_preflight(
    ui: &Rc<Ui>,
    source: Dump,
    selection: RestoreSelection,
    restore_path: Option<PathBuf>,
    continue_on_failure: bool,
    uid_target: Option<[u8; 4]>,
    advanced: bool,
) {
    set_busy(ui, true);
    ui.status.set_localized("Building the write plan…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let include_common_keys = ui.preferences.borrow().use_common_keys;
    let expected_card = source.card.clone();
    let request_source = source.clone();
    let use_workbench_baseline = restore_path.is_none();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = ClassicWritePreflight::new(request_source, &keys)
                    .include_common_keys(include_common_keys)
                    .selection(selection)
                    .continue_on_failure(continue_on_failure)
                    .advanced(advanced);
                if let Some(uid) = uid_target {
                    request = request.uid_target(uid);
                }
                if let Some(card) = expected_card {
                    request = request.expected_card(card);
                }
                if use_workbench_baseline && let Some(baseline) = service.workbench_baseline_dump()
                {
                    request = request.baseline(baseline);
                }
                service.classic_write_preflight(request)
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_preflight(&ui, source, restore_path, advanced, result),
            Err(_) => finish_preflight(
                &ui,
                source,
                restore_path,
                advanced,
                Err(NfcError::Io("reader worker stopped".to_owned())),
            ),
        }
    });
}

fn finish_preflight(
    ui: &Rc<Ui>,
    source: Dump,
    restore_path: Option<PathBuf>,
    advanced: bool,
    result: NfcResult<ClassicWritePlan>,
) {
    set_busy(ui, false);
    match result {
        Ok(plan) => {
            let uid_changes = plan.uid.as_ref().is_some_and(|uid| uid.changed);
            if plan.blocks.is_empty() && !uid_changes {
                ui.status.set_localized("No writes planned");
                toast_info(ui, "The card already matches the selected blocks");
                return;
            }
            append_log(
                ui,
                &format!(
                    "plan · {} · {} block(s), {} trailer(s){}",
                    plan.selection.label(),
                    plan.blocks.len(),
                    plan.trailer_writes(),
                    plan.uid
                        .as_ref()
                        .map(|uid| format!(" · UID {}", format_uid(&uid.target_uid)))
                        .unwrap_or_default()
                ),
            );
            show_plan_dialog(
                ui,
                PendingPlan {
                    plan,
                    source,
                    restore_path,
                    advanced,
                },
            );
        }
        Err(error) => {
            ui.status.set_localized("Write preflight failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn plan_body(plan: &ClassicWritePlan) -> String {
    let mut body = format!(
        "{} · UID {}\n{} · {} block(s), {} trailer(s)\n{} unchanged, {} unknown skipped · plan expires at {}\n",
        classic::display_layout(plan.layout),
        plan.card.uid_hex(),
        plan.selection.label(),
        plan.blocks.len(),
        plan.trailer_writes(),
        plan.unchanged_blocks,
        plan.skipped_unknown_blocks,
        plan.expires_at
    );
    body.push_str(
        "\nA project backup is written before the write, and every block is read back.\n",
    );
    if plan.uid.is_none() {
        body.push_str("Block 0 is never written.\n");
    }
    if let Some(uid) = &plan.uid {
        let backends = uid
            .backends
            .iter()
            .map(|backend| backend.label())
            .collect::<Vec<_>>()
            .join(", ");
        body.push_str(&format!(
            "\nUID target {} · current UID {}\nPost-restore key {} · write with {} · backends: {backends}\nOld block 0 {}\nNew block 0 {}\nThe UID is written only after the restore and sector 0 are verified.\n",
            format_uid(&uid.target_uid),
            format_uid(&uid.current_uid),
            uid.sector0_key.0.label(),
            uid.write_keys.label(),
            classic::hex_with_mask(&uid.old_block0, u16::MAX),
            classic::hex_with_mask(&uid.new_block0, u16::MAX),
        ));
    }
    for block in plan.blocks.iter().take(12) {
        body.push_str(&format!(
            "\nBlock {:03} · sector {:02} · {} · {}",
            block.block,
            block.sector,
            block.role_label(),
            block.access_note
        ));
        body.push_str(&format!(
            "\n  old {}\n  new {}",
            classic::hex_with_mask(&block.old, u16::MAX),
            classic::hex_with_mask(&block.new, u16::MAX)
        ));
    }
    if plan.blocks.len() > 12 {
        body.push_str(&format!("\n… and {} more block(s)", plan.blocks.len() - 12));
    }
    for warning in &plan.warnings {
        body.push_str(&format!("\nWarning: {warning}"));
    }
    body
}

fn show_plan_dialog(ui: &Rc<Ui>, pending: PendingPlan) {
    let restore = pending.restore_path.is_some();
    let uid = pending.plan.uid.is_some();
    let heading = if uid {
        "Restore the plan and write the target UID?"
    } else if restore {
        "Restore the plan to the live card?"
    } else {
        "Write the edited blocks?"
    };
    let action = if uid {
        "Restore and write UID"
    } else if restore {
        "Restore"
    } else {
        "Write changes"
    };
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(plan_body(&pending.plan))
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "write", action);
    dialog.set_response_appearance("write", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_plan_dialog.replace(None);
        let pending = ui2.pending_plan.borrow_mut().take();
        if response == "write"
            && let Some(pending) = pending
        {
            start_plan_execution(&ui2, pending);
        }
    });
    ui.pending_plan.replace(Some(pending));
    ui.pending_plan_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_plan_execution(ui: &Rc<Ui>, pending: PendingPlan) {
    set_busy(ui, true);
    ui.status.set_localized("Writing the planned blocks…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.borrow().clone();
    let token = pending.plan.token;
    let restore_path = pending.restore_path.clone();
    let advanced = pending.advanced;
    let source = if restore_path.is_none() {
        ui.dump
            .borrow()
            .clone()
            .unwrap_or_else(|| pending.source.clone())
    } else {
        pending.source.clone()
    };
    let include_common_keys = ui.preferences.borrow().use_common_keys;
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.classic_write_execute(
                ClassicWriteExecution::new(token, source, &keys, &backup)
                    .include_common_keys(include_common_keys)
                    .advanced(advanced)
                    .confirmed(true),
            ),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => {
                if let Some(path) = restore_path {
                    finish_restore(&ui, path, result);
                } else {
                    finish_workbench_write(&ui, result);
                }
            }
            Err(_) => {
                let error = Err(NfcError::Io("reader worker stopped".to_owned()));
                if let Some(path) = restore_path {
                    finish_restore(&ui, path, error);
                } else {
                    finish_workbench_write(&ui, error);
                }
            }
        }
    });
}

fn finish_workbench_write(ui: &Rc<Ui>, result: NfcResult<ClassicRestoreResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            let backup = result
                .backup
                .as_ref()
                .map(|backup| backup.path.display().to_string())
                .unwrap_or_else(|| "no backup needed".to_owned());
            append_log(
                ui,
                &format!(
                    "write changes · {} block(s), {} trailer(s) · {backup}",
                    result.written_blocks, result.trailer_writes
                ),
            );
            for warning in &result.warnings {
                append_log(ui, &format!("warning · {warning}"));
            }
            for failure in &result.failures {
                append_log(ui, &format!("failure · {failure}"));
            }
            refresh_workbench(ui);
            ui.status.set_localized(&format!(
                "Changes written · {} block(s)",
                result.written_blocks
            ));
            if result.failures.is_empty() {
                toast_info(ui, "Changes written and read back");
            } else {
                toast_info(ui, "Write finished with failures; see the activity log");
            }
        }
        Err(error) => {
            ui.status.set_localized("Write failed");
            toast_error(ui, &error.to_string());
            refresh_workbench(ui);
        }
    }
}

fn update_classic_uid_actions(ui: &Ui) {
    let classic_1k = ui
        .dump
        .borrow()
        .as_ref()
        .and_then(|dump| dump.card.as_ref())
        .is_some_and(|card| card.card_type == CardType::MifareClassic1K && card.uid.len() == 4);
    if !classic_1k {
        ui.inspector.uid_advanced.set_active(false);
    }
    let live = ui.live.get() && classic_1k && !ui.busy.get();
    ui.inspector.uid_advanced.set_sensitive(live);
    let advanced = ui.inspector.uid_advanced.is_active();
    ui.inspector.uid.set_sensitive(live && advanced);
    ui.inspector.uid_write.set_sensitive(live && advanced);
}

fn confirm_uid_write(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if !ui.inspector.uid_advanced.is_active() {
        toast_error(&ui, "Enable Advanced mode before changing the UID");
        return;
    }
    if !ui.live.get() {
        toast_error(&ui, "A UID write needs a live selected card");
        return;
    }
    let card = ui.dump.borrow().as_ref().and_then(|dump| dump.card.clone());
    let Some(card) = card else {
        toast_error(&ui, "Read a card before changing its UID");
        return;
    };
    if card.card_type != CardType::MifareClassic1K || card.uid.len() != 4 {
        toast_error(
            &ui,
            "UID writes require a four-byte UID MIFARE Classic 1K card",
        );
        return;
    }
    let target = match parse_uid(ui.inspector.uid.text().as_str()) {
        Ok(uid) => uid,
        Err(error) => {
            toast_error(&ui, &error.to_string());
            return;
        }
    };
    if target.as_slice() == card.uid.as_slice() {
        toast_error(&ui, "The target UID already matches the card");
        return;
    }
    set_busy(&ui, true);
    ui.status
        .set_localized("Reading sector 0 for the UID plan…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let include_common_keys = ui.preferences.borrow().use_common_keys;
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.classic_uid_preflight(
                ClassicUidPreflight::new(target, &keys)
                    .include_common_keys(include_common_keys)
                    .expected_card(card)
                    .advanced(true),
            ),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_uid_preflight(&ui, result),
            Err(_) => {
                finish_uid_preflight(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_uid_preflight(ui: &Rc<Ui>, result: NfcResult<ClassicUidPlan>) {
    set_busy(ui, false);
    match result {
        Ok(plan) => show_uid_plan_dialog(ui, plan),
        Err(error) => {
            ui.status.set_localized("UID plan failed");
            toast_error(ui, &error.to_string());
        }
    }
}

fn uid_plan_body(plan: &ClassicUidPlan) -> String {
    let backends = plan
        .backends
        .iter()
        .map(|backend| backend.label())
        .collect::<Vec<_>>()
        .join(", ");
    let write_key = plan
        .write_key
        .map(|(key_type, _)| key_type.label().to_string())
        .unwrap_or_else(|| "none".to_owned());
    let mut body = format!(
        "Target UID {}\nCurrent UID {}\nSector 0 read key {} · write key {write_key} · write with {}\nBackends: {}\n\nOld {}\nNew {}\n\nA sector-0 backup is written first. The card is reselected after the write and the complete block 0, BCC, and trailer are verified.",
        format_uid(&plan.target_uid),
        plan.card.uid_hex(),
        plan.sector0_key.0.label(),
        plan.write_keys.label(),
        backends,
        classic::hex_with_mask(&plan.old_block0, u16::MAX),
        classic::hex_with_mask(&plan.new_block0, u16::MAX),
    );
    for warning in &plan.warnings {
        body.push_str("\nWarning: ");
        body.push_str(warning);
    }
    body
}

fn show_uid_plan_dialog(ui: &Rc<Ui>, plan: ClassicUidPlan) {
    let dialog = confirmation_dialog(
        "Write manufacturer block 0?",
        &uid_plan_body(&plan),
        "Write UID",
    );
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_uid_dialog.replace(None);
        let plan = ui2.pending_uid_plan.borrow_mut().take();
        if response == "confirm"
            && let Some(plan) = plan
        {
            start_uid_execution(&ui2, plan);
        }
    });
    ui.pending_uid_plan.replace(Some(plan));
    ui.pending_uid_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_uid_execution(ui: &Rc<Ui>, plan: ClassicUidPlan) {
    set_busy(ui, true);
    ui.status.set_localized("Writing manufacturer block 0…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.borrow().clone();
    let include_common_keys = ui.preferences.borrow().use_common_keys;
    let token = plan.token;
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.classic_uid_execute(
                ClassicUidExecution::new(token, &keys, &backup)
                    .include_common_keys(include_common_keys)
                    .advanced(true)
                    .confirmed(true),
            ),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_uid_execution(&ui, result),
            Err(_) => {
                finish_uid_execution(&ui, Err(NfcError::Io("reader worker stopped".to_owned())))
            }
        }
    });
}

fn finish_uid_execution(ui: &Rc<Ui>, result: NfcResult<nfc_app::classic::ClassicUidResult>) {
    set_busy(ui, false);
    match result {
        Ok(result) => {
            for warning in &result.warnings {
                append_log(ui, &format!("warning · {warning}"));
            }
            if !result.changed {
                ui.status
                    .set_localized("The target UID already matches the card");
                toast_info(ui, "No UID write was needed");
                return;
            }
            let backend = result
                .backend
                .map(|backend| backend.label())
                .unwrap_or("unknown");
            let backup = result
                .backup
                .as_ref()
                .map(|record| record.backup.path.display().to_string())
                .unwrap_or_else(|| "no backup".to_owned());
            append_log(
                ui,
                &format!(
                    "uid · {} -> {} · {backend} · {backup}",
                    result.card_before.uid_hex(),
                    result.card_after.uid_hex()
                ),
            );
            toast_info(
                ui,
                &format!("UID changed to {}", result.card_after.uid_hex()),
            );
            invalidate_live_image(ui);
            ui.status.set_localized("UID changed · re-reading the card");
            start_read(ui.clone());
        }
        Err(error) => {
            ui.status.set_localized("UID write failed");
            toast_error(ui, &error.to_string());
            invalidate_live_image(ui);
        }
    }
}

fn invalidate_live_image(ui: &Rc<Ui>) {
    ui.live.set(false);
    ui.dump.replace(None);
    ui.dump_source.replace(None);
    if let Ok(mut service) = ui.service.lock() {
        service.workbench_clear();
    }
    ui.dirty_blocks.replace(Vec::new());
    ui.undo_depth.set(0);
    ui.ndef.replace(None);
    ui.rows.remove_all();
    ui.table_stack.set_visible_child_name("empty");
    ui.selection.set_selected(gtk::INVALID_LIST_POSITION);
    ui.inspector.uid.set_text("");
    ui.inspector.uid_advanced.set_active(false);
    ui.restore_view.uid.set_text("");
    ui.restore_view.uid_advanced.set_active(false);
    render_ndef(ui, None, None);
    update_card(ui);
    update_inspector(ui);
    update_restore_sensitivity(ui);
    update_ndef_actions(ui);
}

fn choose_ndef_file(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Load raw NDEF message"))
        .build();
    set_dialog_folder(&dialog, DataFolder::Ndef);
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
        load_ndef_file(&ui2, &path);
    });
}

fn load_ndef_file(ui: &Rc<Ui>, path: &Path) {
    if ui.busy.get() {
        return;
    }
    let loaded = ui.ndef_view.editor.borrow_mut().load_file(path);
    match loaded {
        Ok(()) => {
            ui.ndef_view.editor_key.replace(Some(current_ndef_key(ui)));
            after_ndef_editor_change(ui);
            ui.status
                .set_localized(&format!("Loaded {}", path.display()));
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

fn export_ndef_message(ui: &Rc<Ui>) {
    let bytes = match ui.ndef_view.editor.borrow().encoded() {
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
    set_dialog_folder(&dialog, DataFolder::Ndef);
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
    if ui.workspace.get() == Workspace::Type2 {
        if ui.type2.borrow().is_some() {
            confirm_type2_ndef_write(ui);
        } else {
            toast_error(&ui, "Read a Type 2 tag first");
        }
        return;
    }
    if ui.type2.borrow().is_some() && ui.dump.borrow().is_none() {
        toast_error(&ui, "Read a Classic card first");
        return;
    }
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "NDEF writes need a live selected card");
        return;
    }
    let (message, bytes, summary) = {
        let editor = ui.ndef_view.editor.borrow();
        let message = match editor.message() {
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
        (message, bytes, editor.summary())
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
        summary,
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
            let written = ui.ndef_view.editor.borrow().message().ok();
            if let Some(message) = written {
                ui.ndef_view.editor.borrow_mut().mark_written(&message);
            }
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
    if ui.workspace.get() == Workspace::Type2 && ui.type2.borrow().is_some() {
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
            ui.ndef_view.editor.borrow_mut().mark_formatted();
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
    let merged = ui
        .service
        .lock()
        .ok()
        .map(|mut service| service.workbench_merge(dump));
    ui.ndef.replace(Some(info.clone()));
    match merged {
        Some(Ok(report)) => {
            if !report.conflicts.is_empty() {
                append_log(
                    ui,
                    &format!(
                        "merge · NDEF write conflicts with {} staged edit(s)",
                        report.conflicts.len()
                    ),
                );
            }
            refresh_workbench(ui);
        }
        _ => {
            reset_workbench(ui, dump);
            populate_rows(ui, dump);
            ui.dump.replace(Some(dump.clone()));
            update_card(ui);
        }
    }
    render_ndef(ui, Some(info), None);
    update_restore_sensitivity(ui);
    record_dump_metadata_keys(ui, dump);
}

fn confirmation_dialog(heading: &str, body: &str, confirm_label: &str) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "confirm", confirm_label);
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog
}

/// Neutral confirmation for an operation that needs authorization but does not
/// destroy card data.
fn authorization_dialog(heading: &str, body: &str, confirm_label: &str) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    add_response(&dialog, "cancel", "Cancel");
    add_response(&dialog, "confirm", confirm_label);
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog
}

fn reset_advanced(ui: &Ui) {
    ui.type2_view.advanced.set_active(false);
    ui.type2_restore.replace(None);
    ui.type2_view
        .restore_label
        .set_localized("No restore image loaded");
    ui.inspector.uid_advanced.set_active(false);
    ui.inspector.uid.set_text("");
    ui.restore_view.uid_advanced.set_active(false);
    ui.restore_view.uid.set_text("");
    ui.desfire_view.advanced.set_active(false);
    ui.desfire_view.ndef_key.set_text("");
    if let Some(dialog) = ui.pending_uid_dialog.borrow_mut().take() {
        dialog.force_close();
    }
    if let Some(dialog) = ui.pending_recovery_dialog.borrow_mut().take() {
        dialog.force_close();
    }
    ui.pending_uid_plan.replace(None);
    update_classic_uid_actions(ui);
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
    let Some(insight) = type2_ui::inspect(session, row.page, advanced, type2_auth_ready(ui)) else {
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

fn type2_auth_from_ui(ui: &Ui) -> std::result::Result<Option<Type2Auth>, String> {
    let password = ui.type2_view.auth_pwd.text();
    let pack = ui.type2_view.auth_pack.text();
    let password = password.trim();
    let pack = pack.trim();
    if password.is_empty() && pack.is_empty() {
        return Ok(None);
    }
    let password = type2_ui::parse_hex_bytes::<4>(password, "access password")?;
    let pack = type2_ui::parse_hex_bytes::<2>(pack, "access PACK")?;
    Ok(Some(Type2Auth::new(password, pack)))
}

fn type2_auth_ready(ui: &Ui) -> bool {
    type2_auth_from_ui(ui).ok().flatten().is_some()
}

fn clear_type2_auth(ui: &Ui) {
    ui.type2_view.auth_pwd.set_text("");
    ui.type2_view.auth_pack.set_text("");
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
    let gate = type2_ui::write_gate(session, row.page, advanced, type2_auth_ready(ui));
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
    let auth_ready = type2_auth_ready(ui);
    let has_session = session.is_some();
    let has_configuration = session
        .as_ref()
        .is_some_and(|session| session.configuration().is_some());
    let needs_current = session.as_ref().is_some_and(|session| {
        session.image.layout.config.is_some_and(|pages| {
            session.configuration().is_some_and(|config| {
                [pages.cfg0, pages.cfg1]
                    .into_iter()
                    .any(|page| config.requires_password(page))
            })
        })
    });
    view.save.set_sensitive(has_session && !busy);
    view.load.set_sensitive(!busy);
    view.advanced.set_sensitive(has_session && !busy);
    view.restore_load.set_sensitive(has_session && !busy);
    view.password_apply.set_sensitive(
        live && !busy && advanced && has_configuration && (!needs_current || auth_ready),
    );
    view.password_note.set_localized(if !has_session {
        "Read a Type 2 tag to inspect its password configuration."
    } else if !has_configuration {
        "This tag has no readable NTAG configuration pages."
    } else if !advanced {
        "Enable Advanced mode to change locks, configuration, or the password."
    } else if needs_current && !auth_ready {
        "This configuration is protected; enter the current access password and PACK first."
    } else {
        "PWD and PACK are write-only; the service verifies PACK with PWD_AUTH after the write."
    });

    let uid_ready = session
        .as_ref()
        .is_some_and(|session| session.image.uid.len() == 7);
    view.uid_target.set_sensitive(live && !busy && advanced);
    view.uid_apply
        .set_sensitive(live && !busy && advanced && uid_ready);
    view.uid_note.set_localized(if !has_session {
        "Read a Type 2 tag to change its UID."
    } else if !uid_ready {
        "This UID length has no supported Type 2 UID workflow."
    } else if !advanced {
        "Enable Advanced mode to change the UID."
    } else {
        "Only a UID-writable NTAG213 clone accepts this. A full backup is written first; page 0/1/2 are read back and reselected, and the original UID is restored on failure."
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
                            if protected && !auth_ready {
                                lock_text = "Password protection covers the lock pages; enter the access password and PACK first.".to_owned();
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
    ui.dump_source.replace(None);
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
    ui.type2_view.tabs.set_visible_child_name("pages");
    switch_workspace(ui, Workspace::Type2);
    invalidate_ndef_editor(ui);
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
    set_dialog_folder(&dialog, DataFolder::Dumps);
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
    set_dialog_folder(&dialog, DataFolder::Dumps);
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
    let auth = match type2_auth_from_ui(&ui) {
        Ok(auth) => auth,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
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
    let dialog = confirmation_dialog("Restore the Type 2 image?", &body, "Restore image");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_type2_restore_dialog.replace(None);
        if response == "confirm" {
            start_type2_restore(ui2.clone(), source.clone(), options, auth);
        }
    });
    ui.pending_type2_restore_dialog
        .replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_type2_restore(
    ui: Rc<Ui>,
    source: Type2Image,
    options: RestoreOptions,
    auth: Option<Type2Auth>,
) {
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
                    .auth(auth)
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
            clear_type2_auth(ui);
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
    let auth = match type2_auth_from_ui(&ui) {
        Ok(auth) => auth,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    if let Err(error) = type2_ui::write_gate(&session, row.page, advanced, auth.is_some()) {
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
        let dialog = confirmation_dialog("Write an Advanced Type 2 page?", &body, "Write page");
        let ui2 = ui.clone();
        dialog.connect_response(None, move |_, response| {
            ui2.pending_type2_advanced_dialog.replace(None);
            if response == "confirm" {
                start_type2_page_write(ui2.clone(), row.page, data, true, auth);
            }
        });
        ui.pending_type2_advanced_dialog
            .replace(Some(dialog.clone()));
        present_alert(&dialog, &ui.window);
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
                start_type2_page_write(ui2.clone(), row.page, data, false, auth);
            }
        });
        ui.pending_type2_page_dialog.replace(Some(dialog.clone()));
        present_alert(&dialog, &ui.window);
    }
}

fn start_type2_page_write(
    ui: Rc<Ui>,
    page: u8,
    data: [u8; 4],
    advanced: bool,
    auth: Option<Type2Auth>,
) {
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
                    .confirm(advanced)
                    .auth(auth);
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
            clear_type2_auth(ui);
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
    let auth = match type2_auth_from_ui(&ui) {
        Ok(auth) => auth,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
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
    let dialog = confirmation_dialog("Apply one-way lock bits?", &body, "Apply lock bits");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_type2_lock_dialog.replace(None);
        if response == "confirm" {
            start_type2_lock(ui2.clone(), pages.clone(), auth);
        }
    });
    ui.pending_type2_lock_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_type2_lock(ui: Rc<Ui>, pages: Vec<u8>, auth: Option<Type2Auth>) {
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
                let mut request = Type2Lock::new(pages, &backup).confirm(true).auth(auth);
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
            clear_type2_auth(ui);
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

fn confirm_type2_uid(ui: Rc<Ui>) {
    if ui.busy.get() || !ui.live.get() {
        toast_error(&ui, "Changing the UID needs a live selected card");
        return;
    }
    if !ui.type2_view.advanced.is_active() {
        toast_error(&ui, "Enable Advanced mode before changing the UID");
        return;
    }
    if ui.type2.borrow().is_none() {
        toast_error(&ui, "Read a Type 2 tag first");
        return;
    }
    let target = match parse_uid_hex(&ui.type2_view.uid_target.text()) {
        Ok(uid) => uid,
        Err(error) => {
            toast_error(&ui, &error.to_string());
            return;
        }
    };
    let service = ui.service.clone();
    set_busy(&ui, true);
    ui.status.set_localized("Checking the Type 2 UID plan…");
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.type2_uid_preflight(&target, CancelToken::new()),
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        set_busy(&ui, false);
        match receiver.recv().await {
            Ok(Ok(plan)) => present_type2_uid_plan(&ui, plan),
            Ok(Err(error)) => toast_error(&ui, &error.to_string()),
            Err(_) => toast_error(&ui, "reader worker stopped"),
        }
    });
}

fn present_type2_uid_plan(ui: &Rc<Ui>, plan: Type2UidPlan) {
    let body = format!(
        "{} · UID {}\n\npage 0: {} -> {}\npage 1: {} -> {}\npage 2: {} -> {}\n\nA project backup is written first. Pages 0/1/2 are read back and the card is reselected. If anything fails, the original UID is written back.",
        plan.card.card_type.label(),
        plan.card.uid_hex(),
        type2_ui::hex_bytes(&plan.current_pages[0]),
        type2_ui::hex_bytes(&plan.new_pages[0]),
        type2_ui::hex_bytes(&plan.current_pages[1]),
        type2_ui::hex_bytes(&plan.new_pages[1]),
        type2_ui::hex_bytes(&plan.current_pages[2]),
        type2_ui::hex_bytes(&plan.new_pages[2]),
    );
    let dialog = confirmation_dialog("Change the Type 2 UID?", &body, "Change UID");
    ui.pending_type2_uid_plan.replace(Some(plan));
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_type2_uid_dialog.replace(None);
        if response == "confirm"
            && let Some(plan) = ui2.pending_type2_uid_plan.borrow_mut().take()
        {
            start_type2_uid_execute(ui2.clone(), plan);
        }
    });
    ui.pending_type2_uid_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_type2_uid_execute(ui: Rc<Ui>, plan: Type2UidPlan) {
    set_busy(&ui, true);
    ui.status.set_localized("Changing the Type 2 UID…");
    let service = ui.service.clone();
    let backup = ui.backup.borrow().clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let executed =
                    service.type2_uid_execute(Type2UidExecution::new(plan, &backup).confirm(true));
                match executed {
                    Ok(result) => {
                        let read = service
                            .type2_read(Type2ReadRequest::new(CancelToken::new()))
                            .ok();
                        Ok((result, read))
                    }
                    Err(error) => Err(error),
                }
            }
            Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
        };
        let _ = sender.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(result) => finish_type2_uid(&ui, result),
            Err(_) => finish_type2_uid(&ui, Err(NfcError::Io("reader worker stopped".to_owned()))),
        }
    });
}

fn finish_type2_uid(ui: &Rc<Ui>, result: NfcResult<(Type2UidResult, Option<Type2ReadResult>)>) {
    set_busy(ui, false);
    match result {
        Ok((result, read)) => {
            clear_type2_auth(ui);
            if let Some(read) = read {
                ui.type2.replace(Some(Type2Session::new(
                    read.card,
                    read.image,
                    read.warnings,
                )));
                render_type2(ui);
                update_card(ui);
                update_ndef(ui);
            }
            ui.status.set_localized(&format!(
                "Type 2 UID changed to {}",
                result.card_after.uid_hex()
            ));
            append_log(
                ui,
                &format!(
                    "type2 uid · {} -> {} · {}",
                    result.card_before.uid_hex(),
                    result.card_after.uid_hex(),
                    result.backup.path.display()
                ),
            );
            toast_info(ui, "Type 2 UID changed and verified");
        }
        Err(error) => {
            ui.status.set_localized("Type 2 UID change failed");
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
    let auth = match type2_auth_from_ui(&ui) {
        Ok(auth) => auth,
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
        "{} · UID {}\n\nOld  {}\nNew  AUTH0 {auth0} · ACCESS {access:02X}\nPWD and PACK are replaced with the new values.\n\nPWD and PACK are write-only. A project backup is written first; the service writes PWD, PACK, ACCESS, and AUTH0, then verifies PACK with a fresh PWD_AUTH.",
        session.image.layout.product.name(),
        session.card.uid_hex(),
        old,
    );
    let dialog = confirmation_dialog(
        "Change the Type 2 password configuration?",
        &body,
        "Apply configuration",
    );
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_type2_password_dialog.replace(None);
        if response == "confirm" {
            start_type2_password(ui2.clone(), auth0, access, password, pack, auth);
        }
    });
    ui.pending_type2_password_dialog
        .replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_type2_password(
    ui: Rc<Ui>,
    auth0: u8,
    access: u8,
    password: [u8; 4],
    pack: [u8; 2],
    current: Option<Type2Auth>,
) {
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
                let mut request = Type2Password::new(auth0, access, password, pack, &backup)
                    .current(current)
                    .confirm(true);
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
            clear_type2_auth(ui);
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
    view.read_only.set_visible(false);
    view.read_only.set_active(false);
    match session.ndef() {
        Err(error) => {
            view.state.set_localized("NDEF unavailable");
            view.facts.set_localized(error);
            view.facts.add_css_class("error");
            refresh_ndef_editor(ui);
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
            match &ndef.state {
                NdefState::Unformatted => facts.push(
                    "Format the NDEF data area before writing; Format creates an empty NDEF Message TLV."
                        .to_owned(),
                ),
                NdefState::Empty => facts.push(format!(
                    "Capacity {} bytes · message empty",
                    ndef.capacity
                )),
                NdefState::Message(message) => facts.push(format!(
                    "Capacity {} bytes · message {} bytes · {} record(s)",
                    ndef.capacity,
                    ndef.message_bytes,
                    message.records().len()
                )),
            }
            facts.extend(session.warnings.iter().cloned());
            view.facts.set_localized(&facts.join("\n"));
            view.facts.remove_css_class("error");
            let key = format!(
                "type2:{}:{}",
                if ui.live.get() { "live" } else { "inspect" },
                session.card.uid_hex()
            );
            sync_ndef_editor(ui, ndef.message(), key);
        }
    }
    refresh_ndef_editor(ui);
}

fn type2_format_gate(
    session: &Type2Session,
    auth_supplied: bool,
) -> std::result::Result<(), String> {
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
    if !auth_supplied && let Some(page) = type2_ui::ndef_password_page(session) {
        return Err(format!(
            "Password protection covers the data area from page {page}; enter the access password and PACK in the Protection tab."
        ));
    }
    Ok(())
}

fn type2_ndef_gate(session: &Type2Session, auth_supplied: bool) -> std::result::Result<(), String> {
    type2_format_gate(session, auth_supplied)?;
    let ndef = session.ndef().map_err(str::to_owned)?;
    if ndef.read_only {
        return Err("The capability container marks the NDEF data area read-only.".to_owned());
    }
    if !matches!(ndef.state, NdefState::Empty | NdefState::Message(_)) {
        return Err("Not NDEF formatted · Format is required before writing.".to_owned());
    }
    Ok(())
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
    let auth = match type2_auth_from_ui(&ui) {
        Ok(auth) => auth,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    if let Err(error) = type2_ndef_gate(&session, auth.is_some()) {
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
    let (message, bytes, summary) = {
        let editor = ui.ndef_view.editor.borrow();
        let message = match editor.message() {
            Ok(message) => message,
            Err(error) => {
                toast_error(&ui, &error);
                return;
            }
        };
        let bytes = match message.encode() {
            Ok(bytes) => bytes,
            Err(error) => {
                toast_error(&ui, &error.to_string());
                return;
            }
        };
        (message, bytes, editor.summary())
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
        summary,
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
            start_type2_ndef_write(ui2.clone(), message.clone(), auth);
        }
    });
    ui.pending_ndef_write_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_type2_ndef_write(ui: Rc<Ui>, message: NdefMessage, auth: Option<Type2Auth>) {
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
                let mut request = Type2NdefWrite::new(message, &backup).auth(auth);
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
            let written = ui.ndef_view.editor.borrow().message().ok();
            if let Some(message) = written {
                ui.ndef_view.editor.borrow_mut().mark_written(&message);
            }
            let backup = result.backup.path.display().to_string();
            clear_type2_auth(ui);
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
    let auth = match type2_auth_from_ui(&ui) {
        Ok(auth) => auth,
        Err(error) => {
            toast_error(&ui, &error);
            return;
        }
    };
    if let Err(error) = type2_format_gate(&session, auth.is_some()) {
        toast_error(&ui, &error);
        return;
    }
    if !session.image.cc.is_present() {
        if !matches!(
            session.image.layout.product,
            Type2Product::UltralightEv1Ul11 | Type2Product::UltralightEv1Ul21
        ) {
            toast_error(
                &ui,
                "The tag has no capability container and no EV1 OTP format path.",
            );
            return;
        }
        let cc = [
            0xE1,
            0x10,
            (session.image.layout.user_bytes() / 8) as u8,
            0x00,
        ];
        let body = format!(
            "{} · UID {}\n\nThe tag has no capability container. Formatting writes the one-way capability container {} to OTP page 3, then writes an empty NDEF Message TLV to the {} bytes of user memory.\n\nOTP bits can only change from 0 to 1; this cannot be undone. A project backup is written first, but it cannot restore page 3.",
            session.image.layout.product.name(),
            session.card.uid_hex(),
            type2_ui::hex_bytes(&cc),
            session.image.layout.user_bytes(),
        );
        let dialog = confirmation_dialog(
            "Write the one-time EV1 capability container?",
            &body,
            "Write OTP CC and format",
        );
        let ui2 = ui.clone();
        dialog.connect_response(None, move |_, response| {
            ui2.pending_type2_format_dialog.replace(None);
            if response == "confirm" {
                start_type2_ndef_format(ui2.clone(), auth, true);
            }
        });
        ui.pending_type2_format_dialog.replace(Some(dialog.clone()));
        present_alert(&dialog, &ui.window);
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
    let dialog = confirmation_dialog("Format the NDEF data area?", &body, "Format NDEF area");
    let ui2 = ui.clone();
    dialog.connect_response(None, move |_, response| {
        ui2.pending_type2_format_dialog.replace(None);
        if response == "confirm" {
            start_type2_ndef_format(ui2.clone(), auth, false);
        }
    });
    ui.pending_type2_format_dialog.replace(Some(dialog.clone()));
    present_alert(&dialog, &ui.window);
}

fn start_type2_ndef_format(ui: Rc<Ui>, auth: Option<Type2Auth>, allow_otp_cc: bool) {
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
                let mut request = Type2Format::new(&backup)
                    .confirm(true)
                    .allow_otp_cc(allow_otp_cc)
                    .auth(auth);
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
            ui.ndef_view.editor.borrow_mut().mark_formatted();
            let backup = result.backup.path.display().to_string();
            clear_type2_auth(ui);
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
    let builtin = common_keys().len();
    ui.keys_view.status.set_localized(&format!(
        "{} stored key(s) · {builtin} built-in · {location}",
        store.len()
    ));
    ui.keys_view.save.set_sensitive(keys_path.is_some());
    let common = if ui.preferences.borrow().use_common_keys {
        format!("{builtin} built-in")
    } else {
        "built-in keys off".to_owned()
    };
    ui.key_value.set_localized(&format!(
        "{} saved · {common}\nA + B candidates",
        store.len()
    ));
    drop(keys_path);
    drop(store);
    update_key_delete(ui);
    render_details(ui);
}

fn selected_key_row(ui: &Ui) -> Option<KeyRow> {
    let index = ui.keys_view.selection.selected();
    if index == gtk::INVALID_LIST_POSITION {
        return None;
    }
    ui.keys_view
        .rows
        .item(index)
        .and_downcast::<glib::BoxedAnyObject>()
        .map(|object| object.borrow::<KeyRow>().clone())
}

fn update_key_delete(ui: &Ui) {
    let stored = selected_key_row(ui).is_some_and(|row| row.stored.is_some());
    ui.keys_view.delete.set_sensitive(stored && !ui.busy.get());
}

/// Removes one stored key entry. Built-in common keys are never stored, so
/// selecting one cannot delete it.
fn delete_key(ui: &Rc<Ui>) {
    let Some(row) = selected_key_row(ui) else {
        toast_info(ui, "Select a stored key first");
        return;
    };
    let Some(index) = row.stored else {
        toast_info(
            ui,
            "Built-in common keys are not stored; there is nothing to delete",
        );
        return;
    };
    let removed = ui.keys.borrow_mut().remove(index);
    match removed {
        Some(entry) => {
            save_keys(ui, false);
            append_log(
                ui,
                &format!(
                    "keys · deleted {} key · {}",
                    entry.kind.label(),
                    entry.sector_label()
                ),
            );
            toast_info(ui, "Deleted the selected key");
        }
        None => toast_error(ui, "The selected key entry no longer exists"),
    }
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

/// Record keys captured by a successful read or scan. Persistence failure is a
/// warning, never a reason to report the card operation as failed.
fn record_captured_keys(ui: &Rc<Ui>, card: &CardInfo, keys: &[AuthenticatedKey]) {
    if keys.is_empty() {
        return;
    }
    let result = {
        let mut store = ui.keys.borrow_mut();
        store.record_verified(card, keys).and_then(|_| {
            if let Some(path) = ui.keys_path.borrow().clone() {
                store.save(&path)
            } else {
                Ok(())
            }
        })
    };
    match result {
        Ok(()) => {
            refresh_keys(ui);
            append_log(
                ui,
                &format!(
                    "keys · captured {} verified key(s) for {}",
                    keys.len(),
                    card.identity_key()
                ),
            );
        }
        Err(error) => append_log(ui, &format!("keys · capture not saved: {error}")),
    }
}

fn record_dump_metadata_keys(ui: &Rc<Ui>, dump: &Dump) {
    let Some(card) = &dump.card else {
        return;
    };
    if dump.metadata.keys.is_empty() {
        return;
    }
    let result = {
        let mut store = ui.keys.borrow_mut();
        store
            .record_dump_keys(card, &dump.metadata.keys)
            .and_then(|_| {
                if let Some(path) = ui.keys_path.borrow().clone() {
                    store.save(&path)
                } else {
                    Ok(())
                }
            })
    };
    match result {
        Ok(()) => {
            refresh_keys(ui);
            append_log(
                ui,
                &format!(
                    "keys · loaded {} dump key(s) for {}",
                    dump.metadata.keys.len(),
                    card.identity_key()
                ),
            );
        }
        Err(error) => append_log(ui, &format!("keys · dump keys not saved: {error}")),
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
    set_dialog_folder(&dialog, DataFolder::Keys);
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
        let report = ui2.keys.borrow_mut().import_text_str(&text);
        match report {
            Ok(report) => {
                append_log(
                    &ui2,
                    &format!(
                        "keys · imported {} from {} · {} duplicate(s), {} ignored, {} issue(s)",
                        report.added,
                        path.display(),
                        report.duplicates,
                        report.ignored,
                        report.issues.len()
                    ),
                );
                for issue in &report.issues {
                    append_log(
                        &ui2,
                        &format!("keys · line {}: {}", issue.line, issue.message),
                    );
                }
                let mut summary = format!(
                    "Import: {} added · {} duplicate(s) · {} ignored · {} issue(s)",
                    report.added,
                    report.duplicates,
                    report.ignored,
                    report.issues.len()
                );
                for issue in &report.issues {
                    summary.push_str(&format!("\nLine {}: {}", issue.line, issue.message));
                }
                ui2.keys_view.report.set_localized(&summary);
                save_keys(&ui2, false);
                toast_info(
                    &ui2,
                    &format!(
                        "Imported {} key(s) · {} ignored · {} issue(s)",
                        report.added,
                        report.ignored,
                        report.issues.len()
                    ),
                );
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
    set_dialog_folder(&dialog, DataFolder::Keys);
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
    set_dialog_folder(&dialog, DataFolder::Dumps);
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
    set_dialog_folder(&dialog, DataFolder::Dumps);
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
    let ready = classic::restore_ready(
        ui.live.get(),
        ui.busy.get(),
        ui.restore_load.borrow().is_some(),
    );
    ui.restore_view.restore.set_sensitive(ready);
    ui.restore_view.uid.set_sensitive(!ui.busy.get());
    ui.restore_view.uid_advanced.set_sensitive(!ui.busy.get());
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
    let selection = match ui.restore_view.mode.selected() {
        1 => RestoreSelection::ChangesOnly,
        2 => match parse_block_list(ui.restore_view.blocks.text().as_str()) {
            Ok(blocks) => RestoreSelection::Blocks(blocks),
            Err(error) => {
                toast_error(ui, &error.to_string());
                return;
            }
        },
        _ => RestoreSelection::All,
    };
    let uid_text = ui.restore_view.uid.text();
    let target = if uid_text.trim().is_empty() {
        None
    } else {
        match parse_uid(uid_text.as_str()) {
            Ok(uid) => Some(uid),
            Err(error) => {
                toast_error(ui, &error.to_string());
                return;
            }
        }
    };
    if target.is_some() {
        if !ui.restore_view.uid_advanced.is_active() {
            toast_error(ui, "Enable Advanced mode to write a target UID");
            return;
        }
        if !matches!(selection, RestoreSelection::All) {
            toast_error(
                ui,
                "A UID target needs a full restore; remove the block selection or changes-only mode.",
            );
            return;
        }
    }
    start_preflight(
        ui,
        load.dump,
        selection,
        Some(path),
        ui.restore_view.continue_on_error.is_active(),
        target,
        ui.restore_view.uid_advanced.is_active(),
    );
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
            if let Some(uid) = &result.uid {
                for warning in &uid.warnings {
                    append_log(ui, &format!("warning · {warning}"));
                }
                match uid.state {
                    RestoreUidState::Applied => {
                        let backend = uid
                            .backend
                            .map(|backend| backend.label())
                            .unwrap_or("unknown");
                        let target = format_uid(&uid.target_uid);
                        text.push_str(&format!(
                            "\nUID changed to {target} · {backend} · block 0 and BCC verified"
                        ));
                        append_log(ui, &format!("restore uid · target {target} · {backend}"));
                    }
                    RestoreUidState::Unchanged => {
                        text.push_str(&format!(
                            "\nUID {} already matches block 0; no UID write was needed",
                            format_uid(&uid.target_uid)
                        ));
                    }
                    RestoreUidState::Failed
                    | RestoreUidState::Uncertain
                    | RestoreUidState::NotAttempted => {
                        let detail = uid.detail.as_deref().unwrap_or("no detail");
                        text.push_str(&format!("\nUID phase {}: {detail}", uid.state.label()));
                        append_log(ui, &format!("uid {} · {detail}", uid.state.label()));
                    }
                }
                if let Some(record) = &uid.backup {
                    text.push_str(&format!("\nUID backup: {}", record.backup.path.display()));
                }
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
            match result.uid.as_ref().map(|uid| uid.state) {
                Some(RestoreUidState::Applied) => {
                    toast_info(ui, "Restore and UID write complete and read back");
                    invalidate_live_image(ui);
                    let target = result
                        .uid
                        .as_ref()
                        .map(|uid| format_uid(&uid.target_uid))
                        .unwrap_or_default();
                    ui.status
                        .set_localized(&format!("UID changed to {target} · re-reading the card"));
                    start_read(ui.clone());
                    return;
                }
                Some(RestoreUidState::Uncertain) => {
                    toast_error(ui, "UID state is uncertain; read the card again");
                    invalidate_live_image(ui);
                    return;
                }
                Some(RestoreUidState::Failed) => {
                    toast_error(ui, "Dump restore finished; UID write failed");
                }
                _ => {
                    if result.failures.is_empty() {
                        toast_info(ui, "Restore complete and read back");
                    } else {
                        toast_info(ui, "Restore finished with failures; see the Restore screen");
                    }
                }
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

#[derive(Clone, Copy)]
enum DataFolder {
    Dumps,
    Keys,
    Ndef,
    Backups,
}

fn set_dialog_folder(dialog: &gtk::FileDialog, kind: DataFolder) {
    let directory = match kind {
        DataFolder::Dumps => nfc_app::paths::dumps_dir(),
        DataFolder::Keys => nfc_app::paths::keys_dir(),
        DataFolder::Ndef => nfc_app::paths::ndef_dir(),
        DataFolder::Backups => nfc_app::paths::backups_dir(),
    };
    let Some(directory) = directory else {
        return;
    };
    if let Err(error) = nfc_app::paths::ensure(&directory) {
        eprintln!("nfc-foundry: {error}");
        return;
    }
    dialog.set_initial_folder(Some(&gio::File::for_path(directory)));
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
        "Complete image. The configured default is preselected; every format can represent it. Raw, Proxmark3 JSON, and MCT do not carry sector keys."
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
    set_dialog_folder(&dialog, DataFolder::Dumps);
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
                    let saved = ui2
                        .service
                        .lock()
                        .ok()
                        .and_then(|mut service| service.workbench_mark_saved().ok());
                    if let Some(snapshot) = saved {
                        apply_workbench_snapshot(&ui2, &snapshot);
                    }
                    let mut key_count = 0;
                    if !dump.metadata.keys.is_empty() {
                        let key_path = KeyFile::path_for_dump(&path);
                        let loaded = if key_path.exists() {
                            KeyFile::load(&key_path)
                        } else {
                            Ok(KeyFile::from_keys(
                                key_path.clone(),
                                String::new(),
                                dump.card.clone(),
                                [],
                            ))
                        };
                        match loaded {
                            Ok(mut key_file) => {
                                key_file.card = dump.card.clone().or_else(|| key_file.card.clone());
                                key_file
                                    .merge_keys(dump.metadata.keys.iter().map(|entry| entry.key));
                                match key_file.save() {
                                    Ok(()) => {
                                        key_count = key_file.keys.len();
                                        append_log(
                                            &ui2,
                                            &format!(
                                                "save · {} · {key_count} key(s)",
                                                key_file.path.display()
                                            ),
                                        );
                                    }
                                    Err(error) => append_log(
                                        &ui2,
                                        &format!("save · key file failed · {error}"),
                                    ),
                                }
                            }
                            Err(error) => {
                                append_log(&ui2, &format!("save · key file not merged · {error}"))
                            }
                        }
                    }
                    let keys_omitted = chosen != DumpFormat::Project
                        && !dump.metadata.keys.is_empty()
                        && key_count == 0;
                    append_log(
                        &ui2,
                        &format!("save · {} · {} format", path.display(), chosen.name()),
                    );
                    if keys_omitted {
                        append_log(&ui2, "save · sector keys were not written to this format");
                    }
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
    set_dialog_folder(&dialog, DataFolder::Dumps);
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
    reset_workbench(ui, &loaded.dump);
    ui.dump.replace(Some(loaded.dump.clone()));
    ui.dump_source.replace(Some(format!(
        "{} · {}",
        path.display(),
        loaded.format.name()
    )));
    populate_rows(ui, &loaded.dump);
    switch_workspace(ui, Workspace::Classic);
    ui.classic_tabs.set_visible_child_name("blocks");
    invalidate_ndef_editor(ui);
    update_card(ui);
    update_ndef(ui);
    update_restore_sensitivity(ui);
    render_type2(ui);
    record_dump_metadata_keys(ui, &loaded.dump);
    match KeyFile::load_for_dump(path) {
        Ok(Some(key_file)) => {
            let mut store = ui.keys.borrow_mut();
            match key_file.merge_into(&mut store) {
                Ok(added) => append_log(
                    ui,
                    &format!(
                        "load · {} · {added} key(s) from {}",
                        path.display(),
                        key_file.path.display()
                    ),
                ),
                Err(error) => append_log(ui, &format!("load · key file failed · {error}")),
            }
        }
        Ok(None) => {}
        Err(error) => append_log(ui, &format!("load · key file ignored · {error}")),
    }
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
    view.select.set_sensitive(!busy && !running);
    view.use_defaults.set_sensitive(!busy && !running);
    view.key_types.set_sensitive(!busy && !running);
    view.dictionary.set_sensitive(!busy && !running);
    view.choose_dictionary.set_sensitive(!busy && !running);
    view.save.set_sensitive(!busy && has_result);
    view.export.set_sensitive(!busy && has_result);
}

fn update_keyscan_selection_label(ui: &Ui) {
    let count = ui.keyscan_view.selected.borrow().len();
    ui.keyscan_view
        .selection_label
        .set_localized(&format!("{count} key(s) selected"));
}

fn save_keyscan_selection(ui: &Ui) {
    let keys = ui.keyscan_view.selected.borrow().clone();
    ui.preferences.borrow_mut().keyscan_keys = Some(keys.iter().map(|key| key.to_hex()).collect());
    if let Some(path) = ui.preferences_path.as_ref()
        && let Err(error) = ui.preferences.borrow().save(path)
    {
        append_log(ui, &format!("keyscan · selection not saved · {error}"));
    }
}

fn show_keyscan_selection(ui: &Rc<Ui>) {
    let window = gtk::Window::builder()
        .title(i18n::translate_lines("Select scan keys"))
        .transient_for(&ui.window)
        .modal(true)
        .default_width(560)
        .default_height(640)
        .build();
    let root = gtk::Box::new(Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);
    let note = gtk::Label::new(Some(&i18n::translate_lines(
        "Checked keys are tried as Key A and Key B. The selection is remembered for the next scan.",
    )));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("caption");
    note.add_css_class("dim-label");

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let checks: Rc<RefCell<Vec<(Key, gtk::CheckButton)>>> = Rc::new(RefCell::new(Vec::new()));
    let selected = ui.keyscan_view.selected.borrow().clone();
    let dictionary = ui.keyscan_view.dictionary_keys.borrow().clone();
    let mut groups: Vec<(&str, Vec<Key>)> = vec![
        ("Default", default_keys()),
        ("Default extended", default_extended_keys()),
        ("EV1 signature", ev1_signature_keys()),
        ("Common", common_only_keys()),
    ];
    if !dictionary.is_empty() {
        groups.push(("Dictionary", dictionary));
    }
    let known: Vec<Key> = groups
        .iter()
        .flat_map(|(_, keys)| keys.iter().copied())
        .collect();
    let custom: Vec<Key> = selected
        .iter()
        .copied()
        .filter(|key| !known.contains(key))
        .collect();
    if !custom.is_empty() {
        groups.push(("Custom", custom));
    }
    for (name, keys) in groups {
        if keys.is_empty() {
            continue;
        }
        let header = gtk::Label::new(Some(&i18n::translate_lines(name)));
        header.set_xalign(0.0);
        header.add_css_class("heading");
        list.append(&header);
        for key in keys {
            let check = gtk::CheckButton::with_label(&key.to_hex());
            check.set_active(selected.contains(&key));
            list.append(&check);
            checks.borrow_mut().push((key, check));
        }
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();
    scroll.set_has_frame(true);

    let use_defaults = gtk::Button::with_label(&i18n::translate_lines("Use defaults"));
    let select_all = gtk::Button::with_label(&i18n::translate_lines("Select all"));
    let clear = gtk::Button::with_label(&i18n::translate_lines("Clear"));
    let apply = gtk::Button::with_label(&i18n::translate_lines("Apply"));
    apply.add_css_class("suggested-action");
    let actions = gtk::Box::new(Orientation::Horizontal, 6);
    actions.append(&use_defaults);
    actions.append(&select_all);
    actions.append(&clear);
    actions.append(&apply);
    root.append(&note);
    root.append(&scroll);
    root.append(&actions);
    window.set_child(Some(&root));

    let checks2 = checks.clone();
    use_defaults.connect_clicked(move |_| {
        let defaults = default_keys();
        for (key, check) in checks2.borrow().iter() {
            check.set_active(defaults.contains(key));
        }
    });
    let checks2 = checks.clone();
    select_all.connect_clicked(move |_| {
        for (_, check) in checks2.borrow().iter() {
            check.set_active(true);
        }
    });
    let checks2 = checks.clone();
    clear.connect_clicked(move |_| {
        for (_, check) in checks2.borrow().iter() {
            check.set_active(false);
        }
    });
    let window2 = window.clone();
    let ui2 = ui.clone();
    let checks2 = checks.clone();
    apply.connect_clicked(move |_| {
        let keys: Vec<Key> = checks2
            .borrow()
            .iter()
            .filter(|(_, check)| check.is_active())
            .map(|(key, _)| *key)
            .collect();
        *ui2.keyscan_view.selected.borrow_mut() = keys;
        update_keyscan_selection_label(&ui2);
        save_keyscan_selection(&ui2);
        window2.close();
    });
    window.present();
}

fn choose_keyscan_dictionary(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Choose a Classic key dictionary"))
        .build();
    set_dialog_folder(&dialog, DataFolder::Keys);
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
        match dictionary_candidates(&ui2) {
            Ok(keys) => {
                ui2.keyscan_view.dictionary_keys.replace(keys.clone());
                {
                    let mut selected = ui2.keyscan_view.selected.borrow_mut();
                    for key in &keys {
                        if !selected.contains(key) {
                            selected.push(*key);
                        }
                    }
                }
                update_keyscan_selection_label(&ui2);
                save_keyscan_selection(&ui2);
                append_log(
                    &ui2,
                    &format!(
                        "keyscan · dictionary {} · {} key(s)",
                        path.display(),
                        keys.len()
                    ),
                );
            }
            Err(error) => toast_error(&ui2, &error),
        }
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
    let report = store
        .import_text_str(&text)
        .map_err(|error| error.to_string())?;
    if !report.issues.is_empty() {
        append_log(
            ui,
            &format!(
                "keyscan dictionary · {} issue(s), {} ignored",
                report.issues.len(),
                report.ignored
            ),
        );
    }
    if store.is_empty() {
        return Err(format!(
            "The dictionary {path} contains no keys ({} issue(s))",
            report.issues.len()
        ));
    }
    Ok(store.all_keys())
}

fn start_keyscan(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    let candidates = ui.keyscan_view.selected.borrow().clone();
    if candidates.is_empty() {
        toast_error(&ui, "Select at least one key");
        return;
    }
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
                    .include_common_keys(false)
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
            record_captured_keys(ui, &result.card, &result.verified_keys);
            let state = if result.cancelled {
                "Scan cancelled; confirmed keys kept."
            } else {
                "Scan complete."
            };
            ui.keyscan_view.summary.set_localized(&format!(
                "{} sector(s) with a key · {} attempts · {} verified trailer reads",
                found, result.attempts, result.verified
            ));
            ui.keyscan_view.progress.set_localized(state);
            ui.status.set_localized(&format!(
                "{} · {found} sector(s)",
                if result.cancelled {
                    "Key scan cancelled"
                } else {
                    "Key scan complete"
                }
            ));
            append_log(
                ui,
                &format!(
                    "keyscan · {}{} sector(s) · {} attempts · {} verified",
                    if result.cancelled {
                        "cancelled · "
                    } else {
                        ""
                    },
                    found,
                    result.attempts,
                    result.verified
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
    export_keyscan_dic(ui);
}

fn export_keyscan_dic(ui: &Rc<Ui>) {
    let initial = ui
        .keyscan_result
        .borrow()
        .as_ref()
        .map(|result| format!("{}.dic", result.card.uid_hex().replace(' ', "")))
        .unwrap_or_else(|| "scanned-keys.dic".to_owned());
    let dialog = gtk::FileDialog::builder()
        .title(i18n::translate_lines("Export scanned keys"))
        .initial_name(&initial)
        .build();
    set_dialog_folder(&dialog, DataFolder::Keys);
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
    let key_file = KeyFile::from_authenticated(
        path.to_owned(),
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("scanned-keys"),
        Some(result.card.clone()),
        &result.verified_keys,
    );
    if key_file.keys.is_empty() {
        return Err("No verified keys to save".to_owned());
    }
    key_file.save().map_err(|error| error.to_string())?;
    let count = key_file.keys.len();
    append_log(
        ui,
        &format!("keyscan · saved {count} key(s) to {}", path.display()),
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
    let target = if strategy == RecoveryStrategy::Auto {
        "every sector".to_owned()
    } else {
        format!(
            "block {} key {}",
            view.target_block.text(),
            key_type_from(view.target_type.selected()).label()
        )
    };
    let body = format!(
        "Strategy: {}\nTarget: {target}\nReader: {}\n\nRecovery authenticates and reads; it \
         never writes the card. The selected reader stays busy until the attack stops or is \
         cancelled.",
        recovery_strategy_label(strategy),
        ui.reader_snapshot
            .borrow()
            .selected
            .clone()
            .unwrap_or_else(|| "no reader selected".to_owned())
    );
    let dialog = authorization_dialog("Start MIFARE Classic recovery?", &body, "Start recovery");
    let ui2 = ui.clone();
    ui.pending_recovery_dialog.replace(Some(dialog.clone()));
    dialog.connect_response(None, move |_, response| {
        ui2.pending_recovery_dialog.borrow_mut().take();
        if response == "confirm" {
            begin_recovery(ui2.clone());
        }
    });
    present_alert(&dialog, &ui.window);
}

fn begin_recovery(ui: Rc<Ui>) {
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

fn recovery_captured_keys(result: &RecoveryResult, target_block: u8) -> Vec<AuthenticatedKey> {
    if !result.sector_keys.is_empty() {
        return result
            .sector_keys
            .iter()
            .map(|entry| AuthenticatedKey {
                sector: entry.sector,
                key_type: entry.key_type,
                key: entry.key,
                source: KeySource::Recovery,
            })
            .collect();
    }
    let Some(card) = &result.card else {
        return Vec::new();
    };
    let layout = match card.card_type {
        nfc_core::CardType::MifareClassic1K => nfc_classic::Layout::Classic1K,
        nfc_core::CardType::MifareClassic4K => nfc_classic::Layout::Classic4K,
        _ => return Vec::new(),
    };
    let Ok(sector) = layout.sector_for_block(usize::from(target_block)) else {
        return Vec::new();
    };
    result
        .keys
        .iter()
        .map(|(key_type, key)| AuthenticatedKey {
            sector,
            key_type: *key_type,
            key: *key,
            source: KeySource::Recovery,
        })
        .collect()
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
            let captured = recovery_captured_keys(&result, target_block);
            if let Some(card) = &result.card {
                record_captured_keys(ui, card, &captured);
            }
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

fn supported_drivers() -> Vec<DriverInfo> {
    vec![
        DriverInfo::new("PN532 UART", nfc_drivers::PN532_UART_PREFIX),
        DriverInfo::new("ACR122U USB", nfc_drivers::ACR122_USB_PREFIX),
    ]
}

fn render_diagnostics(ui: &Ui) {
    let view = &ui.diagnostics_view;
    let devices = ui
        .devices
        .borrow()
        .iter()
        .map(|device| ReaderDevice {
            name: device.name.clone(),
            conn_string: device.conn_string.clone(),
        })
        .collect();
    let snapshot = ui.reader_snapshot.borrow().clone();
    let tables_dir = ui.preferences.borrow().hardnested_tables.clone();
    let report = SelfCheck::new(
        supported_drivers(),
        Ok(devices),
        &snapshot,
        tables_dir.as_deref(),
    );
    view.version.set_localized(&format!(
        "NFC Foundry {} · commit {}\nBuild date: {}",
        report.app.version, report.app.commit, report.app.build_date
    ));
    view.platform.set_localized(&format!(
        "Platform: {} · {}",
        report.platform.os, report.platform.arch
    ));
    view.drivers.set_localized(&format!(
        "Supported readers: {}",
        report
            .drivers
            .iter()
            .map(|driver| format!("{}<{}>", driver.name, driver.conn_string_prefix))
            .collect::<Vec<_>>()
            .join(" · ")
    ));
    match &report.readers.firmware {
        Some(firmware) => view
            .firmware
            .set_localized(&format!("Firmware: {firmware}")),
        None => view.firmware.set_localized("Firmware: reader not opened"),
    }
    let capabilities = capability_text(snapshot.capabilities);
    view.capabilities
        .set_localized(&format!("Reader capabilities: {capabilities}"));
    let tables = match (
        &report.hardnested_tables.path,
        &report.hardnested_tables.error,
    ) {
        (Some(path), _) => format!("Hardnested tables: {path}"),
        (None, Some(error)) => format!("Hardnested tables: not resolved ({error})"),
        (None, None) => "Hardnested tables: not resolved".to_owned(),
    };
    view.tables.set_localized(&tables);
    let readers = if !report.readers.available {
        format!(
            "Readers: unavailable ({})",
            report
                .readers
                .error
                .as_deref()
                .unwrap_or("enumeration failed")
        )
    } else if report.readers.devices.is_empty() {
        "Readers: none found".to_owned()
    } else {
        let mut text = format!("Readers: {}", report.readers.devices.len());
        for device in &report.readers.devices {
            text.push_str(&format!("\n{}\t{}", device.name, device.conn_string));
        }
        if let Some(selected) = &report.readers.selected {
            text.push_str(&format!(
                "\nSelected: {selected} · {}",
                report.readers.status
            ));
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
        report.platform.os,
        report.platform.arch
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
    set_dialog_folder(&dialog, DataFolder::Keys);
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
    if matches!(kind, SettingsPath::Backup) {
        set_dialog_folder(&dialog, DataFolder::Backups);
    }
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
        keyscan_keys: ui.preferences.borrow().keyscan_keys.clone(),
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
    let keyscan_selected = match &preferences.keyscan_keys {
        Some(values) => values
            .iter()
            .filter_map(|hex| Key::from_hex(hex).ok())
            .collect(),
        None => default_keys(),
    };
    *ui.keyscan_view.selected.borrow_mut() = keyscan_selected;
    update_keyscan_selection_label(ui);
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
    update_desfire_actions(ui);
}

fn toast_info(ui: &Rc<Ui>, message: &str) {
    let toast = adw::Toast::new(&i18n::translate_lines(message));
    #[cfg(test)]
    ui.toasts.borrow_mut().push(toast.clone());
    ui.toast.add_toast(toast);
}

fn toast_error(ui: &Rc<Ui>, message: &str) {
    append_log(ui, &format!("error · {message}"));
    let toast = adw::Toast::new(&i18n::translate_lines(message));
    #[cfg(test)]
    ui.toasts.borrow_mut().push(toast.clone());
    ui.toast.add_toast(toast);
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

    use nfc_app::ReaderMonitorConfig;
    use nfc_classic::{Layout, bcc_for_uid};
    use nfc_classic_ndef::{
        MAD_ACCESS_BITS, MAD_KEY_A, MAD1_GPB, Mad, MadVersion, NFC_GPB_RO, NFC_GPB_RW, NFC_KEY_A,
        NFC_RO_ACCESS_BITS, NFC_RW_ACCESS_BITS, format_ndef_area,
    };
    use nfc_core::{CardInfo, CardType};
    use nfc_ndef::{Tnf, UriRecord};
    use nfc_transport::mock::{MockHandle, MockReader, MockTransport, MockType4Tag};

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

    fn protected_ntag_pages() -> Vec<[u8; 4]> {
        let mut pages = ntag_pages(false, false, true);
        pages[41] = [0x00, 0x00, 0x00, 0x20];
        pages[42] = [0x80, 0x00, 0x00, 0x00];
        pages
    }

    fn ev1_pages() -> Vec<[u8; 4]> {
        let mut pages = vec![[0u8; 4]; 20];
        pages[0] = [0x04, 0x43, 0x47, 0x88];
        pages[1] = [0x4A, 0x75, 0x22, 0x91];
        pages[2] = [0x8C, 0x48, 0x00, 0x00];
        pages[16] = [0x00, 0x00, 0x00, 0xFF];
        pages[17] = [0x00, 0x05, 0x00, 0x00];
        pages
    }

    fn magic_ntag213_pages() -> Vec<[u8; 4]> {
        let mut pages = vec![[0u8; 4]; 45];
        pages[0] = [0x04, 0x00, 0xE3, 0x6F];
        pages[1] = [0xC2, 0x00, 0x00, 0x02];
        pages[2] = [0xC0, 0x08, 0x00, 0x00];
        pages[3] = [0xE1, 0x10, 0x12, 0x00];
        pages[4] = [0x03, 0x00, 0xFE, 0x00];
        pages[40] = [0x00, 0x00, 0x00, 0xBD];
        pages[41] = [0x00, 0x00, 0x00, 0xFF];
        pages
    }

    fn magic_card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x00, 0xE3, 0xC2, 0x00, 0x00, 0x02],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )
    }

    fn install_ntag(handle: &MockHandle, pages: &[[u8; 4]]) {
        handle.set_card(Some(type2_card(0x80)));
        handle.set_type2_version(None);
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

    fn alert_pending(slot: &RefCell<Option<adw::AlertDialog>>) -> adw::AlertDialog {
        slot.borrow().clone().expect("alert dialog")
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

    /// A raw fixture carries the trailer Key A bytes, but a card read masks
    /// them. Restore sources use the read representation so unchanged trailers
    /// are not rewritten.
    fn as_read_dump(mut dump: Dump) -> Dump {
        for block in dump.blocks.iter_mut() {
            if block.number % 4 == 3 {
                block.data[..6].fill(0);
            }
        }
        dump
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
        invalidate_ndef_editor(ui);
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
                auto_monitor: false,
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

    fn reset(harness: &Harness) {
        harness.handle.set_card(Some(test_card()));
        for (block, data) in image().into_iter().enumerate() {
            harness.handle.set_block(block as u8, data);
        }
        install_ntag(&harness.ntag, &ntag_pages(false, false, true));
        {
            let mut service = harness.ui.service.lock().expect("service");
            service
                .select_device("mock:classic")
                .expect("select classic");
            service.retry_reader().expect("restart reader monitor");
        }
        harness.ui.reader_combo.set_selected(0);
        let ui = &harness.ui;
        #[cfg(test)]
        for toast in ui.toasts.borrow_mut().drain(..) {
            toast.dismiss();
        }
        ui.live.set(false);
        ui.busy.set(false);
        ui.dump.replace(None);
        ui.dump_source.replace(None);
        if let Ok(mut service) = ui.service.lock() {
            service.workbench_clear();
        }
        ui.dirty_blocks.replace(Vec::new());
        ui.undo_depth.set(0);
        ui.pending_plan.replace(None);
        close_alert(&ui.pending_uid_dialog);
        ui.pending_uid_plan.replace(None);
        ui.inspector.uid_advanced.set_active(false);
        ui.inspector.uid.set_text("");
        ui.type2.replace(None);
        ui.type2_restore.replace(None);
        close_alert(&ui.pending_type2_page_dialog);
        close_alert(&ui.pending_type2_advanced_dialog);
        close_alert(&ui.pending_type2_lock_dialog);
        close_alert(&ui.pending_type2_password_dialog);
        close_alert(&ui.pending_type2_uid_dialog);
        ui.pending_type2_uid_plan.replace(None);
        close_alert(&ui.pending_type2_restore_dialog);
        close_alert(&ui.pending_type2_format_dialog);
        ui.type2_view.advanced.set_active(false);
        ui.type2_view.rows.remove_all();
        ui.type2_view.table_stack.set_visible_child_name("empty");
        ui.type2_view.lock_pages.set_text("");
        ui.type2_view.auth0.set_text("");
        ui.type2_view.access.set_text("");
        ui.type2_view.auth_pwd.set_text("");
        ui.type2_view.auth_pack.set_text("");
        ui.type2_view.pwd.set_text("");
        ui.type2_view.pack.set_text("");
        ui.type2_view.include_memory.set_active(true);
        ui.type2_view.include_configuration.set_active(false);
        ui.type2_view.include_cc.set_active(false);
        ui.type2_view.include_locks.set_active(false);
        render_type2(ui);
        ui.restore_load.replace(None);
        ui.restore_view.uid.set_text("");
        ui.restore_view.uid_advanced.set_active(false);
        ui.diff_before.replace(None);
        ui.diff_after.replace(None);
        close_alert(&ui.pending_plan_dialog);
        close_alert(&ui.pending_ndef_write_dialog);
        close_alert(&ui.pending_ndef_format_dialog);
        ui.ndef.replace(None);
        ui.ndef_view.editor.borrow_mut().load_message(None);
        ui.ndef_view.editor_key.replace(None);
        ui.ndef_view.force_editor_reset.set(false);
        populate_ndef_rows(ui);
        update_ndef_inspector(ui);
        let record_dialog = ui.ndef_view.pending_record_dialog.borrow_mut().take();
        if let Some(window) = record_dialog {
            window.close();
        }
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
        *ui.keyscan_view.selected.borrow_mut() = default_keys();
        update_keyscan_selection_label(ui);
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
        reset_workbench(ui, &dump);
        ui.dump.replace(Some(dump.clone()));
        populate_rows(ui, &dump);
        ui.live.set(true);
        update_inspector(ui);
        update_card(ui);
        update_restore_sensitivity(ui);
    }

    fn changed_block() -> [u8; 16] {
        [0xAA; 16]
    }

    fn widget_contains_text(widget: &impl IsA<gtk::Widget>, text: &str) -> bool {
        let widget = widget.as_ref();
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.label() == text
        {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if widget_contains_text(&widget, text) {
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }

    fn find_button(widget: &impl IsA<gtk::Widget>, label: &str) -> Option<gtk::Button> {
        let widget = widget.as_ref();
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && (button.label().as_deref() == Some(label) || widget_contains_text(button, label))
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(found) = find_button(&widget, label) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
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
    fn gtk_workbench_scenarios() {
        if std::env::var_os("NFC_FOUNDRY_GUI_TESTS").is_none() {
            eprintln!("skipping GTK UI tests; set NFC_FOUNDRY_GUI_TESTS=1 under Xvfb");
            return;
        }
        let harness = harness();
        selection_updates_inspector(&harness);
        write_gates_follow_state(&harness);
        reader_refresh_invalidates(&harness);
        reader_monitor_invalidates(&harness);
        workbench_stage_undo_revert_and_write(&harness);
        identity_refusal_is_visible(&harness);
        restore_cancel_then_confirm(&harness);
        restore_with_uid(&harness);
        diff_screen_classifies_blocks(&harness);
        keys_screen_adds_and_saves(&harness);
        details_view_matches_the_dump_and_key_model(&harness);
        read_captures_keys(&harness);
        identity_only_read_stays_inspect(&harness);
        unsupported_card_reports_identity(&harness);
        classic_uid_write(&harness);
        keyscan_finds_and_saves(&harness);
        recovery_limitation_and_saved_key(&harness);
        diagnostics_and_activity(&harness);
        settings_persist_and_reject(&harness);
        ndef_composer_sources(&harness);
        ndef_record_dialog_forms(&harness);
        ndef_write_confirmation_cancel_and_success(&harness);
        ndef_write_gates(&harness);
        ndef_format_cancel_and_success(&harness);
        type2_read_pages_and_advanced(&harness);
        type2_user_page_write(&harness);
        type2_advanced_page_write(&harness);
        type2_lock_and_password(&harness);
        type2_protected_access(&harness);
        type2_ndef_write_and_format(&harness);
        type2_ev1_otp_format_dialog(&harness);
        type2_uid_change_and_restore(&harness);
        type2_partial_ndef(&harness);
        type2_dump_and_restore(&harness);
        navigation_busy_and_focus(&harness);
        language_switch_preserves_state(&harness);
        start_reader_monitor(harness.ui.clone());
        pump_until(&harness.ui, "background reader monitor", || {
            harness.ui.reader_state.label() == "Polling · card present"
        });
        harness.ui.window.close();
        drop(harness.app);
    }

    fn navigation_busy_and_focus(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        switch_workspace(ui, Workspace::Classic);
        for page in ["blocks", "ndef", "keys", "recovery", "dumps"] {
            ui.classic_tabs.set_visible_child_name(page);
            assert_eq!(ui.classic_tabs.visible_child_name().as_deref(), Some(page));
        }
        ui.classic_tabs.set_visible_child_name("blocks");
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
        let diagnostics_item = find_button(
            &ui.menu.popover().expect("main menu popover"),
            "Diagnostics",
        )
        .expect("diagnostics menu item");
        diagnostics_item.emit_clicked();
        for _ in 0..5 {
            while context.iteration(false) {}
        }
        assert_eq!(ui.views.visible_child_name().as_deref(), Some("system"));
        assert_eq!(
            ui.system_tabs.visible_child_name().as_deref(),
            Some("diagnostics")
        );
        assert!(ui.back.is_visible());
        assert!(
            !ui.diagnostics_view.version.has_focus(),
            "diagnostics focused a selectable label"
        );
        assert!(
            ui.diagnostics_view.version.selection_bounds().is_none(),
            "diagnostics left its version text selected"
        );
        assert!(
            !gtk::prelude::GtkWindowExt::focus(&ui.window)
                .is_some_and(|widget| widget.is::<gtk::Label>()),
            "a label holds focus after the page switch"
        );
        ui.back.emit_clicked();
        for _ in 0..5 {
            while context.iteration(false) {}
        }
        assert_eq!(ui.views.visible_child_name().as_deref(), Some("classic"));
        assert!(!ui.back.is_visible());

        switch_workspace(ui, Workspace::Type2);
        let protection_title = i18n::translate_lines("Protection");
        let protection_button = find_button(&ui.type2_view.switcher, "Protection")
            .or_else(|| find_button(&ui.type2_view.switcher, &protection_title))
            .expect("protection tab button");
        protection_button.emit_clicked();
        for _ in 0..5 {
            while context.iteration(false) {}
        }
        assert_eq!(
            ui.type2_view.tabs.visible_child_name().as_deref(),
            Some("protection")
        );
        assert!(
            !ui.type2_view.configuration.has_focus(),
            "protection focused the configuration label"
        );
        assert!(
            ui.type2_view.configuration.selection_bounds().is_none(),
            "protection left the configuration text selected"
        );
        assert!(
            ui.type2_view.summary.selection_bounds().is_none(),
            "type 2 summary text is selected"
        );
        assert!(
            !gtk::prelude::GtkWindowExt::focus(&ui.window)
                .is_some_and(|widget| widget.is::<gtk::Label>()),
            "a label holds focus after the tab switch"
        );
        assert!(ui.type2_view.ndef_slot.first_child().is_some());
        let classic_item = find_button(
            &ui.workspace_button
                .popover()
                .expect("workspace menu popover"),
            "MIFARE Classic",
        )
        .expect("MIFARE Classic workspace item");
        classic_item.emit_clicked();
        for _ in 0..5 {
            while context.iteration(false) {}
        }
        assert_eq!(ui.workspace.get(), Workspace::Classic);
        assert_eq!(ui.views.visible_child_name().as_deref(), Some("classic"));
        assert!(ui.classic_ndef_slot.first_child().is_some());
        assert!(ui.type2_view.ndef_slot.first_child().is_none());
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
            || matches!(
                text,
                "keys"
                    | "keyscan"
                    | "read"
                    | "write"
                    | "save"
                    | "load"
                    | "recovery"
                    | "diff"
                    | "restore"
                    | "warning"
                    | "error"
            )
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
        switch_workspace(&harness.ui, Workspace::Classic);
        harness.ui.classic_tabs.set_visible_child_name("blocks");
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
                .classic_tabs
                .pages()
                .item(0)
                .and_downcast::<gtk::StackPage>()
                .and_then(|page| page.title())
                .is_some_and(|title| title == "数据块")
        );
        assert_eq!(
            harness.ui.views.visible_child_name().as_deref(),
            Some("classic")
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
        assert_eq!(
            harness.ui.inspector.write.label().as_deref(),
            Some("暂存编辑")
        );
        assert_eq!(
            harness.ui.inspector.commit.label().as_deref(),
            Some("写入更改…")
        );
        stage_edit(harness.ui.clone());
        assert!(harness.ui.inspector.commit.is_sensitive());
        assert_eq!(harness.ui.dirty_blocks.borrow().as_slice(), &[4]);
        assert_eq!(harness.handle.block(4), Some(image()[4]));

        confirm_workbench_write(harness.ui.clone());
        pump_until(&harness.ui, "Chinese write plan", || {
            harness.ui.pending_plan_dialog.borrow().is_some()
        });
        let dialog = harness
            .ui
            .pending_plan_dialog
            .borrow()
            .clone()
            .expect("plan dialog");
        assert_eq!(dialog.heading().as_deref(), Some("写入已编辑的数据块？"));
        assert_eq!(dialog.response_label("write"), "写入更改");
        assert!(dialog.body().contains("数据块 004"), "{}", dialog.body());
        assert!(dialog.body().contains("使用"), "{}", dialog.body());
        emit_response(&dialog, "cancel");
        assert!(harness.ui.pending_plan_dialog.borrow().is_none());

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
        assert!(!harness.ui.inspector.commit.is_sensitive());
        harness.ui.live.set(false);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        assert!(
            harness.ui.inspector.write.is_sensitive(),
            "a loaded image is editable without a live card"
        );
        assert!(!harness.ui.inspector.commit.is_sensitive());

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
        stage_edit(harness.ui.clone());
        assert!(harness.ui.inspector.commit.is_sensitive());
        refresh_devices(&harness.ui);
        assert!(!harness.ui.live.get());
        assert!(!harness.ui.inspector.commit.is_sensitive());
        assert!(harness.ui.status.label().contains("Inspection only"));
    }

    fn reader_monitor_invalidates(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        {
            let mut service = ui.service.lock().expect("service");
            service.set_reader_monitor_config(ReaderMonitorConfig {
                poll_interval: Duration::ZERO,
                ..ReaderMonitorConfig::default()
            });
            service.retry_reader().expect("restart monitor");
        }
        poll_reader_once(ui);
        assert_eq!(ui.reader_state.label(), "Polling · card present");
        show_dump(ui);
        assert!(ui.live.get());
        assert!(ui.reader_caps.label().contains("MIFARE Classic read/write"));
        assert!(ui.reader_caps.label().contains("UID write"));

        harness.handle.set_card(None);
        poll_reader_once(ui);
        assert!(ui.live.get(), "one missed poll keeps the image");
        poll_reader_once(ui);
        assert!(!ui.live.get());
        assert_eq!(ui.reader_state.label(), "Polling · no card");
        assert_eq!(ui.status.label(), "Card removed · read the card again");
        assert!(!ui.inspector.commit.is_sensitive());

        harness.handle.set_card(Some(test_card()));
        poll_reader_once(ui);
        assert_eq!(ui.reader_state.label(), "Polling · card present");
        assert!(!ui.live.get(), "the old image stays inspection-only");
    }

    fn workbench_stage_undo_revert_and_write(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        harness.ui.selection.set_selected(4);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        stage_edit(harness.ui.clone());
        assert_eq!(harness.handle.block(4), Some(image()[4]));
        assert_eq!(harness.ui.dirty_blocks.borrow().as_slice(), &[4]);
        assert!(harness.ui.inspector.commit.is_sensitive());
        let row = harness
            .ui
            .rows
            .item(4)
            .and_downcast::<glib::BoxedAnyObject>()
            .expect("row")
            .borrow::<BlockRow>()
            .clone();
        assert_eq!(row.display_status(), "edited");

        undo_edit(harness.ui.clone());
        assert!(harness.ui.dirty_blocks.borrow().is_empty());
        assert_eq!(harness.ui.dirty_blocks.borrow().len(), 0);
        stage_edit(harness.ui.clone());
        revert_edits(harness.ui.clone());
        assert!(harness.ui.dirty_blocks.borrow().is_empty());
        assert_eq!(
            harness.ui.dump.borrow().as_ref().expect("dump").blocks[4].data,
            image()[4]
        );

        harness.ui.selection.set_selected(4);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        stage_edit(harness.ui.clone());
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
        stage_edit(harness.ui.clone());
        assert_eq!(harness.ui.dirty_blocks.borrow().as_slice(), &[4, 7]);

        confirm_workbench_write(harness.ui.clone());
        pump_until(&harness.ui, "write plan", || {
            harness.ui.pending_plan_dialog.borrow().is_some()
        });
        let dialog = harness
            .ui
            .pending_plan_dialog
            .borrow()
            .clone()
            .expect("plan dialog");
        assert!(
            dialog
                .heading()
                .as_deref()
                .is_some_and(|heading| heading.contains("edited blocks"))
        );
        assert!(dialog.body().contains("Block 004"));
        emit_response(&dialog, "cancel");
        assert!(harness.ui.pending_plan_dialog.borrow().is_none());
        assert_eq!(harness.handle.block(4), Some(image()[4]));

        confirm_workbench_write(harness.ui.clone());
        pump_until(&harness.ui, "write plan", || {
            harness.ui.pending_plan_dialog.borrow().is_some()
        });
        let dialog = harness
            .ui
            .pending_plan_dialog
            .borrow()
            .clone()
            .expect("plan dialog");
        emit_response(&dialog, "write");
        pump_until(&harness.ui, "planned write", || {
            !harness.ui.busy.get() && harness.ui.status.label().contains("Changes written")
        });
        assert_eq!(harness.handle.block(4), Some(changed_block()));
        assert_eq!(harness.handle.block(7), Some(trailer));
        assert!(harness.ui.dirty_blocks.borrow().is_empty());
        assert_eq!(
            std::fs::read_dir(harness.backups.path())
                .expect("backup dir")
                .count(),
            1
        );
    }

    fn identity_refusal_is_visible(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        harness.ui.selection.set_selected(4);
        harness
            .ui
            .inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(&harness.ui);
        stage_edit(harness.ui.clone());
        confirm_workbench_write(harness.ui.clone());
        pump_until(&harness.ui, "write plan", || {
            harness.ui.pending_plan_dialog.borrow().is_some()
        });
        harness.handle.set_card(Some(other_card()));
        let dialog = harness
            .ui
            .pending_plan_dialog
            .borrow()
            .clone()
            .expect("plan dialog");
        emit_response(&dialog, "write");
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
        let mut source = as_read_dump(fixture_dump());
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
        pump_until(&harness.ui, "restore plan", || {
            harness.ui.pending_plan_dialog.borrow().is_some()
        });
        let dialog = harness
            .ui
            .pending_plan_dialog
            .borrow()
            .clone()
            .expect("restore dialog");
        emit_response(&dialog, "cancel");
        assert!(harness.ui.pending_plan_dialog.borrow().is_none());
        assert_eq!(harness.handle.block(4), Some(image()[4]));

        confirm_restore(&harness.ui);
        pump_until(&harness.ui, "restore plan", || {
            harness.ui.pending_plan_dialog.borrow().is_some()
        });
        let dialog = harness
            .ui
            .pending_plan_dialog
            .borrow()
            .clone()
            .expect("restore dialog");
        emit_response(&dialog, "write");
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

    fn restore_with_uid(harness: &Harness) {
        reset(harness);
        show_dump(&harness.ui);
        let ui = &harness.ui;
        let mut source = as_read_dump(fixture_dump());
        source.blocks[4].set_read([0xCC; 16]);
        ui.restore_load.replace(Some((
            PathBuf::from("test-restore-uid.bin"),
            DumpLoad {
                format: DumpFormat::Raw,
                dump: source,
                warnings: Vec::new(),
            },
        )));
        update_restore_sensitivity(ui);
        ui.restore_view.mode.set_selected(0);
        ui.restore_view.uid.set_text("DEADBEEF");

        ui.restore_view.restore.emit_clicked();
        assert!(ui.pending_plan_dialog.borrow().is_none());
        assert_eq!(harness.handle.block(4), Some(image()[4]));

        ui.restore_view.uid_advanced.set_active(true);
        ui.restore_view.restore.emit_clicked();
        pump_until(ui, "restore UID plan", || {
            ui.pending_plan_dialog.borrow().is_some()
        });
        let dialog = alert_pending(&ui.pending_plan_dialog);
        assert!(
            dialog
                .heading()
                .as_deref()
                .is_some_and(|heading| heading.contains("target UID"))
        );
        assert!(dialog.body().contains("UID target DEADBEEF"));
        assert!(dialog.body().contains("restore and sector 0 are verified"));
        emit_response(&dialog, "write");
        pump_until(ui, "restore UID write and re-read", || {
            !ui.busy.get() && ui.status.label().contains("Read complete")
        });
        assert_eq!(harness.handle.block(4), Some([0xCC; 16]));
        assert_eq!(
            harness.handle.block(0).expect("block 0")[0..4],
            [0xDE, 0xAD, 0xBE, 0xEF]
        );
        assert!(
            activity_text(ui).contains("restore uid · target DEADBEEF"),
            "{}",
            activity_text(ui)
        );
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
        let rows = harness.ui.keys_view.rows.n_items();
        assert!(
            rows >= common_keys().len() as u32,
            "built-in keys are not listed: {rows}"
        );
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

        harness.ui.keys_view.selection.set_selected(0);
        update_key_delete(&harness.ui);
        assert!(harness.ui.keys_view.delete.is_sensitive());
        delete_key(&harness.ui);
        assert!(harness.ui.keys.borrow().is_empty());
        assert!(KeyStore::load(&path).expect("store").entries().is_empty());

        let builtin = (0..harness.ui.keys_view.rows.n_items())
            .find(|index| {
                harness
                    .ui
                    .keys_view
                    .rows
                    .item(*index)
                    .and_downcast::<glib::BoxedAnyObject>()
                    .expect("key row")
                    .borrow::<KeyRow>()
                    .stored
                    .is_none()
            })
            .expect("built-in row");
        harness.ui.keys_view.selection.set_selected(builtin);
        update_key_delete(&harness.ui);
        assert!(!harness.ui.keys_view.delete.is_sensitive());
        delete_key(&harness.ui);
        assert!(harness.ui.keys.borrow().is_empty());
    }

    fn details_view_matches_the_dump_and_key_model(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        show_dump(ui);
        let body = ui.details_view.body.label().to_string();
        assert!(body.contains("Card: MIFARE Classic 1K"), "{body}");
        assert!(body.contains("UID 04 A2 24 8A (4 bytes)"), "{body}");
        assert!(body.contains("ATQA 0004 · SAK 08"), "{body}");
        assert!(body.contains("Layout: MIFARE Classic 1K"), "{body}");
        assert!(body.contains("Blocks: 64/64 readable"), "{body}");
        assert!(body.contains("Dirty: none"), "{body}");
        assert!(body.contains("Validation: ok"), "{body}");
        assert!(body.contains("Verified: 0/16 sectors"), "{body}");

        ui.selection.set_selected(4);
        ui.inspector
            .hex
            .set_text(&classic::hex_with_mask(&changed_block(), u16::MAX));
        update_write_sensitivity(ui);
        stage_edit(ui.clone());
        let body = ui.details_view.body.label().to_string();
        assert!(body.contains("Dirty: 1 block(s) · 004"), "{body}");

        let key = Key::from_hex("A0A1A2A3A4A5").expect("key");
        record_captured_keys(
            ui,
            &test_card(),
            &[AuthenticatedKey {
                sector: 0,
                key_type: nfc_core::KeyType::A,
                key,
                source: KeySource::ReadContext,
            }],
        );
        let body = ui.details_view.body.label().to_string();
        assert!(
            body.contains("Verified: 1/16 sectors · A 1 · B 0 · both 0"),
            "{body}"
        );
    }

    fn read_captures_keys(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        let before = ui.keys.borrow().verified_count(&test_card());
        choose_reader(harness, "mock:classic");
        start_read(ui.clone());
        pump_until(ui, "classic read", || {
            !ui.busy.get() && ui.status.label().contains("Read complete")
        });
        let after = ui.keys.borrow().verified_count(&test_card());
        assert!(after > before, "verified keys were not captured");
        let path = harness.keys_dir.path().join("classic-keys.json");
        let loaded = KeyStore::load(&path).expect("persisted key store");
        assert!(loaded.verified_count(&test_card()) >= after);
        assert!(activity_text(ui).contains("captured"));
    }

    fn identity_only_read_stays_inspect(harness: &Harness) {
        reset(harness);
        harness.handle.set_mifare_unsupported(true);
        let ui = &harness.ui;
        choose_reader(harness, "mock:classic");
        start_read(ui.clone());
        pump_until(ui, "identity-only read", || {
            !ui.busy.get() && ui.status.label().contains("Card identified")
        });
        assert!(!ui.live.get(), "identity-only images are not live");
        let dump = ui.dump.borrow().clone().expect("identity-only dump");
        assert_eq!(dump.card.as_ref(), Some(&test_card()));
        assert_eq!(dump.readability().readable_blocks, 0);
        assert!(
            ui.status.label().contains("04 A2 24 8A"),
            "{}",
            ui.status.label()
        );
        assert!(!ui.inspector.commit.is_sensitive());
        assert!(activity_text(ui).contains("identity only"));
        harness.handle.set_mifare_unsupported(false);
    }

    fn unsupported_card_reports_identity(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        choose_reader(harness, "mock:unknown");
        start_read(ui.clone());
        pump_until(ui, "identity-only unsupported read", || {
            !ui.busy.get() && ui.status.label().contains("no supported read path")
        });
        assert!(!ui.live.get());
        assert!(ui.dump.borrow().is_none());
        assert!(
            ui.status.label().contains("08 11 22 33"),
            "{}",
            ui.status.label()
        );
        assert!(activity_text(ui).contains("identity only"));
        choose_reader(harness, "mock:classic");
    }

    fn classic_uid_write(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        show_dump(ui);
        assert!(ui.inspector.uid_advanced.is_sensitive());
        assert!(!ui.inspector.uid_write.is_sensitive());
        ui.inspector.uid_advanced.set_active(true);
        assert!(ui.inspector.uid_write.is_sensitive());

        ui.inspector.uid.set_text("04A2248A");
        ui.inspector.uid_write.emit_clicked();
        assert!(ui.pending_uid_dialog.borrow().is_none());

        ui.inspector.uid.set_text("DEADBEEF");
        ui.inspector.uid_write.emit_clicked();
        pump_until(ui, "UID plan", || ui.pending_uid_dialog.borrow().is_some());
        let dialog = alert_pending(&ui.pending_uid_dialog);
        emit_response(&dialog, "cancel");
        assert!(ui.pending_uid_plan.borrow().is_none());
        assert_eq!(harness.handle.block(0).expect("block 0")[0..4], TEST_UID);

        ui.inspector.uid.set_text("DE:AD:BE:EF");
        ui.inspector.uid_write.emit_clicked();
        pump_until(ui, "UID plan", || ui.pending_uid_dialog.borrow().is_some());
        let dialog = alert_pending(&ui.pending_uid_dialog);
        emit_response(&dialog, "confirm");
        pump_until(ui, "UID write and re-read", || {
            !ui.busy.get() && ui.status.label().contains("Read complete")
        });
        let block0 = harness.handle.block(0).expect("block 0");
        assert_eq!(block0[0..4], [0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(
            ui.dump.borrow().as_ref().expect("dump").blocks[0].data,
            block0
        );
        assert!(
            activity_text(ui).contains("uid · 04 A2 24 8A -> DE AD BE EF"),
            "{}",
            activity_text(ui)
        );
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
        *ui.keyscan_view.selected.borrow_mut() = vec![Key::from_hex("A0A1A2A3A4A5").expect("key")];
        update_keyscan_selection_label(ui);
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

        let dic = harness.keys_dir.path().join("scan-export.dic");
        write_keyscan_dic(ui, &dic).expect("export dic");
        let text = std::fs::read_to_string(&dic).expect("dic text");
        assert!(text.starts_with("# NFC Foundry"), "{text}");
        assert!(text.contains("# card: 04A2248A/0004/08"), "{text}");
        assert!(text.contains("A0A1A2A3A4A5\n"), "{text}");

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
        assert!(
            !ui.busy.get(),
            "recovery must not start before authorization"
        );
        let dialog = alert_pending(&ui.pending_recovery_dialog);
        assert!(
            dialog
                .heading()
                .as_deref()
                .is_some_and(|heading| heading.contains("recovery")),
            "unexpected heading: {:?}",
            dialog.heading()
        );
        assert!(
            dialog.body().to_ascii_lowercase().contains("darkside"),
            "{}",
            dialog.body()
        );
        emit_response(&dialog, "confirm");
        pump_until(ui, "darkside recovery", || !ui.busy.get());
        let log = ui.recovery_view.log.text();
        assert!(log.contains("unsupported"), "{log}");
        assert!(activity_text(ui).contains("recovery · limitation"));
        assert!(!activity_text(ui).contains("recovery · failed"));

        let key = Key::from_hex("B0B1B2B3B4B5").expect("key");
        let result = RecoveryResult {
            card: None,
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
            card: None,
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
        switch_workspace(ui, Workspace::Classic);
        ui.classic_tabs.set_visible_child_name("recovery");
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
        assert!(
            ui.diagnostics_view
                .platform
                .label()
                .contains(std::env::consts::ARCH)
        );
        assert!(
            ui.diagnostics_view
                .firmware
                .label()
                .starts_with("Firmware:")
        );
        assert!(
            ui.diagnostics_view
                .capabilities
                .label()
                .starts_with("Reader capabilities:")
        );
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

    fn set_ndef_records(ui: &Rc<Ui>, records: Vec<RecordDraft>) {
        {
            let mut editor = ui.ndef_view.editor.borrow_mut();
            editor.clear();
            for record in records {
                editor.add(record);
            }
        }
        ui.ndef_view.editor_key.replace(Some(current_ndef_key(ui)));
        after_ndef_editor_change(ui);
    }

    fn set_ndef_text(ui: &Rc<Ui>, text: &str) {
        set_ndef_records(
            ui,
            vec![RecordDraft::Text(TextDraft {
                text: text.to_owned(),
                language: "en".to_owned(),
                encoding: TextEncoding::Utf8,
            })],
        );
    }

    fn add_ndef_record(ui: &Rc<Ui>, record: RecordDraft) -> usize {
        let index = ui.ndef_view.editor.borrow_mut().add(record);
        ui.ndef_view.editor_key.replace(Some(current_ndef_key(ui)));
        after_ndef_editor_change(ui);
        index
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
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("Add at least one"));
        assert_eq!(ui.ndef_view.rows.n_items(), 0);

        set_ndef_text(ui, "Hello");
        assert!(ui.ndef_view.write.is_sensitive());
        assert_eq!(ui.ndef_view.rows.n_items(), 1);
        assert_eq!(ui.ndef_view.inspector_title.label(), "Record 1 of 1");
        assert!(ui.ndef_view.raw_hex.label().starts_with("03 "));
        assert!(ui.ndef_view.raw_head.label().contains("TLV 0x03"));
        assert_eq!(ui.ndef_view.capacity_state.label(), "Valid NDEF");
        assert!(
            ui.ndef_view.capacity_used.label().contains('/'),
            "{}",
            ui.ndef_view.capacity_used.label()
        );

        add_ndef_record(
            ui,
            RecordDraft::Uri(UriDraft {
                uri: "https://example.com/a".to_owned(),
            }),
        );
        assert_eq!(ui.ndef_view.rows.n_items(), 2);
        assert_eq!(ui.ndef_view.inspector_title.label(), "Record 2 of 2");
        assert!(ui.ndef_view.move_up.is_sensitive());
        assert!(!ui.ndef_view.move_down.is_sensitive());
        move_selected_ndef_record(ui, true);
        assert_eq!(ui.ndef_view.selection.selected(), 0);
        assert!(ui.ndef_view.move_down.is_sensitive());
        remove_selected_ndef_record(ui);
        assert_eq!(ui.ndef_view.rows.n_items(), 1);
        assert!(ui.ndef_view.write.is_sensitive());

        set_ndef_records(ui, vec![RecordDraft::Uri(UriDraft { uri: String::new() })]);
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("must not be empty"));

        set_ndef_records(
            ui,
            vec![RecordDraft::Text(TextDraft {
                text: "Hello".to_owned(),
                language: String::new(),
                encoding: TextEncoding::Utf8,
            })],
        );
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("language code"));

        set_ndef_text(ui, &"x".repeat(120));
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(
            ui.ndef_view.feedback.label().contains("at most 93"),
            "{}",
            ui.ndef_view.feedback.label()
        );

        let dir = tempfile::tempdir().expect("raw NDEF dir");
        let path = dir.path().join("message.ndef");
        let message = NdefMessage::new(vec![
            TextRecord::new("raw", "en")
                .expect("text")
                .to_record()
                .expect("record"),
            UriRecord::new("https://example.com/raw")
                .expect("uri")
                .to_record()
                .expect("record"),
        ])
        .expect("message");
        std::fs::write(&path, message.encode().expect("encode")).expect("raw file");
        load_ndef_file(ui, &path);
        assert_eq!(ui.ndef_view.rows.n_items(), 2);
        assert_eq!(ui.ndef_view.capacity_state.label(), "Valid NDEF");
        assert!(ui.ndef_view.write.is_sensitive());

        let bad = dir.path().join("bad.ndef");
        std::fs::write(&bad, [0xD1, 0x01]).expect("bad raw file");
        load_ndef_file(ui, &bad);
        assert!(ndef_log(ui).contains("not a valid NDEF message"));
        assert_eq!(
            ui.ndef_view.rows.n_items(),
            2,
            "a malformed file keeps the previous message"
        );

        open_ndef_record_dialog(ui.clone(), None);
        let dialog = ui
            .ndef_view
            .pending_record_dialog
            .borrow()
            .clone()
            .expect("record dialog");
        let save = find_button(&dialog, "Save record").expect("save record button");
        save.emit_clicked();
        assert_eq!(ui.ndef_view.rows.n_items(), 3);
        assert!(ui.ndef_view.pending_record_dialog.borrow().is_none());

        ui.live.set(false);
        update_ndef_actions(ui);
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(!ui.ndef_view.format.is_sensitive());
    }

    fn ndef_record_dialog_forms(_harness: &Harness) {
        let text = build_record_form(&RecordDraft::Text(TextDraft {
            text: "Hello".to_owned(),
            language: "en".to_owned(),
            encoding: TextEncoding::Utf8,
        }));
        assert!(matches!(
            collect_record_form(&text).expect("text form"),
            RecordDraft::Text(_)
        ));

        let uri = build_record_form(&RecordKind::Uri.default_draft());
        uri.uri.set_text("https://example.com/a");
        assert!(matches!(
            collect_record_form(&uri).expect("URI form"),
            RecordDraft::Uri(draft) if draft.uri == "https://example.com/a"
        ));

        let media = build_record_form(&RecordKind::Media.default_draft());
        media.media_type.set_text("application/json");
        text_view_set(&media.media_payload, "{\"ok\":true}");
        assert!(matches!(
            collect_record_form(&media).expect("media form"),
            RecordDraft::Media(draft) if draft.media_type == b"application/json"
        ));

        let wifi = build_record_form(&RecordKind::Wifi.default_draft());
        wifi.wifi_ssid.set_text("Foundry");
        wifi.wifi_auth.set_selected(2);
        wifi.wifi_encryption.set_selected(3);
        wifi.wifi_key.set_text("testpass1234");
        assert!(matches!(
            collect_record_form(&wifi).expect("Wi-Fi form"),
            RecordDraft::Wifi(draft) if draft.authentication == 0x0020 && draft.encryption == 0x0008
        ));

        let vcard = build_record_form(&RecordKind::VCard.default_draft());
        vcard.vcard_name.set_text("Ada Lovelace");
        assert!(matches!(
            collect_record_form(&vcard).expect("vCard form"),
            RecordDraft::VCard(draft) if draft.formatted_name == "Ada Lovelace"
        ));

        let ble = build_record_form(&RecordKind::BluetoothLe.default_draft());
        ble.ble_address.set_text("AA:BB:CC:DD:EE:FF");
        ble.ble_role.set_selected(1);
        assert!(matches!(
            collect_record_form(&ble).expect("Bluetooth form"),
            RecordDraft::BluetoothLe(draft) if draft.address.is_some() && draft.role.is_some()
        ));

        let dpp = build_record_form(&RecordKind::Dpp.default_draft());
        dpp.dpp_uri
            .set_text("DPP:C:81/1;M:001122334455;K:303132333435363738393A3B3C3D3E3F;;");
        assert!(matches!(
            collect_record_form(&dpp).expect("DPP form"),
            RecordDraft::Dpp(_)
        ));

        let poster = build_record_form(&RecordKind::SmartPoster.default_draft());
        poster.poster_uri.set_text("https://example.com");
        poster.poster_title.set_text("Example");
        assert!(matches!(
            collect_record_form(&poster).expect("poster form"),
            RecordDraft::SmartPoster(draft) if draft.titles.len() == 1
        ));

        let raw = build_record_form(&RecordKind::Raw.default_draft());
        raw.raw_tnf.set_selected(1);
        raw.raw_type.set_text("U");
        text_view_set(&raw.raw_payload, "00");
        assert!(matches!(
            collect_record_form(&raw).expect("raw form"),
            RecordDraft::Raw(draft) if draft.tnf == Tnf::WellKnown
        ));
    }

    fn ndef_write_confirmation_cancel_and_success(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;
        let blocks = formatted_image(false);
        install_image(harness, &blocks);
        show_image(ui, &dump_for(&blocks));
        set_ndef_records(
            ui,
            vec![
                RecordDraft::Text(TextDraft {
                    text: "Hello".to_owned(),
                    language: "en".to_owned(),
                    encoding: TextEncoding::Utf8,
                }),
                RecordDraft::Uri(UriDraft {
                    uri: "https://example.com/a".to_owned(),
                }),
            ],
        );
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
                assert_eq!(message.records().len(), 2);
                let record = TextRecord::from_record(&message.records()[0]).expect("text record");
                assert_eq!(record.text, "Hello");
                let uri = UriRecord::from_record(&message.records()[1]).expect("URI record");
                assert_eq!(uri.uri, "https://example.com/a");
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
        let message = ui
            .ndef_view
            .editor
            .borrow()
            .message()
            .expect("stored NDEF message");
        assert!(!message.encode().expect("encode").is_empty());
        assert!(ui.ndef_view.feedback.label().contains("already stores"));
    }

    fn ndef_write_gates(harness: &Harness) {
        reset(harness);
        let ui = &harness.ui;

        show_image(ui, &fixture_dump());
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.format.is_sensitive());
        set_ndef_text(ui, "Hello");
        confirm_ndef_write(ui.clone());
        assert!(ui.pending_ndef_write_dialog.borrow().is_none());
        assert!(ui.ndef_view.feedback.label().contains("Format is required"));

        let read_only = formatted_image(true);
        install_image(harness, &read_only);
        show_image(ui, &dump_for(&read_only));
        set_ndef_text(ui, "Hello");
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
        set_ndef_text(ui, "Hello");
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
        let dialog = alert_pending(&ui.pending_type2_advanced_dialog);
        let body = dialog.body();
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        assert!(body.contains("Old  E1 10 12 00"), "{body}");
        assert!(body.contains("New  E1 10 12 F0"), "{body}");
        assert!(body.contains("backup"), "{body}");
        emit_response(&dialog, "confirm");
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
        let dialog = alert_pending(&ui.pending_type2_lock_dialog);
        let body = dialog.body();
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        assert!(body.contains("Static lock 00 00 → 30 00"), "{body}");
        assert!(body.contains("one-way"), "{body}");
        emit_response(&dialog, "confirm");
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
        let dialog = alert_pending(&ui.pending_type2_password_dialog);
        let body = dialog.body();
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        assert!(body.contains("AUTH0 32"), "{body}");
        assert!(
            body.contains("PWD and PACK are replaced with the new values"),
            "{body}"
        );
        assert!(!body.contains("11 22 33 44"), "{body}");
        emit_response(&dialog, "confirm");
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

    fn type2_protected_access(harness: &Harness) {
        let pages = protected_ntag_pages();
        reset(harness);
        install_ntag(&harness.ntag, &pages);
        harness
            .ntag
            .set_password([0x11, 0x22, 0x33, 0x44], [0xAA, 0xBB]);
        choose_reader(harness, "mock:ntag");
        let ui = &harness.ui;

        start_read(ui.clone());
        pump_until(ui, "protected read refusal", || {
            !ui.busy.get() && ui.status.label().contains("Read failed")
        });
        assert!(ui.type2.borrow().is_none());

        ui.type2_view.auth_pwd.set_text("11223344");
        ui.type2_view.auth_pack.set_text("AABB");
        start_read(ui.clone());
        pump_until(ui, "protected read", || {
            !ui.busy.get() && ui.status.label().contains("Read complete")
        });
        assert!(ui.type2.borrow().is_some());
        assert_eq!(ui.type2_view.auth_pwd.text(), "");
        assert_eq!(ui.type2_view.auth_pack.text(), "");

        ui.type2_view.advanced.set_active(true);
        select_type2_page(ui, 32);
        update_type2_inspector(ui);
        assert!(!ui.type2_view.editor.write.is_sensitive());
        assert!(
            ui.type2_view
                .editor
                .feedback
                .label()
                .contains("enter the access password"),
            "{}",
            ui.type2_view.editor.feedback.label()
        );
        ui.type2_view.auth_pwd.set_text("11223344");
        ui.type2_view.auth_pack.set_text("AABB");
        ui.type2_view.editor.hex.set_text("AA BB CC DD");
        update_type2_write_sensitivity(ui);
        assert!(ui.type2_view.editor.write.is_sensitive());
        confirm_type2_page_write(ui.clone());
        let dialog = alert_pending(&ui.pending_type2_page_dialog);
        emit_response(&dialog, "write");
        pump_until(ui, "protected page write", || {
            !ui.busy.get() && ui.status.label().contains("Type 2 page 032 written")
        });
        assert_eq!(harness.ntag.page(32), Some([0xAA, 0xBB, 0xCC, 0xDD]));
        assert_eq!(ui.type2_view.auth_pwd.text(), "");

        set_ndef_text(ui, "Protected");
        update_ndef_actions(ui);
        assert!(!ui.ndef_view.write.is_sensitive());
        ui.type2_view.auth_pwd.set_text("11223344");
        ui.type2_view.auth_pack.set_text("AABB");
        update_ndef_actions(ui);
        assert!(
            ui.ndef_view.write.is_sensitive(),
            "{}",
            ui.ndef_view.feedback.label()
        );
        confirm_ndef_write(ui.clone());
        let dialog = alert_pending(&ui.pending_ndef_write_dialog);
        emit_response(&dialog, "write");
        pump_until(ui, "protected NDEF write", || {
            !ui.busy.get() && ui.status.label().contains("NDEF written")
        });

        ui.type2_view.auth0.set_text("255");
        ui.type2_view.access.set_text("00");
        ui.type2_view.pwd.set_text("11223344");
        ui.type2_view.pack.set_text("AABB");
        update_type2_actions(ui);
        assert!(!ui.type2_view.password_apply.is_sensitive());
        ui.type2_view.auth_pwd.set_text("11223344");
        ui.type2_view.auth_pack.set_text("AABB");
        update_type2_actions(ui);
        assert!(ui.type2_view.password_apply.is_sensitive());
        confirm_type2_password(ui.clone());
        let dialog = alert_pending(&ui.pending_type2_password_dialog);
        emit_response(&dialog, "confirm");
        pump_until(ui, "protection clear", || {
            !ui.busy.get() && ui.status.label().contains("Password configuration written")
        });
        assert_eq!(harness.ntag.page(41).expect("cfg0")[3], 0xFF);
        assert_eq!(harness.ntag.page(42).expect("cfg1")[0], 0x00);
        assert_eq!(ui.type2_view.auth_pwd.text(), "");

        start_read(ui.clone());
        pump_until(ui, "unprotected read", || {
            !ui.busy.get() && ui.status.label().contains("Read complete")
        });
        assert!(ui.type2.borrow().is_some());
    }

    fn type2_ndef_write_and_format(harness: &Harness) {
        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        set_ndef_text(ui, "Hello");
        assert!(ui.ndef_view.write.is_sensitive());
        assert_eq!(ui.ndef_view.rows.n_items(), 1);
        assert_eq!(ui.ndef_view.capacity_state.label(), "Valid NDEF");

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
        let dialog = alert_pending(&ui.pending_type2_format_dialog);
        let body = dialog.body();
        assert!(body.contains("NDEF data area"), "{body}");
        assert!(body.contains("read back"), "{body}");
        emit_response(&dialog, "confirm");
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
        set_ndef_text(ui, "Hello");
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
        set_ndef_text(ui, "Hello");
        assert!(!ui.ndef_view.write.is_sensitive());
        assert!(ui.ndef_view.feedback.label().contains("locked"));
        assert!(!ui.ndef_view.format.is_sensitive());

        read_type2(harness, &ntag_pages(false, false, true));
        let ui = &harness.ui;
        harness.ntag.set_card(Some(other_type2_card()));
        set_ndef_text(ui, "Hello");
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

    fn type2_uid_change_and_restore(harness: &Harness) {
        reset(harness);
        install_ntag(&harness.ntag, &magic_ntag213_pages());
        harness.ntag.set_card(Some(magic_card()));
        harness
            .ntag
            .set_type2_version(Some([0x00, 0x04, 0x04, 0x02, 0x01, 0x00, 0x0F, 0x03]));
        harness.ntag.set_type2_uid_writable(true);
        choose_reader(harness, "mock:ntag");
        start_read(harness.ui.clone());
        pump_until(&harness.ui, "magic read", || {
            !harness.ui.busy.get() && harness.ui.status.label().contains("Read complete")
        });
        let ui = &harness.ui;
        ui.type2_view.advanced.set_active(true);
        ui.type2_view.uid_target.set_text("0400E3C2000003");
        confirm_type2_uid(ui.clone());
        pump_until(ui, "Type 2 UID plan", || {
            !ui.busy.get() && ui.pending_type2_uid_dialog.borrow().is_some()
        });
        let dialog = alert_pending(&ui.pending_type2_uid_dialog);
        let body = dialog.body();
        assert!(body.contains("page 1"), "{body}");
        assert!(body.contains("C2 00 00 03"), "{body}");
        emit_response(&dialog, "confirm");
        pump_until(ui, "Type 2 UID change", || {
            !ui.busy.get() && ui.status.label().contains("UID changed")
        });
        assert_eq!(harness.ntag.page(1), Some([0xC2, 0x00, 0x00, 0x03]));
        assert_eq!(harness.ntag.page(2), Some([0xC1, 0x08, 0x00, 0x00]));

        ui.type2_view.uid_target.set_text("0400E3C2000002");
        confirm_type2_uid(ui.clone());
        pump_until(ui, "Type 2 UID restore plan", || {
            !ui.busy.get() && ui.pending_type2_uid_dialog.borrow().is_some()
        });
        let dialog = alert_pending(&ui.pending_type2_uid_dialog);
        emit_response(&dialog, "confirm");
        pump_until(ui, "Type 2 UID restore", || {
            !ui.busy.get() && ui.status.label().contains("UID changed")
        });
        assert_eq!(harness.ntag.page(1), Some([0xC2, 0x00, 0x00, 0x02]));
        assert_eq!(harness.ntag.page(2), Some([0xC0, 0x08, 0x00, 0x00]));
    }

    fn type2_ev1_otp_format_dialog(harness: &Harness) {
        reset(harness);
        install_ntag(&harness.ntag, &ev1_pages());
        harness
            .ntag
            .set_type2_version(Some([0x00, 0x04, 0x03, 0x01, 0x01, 0x00, 0x0B, 0x03]));
        choose_reader(harness, "mock:ntag");
        start_read(harness.ui.clone());
        pump_until(&harness.ui, "EV1 read", || {
            !harness.ui.busy.get() && harness.ui.status.label().contains("Read complete")
        });
        let ui = &harness.ui;
        assert_eq!(
            ui.type2
                .borrow()
                .as_ref()
                .expect("session")
                .image
                .layout
                .product,
            Type2Product::UltralightEv1Ul11
        );
        assert!(ui.ndef_view.format.is_sensitive());

        confirm_ndef_format(ui.clone());
        let dialog = alert_pending(&ui.pending_type2_format_dialog);
        let body = dialog.body();
        assert!(body.contains("OTP page 3"), "{body}");
        assert!(body.contains("cannot be undone"), "{body}");
        emit_response(&dialog, "cancel");
        assert!(ui.pending_type2_format_dialog.borrow().is_none());
        assert_eq!(harness.ntag.page(3), Some([0; 4]));
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
        let dialog = alert_pending(&ui.pending_type2_restore_dialog);
        emit_response(&dialog, "cancel");
        assert_eq!(harness.ntag.page(6), Some([0; 4]));

        confirm_type2_restore(ui.clone());
        let dialog = alert_pending(&ui.pending_type2_restore_dialog);
        let body = dialog.body();
        assert!(body.contains("Target NTAG213"), "{body}");
        assert!(body.contains("Restore user memory"), "{body}");
        assert!(body.contains("04 5B 1C 2A 3B 4C 80"), "{body}");
        emit_response(&dialog, "confirm");
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
        let dialog = alert_pending(&ui.pending_type2_restore_dialog);
        emit_response(&dialog, "confirm");
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

    #[test]
    fn desfire_workspace_switches_and_parses_inputs() {
        let harness = harness();
        let ui = &harness.ui;
        let item = find_button(
            &ui.workspace_button
                .popover()
                .expect("workspace menu popover"),
            "DESFire",
        )
        .expect("DESFire workspace item");
        item.emit_clicked();
        let context = glib::MainContext::default();
        for _ in 0..5 {
            while context.iteration(false) {}
        }
        assert_eq!(ui.workspace.get(), Workspace::Desfire);
        assert_eq!(ui.views.visible_child_name().as_deref(), Some("desfire"));
        assert_eq!(ui.read_button.label().as_deref(), Some("Refresh card"));
        assert_eq!(
            ui.desfire_view.tabs.visible_child_name().as_deref(),
            Some("inventory")
        );
        assert_eq!(
            ui.desfire_view.summary.label(),
            "Read the card to list DESFire applications."
        );
        assert!(!ui.desfire_view.format_ndef.is_sensitive());
        assert!(!ui.desfire_view.delete_ndef.is_sensitive());
        assert!(!ui.desfire_view.advanced.is_active());
        assert_eq!(
            ui.desfire_view.read_ndef.label().as_deref(),
            Some("Read NDEF")
        );
        assert!(ui.desfire_view.read_ndef.is_sensitive());
        assert!(ui.desfire_view.tabs.child_by_name("write").is_some());
        assert_eq!(ui.desfire_view.write.label().as_deref(), Some("Write file"));
        assert!(ui.desfire_view.write.is_sensitive());
        assert_eq!(ui.desfire_view.write_encoding.selected(), 0);
        assert!(ui.desfire_view.tabs.child_by_name("verify").is_some());
        assert_eq!(
            ui.desfire_view.verify.label().as_deref(),
            Some("Verify key")
        );
        assert_eq!(
            ui.desfire_view.read_uid.label().as_deref(),
            Some("Read UID")
        );
        assert!(ui.desfire_view.verify.is_sensitive());
        assert!(ui.desfire_view.read_uid.is_sensitive());
        assert_eq!(ui.desfire_view.read_key_type.selected(), 0);
        assert_eq!(ui.desfire_view.verify_key_type.selected(), 0);
        assert_eq!(ui.desfire_view.ndef_key_type.selected(), 0);
        assert!(ui.desfire_view.tabs.child_by_name("manage").is_some());
        assert!(ui.desfire_view.tabs.child_by_name("mutation").is_some());
        assert_eq!(
            ui.desfire_view.mutation_run.label().as_deref(),
            Some("Apply value or record change")
        );
        assert_eq!(ui.desfire_view.mutation_operation.selected(), 0);
        assert_eq!(ui.desfire_view.mutation_record.text(), "0");
        assert_eq!(ui.desfire_view.mutation_offset.text(), "0");
        assert_eq!(
            parse_desfire_bytes("AA BB : cc", "record data").expect("hex bytes"),
            [0xAA, 0xBB, 0xCC]
        );
        assert!(parse_desfire_bytes("", "record data").is_err());
        assert!(parse_desfire_bytes("ABC", "record data").is_err());
        assert_eq!(
            ui.desfire_view.create_app.label().as_deref(),
            Some("Create application")
        );
        assert_eq!(
            ui.desfire_view.delete_app.label().as_deref(),
            Some("Delete application")
        );
        assert_eq!(
            ui.desfire_view.change_file_settings.label().as_deref(),
            Some("Change file settings")
        );
        assert!(ui.desfire_view.create_app.is_sensitive());
        assert!(ui.desfire_view.create_file.is_sensitive());
        assert!(ui.desfire_view.read_memory.is_sensitive());
        assert!(!ui.desfire_view.delete_app.is_sensitive());
        assert!(!ui.desfire_view.delete_file.is_sensitive());
        assert_eq!(ui.desfire_view.manage_access_rights.text(), "0000");
        ui.desfire_view.advanced.set_active(true);
        assert!(ui.desfire_view.format_ndef.is_sensitive());
        assert!(ui.desfire_view.delete_ndef.is_sensitive());
        assert!(ui.desfire_view.delete_app.is_sensitive());
        assert!(ui.desfire_view.delete_file.is_sensitive());
        ui.desfire_view.advanced.set_active(false);
        assert!(!ui.desfire_view.format_ndef.is_sensitive());
        assert!(!ui.desfire_view.delete_app.is_sensitive());

        ui.desfire_view.manage_aid.set_text("F00D01");
        ui.desfire_view.manage_file.set_text("02");
        ui.desfire_view.manage_size.set_text("16");
        let manage = desfire_manage_key(&ui.desfire_view).expect("manage input");
        assert_eq!(manage.aid, [0xF0, 0x0D, 0x01]);
        assert_eq!(manage.key_number, 0);
        let spec = desfire_manage_file_spec(&ui.desfire_view).expect("standard file spec");
        assert_eq!(spec.file_type(), nfc_app::desfire::FileType::Standard);
        assert_eq!(spec.required_bytes(), 16);
        assert_eq!(spec.access_rights(), 0);
        ui.desfire_view.manage_iso_file.set_text("E103");
        let spec = desfire_manage_file_spec(&ui.desfire_view).expect("ISO file spec");
        assert_eq!(spec.file_type(), nfc_app::desfire::FileType::Standard);
        ui.desfire_view.manage_iso_file.set_text("");
        ui.desfire_view.manage_file_type.set_selected(2);
        ui.desfire_view.manage_lower_limit.set_text("-50");
        ui.desfire_view.manage_upper_limit.set_text("50");
        ui.desfire_view.manage_value.set_text("7");
        ui.desfire_view.manage_flags.set_text("03");
        let spec = desfire_manage_file_spec(&ui.desfire_view).expect("value file spec");
        assert_eq!(spec.file_type(), nfc_app::desfire::FileType::Value);
        assert_eq!(spec.required_bytes(), 13);
        ui.desfire_view.manage_file_type.set_selected(5);
        ui.desfire_view.manage_tmackey.set_text(&"11".repeat(16));
        let spec = desfire_manage_file_spec(&ui.desfire_view).expect("TMAC file spec");
        assert_eq!(spec.file_type(), nfc_app::desfire::FileType::TransactionMac);
        ui.desfire_view.manage_file_type.set_selected(0);
        assert_eq!(
            desfire_manage_app_key_type(&ui.desfire_view.manage_app_key_type),
            ApplicationKeyType::Tdes2
        );
        ui.desfire_view.manage_app_key_type.set_selected(2);
        assert_eq!(
            desfire_manage_app_key_type(&ui.desfire_view.manage_app_key_type),
            ApplicationKeyType::Aes
        );
        assert_eq!(
            desfire_manage_communication(&ui.desfire_view.manage_communication),
            CommunicationMode::Plain
        );
        ui.desfire_view.manage_communication.set_selected(2);
        assert_eq!(
            desfire_manage_communication(&ui.desfire_view.manage_communication),
            CommunicationMode::Enciphered
        );
        ui.desfire_view.manage_access_rights.set_text("F1E2");
        assert_eq!(
            desfire_manage_access_rights(&ui.desfire_view).expect("access rights"),
            0xE2F1
        );

        assert_eq!(desfire_key_number("0").expect("key number"), 0);
        assert!(desfire_key_number("15").is_err());
        assert!(desfire_key_number("x").is_err());
        assert_eq!(
            desfire_optional_u32("", "record count").expect("empty optional"),
            None
        );
        assert_eq!(
            desfire_optional_u32("3", "record count").expect("record count"),
            Some(3)
        );
        assert!(desfire_optional_u32("x", "record count").is_err());
        assert_eq!(desfire_key_type(&ui.desfire_view.read_key_type), None);
        ui.desfire_view.read_key_type.set_selected(4);
        assert_eq!(
            desfire_key_type(&ui.desfire_view.read_key_type),
            Some(AuthKeyType::Aes)
        );
        ui.desfire_view.read_key_type.set_selected(0);
        assert!(desfire_required_key("").is_err());
        assert_eq!(
            desfire_required_key("0000000000000000")
                .expect("required key")
                .len(),
            8
        );
        assert!(desfire_required_application_key("").is_err());
        let input = desfire_verify_input(
            "A1B2C3",
            "0000000000000000",
            &ui.desfire_view.verify_key_type,
            "0",
        )
        .expect("verification input");
        assert_eq!(input.aid, [0xA1, 0xB2, 0xC3]);
        assert_eq!(input.key.len(), 8);
        assert_eq!(input.key_type, None);
        assert_eq!(input.key_number, 0);
        assert!(desfire_verify_input("A1B2", "00", &ui.desfire_view.verify_key_type, "0").is_err());

        let dialog = typed_confirmation_dialog("Test", "Body", "Confirm", "FORMAT");
        assert!(!dialog.is_response_enabled("confirm"));
        let entry = dialog
            .extra_child()
            .and_downcast::<gtk::Entry>()
            .expect("typed confirmation entry");
        entry.set_text("FORMAT");
        assert!(dialog.is_response_enabled("confirm"));
        entry.set_text("format");
        assert!(!dialog.is_response_enabled("confirm"));

        assert_eq!(
            parse_desfire_hex("A1 B2 C3", 3, "AID").expect("AID"),
            vec![0xA1, 0xB2, 0xC3]
        );
        assert!(parse_desfire_hex("A1B2", 3, "AID").is_err());
        assert!(parse_desfire_key("").expect("empty key").is_none());
        assert_eq!(
            parse_desfire_key("0000000000000000")
                .expect("DES key")
                .expect("some key")
                .len(),
            8
        );
        assert!(parse_desfire_key("00").is_err());
        assert_eq!(
            parse_desfire_key(&"00".repeat(24))
                .expect("3K3DES key")
                .expect("some key")
                .len(),
            24
        );
        assert_eq!(
            parse_desfire_key(&"00".repeat(32))
                .expect("AES key")
                .expect("some key")
                .len(),
            32
        );
        assert_eq!(
            parse_desfire_data("AA BB CC").expect("payload"),
            vec![0xAA, 0xBB, 0xCC]
        );
        assert!(parse_desfire_data("").is_err());
        assert!(parse_desfire_data("ABC").is_err());
        assert_eq!(desfire_ascii(b"NFC\0"), "NFC.");
    }

    #[test]
    fn desfire_ndef_text_reports_records_and_extra_tlvs() {
        let cc = vec![
            0x00, 0x1B, 0x20, 0x00, 0x3A, 0x00, 0x34, // header
            0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00, 0x00, // NDEF file E104
            0x05, 0x06, 0xE1, 0x05, 0x00, 0x10, 0x00, 0x00, // proprietary file E105
            0x06, 0x02, 0xAA, 0xBB, // unknown TLV
        ];
        let record = nfc_ndef::TextRecord::new("NFC Foundry Type 4", "en")
            .expect("text record")
            .to_record()
            .expect("NDEF record");
        let message = nfc_ndef::NdefMessage::single(record).expect("NDEF message");
        let encoded = message.encode().expect("encode NDEF");
        let mut ndef_file = (encoded.len() as u16).to_be_bytes().to_vec();
        ndef_file.extend_from_slice(&encoded);

        let card = CardInfo::new(
            vec![0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26],
            [0x00, 0x44],
            0x20,
            CardType::Iso14443,
        );
        let mut reader = MockReader::new(
            DeviceInfo::new("Mock Type 4", "mock:type4"),
            Some(card.clone()),
        );
        reader.set_type4_tag(MockType4Tag::new(cc, ndef_file));
        let read = nfc_type4::read_tag(&mut reader, &card, &CancelToken::new()).expect("read");
        let text = desfire_ndef_text(&read);
        assert!(text.contains("mapping 2.0"), "{text}");
        assert!(
            text.contains("CC TLV: proprietary file E105 · 16 bytes · read free · write free"),
            "{text}"
        );
        assert!(
            text.contains("CC TLV: unknown tag 0x06 · 2 bytes · AA BB"),
            "{text}"
        );
        assert!(
            text.contains("NDEF file E104 · 256 bytes · read free · write free · 1 record(s)"),
            "{text}"
        );
        assert!(text.contains("1. Text"), "{text}");
        assert!(text.contains("NFC Foundry Type 4"), "{text}");
    }
}
