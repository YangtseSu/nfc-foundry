//! GTK4 workbench shell and the first real NFC Foundry slice.
//!
//! The view owns navigation and presentation only. Every card operation goes
//! through `nfc-app`; reader I/O runs on a worker thread and returns to the GTK
//! main context with an `async-channel`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use gtk::{Orientation, gio, glib};
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicRead, ClassicReadResult, ClassicRestore,
    ClassicRestoreResult, ClassicWriteResult, KeyEntry, KeyKind, KeySource, KeyStore,
};
use nfc_app::{ClassicNdefInfo, ClassicNdefState, Service};
use nfc_classic::{Dump, DumpFormat, DumpLoad, diff, exchange};
use nfc_core::{CancelToken, Key, NfcError, Result as NfcResult};
use nfc_drivers::DriverRegistry;
use nfc_ndef::{NdefRecord, TextRecord, Tnf, UriRecord};
use nfc_transport::DeviceInfo;

use crate::classic::{self, BlockRow};

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
}

impl Workbench {
    pub fn hardware() -> Self {
        let service = Arc::new(Mutex::new(Service::with_transport(Arc::new(
            DriverRegistry,
        ))));
        let keys_path = KeyStore::default_path();
        let (keys, key_error) = match &keys_path {
            Some(path) => match KeyStore::load_or_default(path) {
                Ok(keys) => (keys, None),
                Err(error) => (KeyStore::new(), Some(error.to_string())),
            },
            None => (KeyStore::new(), None),
        };
        Self {
            service,
            keys,
            keys_path,
            backup: BackupStore::default(),
            key_error,
        }
    }
}

