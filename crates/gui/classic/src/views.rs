//! Rendering for the Classic workspace.

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::Input,
    radio::RadioGroup,
    tab::{Tab, TabBar},
    table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, IntoElement, Render, Role, SharedString, TestSupportExt,
    Window, div,
};
use nfc_app::ClassicNdefState;
use nfc_classic::{AccessCondition, BlockStatus, KeyMask};
use nfc_ndef::{NdefRecord, TextRecord, UriRecord};

use crate::model::{StatusKind, WorkspaceTab};
use crate::workspace::{ClassicWorkspace, hex};

impl Render for ClassicWorkspace {
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

impl ClassicWorkspace {
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
                    Button::new(("classic-device", index))
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
                                    .child("MIFARE Classic"),
                            )
                            .child(muted(
                                "Read, inspect, back up, and restore 1K and 4K cards.",
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(muted(selected, cx))
                            .child(Button::new("classic-refresh").label("Refresh").on_click(
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
        let has_dump = self.dump.is_some();
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
                        Button::new("classic-read")
                            .label("Read card")
                            .primary()
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.read_card(window, cx);
                            })),
                    )
                    .child(
                        Button::new("classic-scan-keys")
                            .label("Scan keys")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.scan_keys(window, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(muted(format!("{} keys", self.keys.len()), cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(Input::new(&self.path_input).id("classic-path").w_80())
                    .child(
                        Button::new("classic-save")
                            .label("Save dump")
                            .disabled(busy || !has_dump)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_dump(cx);
                            })),
                    )
                    .child(
                        Button::new("classic-load")
                            .label("Load dump")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_dump(window, cx);
                            })),
                    )
                    .child(
                        Button::new("classic-restore")
                            .label("Restore…")
                            .danger()
                            .disabled(busy || !has_dump)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.restore_loaded(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let diff_count = self.diff.as_ref().map_or(0, |diff| diff.changed_blocks);
        let tab_entity = cx.entity();
        v_flex()
            .px_4()
            .pt_2()
            .child(
                TabBar::new("classic-tabs")
                    .underline()
                    .selected_index(self.tab.index())
                    .child(Tab::new().label("Blocks"))
                    .child(Tab::new().label("NDEF"))
                    .child(Tab::new().label(format!("Diff ({diff_count})")))
                    .child(Tab::new().label(format!("Keys ({})", self.keys.len())))
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
            WorkspaceTab::Blocks => self.render_blocks(cx),
            WorkspaceTab::Ndef => self.render_ndef(cx),
            WorkspaceTab::Diff => self.render_diff(cx),
            WorkspaceTab::Keys => self.render_keys(cx),
        }
    }

    fn render_ndef(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dump) = &self.dump else {
            return empty_state(
                "No card image",
                "Read a MIFARE Classic card or load a dump file.",
                cx,
            );
        };
        let mut content = v_flex()
            .id("classic-ndef-scroll")
            .gap_3()
            .p_4()
            .overflow_y_scroll();
        content = content.child(
            h_flex()
                .gap_3()
                .items_center()
                .flex_wrap()
                .child(div().text_lg().child("Classic NDEF"))
                .child(muted(format!("{} · current image", dump.layout.name()), cx)),
        );

        if let Some(error) = &self.ndef_error {
            content = content.child(muted(
                format!("The NDEF mapping could not be parsed: {error}"),
                cx,
            ));
        }

        let writable = self
            .ndef
            .as_ref()
            .and_then(|info| info.layout.as_ref())
            .is_some_and(|layout| layout.writable && !layout.read_only);
        let formatted = self.ndef.as_ref().is_some_and(|info| {
            info.layout
                .as_ref()
                .is_some_and(|layout| !layout.sectors.is_empty())
        });
        let read_only = self
            .ndef
            .as_ref()
            .and_then(|info| info.layout.as_ref())
            .is_some_and(|layout| layout.read_only);

        if let Some(info) = &self.ndef {
            match &info.layout {
                Some(layout) => {
                    content = content.child(info_block(
                        "Mapping",
                        format!(
                            "version {}.{} · {} NFC sector(s) · {} data block(s), {} bytes · {}",
                            layout.version.major,
                            layout.version.minor,
                            layout.sectors.len(),
                            layout.data_blocks.len(),
                            layout.data_len(),
                            if layout.read_only {
                                "read-only"
                            } else if layout.writable {
                                "read/write"
                            } else {
                                "not writable"
                            }
                        ),
                        cx,
                    ));
                    content = content.child(info_block(
                        "NFC sectors",
                        layout
                            .sectors
                            .iter()
                            .map(u8::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        cx,
                    ));
                    for warning in &layout.warnings {
                        content = content.child(muted(format!("Warning: {warning}"), cx));
                    }
                }
                None => {
                    content = content.child(muted(
                        "No MAD is present. Format the card to create MAD1/MAD2 and NFC sectors.",
                        cx,
                    ));
                }
            }

            match &info.ndef.state {
                ClassicNdefState::Unformatted => {
                    content = content.child(muted(
                        "No mandatory NDEF Message TLV. Format the card first.",
                        cx,
                    ));
                }
                ClassicNdefState::Empty => {
                    content = content.child(muted(
                        format!(
                            "Empty NDEF Message TLV · {} bytes of capacity",
                            info.ndef.capacity
                        ),
                        cx,
                    ));
                }
                ClassicNdefState::Message(message) => {
                    let mut records = v_flex().gap_1();
                    for (index, record) in message.records().iter().enumerate() {
                        records = records.child(ndef_record_row(index, record, cx));
                    }
                    content = content
                        .child(section("Records", cx))
                        .child(records)
                        .child(muted(
                            format!(
                                "{} / {} bytes used · TLV at byte {} · length field {} byte(s)",
                                info.ndef.message_bytes,
                                info.ndef.capacity,
                                info.ndef.tlv_offset,
                                info.ndef.length_field_bytes
                            ),
                            cx,
                        ));
                }
            }
        }

        content = content
            .child(section("Write Text record", cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(Input::new(&self.ndef_text_input).id("classic-ndef-text").w_96())
                    .child(
                        Input::new(&self.ndef_language_input)
                            .id("classic-ndef-language")
                            .w_32(),
                    )
                    .child(
                        Button::new("classic-ndef-write")
                            .primary()
                            .label("Write NDEF")
                            .disabled(
                                self.busy.is_some() || !writable || !formatted,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.write_ndef(window, cx);
                            })),
                    ),
            )
            .child(section("Format", cx))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .flex_wrap()
                    .child(
                        Checkbox::new("classic-ndef-read-only")
                            .checked(self.format_read_only)
                            .label("Read-only after format")
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.format_read_only = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("classic-ndef-format")
                            .danger()
                            .label("Format…")
                            .disabled(self.busy.is_some() || read_only)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_format(window, cx);
                            })),
                    ),
            )
            .child(muted(
                "Formatting rewrites every NFC sector and the MAD, then changes sector keys and access bits. A backup is written first.",
                cx,
            ));

        if read_only {
            content = content.child(muted(
                "This tag is read-only for NDEF; writes and reformatting are blocked.",
                cx,
            ));
        }
        content.into_any_element()
    }

    fn render_blocks(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dump) = &self.dump else {
            return empty_state(
                "No card image",
                "Read a MIFARE Classic card or load a dump file.",
                cx,
            );
        };
        let rows = dump
            .blocks
            .iter()
            .map(|block| {
                let number = block.number;
                let selected = self.selected_block == Some(number);
                let is_trailer = dump.layout.is_trailer(number).unwrap_or(false);
                let kind = if number == 0 {
                    "M"
                } else if is_trailer {
                    "T"
                } else {
                    "D"
                };
                let data = if block.complete() {
                    hex(&block.data)
                } else {
                    "—".to_owned()
                };
                TableRow::new()
                    .when(selected, |row| row.bg(cx.theme().accent))
                    .child(
                        TableCell::new().child(
                            div()
                                .id(("classic-block", number))
                                .test_support()
                                .flex()
                                .items_center()
                                .w_full()
                                .h_full()
                                .child(format!("{number}"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select_block(number, window, cx);
                                })),
                        ),
                    )
                    .child(TableCell::new().child(format!("{}", block.sector)))
                    .child(TableCell::new().child(kind))
                    .child(TableCell::new().child(block_status(block.status)))
                    .child(TableCell::new().child(format!("{}/16", block.known_mask.count_ones())))
                    .child(TableCell::new().child(mono(data, cx)))
            })
            .collect::<Vec<_>>();

        let table = Table::new()
            .small()
            .accessibility_label("MIFARE Classic blocks")
            .child(
                TableHeader::new().child(
                    TableRow::new()
                        .child(TableHead::new().child("Block"))
                        .child(TableHead::new().child("Sector"))
                        .child(TableHead::new().child("Type"))
                        .child(TableHead::new().child("Status"))
                        .child(TableHead::new().child("Known"))
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
                    .id("classic-block-scroll")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(table),
            )
            .child(
                div()
                    .id("classic-inspector-scroll")
                    .w_80()
                    .flex_shrink_0()
                    .h_full()
                    .overflow_y_scroll()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(self.render_inspector(cx)),
            )
            .into_any_element()
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dump) = &self.dump else {
            return div().into_any_element();
        };
        let Some(number) = self.selected_block else {
            return empty_state("No block selected", "Select a block in the table.", cx);
        };
        let Some(block) = dump.blocks.get(number) else {
            return empty_state("Block unavailable", "Select another block.", cx);
        };
        let is_trailer = dump.layout.is_trailer(number).unwrap_or(false);
        let kind = if number == 0 {
            "Manufacturer"
        } else if is_trailer {
            "Sector trailer"
        } else {
            "Data"
        };

        let mut inspector = v_flex()
            .id("classic-inspector")
            .role(Role::Group)
            .test_support()
            .aria_label(format!("Block {number} · sector {} · {kind}", block.sector))
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
                            .child(format!("Block {number}")),
                    )
                    .child(muted(kind, cx)),
            )
            .child(muted(
                format!(
                    "Sector {} · {} · {}/16 bytes known",
                    block.sector,
                    block_status(block.status),
                    block.known_mask.count_ones()
                ),
                cx,
            ));

        if block.complete() {
            inspector = inspector
                .child(section("Hex", cx))
                .child(mono(hex(&block.data), cx))
                .child(section("ASCII", cx))
                .child(mono(ascii(&block.data), cx));
            if !is_trailer && let Ok(value) = nfc_classic::value::decode(block.data) {
                inspector = inspector.child(section("Value block", cx)).child(mono(
                    format!("value {} · address {}", value.value, value.address),
                    cx,
                ));
            }
        } else if let Some(error) = &block.error {
            inspector = inspector.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }

        if let Ok(access) = dump.access_bits(block.sector) {
            let group = dump.layout.access_group_for_block(number).unwrap_or(3);
            let condition = access.groups[group];
            inspector = inspector
                .child(section("Access", cx))
                .child(mono(access_line(group, condition), cx));
        }

        inspector = inspector
            .child(
                div()
                    .mt_3()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Write block"),
            )
            .child(Input::new(&self.block_input).id("classic-block-data"))
            .child(
                Button::new("classic-write-block")
                    .label(if is_trailer {
                        "Write trailer…"
                    } else {
                        "Write block"
                    })
                    .when(is_trailer, |button| button.danger())
                    .disabled(number == 0 || self.busy.is_some())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_write_selected(window, cx);
                    })),
            );
        if number == 0 {
            inspector = inspector.child(muted("Block 0 is read-only in this build.", cx));
        }
        inspector.into_any_element()
    }

    fn render_diff(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(diff) = &self.diff else {
            return empty_state(
                "No comparison yet",
                "Read or load two card images to see changed blocks.",
                cx,
            );
        };
        let mut content = v_flex()
            .gap_3()
            .p_4()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .flex_wrap()
                    .child(div().text_lg().child(diff.layout.name().to_owned()))
                    .child(muted(
                        format!(
                            "{} changed · {} equal · {} unknown · {} trailer changes",
                            diff.changed_blocks,
                            diff.equal_blocks,
                            diff.unknown_blocks,
                            diff.trailer_changes
                        ),
                        cx,
                    )),
            )
            .child(if diff.identity_changed {
                div()
                    .text_sm()
                    .text_color(cx.theme().warning)
                    .child("Card identity changed between the two images.")
                    .into_any_element()
            } else {
                div().into_any_element()
            });

        let changed: Vec<_> = diff
            .blocks
            .iter()
            .filter(|block| block.state != nfc_classic::DiffState::Equal)
            .collect();
        if changed.is_empty() {
            return content
                .child(muted("The two images are identical.", cx))
                .into_any_element();
        }
        let rows = changed
            .iter()
            .map(|block| {
                TableRow::new()
                    .child(TableCell::new().child(format!("{}", block.block)))
                    .child(TableCell::new().child(format!("{}", block.sector)))
                    .child(TableCell::new().child(match block.state {
                        nfc_classic::DiffState::Equal => "Equal",
                        nfc_classic::DiffState::Changed => "Changed",
                        nfc_classic::DiffState::Unknown => "Unknown",
                    }))
                    .child(TableCell::new().child(diff_detail(block, cx)))
            })
            .collect::<Vec<_>>();
        content = content.child(
            div()
                .id("classic-diff-scroll")
                .test_support()
                .overflow_y_scroll()
                .child(
                    Table::new()
                        .small()
                        .accessibility_label("Changed blocks")
                        .child(
                            TableHeader::new().child(
                                TableRow::new()
                                    .child(TableHead::new().child("Block"))
                                    .child(TableHead::new().child("Sector"))
                                    .child(TableHead::new().child("State"))
                                    .child(TableHead::new().child("Detail")),
                            ),
                        )
                        .child(TableBody::new().children(rows)),
                ),
        );
        content.into_any_element()
    }

    fn render_keys(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self
            .keys
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                TableRow::new()
                    .child(TableCell::new().child(entry.sector_label()))
                    .child(TableCell::new().child(entry.kind.label().to_owned()))
                    .child(TableCell::new().child(mono(entry.key.to_hex(), cx)))
                    .child(TableCell::new().child(entry.source.label().to_owned()))
                    .child(
                        TableCell::new().child(
                            Button::new(("classic-remove-key", index))
                                .small()
                                .ghost()
                                .label("Remove")
                                .disabled(self.busy.is_some())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_key(index, cx);
                                })),
                        ),
                    )
            })
            .collect::<Vec<_>>();

        let mut content = v_flex()
            .gap_3()
            .p_4()
            .child(div().text_lg().child("Key store"))
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .flex_wrap()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(label("Key"))
                            .child(Input::new(&self.key_hex_input).id("classic-key-hex").w_48()),
                    )
                    .child(
                        v_flex().gap_1().child(label("Sector")).child(
                            Input::new(&self.key_sector_input)
                                .id("classic-key-sector")
                                .w_48(),
                        ),
                    )
                    .child(
                        v_flex().gap_1().child(label("Type")).child(
                            RadioGroup::new("classic-key-kind")
                                .children(["Key A", "Key B", "Any"])
                                .selected_index(Some(self.key_kind))
                                .on_change(cx.listener(|this, index, _, cx| {
                                    this.key_kind = *index;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        Button::new("classic-add-key")
                            .primary()
                            .label("Add key")
                            .disabled(self.busy.is_some())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_key(window, cx);
                            })),
                    ),
            );

        if rows.is_empty() {
            content = content.child(muted("No stored keys. Add a key or scan the card.", cx));
        } else {
            content = content.child(
                div().id("classic-keys-scroll").overflow_y_scroll().child(
                    Table::new()
                        .small()
                        .accessibility_label("Stored keys")
                        .child(
                            TableHeader::new().child(
                                TableRow::new()
                                    .child(TableHead::new().child("Sector"))
                                    .child(TableHead::new().child("Type"))
                                    .child(TableHead::new().child("Key"))
                                    .child(TableHead::new().child("Source"))
                                    .child(TableHead::new().child("Action")),
                            ),
                        )
                        .child(TableBody::new().children(rows)),
                ),
            );
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
                    .id("classic-status")
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

fn ndef_record_row(index: usize, record: &NdefRecord, cx: &App) -> AnyElement {
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
        .id(("classic-ndef-record", index))
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

fn diff_detail(block: &nfc_classic::BlockDiff, cx: &App) -> AnyElement {
    if let Some(trailer) = &block.trailer {
        let mut parts = Vec::new();
        if trailer.key_a_changed {
            parts.push("Key A".to_owned());
        }
        if trailer.key_b_changed {
            parts.push("Key B".to_owned());
        }
        if trailer.access_changed {
            parts.push("access bits".to_owned());
        }
        if trailer.gpb_changed {
            parts.push("GPB".to_owned());
        }
        if !trailer.valid {
            parts.push("invalid trailer".to_owned());
        }
        return mono(
            if parts.is_empty() {
                "trailer changed".to_owned()
            } else {
                parts.join(", ")
            },
            cx,
        );
    }
    if block.bytes.is_empty() {
        return mono("known state changed".to_owned(), cx);
    }
    mono(
        block
            .bytes
            .iter()
            .take(6)
            .map(|byte| {
                format!(
                    "[{:02X}] {} → {}",
                    byte.offset,
                    if byte.before_known {
                        format!("{:02X}", byte.before)
                    } else {
                        "??".to_owned()
                    },
                    if byte.after_known {
                        format!("{:02X}", byte.after)
                    } else {
                        "??".to_owned()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("  "),
        cx,
    )
}

fn access_line(group: usize, condition: AccessCondition) -> String {
    if group == 3 {
        let permissions = condition.trailer_access();
        format!(
            "group 3 · C1 {} C2 {} C3 {} · read {} · trailer write {}",
            condition.c1 as u8,
            condition.c2 as u8,
            condition.c3 as u8,
            key_mask(permissions.read_keys),
            key_mask(condition.full_trailer_write_keys())
        )
    } else {
        format!(
            "group {group} · C1 {} C2 {} C3 {} · read {} · write {}",
            condition.c1 as u8,
            condition.c2 as u8,
            condition.c3 as u8,
            key_mask(condition.data_read_keys()),
            key_mask(condition.data_write_keys())
        )
    }
}

fn key_mask(mask: KeyMask) -> &'static str {
    match mask {
        KeyMask::ANY => "A or B",
        KeyMask::A => "A",
        KeyMask::B => "B",
        _ => "none",
    }
}

fn block_status(status: BlockStatus) -> &'static str {
    match status {
        BlockStatus::Unread => "unread",
        BlockStatus::Read => "read",
        BlockStatus::AuthFailed => "auth failed",
        BlockStatus::ReadFailed => "read failed",
        BlockStatus::Synthetic => "synthetic",
        BlockStatus::Verified => "verified",
    }
}

fn ascii(bytes: &[u8]) -> String {
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
