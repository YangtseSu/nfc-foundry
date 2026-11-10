//! Rendering for the Type 2 / NDEF workspace.

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
    tab::{Tab, TabBar},
    table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, IntoElement, Render, Role, SharedString, TestSupportExt,
    Window, div,
};
use nfc_ndef::{NdefRecord, TextRecord, UriRecord};
use nfc_type2::{LockBits, NdefState, PageRole};

use crate::model::{StatusKind, WorkspaceTab};
use crate::workspace::{Type2Workspace, ascii, configuration, hex, locked_pages, ndef_label};

impl Render for Type2Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_header(cx))
            .child(self.render_toolbar(cx))
            .child(self.render_tabs(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_content(cx)),
            )
            .child(self.render_status(cx))
    }
}

impl Type2Workspace {
    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let selected = self
            .selected_device
            .clone()
            .unwrap_or_else(|| "No reader selected".to_owned());
        let mut device_row = h_flex().gap_2().flex_wrap().px_4().pb_3();
        if self.devices.is_empty() {
            device_row = device_row.child(muted(
                "No readers found. Connect a PN532 or ACR122U, then refresh.",
                cx,
            ));
        } else {
            for (index, device) in self.devices.iter().enumerate() {
                let is_selected = self.selected_device.as_deref() == Some(&device.conn_string);
                device_row = device_row.child(
                    Button::new(("type2-device", index))
                        .small()
                        .label(device.name.clone())
                        .disabled(busy)
                        .when(is_selected, |button| button.primary())
                        .when(!is_selected, |button| button.outline())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_device(index, cx);
                        })),
                );
            }
        }

        v_flex()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .p_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Type 2 / NDEF"),
                            )
                            .child(muted(
                                "Read NTAG memory, edit NDEF, and manage protection.",
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(muted(selected, cx))
                            .child(Button::new("type2-refresh").label("Refresh").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.reload_devices(cx);
                                }),
                            )),
                    ),
            )
            .child(device_row)
            .into_any_element()
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let has_image = self.image.is_some();
        v_flex()
            .gap_2()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(
                        Button::new("type2-read")
                            .label("Read tag")
                            .primary()
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.read_card(window, cx);
                            })),
                    )
                    .child(
                        Button::new("type2-save-dump")
                            .label("Save dump")
                            .disabled(busy || !has_image)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_dump(cx);
                            })),
                    )
                    .child(
                        Button::new("type2-load-dump")
                            .label("Load dump")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_dump(cx);
                            })),
                    )
                    .child(
                        Button::new("type2-restore")
                            .label("Restore…")
                            .danger()
                            .disabled(busy || self.loaded.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_restore(window, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(muted(
                        self.image
                            .as_ref()
                            .map(|image| {
                                format!("{} · {}", image.layout.product.name(), image.uid_hex())
                            })
                            .unwrap_or_else(|| "No tag read".to_owned()),
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(Input::new(&self.path_input).id("type2-path").w_80())
                    .when(self.loaded.is_some(), |row| {
                        row.child(muted(
                            format!(
                                "loaded: {}",
                                self.loaded
                                    .as_ref()
                                    .map(|image| image.layout.product.name())
                                    .unwrap_or_default()
                            ),
                            cx,
                        ))
                    }),
            )
            .into_any_element()
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab_entity = cx.entity();
        v_flex()
            .px_4()
            .pt_2()
            .child(
                TabBar::new("type2-tabs")
                    .underline()
                    .selected_index(self.tab.index())
                    .child(Tab::new().label("Memory"))
                    .child(Tab::new().label("NDEF"))
                    .child(Tab::new().label("Protection"))
                    .on_click(move |index, _, cx| {
                        tab_entity.update(cx, |this, cx| {
                            this.select_tab(*index, cx);
                        });
                    }),
            )
            .into_any_element()
    }

    fn render_content(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.tab {
            WorkspaceTab::Memory => self.render_memory(cx),
            WorkspaceTab::Ndef => self.render_ndef(cx),
            WorkspaceTab::Protection => self.render_protection(cx),
        }
    }

    fn render_memory(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(image) = &self.image else {
            return empty_state(
                "No tag image",
                "Read a Type 2 / NTAG tag to inspect its pages.",
                cx,
            );
        };
        let locks = LockBits::from_image(image).ok();
        let rows = image
            .pages()
            .map(|(page, data)| {
                let selected = self.selected_page == Some(page);
                let role = image.role(page);
                let locked = locks
                    .as_ref()
                    .is_some_and(|locks| locks.is_locked(&image.layout, page));
                let data = data.map(|data| hex(data)).unwrap_or_else(|| "—".to_owned());
                TableRow::new()
                    .when(selected, |row| row.bg(cx.theme().accent))
                    .child(
                        TableCell::new().child(
                            div()
                                .id(("type2-page", page as usize))
                                .test_support()
                                .flex()
                                .items_center()
                                .w_full()
                                .h_full()
                                .child(format!("{page}"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select_page(page, window, cx);
                                })),
                        ),
                    )
                    .child(TableCell::new().child(role.label()))
                    .child(TableCell::new().child(if locked { "locked" } else { "—" }))
                    .child(TableCell::new().child(mono(data, cx)))
            })
            .collect::<Vec<_>>();

        let table = Table::new()
            .small()
            .accessibility_label("Type 2 pages")
            .child(
                TableHeader::new().child(
                    TableRow::new()
                        .child(TableHead::new().child("Page"))
                        .child(TableHead::new().child("Role"))
                        .child(TableHead::new().child("Lock"))
                        .child(TableHead::new().child("Data")),
                ),
            )
            .child(TableBody::new().children(rows));

        h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("type2-page-scroll")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(table),
            )
            .child(
                div()
                    .id("type2-inspector-scroll")
                    .w_80()
                    .flex_shrink_0()
                    .h_full()
                    .overflow_y_scroll()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(self.render_page_inspector(cx)),
            )
            .into_any_element()
    }

    fn render_page_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(image) = &self.image else {
            return div().into_any_element();
        };
        let Some(page) = self.selected_page else {
            return empty_state("No page selected", "Select a page in the table.", cx);
        };
        let Some(data) = image.page(page) else {
            return empty_state("Page unavailable", "This page was not readable.", cx);
        };
        let role = image.role(page);
        let locks = LockBits::from_image(image).ok();
        let locked = locks
            .as_ref()
            .is_some_and(|locks| locks.is_locked(&image.layout, page));
        let writable = !matches!(role, PageRole::Manufacturer | PageRole::Reserved);

        let mut inspector = v_flex()
            .id("type2-inspector")
            .role(Role::Group)
            .test_support()
            .aria_label(format!("Page {page} · {}", role.label()))
            .gap_2()
            .p_4()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Page {page}")),
                    )
                    .child(muted(role.label(), cx)),
            )
            .child(muted(
                format!(
                    "{} · {}",
                    crate::workspace::ascii(&data[..]),
                    if locked {
                        "locked read-only"
                    } else {
                        "writable"
                    }
                ),
                cx,
            ))
            .child(section("Hex", cx))
            .child(mono(hex(data), cx))
            .child(section("ASCII", cx))
            .child(mono(ascii(data), cx));

        if role.is_advanced() {
            inspector = inspector.child(div().text_sm().text_color(cx.theme().warning).child(
                format!(
                    "{} pages need an Advanced confirmation and a backup.",
                    role.label()
                ),
            ));
        }

        inspector = inspector
            .child(
                div()
                    .mt_3()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Write page"),
            )
            .child(Input::new(&self.page_data_input).id("type2-page-data"))
            .child(
                Button::new("type2-write-page")
                    .label(if role.is_advanced() {
                        "Write page…"
                    } else {
                        "Write page"
                    })
                    .when(role.is_advanced(), |button| button.danger())
                    .disabled(!writable || locked || self.busy.is_some())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_write_selected(window, cx);
                    })),
            );
        if locked {
            inspector = inspector.child(muted("This page is locked and cannot be written.", cx));
        }
        if matches!(role, PageRole::Password | PageRole::PasswordAck) {
            inspector = inspector.child(muted(
                "PWD and PACK never read back; writes are verified with PWD_AUTH.",
                cx,
            ));
        }
        inspector.into_any_element()
    }

    fn render_ndef(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(image) = &self.image else {
            return empty_state("No tag image", "Read a Type 2 tag first.", cx);
        };
        let ndef = self.ndef.as_ref();
        let state = ndef
            .map(|ndef| ndef_label(&ndef.state))
            .unwrap_or_else(|| "unknown".to_owned());
        let capacity = ndef
            .map(|ndef| ndef.capacity)
            .unwrap_or_else(|| image.cc.ndef_capacity());

        let mut records = v_flex().gap_2();
        if let Some(ndef) = ndef {
            match &ndef.state {
                NdefState::Unformatted => {
                    records = records.child(muted(
                        "No NDEF Message TLV. Write a message or format the tag.",
                        cx,
                    ));
                }
                NdefState::Empty => {
                    records = records.child(muted("Formatted tag with an empty NDEF message.", cx));
                }
                NdefState::Message(message) => {
                    for (index, record) in message.records().iter().enumerate() {
                        records = records.child(record_row(index, record, cx));
                    }
                    records = records.child(muted(
                        format!("{} / {capacity} bytes used", ndef.message_bytes),
                        cx,
                    ));
                }
            }
        }

        let mut content = v_flex()
            .gap_3()
            .p_4()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .flex_wrap()
                    .child(div().text_lg().child("NDEF"))
                    .child(muted(
                        format!("state {state} · capacity {capacity} bytes"),
                        cx,
                    )),
            )
            .child(records)
            .child(
                div()
                    .mt_2()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Write Text record"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(
                        Input::new(&self.ndef_text_input)
                            .id("type2-ndef-text")
                            .w_96(),
                    )
                    .child(
                        Input::new(&self.ndef_language_input)
                            .id("type2-ndef-language")
                            .w_32(),
                    )
                    .child(
                        Button::new("type2-ndef-write")
                            .primary()
                            .label("Write NDEF")
                            .disabled(
                                self.busy.is_some() || ndef.is_some_and(|ndef| ndef.read_only),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.write_ndef(window, cx);
                            })),
                    )
                    .child(
                        Button::new("type2-ndef-format")
                            .danger()
                            .label("Format…")
                            .disabled(
                                self.busy.is_some() || ndef.is_some_and(|ndef| ndef.read_only),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_format(window, cx);
                            })),
                    ),
            );

        if ndef.is_some_and(|ndef| ndef.read_only) {
            content = content.child(muted(
                "The capability container marks this tag read-only for NDEF.",
                cx,
            ));
        }
        content.into_any_element()
    }

    fn render_protection(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(image) = &self.image else {
            return empty_state("No tag image", "Read a Type 2 tag first.", cx);
        };
        let locks = LockBits::from_image(image).ok();
        let config = configuration(image);
        let locked = locks
            .as_ref()
            .map(|locks| locked_pages(image, locks))
            .unwrap_or_default();

        let mut content = v_flex()
            .gap_3()
            .p_4()
            .child(div().text_lg().child("Protection"))
            .child(
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .child(info_block(
                        "Static lock",
                        locks
                            .as_ref()
                            .map(|locks| hex(&locks.static_lock))
                            .unwrap_or_else(|| "unreadable".to_owned()),
                        cx,
                    ))
                    .child(info_block(
                        "Dynamic lock",
                        locks
                            .as_ref()
                            .map(|locks| hex(&locks.dynamic_lock))
                            .unwrap_or_else(|| "unreadable".to_owned()),
                        cx,
                    ))
                    .child(info_block(
                        "Locked pages",
                        if locked.is_empty() {
                            "none".to_owned()
                        } else {
                            locked
                                .iter()
                                .map(u8::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                        cx,
                    )),
            )
            .child(match &config {
                Some(config) => info_block(
                    "Configuration",
                    format!(
                        "AUTH0 {:02X} · ACCESS {:02X} · CFGLCK {} · AUTHLIM {}",
                        config.auth0,
                        config.access,
                        config.cfg_locked(),
                        config.auth_lim()
                    ),
                    cx,
                ),
                None => muted("This tag has no NTAG configuration pages.", cx),
            })
            .child(section("Lock pages", cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(
                        Input::new(&self.lock_pages_input)
                            .id("type2-lock-pages")
                            .w_64(),
                    )
                    .child(
                        Button::new("type2-lock-apply")
                            .danger()
                            .label("Apply locks…")
                            .disabled(self.busy.is_some())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_lock(window, cx);
                            })),
                    ),
            )
            .child(muted(
                "Lock bits are one-way. A backup is written before the first lock.",
                cx,
            ))
            .child(section("Password configuration", cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(Input::new(&self.auth0_input).id("type2-auth0").w_32())
                    .child(Input::new(&self.access_input).id("type2-access").w_32())
                    .child(Input::new(&self.pwd_input).id("type2-pwd").w_48())
                    .child(Input::new(&self.pack_input).id("type2-pack").w_32())
                    .child(
                        Button::new("type2-password-apply")
                            .danger()
                            .label("Set password…")
                            .disabled(self.busy.is_some() || config.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_password(window, cx);
                            })),
                    ),
            );
        if config.as_ref().is_some_and(|config| config.cfg_locked()) {
            content = content.child(muted(
                "The configuration is permanently locked; AUTH0/ACCESS cannot change.",
                cx,
            ));
        }
        content.into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let (text, color) = if let Some(busy) = &self.busy {
            (
                SharedString::from(format!("{busy}…")),
                cx.theme().muted_foreground,
            )
        } else if let Some(status) = &self.status {
            let color = match status.kind {
                StatusKind::Info => cx.theme().muted_foreground,
                StatusKind::Success => cx.theme().success,
                StatusKind::Warning => cx.theme().warning,
                StatusKind::Error => cx.theme().danger,
            };
            (status.text.clone(), color)
        } else {
            (SharedString::from("Ready"), cx.theme().muted_foreground)
        };
        h_flex()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .id("type2-status")
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .test_support()
                    .text_sm()
                    .text_color(color)
                    .child(text),
            )
            .into_any_element()
    }
}

