//! GTK4 workbench shell and the first real NFC Foundry slice.
//!
//! The view owns navigation and presentation only. Every card operation goes
//! through `nfc-app`; reader I/O runs on a worker thread and returns to the GTK
//! main context with an `async-channel`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use gtk::{Orientation, gio, glib};
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicRead, ClassicReadResult, ClassicWriteResult, KeyStore,
};
use nfc_app::{ClassicNdefInfo, ClassicNdefState, Service};
use nfc_classic::{Dump, DumpFormat, exchange};
use nfc_core::{CancelToken, NfcError, Result as NfcResult};
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

pub fn build_ui(app: &adw::Application) {
    install_css();

    let service = Arc::new(Mutex::new(Service::with_transport(Arc::new(
        DriverRegistry,
    ))));
    let mut key_error = None;
    let keys = match KeyStore::default_path() {
        Some(path) => match KeyStore::load_or_default(&path) {
            Ok(keys) => keys,
            Err(error) => {
                key_error = Some(error.to_string());
                KeyStore::new()
            }
        },
        None => KeyStore::new(),
    };
    let backup = BackupStore::default();

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

    let views = gtk::Stack::new();
    views.set_transition_type(gtk::StackTransitionType::Crossfade);
    views.set_vexpand(true);
    views.add_titled(&card_page, Some("card"), "Card");
    views.add_titled(&ndef_page, Some("ndef"), "NDEF");
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
        keys,
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
        devices: RefCell::new(Vec::new()),
    });

    connect_signals(&ui);
    build_actions_menu(&ui, &actions_button);
    update_card(&ui);
    update_ndef(&ui);
    window.present();
    refresh_devices(&ui);

    if let Some(error) = key_error {
        toast_error(&ui, &format!("Key store: {error}"));
    }
}

struct Inspector {
    root: gtk::Box,
    title: gtk::Label,
    meta: gtk::Label,
    error: gtk::Label,
    hex: gtk::Entry,
    read: gtk::Button,
    write: gtk::Button,
}

struct Ui {
    window: adw::ApplicationWindow,
    toast: adw::ToastOverlay,
    service: Arc<Mutex<Service>>,
    keys: KeyStore,
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
    root.append(&gtk::Separator::new(Orientation::Horizontal));
    root.append(&hex);
    root.append(&actions);
    root.append(&note);

    Inspector {
        root,
        title,
        meta,
        error,
        hex,
        read,
        write,
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
        ui.keys.len(),
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
    let keys = ui.keys.clone();
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
    let keys = ui.keys.clone();
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
        if response == "write" {
            start_write(ui2.clone(), block, data, trailer);
        }
    });
    dialog.present(Some(&ui.window));
}

fn start_write(ui: Rc<Ui>, block: usize, data: [u8; 16], trailer: bool) {
    set_busy(&ui, true);
    ui.status.set_label(&format!("Writing block {block:03}…"));
    let service = ui.service.clone();
    let keys = ui.keys.clone();
    let backup = ui.backup.clone();
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match service.lock() {
            Ok(mut service) => {
                let mut request = ClassicBlockWrite::new(block, data, &keys, &backup);
                if trailer {
                    request = request.confirm_trailer(true);
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

fn save_dump(ui: &Rc<Ui>) {
    let dump = ui.dump.borrow().clone();
    let Some(dump) = dump else {
        toast_error(ui, "No dump to save");
        return;
    };
    let dialog = gtk::FileDialog::builder()
        .title("Save Classic dump")
        .initial_name("classic-dump.json")
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
        match exchange::export(&dump, DumpFormat::Project) {
            Ok(bytes) => match std::fs::write(&path, &bytes) {
                Ok(()) => {
                    append_log(&ui2, &format!("save · {}", path.display()));
                    toast_info(&ui2, &format!("Saved {}", path.display()));
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