pub fn build_ui(app: &adw::Application) {
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
    } = workbench;

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("NFC Foundry")
        .default_width(1360)
        .default_height(880)
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

    let ndef_state = gtk::Label::new(Some("No card loaded"));
    ndef_state.set_xalign(0.0);
    ndef_state.add_css_class("title-3");
    let ndef_box = gtk::Box::new(Orientation::Vertical, 8);
    let ndef_scroll = gtk::ScrolledWindow::builder().child(&ndef_box).build();
    ndef_scroll.set_vexpand(true);
    ndef_scroll.set_has_frame(false);
    let ndef_page = gtk::Box::new(Orientation::Vertical, 8);
    ndef_page.set_margin_top(12);
    ndef_page.set_margin_bottom(12);
    ndef_page.set_margin_start(12);
    ndef_page.set_margin_end(12);
    ndef_page.append(&ndef_state);
    ndef_page.append(&ndef_scroll);

    let keys_view = build_keys_view();
    let diff_view = build_diff_view();
    let restore_view = build_restore_view();

    let views = gtk::Stack::new();
    views.set_transition_type(gtk::StackTransitionType::Crossfade);
    views.set_vexpand(true);
    views.add_titled(&card_page, Some("card"), "Card");
    views.add_titled(&ndef_page, Some("ndef"), "NDEF");
    views.add_titled(&keys_view.root, Some("keys"), "Keys");
    views.add_titled(&diff_view.root, Some("diff"), "Diff");
    views.add_titled(&restore_view.root, Some("restore"), "Restore");
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&views));
    switcher.set_halign(gtk::Align::Start);
    switcher.set_hexpand(false);
    let switcher_row = gtk::Box::new(Orientation::Horizontal, 0);
    switcher_row.set_halign(gtk::Align::Fill);
    switcher_row.set_margin_top(8);
    switcher_row.set_margin_start(12);
    switcher_row.append(&switcher);
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

    let (log_card, _, log_body) = rail_card("TASK & SECURITY LOG");
    let log_view = gtk::TextView::new();
    log_view.set_editable(false);
    log_view.set_monospace(true);
    log_view.set_wrap_mode(gtk::WrapMode::WordChar);
    let log_buffer = log_view.buffer();
    let log_scroll = gtk::ScrolledWindow::builder()
        .child(&log_view)
        .min_content_height(150)
        .vexpand(true)
        .build();
    log_scroll.set_has_frame(true);
    log_scroll.set_overlay_scrolling(false);
    log_body.append(&log_scroll);

    let rail = gtk::Box::new(Orientation::Vertical, 10);
    rail.set_size_request(280, -1);
    rail.set_halign(gtk::Align::Start);
    rail.append(&reader_card);
    rail.append(&card_card);
    rail.append(&key_card);
    rail.append(&selected_card);
    rail.append(&log_card);

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
        "reader workbench",
    )));
    let read_button = gtk::Button::with_label("Read card");
    read_button.add_css_class("suggested-action");
    header.pack_end(&read_button);
    let actions_button = gtk::MenuButton::new();
    actions_button.set_label("Actions");
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
        keys_path,
        backup,
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
        log_buffer,
        ndef_box,
        ndef_state,
        inspector,
        keys_view,
        diff_view,
        restore_view,
        diff_before: RefCell::new(None),
        diff_after: RefCell::new(None),
        restore_load: RefCell::new(None),
        pending_restore_dialog: RefCell::new(None),
        pending_write_dialog: RefCell::new(None),
        devices: RefCell::new(Vec::new()),
    });

    connect_signals(&ui);
    build_actions_menu(&ui, &actions_button);
    update_card(&ui);
    update_ndef(&ui);
    refresh_keys(&ui);
    update_restore_sensitivity(&ui);
    window.present();
    refresh_devices(&ui);

    if let Some(error) = key_error {
        toast_error(&ui, &format!("Key store: {error}"));
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

struct Ui {
    window: adw::ApplicationWindow,
    toast: adw::ToastOverlay,
    service: Arc<Mutex<Service>>,
    keys: RefCell<KeyStore>,
    keys_path: Option<PathBuf>,
    backup: BackupStore,
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
    log_buffer: gtk::TextBuffer,
    ndef_box: gtk::Box,
    ndef_state: gtk::Label,
    inspector: Inspector,
    keys_view: KeysView,
    diff_view: DiffView,
    restore_view: RestoreView,
    diff_before: RefCell<Option<DumpLoad>>,
    diff_after: RefCell<Option<DumpLoad>>,
    restore_load: RefCell<Option<(PathBuf, DumpLoad)>>,
    pending_restore_dialog: RefCell<Option<adw::AlertDialog>>,
    pending_write_dialog: RefCell<Option<adw::AlertDialog>>,
    devices: RefCell<Vec<DeviceInfo>>,
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
                update_inspector(&ui2);
                update_card(&ui2);
                update_restore_sensitivity(&ui2);
                ui2.status.set_label("Reader changed · read the card again");
                ui2.reader_value
                    .set_label(&format!("{}\n{}", device.name, device.conn_string));
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
        label.set_label(&getter(&value));
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
        ui.inspector.title.set_label("No block");
        ui.inspector
            .meta
            .set_label("Read a card to inspect a block.");
        ui.inspector.error.set_visible(false);
        ui.inspector.details.set_label("");
        ui.inspector.hex.set_text("");
        ui.inspector.hex.set_sensitive(false);
        ui.inspector.read.set_sensitive(false);
        ui.inspector.write.set_sensitive(false);
        ui.selected_label.set_label("No block selected");
        return;
    };

    ui.inspector
        .title
        .set_label(&format!("Block {:03}", row.number));
    let role = if row.trailer {
        "sector trailer"
    } else if row.number == 0 {
        "manufacturer"
    } else {
        "data block"
    };
    ui.inspector.meta.set_label(&format!(
        "Sector {:02} · {role} · {}",
        row.sector, row.status
    ));
    match &row.error {
        Some(error) => {
            ui.inspector.error.set_label(error);
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
    ui.inspector.details.set_label(&details);
    ui.inspector
        .hex
        .set_text(if row.complete { &row.data } else { "" });
    let can_edit = ui.live.get() && !ui.busy.get() && row.complete && row.number != 0;
    ui.inspector.hex.set_sensitive(can_edit);
    ui.inspector
        .read
        .set_sensitive(ui.live.get() && !ui.busy.get());
    ui.selected_label.set_label(&format!(
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
            ui.card_title.set_label(layout);
            let live = if ui.live.get() {
                ""
            } else {
                "\nInspection only · read again to write"
            };
            ui.card_detail.set_label(&format!(
                "UID {uid}\n{} sectors · {} blocks\n{}/{} blocks readable{}",
                dump.layout.sector_count(),
                dump.layout.block_count(),
                readability.readable_blocks,
                readability.total_blocks,
                live
            ));
            ui.detail
                .set_label(&format!("{layout} · {} blocks", dump.layout.block_count()));
        }
        None => {
            ui.card_title.set_label("No card");
            ui.card_detail.set_label("Select a reader and read a card.");
            ui.detail.set_label("No card");
        }
    }
    drop(dump);
    ui.key_value.set_label(&format!(
        "{} saved · {} common\nA + B candidates",
        ui.keys.borrow().len(),
        nfc_app::classic::COMMON_KEYS.len()
    ));
}

fn update_ndef(ui: &Rc<Ui>) {
    while let Some(child) = ui.ndef_box.first_child() {
        ui.ndef_box.remove(&child);
    }
    let dump = ui.dump.borrow().clone();
    let Some(dump) = dump else {
        ui.ndef_state.set_label("No card loaded");
        let hint = gtk::Label::new(Some(
            "Read a card to inspect its MAD mapping and NDEF message.",
        ));
        hint.set_xalign(0.0);
        hint.set_wrap(true);
        hint.add_css_class("dim-label");
        ui.ndef_box.append(&hint);
        ui.ndef.replace(None);
        return;
    };

    let result = match ui.service.lock() {
        Ok(service) => service.classic_ndef_inspect(&dump),
        Err(_) => Err(NfcError::Io("reader service lock was poisoned".to_owned())),
    };
    let info = match result {
        Ok(info) => info,
        Err(error) => {
            ui.ndef_state.set_label("NDEF unavailable");
            let label = gtk::Label::new(Some(&error.to_string()));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("error");
            ui.ndef_box.append(&label);
            ui.ndef.replace(None);
            return;
        }
    };

    ui.ndef_state.set_label(match &info.ndef.state {
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
        let label = gtk::Label::new(Some(&mapping));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        ui.ndef_box.append(&label);
    } else {
        let label = gtk::Label::new(Some("No MAD or NFC sector mapping was found on this card."));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("dim-label");
        ui.ndef_box.append(&label);
    }

    match &info.ndef.state {
        ClassicNdefState::Unformatted => {
            let label = gtk::Label::new(Some(
                "Format the card from a future Advanced action before writing NDEF.",
            ));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("dim-label");
            ui.ndef_box.append(&label);
        }
        ClassicNdefState::Empty => {
            let label = gtk::Label::new(Some(&format!(
                "Capacity {} bytes · message empty",
                info.ndef.capacity
            )));
            label.set_xalign(0.0);
            label.set_wrap(true);
            ui.ndef_box.append(&label);
        }
        ClassicNdefState::Message(message) => {
            let capacity = gtk::Label::new(Some(&format!(
                "Capacity {} bytes · message {} bytes",
                info.ndef.capacity, info.ndef.message_bytes
            )));
            capacity.set_xalign(0.0);
            capacity.add_css_class("caption");
            capacity.add_css_class("dim-label");
            ui.ndef_box.append(&capacity);
            for (index, record) in message.records().iter().enumerate() {
                ui.ndef_box.append(&record_card(index, record));
            }
        }
    }

    for warning in &info.warnings {
        let label = gtk::Label::new(Some(warning));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        ui.ndef_box.append(&label);
    }
    ui.ndef.replace(Some(info));
}

fn record_card(index: usize, record: &NdefRecord) -> gtk::Box {
    let card = gtk::Box::new(Orientation::Vertical, 4);
    card.add_css_class("card");
    let kicker = gtk::Label::new(Some(&format!("RECORD {}", index + 1)));
    kicker.set_xalign(0.0);
    kicker.add_css_class("rail-title");
    let (kind, content) = record_summary(record);
    let title = gtk::Label::new(Some(&kind));
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
    let payload = String::from_utf8_lossy(record.payload());
    let preview: String = payload.chars().take(256).collect();
    let suffix = if payload.chars().count() > 256 {
        "…"
    } else {
        ""
    };
    (kind, format!("{preview}{suffix}"))
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
    update_inspector(ui);
    update_card(ui);
    update_restore_sensitivity(ui);
    if ui.dump.borrow().is_some() {
        ui.status.set_label("Inspection only · read the card again");
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
        ui.reader_value.set_label("No supported reader found");
        ui.reader_combo.set_sensitive(false);
        append_log(ui, "refresh · no supported reader");
        ui.status.set_label("No reader found");
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
}

fn start_read(ui: Rc<Ui>) {
    if ui.busy.get() {
        return;
    }
    if ui.devices.borrow().is_empty() {
        toast_error(&ui, "No reader selected");
        return;
    }
    set_busy(&ui, true);
    ui.status.set_label("Reading card…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => service.classic_read(ClassicRead::new(&keys)),
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

fn finish_read(ui: &Rc<Ui>, result: NfcResult<ClassicReadResult>) {
    set_busy(ui, false);
    match result {
        Ok(read) => {
            let warnings = read.warnings;
            let dump = read.dump;
            ui.live.set(true);
            ui.dump.replace(Some(dump.clone()));
            populate_rows(ui, &dump);
            update_card(ui);
            update_ndef(ui);
            update_restore_sensitivity(ui);
            let readable = dump.readability().readable_blocks;
            ui.status
                .set_label(&format!("Read complete · {readable} blocks"));
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
        Err(error) => {
            ui.status.set_label("Read failed");
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
    ui.status.set_label(&format!("Reading block {block:03}…"));
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
            ui.status.set_label(&format!("Block {block:03} read"));
            append_log(ui, &format!("read block · {block:03}"));
        }
        Err(error) => {
            ui.status.set_label("Block read failed");
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
    dialog.add_response("cancel", "Cancel");
    dialog.add_response(
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
    dialog.present(Some(&ui.window));
}

fn start_write(ui: Rc<Ui>, block: usize, data: [u8; 16], trailer: bool) {
    set_busy(&ui, true);
    ui.status.set_label(&format!("Writing block {block:03}…"));
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.clone();
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
                ui.status.set_label(&format!("Block {block:03} written"));
                append_log(ui, &format!("write · block {block:03} · {backup}"));
                toast_info(ui, &format!("Block {block:03} written and read back"));
            } else {
                ui.status.set_label("No change");
                toast_info(ui, "The block already contained these bytes");
            }
        }
        Err(error) => {
            ui.status.set_label("Write failed");
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
    let location = ui
        .keys_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "no store path".to_owned());
    ui.keys_view
        .status
        .set_label(&format!("{} key(s) · {}", store.len(), location));
    ui.keys_view.save.set_sensitive(ui.keys_path.is_some());
    ui.key_value.set_label(&format!(
        "{} saved · {} common\nA + B candidates",
        store.len(),
        nfc_app::classic::COMMON_KEYS.len()
    ));
}

fn save_keys(ui: &Rc<Ui>, notify: bool) {
    let Some(path) = ui.keys_path.clone() else {
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
            append_log(
                ui,
                &format!("keys · added {} {}", kind.label(), key.to_hex()),
            );
            save_keys(ui, false);
        }
        Ok(false) => toast_info(ui, "That key entry is already present"),
        Err(error) => toast_error(ui, &error.to_string()),
    }
}

fn import_keys(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title("Import Classic keys")
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
        .title("Export Proxmark3 key dictionary")
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
        .title("Load Classic dump for diff")
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
                target_label.set_label(&label);
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
            ui.diff_view.summary.set_label(&summary);
            append_log(ui, &format!("diff · {summary}"));
        }
        Err(error) => {
            ui.diff_view.summary.set_label(&error.to_string());
            toast_error(ui, &error.to_string());
        }
    }
}

fn load_restore_dump(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title("Load Classic dump to restore")
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
                ui2.restore_view.file.set_label(&path.display().to_string());
                ui2.restore_view.summary.set_label(&format!(
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
                ui2.restore_view.result.set_label("");
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
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("restore", "Restore");
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
    dialog.present(Some(&ui.window));
}

fn start_restore(ui: &Rc<Ui>, path: PathBuf, dump: Dump) {
    let ui = ui.clone();
    set_busy(&ui, true);
    ui.status.set_label("Restoring dump…");
    let service = ui.service.clone();
    let keys = ui.keys.borrow().clone();
    let backup = ui.backup.clone();
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
            ui.restore_view.result.set_label(&text);
            ui.status
                .set_label(&format!("Restore complete · {}", path.display()));
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
            ui.status.set_label("Restore failed");
            ui.restore_view.rows.remove_all();
            ui.restore_view.result.set_label(&error.to_string());
            toast_error(ui, &error.to_string());
            update_restore_sensitivity(ui);
        }
    }
}

fn save_dump(ui: &Rc<Ui>) {
    let dump = ui.dump.borrow().clone();
    let Some(dump) = dump else {
        toast_error(ui, "No dump to save");
        return;
    };
    let options = classic::save_format_options(&dump);
    let body = if dump.complete() {
        "Complete image. Proxmark3 raw .bin is the default; JSON, MCT, and the project format are also available."
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
        if option.format == DumpFormat::Auto {
            button.set_active(true);
        }
        list.append(&button);
        buttons.push((option.format, button));
    }
    dialog.set_extra_child(Some(&list));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Choose file…");
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
    dialog.present(Some(&ui.window));
}

fn choose_save_path(ui: &Rc<Ui>, dump: &Dump, format: DumpFormat) {
    let chosen = format.resolve(dump);
    let dialog = gtk::FileDialog::builder()
        .title("Save Classic dump")
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
        .title("Load Classic dump")
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
            Ok(loaded) => {
                ui2.live.set(false);
                ui2.dump.replace(Some(loaded.dump.clone()));
                populate_rows(&ui2, &loaded.dump);
                update_card(&ui2);
                update_ndef(&ui2);
                update_restore_sensitivity(&ui2);
                ui2.status.set_label(&format!(
                    "Loaded {} · {}",
                    path.display(),
                    loaded.format.name()
                ));
                append_log(
                    &ui2,
                    &format!(
                        "load · {} · {} · {} warning(s)",
                        path.display(),
                        loaded.format.name(),
                        loaded.warnings.len()
                    ),
                );
                for warning in &loaded.warnings {
                    append_log(&ui2, &format!("warning · {warning}"));
                }
                toast_info(&ui2, "Dump loaded; writes are disabled without a live card");
            }
            Err(error) => toast_error(&ui2, &error.to_string()),
        }
    });
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
}

fn toast_info(ui: &Rc<Ui>, message: &str) {
    ui.toast.add_toast(adw::Toast::new(message));
}

fn toast_error(ui: &Rc<Ui>, message: &str) {
    append_log(ui, &format!("error · {message}"));
    ui.toast.add_toast(adw::Toast::new(message));
}

fn append_log(ui: &Ui, message: &str) {
    let mut end = ui.log_buffer.end_iter();
    ui.log_buffer.insert(&mut end, &format!("{message}\n"));
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nfc_classic::bcc_for_uid;
    use nfc_core::{CardInfo, CardType};
    use nfc_transport::mock::{MockHandle, MockTransport};

    use super::*;

    const TEST_UID: [u8; 4] = [0x04, 0xA2, 0x24, 0x8A];

    fn test_card() -> CardInfo {
        CardInfo::new(
            TEST_UID.to_vec(),
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )
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
        let mut raw = Vec::new();
        for block in image() {
            raw.extend_from_slice(&block);
        }
        Dump::from_raw(&raw)
            .expect("fixture image")
            .with_card(test_card())
    }

    struct Harness {
        app: adw::Application,
        ui: Rc<Ui>,
        handle: MockHandle,
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
        let mut service = Service::with_transport(Arc::new(transport));
        service.refresh_devices().expect("enumerate");
        service.select_device("mock:classic").expect("select");

        let keys_dir = tempfile::tempdir().expect("keys dir");
        let backups = tempfile::tempdir().expect("backup dir");
        let keys_path = keys_dir.path().join("classic-keys.json");
        let ui = build_ui_with(
            &app,
            Workbench {
                service: Arc::new(Mutex::new(service)),
                keys: KeyStore::new(),
                keys_path: Some(keys_path),
                backup: BackupStore::new(backups.path()),
                key_error: None,
            },
        );
        Harness {
            app,
            ui,
            handle,
            keys_dir,
            backups,
        }
    }

    fn reset(harness: &Harness) {
        harness.handle.set_card(Some(test_card()));
        for (block, data) in image().into_iter().enumerate() {
            harness.handle.set_block(block as u8, data);
        }
        let ui = &harness.ui;
        ui.live.set(false);
        ui.busy.set(false);
        ui.dump.replace(None);
        ui.restore_load.replace(None);
        ui.diff_before.replace(None);
        ui.diff_after.replace(None);
        ui.pending_restore_dialog.replace(None);
        ui.pending_write_dialog.replace(None);
        ui.rows.remove_all();
        ui.diff_view.rows.remove_all();
        ui.restore_view.rows.remove_all();
        ui.restore_view.result.set_label("");
        ui.diff_view
            .summary
            .set_label("Load both images, then compare.");
        ui.status.set_label("Ready");
        ui.keys.borrow_mut().clear();
        refresh_keys(ui);
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
        harness.ui.window.close();
        drop(harness.app);
    }

    fn selection_updates_inspector(harness: &Harness) {
        reset(harness);
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
        let log = harness.ui.log_buffer.text(
            &harness.ui.log_buffer.start_iter(),
            &harness.ui.log_buffer.end_iter(),
            false,
        );
        assert!(
            log.contains("the card identity changed"),
            "log did not report the identity refusal: {log}"
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
}