fn record_row(index: usize, record: &NdefRecord, cx: &App) -> AnyElement {
    let detail = if let Ok(text) = TextRecord::from_record(record) {
        format!(
            "{} · text {:?} · language {}",
            index, text.text, text.language
        )
    } else if let Ok(uri) = UriRecord::from_record(record) {
        format!("{} · uri {}", index, uri.uri)
    } else {
        format!(
            "{} · {:?} · type {} · payload {}",
            index,
            record.tnf(),
            hex(record.record_type()),
            hex(record.payload())
        )
    };
    div()
        .id(("type2-ndef-record", index))
        .test_support()
        .child(mono(detail, cx))
        .into_any_element()
}

fn info_block(title: &'static str, value: String, cx: &App) -> AnyElement {
    v_flex()
        .gap_1()
        .child(label(title))
        .child(mono(value, cx))
        .into_any_element()
}

fn muted(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

fn label(text: &'static str) -> AnyElement {
    div()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .child(text)
        .into_any_element()
}

fn section(text: &'static str, cx: &App) -> AnyElement {
    div()
        .mt_2()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn mono(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .child(text.into())
        .into_any_element()
}

fn empty_state(title: &'static str, detail: &'static str, cx: &App) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .p_8()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(muted(detail, cx))
        .into_any_element()
}
