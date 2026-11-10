//! Rendering for the Classic screens.

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::Input,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, IntoElement, Role, TestSupportExt as _, div, px,
};
use nfc_app::ClassicNdefState;
use nfc_classic::{AccessCondition, BlockStatus, KeyMask};
use nfc_ndef::{NdefRecord, TextRecord, UriRecord};
use nfc_ui::{
    Key, StatusKind, ascii, byte_grid, callout, callout_with_id, card_head, empty_state, field,
    hex_bytes, inspector, kv_row, metric, page_header, pill, section_title, text_value,
};

use crate::model::{
    DUMP_FORMATS, NdefSource, STRATEGIES, dump_format_key, strategy_description_key, strategy_key,
};
use crate::workspace::ClassicWorkspace;

impl ClassicWorkspace {
    pub fn render_blocks(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let subtitle = self
            .dump
            .as_ref()
            .map(|dump| {
                let readability = dump.readability();
                format!(
                    "{} · {}/{} {} · {}/{} {}",
                    dump.layout.name(),
                    readability.readable_blocks,
                    readability.total_blocks,
                    translator.text(Key::BlocksUnit),
                    readability.readable_sectors,
                    readability.total_sectors,
                    translator.text(Key::SectorsUnit)
                )
            })
            .unwrap_or_else(|| translator.text(Key::BlocksSubtitle).to_owned());
        let header = page_header(
            cx,
            translator,
            Key::EyebrowClassicMemory,
            Key::BlocksTitle,
            subtitle,
            Vec::new(),
        );
        let mut page = v_flex()
            .size_full()
            .child(header)
            .child(self.render_file_row(cx));
        let Some(dump) = &self.dump else {
            return page
                .child(empty_state(
                    cx,
                    translator,
                    Key::NoCardTitle,
                    Key::NoCardBody,
                ))
                .into_any_element();
        };
        page = page.child(self.render_metrics(cx, dump));
        let workbench = h_flex()
            .items_start()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(self.render_block_table(cx, dump))
            .child(self.render_block_inspector(cx));
        page.child(workbench).into_any_element()
    }

    fn render_file_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let busy = self.is_busy(cx);
        h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .pb_3()
            .child(Input::new(&self.path_input).id("classic-path").w(px(260.)))
            .children(DUMP_FORMATS.iter().enumerate().map(|(index, format)| {
                format_button(
                    ("classic-format", index),
                    translator.text(dump_format_key(*format)),
                    index == self.dump_format,
                    cx.listener(move |this, _, _, cx| this.select_dump_format(index, cx)),
                )
            }))
            .child(
                Button::new("classic-save")
                    .small()
                    .label(translator.text(Key::SaveDump))
                    .disabled(busy || self.dump.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.save_dump(cx))),
            )
            .child(
                Button::new("classic-load")
                    .small()
                    .label(translator.text(Key::LoadDump))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.load_dump(window, cx))),
            )
            .child(
                Button::new("classic-restore")
                    .small()
                    .danger()
                    .label(translator.text(Key::RestoreTag))
                    .disabled(busy || self.dump.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.restore_loaded(window, cx))),
            )
            .child(
                Checkbox::new("classic-restore-continue")
                    .checked(self.restore_continue)
                    .label(translator.text(Key::ContinueOnError))
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.restore_continue = *checked;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_metrics(&self, cx: &mut Context<Self>, dump: &nfc_classic::Dump) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let readability = dump.readability();
        let identity = dump
            .card
            .as_ref()
            .map(|card| card.uid_hex())
            .unwrap_or_else(|| "—".to_owned());
        let last_backup = self
            .session
            .read(cx)
            .last_backup
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| translator.text(Key::NoBackup).to_owned());
        h_flex()
            .gap_2()
            .flex_wrap()
            .pb_3()
            .child(metric(
                cx,
                translator.text(Key::Readability),
                format!(
                    "{}/{}",
                    readability.readable_blocks, readability.total_blocks
                ),
                format!(
                    "{}/{} {}",
                    readability.readable_sectors,
                    readability.total_sectors,
                    translator.text(Key::SectorsUnit)
                ),
            ))
            .child(metric(
                cx,
                translator.text(Key::CardIdentity),
                identity,
                dump.card
                    .as_ref()
                    .map(|card| {
                        format!(
                            "ATQA {:02X} {:02X} · SAK {:02X}",
                            card.atqa[0], card.atqa[1], card.sak
                        )
                    })
                    .unwrap_or_default(),
            ))
            .child(metric(
                cx,
                translator.text(Key::KeyStoreTitle),
                self.keys.len().to_string(),
                translator.text(Key::KeysUnit),
            ))
            .child(metric(
                cx,
                translator.text(Key::LastBackup),
                last_backup,
                "",
            ))
            .into_any_element()
    }

    fn render_block_table(&self, cx: &mut Context<Self>, dump: &nfc_classic::Dump) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let query = self
            .block_search
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let filter = self.block_filter;
        let rows: Vec<&nfc_classic::Block> = dump
            .blocks
            .iter()
            .filter(|block| {
                let is_trailer = dump.layout.is_trailer(block.number).unwrap_or(false);
                let matches_filter = match filter {
                    1 => block.number != 0 && !is_trailer,
                    2 => is_trailer,
                    3 => block.number == 0,
                    _ => true,
                };
                if !matches_filter {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                let data = if block.complete() {
                    hex_bytes(&block.data)
                } else {
                    String::new()
                };
                format!(
                    "{} {} {} {}",
                    block.number,
                    block.sector,
                    block_status(block.status, translator),
                    data
                )
                .to_ascii_lowercase()
                .contains(&query)
            })
            .collect();

        let mut body = v_flex().gap_1().p_2();
        for block in &rows {
            body = body.child(self.render_block_row(cx, dump, block));
        }
        if rows.is_empty() {
            body = body.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::NoDumpBody)),
            );
        }

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(translator.text(Key::CardMemory)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(translator.text(Key::CardMemorySub)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Input::new(&self.block_search)
                            .id("classic-block-search")
                            .small()
                            .w(px(120.)),
                    )
                    .children(
                        [
                            Key::AllTypes,
                            Key::DataType,
                            Key::Trailers,
                            Key::ManufacturerType,
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, key)| {
                            format_button(
                                ("classic-filter", index),
                                translator.text(key),
                                index == filter,
                                cx.listener(move |this, _, _, cx| {
                                    this.select_block_filter(index, cx)
                                }),
                            )
                        }),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().w(px(44.)).child(translator.text(Key::BlockLabel)))
                    .child(div().w(px(44.)).child(translator.text(Key::SectorLabel)))
                    .child(div().w(px(32.)).child(translator.text(Key::TypeLabel)))
                    .child(div().w(px(74.)).child(translator.text(Key::StatusLabel)))
                    .child(div().w(px(48.)).child(translator.text(Key::KnownLabel)))
                    .child(div().flex_1().child(translator.text(Key::DataLabel))),
            )
            .child(
                div()
                    .id("classic-block-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(legend(
                        cx,
                        StatusKind::Success,
                        translator.text(Key::ReadLegend),
                    ))
                    .child(legend(
                        cx,
                        StatusKind::Warning,
                        translator.text(Key::UnreadableLegend),
                    ))
                    .child(legend(
                        cx,
                        StatusKind::Info,
                        translator.text(Key::UnknownLegend),
                    ))
                    .child(div().flex_1())
                    .child(format!(
                        "{} {}",
                        rows.len(),
                        translator.text(Key::BlockCount)
                    )),
            )
            .into_any_element()
    }

    fn render_block_row(
        &self,
        cx: &mut Context<Self>,
        dump: &nfc_classic::Dump,
        block: &nfc_classic::Block,
    ) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let selected = self.selected_block == Some(block.number);
        let is_trailer = dump.layout.is_trailer(block.number).unwrap_or(false);
        let mark = if block.number == 0 {
            "M"
        } else if is_trailer {
            "T"
        } else {
            "D"
        };
        let number = block.number;
        let status = block_status(block.status, translator);
        let known = block.known_mask.count_ones();
        let data = if block.complete() {
            hex_bytes(&block.data)
        } else {
            "—".to_owned()
        };
        h_flex()
            .id(("classic-block", number))
            .test_support()
            .role(Role::Button)
            .aria_label(format!(
                "{} {number} · {} {}",
                translator.text(Key::BlockLabel),
                translator.text(Key::SectorLabel),
                block.sector
            ))
            .gap_2()
            .items_center()
            .px_2()
            .py_1()
            .rounded_sm()
            .when(selected, |row| row.bg(cx.theme().accent))
            .when(!selected, |row| {
                row.hover(|row| row.bg(cx.theme().secondary))
            })
            .text_xs()
            .cursor_pointer()
            .child(div().w(px(44.)).child(format!("{:02}", block.number)))
            .child(div().w(px(44.)).child(block.sector.to_string()))
            .child(div().w(px(32.)).child(mark))
            .child(div().w(px(74.)).child(pill(
                cx,
                match block.status {
                    BlockStatus::Read | BlockStatus::Verified => StatusKind::Success,
                    BlockStatus::Unread | BlockStatus::Synthetic => StatusKind::Info,
                    _ => StatusKind::Warning,
                },
                status,
            )))
            .child(div().w(px(48.)).child(format!("{known}/16")))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(data),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_block(number, window, cx);
            }))
            .into_any_element()
    }

    fn render_block_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let Some(dump) = &self.dump else {
            return div().into_any_element();
        };
        let Some(number) = self.selected_block else {
            return div()
                .w(px(360.))
                .child(empty_state(
                    cx,
                    translator,
                    Key::SelectBlock,
                    Key::NoDumpBody,
                ))
                .into_any_element();
        };
        let Some(block) = dump.blocks.get(number) else {
            return div().into_any_element();
        };
        let is_trailer = dump.layout.is_trailer(number).unwrap_or(false);
        let kind = if number == 0 {
            translator.text(Key::ManufacturerType)
        } else if is_trailer {
            translator.text(Key::Trailers)
        } else {
            translator.text(Key::DataType)
        };
        let mut body = v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(pill(
                        cx,
                        StatusKind::Info,
                        format!("{} {}", translator.text(Key::SectorLabel), block.sector),
                    ))
                    .child(pill(
                        cx,
                        StatusKind::Info,
                        format!(
                            "{}/16 {}",
                            block.known_mask.count_ones(),
                            translator.text(Key::KnownBytes)
                        ),
                    ))
                    .when(number == 0, |body| {
                        body.child(pill(cx, StatusKind::Error, translator.text(Key::ReadOnly)))
                    }),
            )
            .child(section_title(cx, translator.text(Key::InspectorHex)));
        if block.complete() {
            body = body
                .child(byte_grid(cx, &block.data, 8))
                .child(section_title(cx, translator.text(Key::InspectorAscii)))
                .child(
                    div()
                        .rounded_sm()
                        .bg(cx.theme().secondary)
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(ascii(&block.data)),
                );
        } else {
            body = body.child(
                div().text_xs().text_color(cx.theme().danger).child(
                    block
                        .error
                        .clone()
                        .unwrap_or_else(|| block_status(block.status, translator).to_owned()),
                ),
            );
        }
        if block.complete()
            && !is_trailer
            && let Ok(value) = nfc_classic::value::decode(block.data)
        {
            body = body
                .child(section_title(cx, translator.text(Key::ValueBlockTitle)))
                .child(kv_row(
                    cx,
                    translator.text(Key::ValueLabel),
                    value.value.to_string(),
                ))
                .child(kv_row(
                    cx,
                    translator.text(Key::AddressLabel),
                    value.address.to_string(),
                ));
        }
        if is_trailer && block.complete() {
            body = body
                .child(section_title(cx, translator.text(Key::TrailerFields)))
                .child(kv_row(
                    cx,
                    translator.text(Key::KeyTypeA),
                    hex_bytes(&block.data[0..6]),
                ))
                .child(kv_row(
                    cx,
                    translator.text(Key::AccessLabel),
                    hex_bytes(&block.data[6..10]),
                ))
                .child(kv_row(
                    cx,
                    translator.text(Key::KeyTypeB),
                    hex_bytes(&block.data[10..16]),
                ));
        }
        if let Ok(access) = dump.access_bits(block.sector) {
            let group = dump.layout.access_group_for_block(number).unwrap_or(3);
            let condition = access.groups[group];
            body = body
                .child(section_title(cx, translator.text(Key::AccessConditions)))
                .child(kv_row(
                    cx,
                    translator.text(Key::AccessRead),
                    access_line(group, condition, false),
                ))
                .child(kv_row(
                    cx,
                    translator.text(Key::AccessWrite),
                    access_line(group, condition, true),
                ));
        }
        if number == 0 {
            body = body.child(callout(
                cx,
                StatusKind::Warning,
                translator.text(Key::BlockZeroReadOnly),
            ));
        }
        let busy = self.is_busy(cx);
        let actions = v_flex()
            .gap_2()
            .child(field(
                cx,
                translator.text(Key::NewBlockData),
                None,
                Input::new(&self.block_input)
                    .id("classic-block-data")
                    .into_any_element(),
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("classic-read-block")
                            .small()
                            .label(translator.text(Key::ReadBlock))
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.read_block(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("classic-write-block")
                            .small()
                            .when(is_trailer, |button| button.danger())
                            .when(!is_trailer, |button| button.primary())
                            .label(if is_trailer {
                                translator.text(Key::WriteTrailer)
                            } else {
                                translator.text(Key::WriteBlock)
                            })
                            .disabled(busy || number == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.write_selected_block(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::ReadBackPlan)),
            )
            .into_any_element();
        div()
            .id("classic-inspector")
            .test_support()
            .w(px(360.))
            .h_full()
            .flex_shrink_0()
            .role(Role::Group)
            .aria_label(format!("Block {number} · sector {} · {kind}", block.sector))
            .child(inspector(
                cx,
                translator.fmt2(Key::BlockKicker, number, block.sector),
                kind,
                Some(pill(
                    cx,
                    if is_trailer {
                        StatusKind::Warning
                    } else {
                        StatusKind::Success
                    },
                    if is_trailer {
                        translator.text(Key::Trailers)
                    } else {
                        translator.text(Key::ReadLegend)
                    },
                )),
                body.into_any_element(),
                Some(actions),
            ))
            .into_any_element()
    }

    pub fn render_ndef(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let busy = self.is_busy(cx);
        let header = page_header(
            cx,
            translator,
            Key::EyebrowClassicNdef,
            Key::ClassicNdefTitle,
            self.ndef
                .as_ref()
                .and_then(|info| info.layout.as_ref())
                .map(|layout| {
                    format!(
                        "MAD {}.{} · {} NFC {}",
                        layout.version.major,
                        layout.version.minor,
                        layout.sectors.len(),
                        translator.text(Key::SectorsUnit)
                    )
                })
                .unwrap_or_else(|| translator.text(Key::ClassicNdefSubtitle).to_owned()),
            vec![],
        );
        let page = v_flex()
            .size_full()
            .child(header)
            .child(self.render_file_row(cx));
        let Some(info) = &self.ndef else {
            return page
                .child(empty_state(
                    cx,
                    translator,
                    Key::NoCardTitle,
                    Key::NoCardBody,
                ))
                .into_any_element();
        };
        let writable = info
            .layout
            .as_ref()
            .is_some_and(|layout| layout.writable && !layout.read_only);
        let formatted = info
            .layout
            .as_ref()
            .is_some_and(|layout| !layout.sectors.is_empty());
        let read_only = info.layout.as_ref().is_some_and(|layout| layout.read_only);

        let message = match &info.ndef.state {
            ClassicNdefState::Unformatted => v_flex().gap_2().child(callout(
                cx,
                StatusKind::Warning,
                translator.text(Key::NdefUnformatted),
            )),
            ClassicNdefState::Empty => v_flex().gap_2().child(div().text_sm().child(format!(
                "{} · {} {}",
                translator.text(Key::NdefEmpty),
                info.ndef.capacity,
                translator.text(Key::BytesUnit)
            ))),
            ClassicNdefState::Message(message) => {
                let mut list = v_flex().gap_2();
                for (index, record) in message.records().iter().enumerate() {
                    list = list.child(record_row(cx, index, record, translator));
                }
                list
            }
        };
        let used = info.ndef.message_bytes;
        let capacity = info.ndef.capacity.max(1);
        let percent = (used.saturating_mul(100) / capacity).min(100);
        let mut left = v_flex().gap_3().flex_1().min_w_0().child(
            v_flex()
                .gap_2()
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .rounded_lg()
                .overflow_hidden()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(translator.text(Key::NdefMessageTitle)),
                        )
                        .child(div().flex_1())
                        .child(pill(
                            cx,
                            if read_only {
                                StatusKind::Warning
                            } else {
                                StatusKind::Success
                            },
                            if read_only {
                                translator.text(Key::NdefReadOnly)
                            } else if writable {
                                translator.text(Key::NdefReadWrite)
                            } else {
                                translator.text(Key::NdefNotWritable)
                            },
                        )),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .p_4()
                        .child(
                            div()
                                .h(px(7.))
                                .w_full()
                                .rounded_full()
                                .bg(cx.theme().secondary)
                                .child(
                                    div()
                                        .h(px(7.))
                                        .w(px(percent as f32 * 3.0))
                                        .rounded_full()
                                        .bg(cx.theme().primary),
                                ),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{used} {}", translator.text(Key::NdefUsed)))
                                .child(format!(
                                    "{capacity} {}",
                                    translator.text(Key::CapacityBytes)
                                )),
                        )
                        .child(message)
                        .child(callout(cx, StatusKind::Info, translator.text(Key::TlvInfo))),
                ),
        );
        if let Some(layout) = &info.layout {
            left =
                left.child(
                    v_flex()
                        .gap_2()
                        .bg(cx.theme().popover)
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded_lg()
                        .overflow_hidden()
                        .child(card_head(
                            cx,
                            translator.text(Key::MappingTitle),
                            "MAD and NFC sector facts",
                        ))
                        .child(
                            v_flex()
                                .gap_1()
                                .p_4()
                                .child(kv_row(
                                    cx,
                                    translator.text(Key::MappingVersion),
                                    format!("{}.{}", layout.version.major, layout.version.minor),
                                ))
                                .child(kv_row(
                                    cx,
                                    translator.text(Key::NfcSectors),
                                    layout
                                        .sectors
                                        .iter()
                                        .map(u8::to_string)
                                        .collect::<Vec<_>>()
                                        .join(", "),
                                ))
                                .child(kv_row(
                                    cx,
                                    translator.text(Key::DataBlocksLabel),
                                    format!(
                                        "{} · {} {}",
                                        layout.data_blocks.len(),
                                        layout.data_len(),
                                        translator.text(Key::BytesUnit)
                                    ),
                                ))
                                .child(kv_row(
                                    cx,
                                    translator.text(Key::AccessLabel),
                                    if layout.read_only {
                                        translator.text(Key::NdefReadOnly)
                                    } else if layout.writable {
                                        translator.text(Key::NdefReadWrite)
                                    } else {
                                        translator.text(Key::NdefNotWritable)
                                    },
                                ))
                                .children(layout.warnings.iter().map(|warning| {
                                    callout(cx, StatusKind::Warning, warning.clone())
                                })),
                        ),
                );
        }
        if let Some(error) = &self.ndef_error {
            left = left.child(callout(cx, StatusKind::Error, error.clone()));
        }

        let source = self.ndef_source;
        let write_card = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::WriteNdef),
                translator.text(Key::ClassicNdefWriteNote),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(
                        h_flex()
                            .gap_1()
                            .children(NdefSource::ALL.iter().enumerate().map(|(index, choice)| {
                                format_button(
                                    ("classic-ndef-source", index),
                                    translator.text(choice.key()),
                                    *choice == source,
                                    cx.listener(move |this, _, _, cx| {
                                        this.select_ndef_source(index, cx);
                                    }),
                                )
                            })),
                    )
                    .child(match source {
                        NdefSource::Text => v_flex()
                            .gap_2()
                            .child(field(
                                cx,
                                translator.text(Key::FieldValue),
                                None,
                                Input::new(&self.ndef_value_input)
                                    .id("classic-ndef-text")
                                    .into_any_element(),
                            ))
                            .child(field(
                                cx,
                                translator.text(Key::FieldLanguage),
                                None,
                                Input::new(&self.ndef_language_input)
                                    .id("classic-ndef-language")
                                    .into_any_element(),
                            ))
                            .into_any_element(),
                        NdefSource::Uri => field(
                            cx,
                            translator.text(Key::FieldValue),
                            None,
                            Input::new(&self.ndef_value_input)
                                .id("classic-ndef-uri")
                                .into_any_element(),
                        ),
                        NdefSource::File => field(
                            cx,
                            translator.text(Key::FieldPath),
                            None,
                            Input::new(&self.ndef_file_input)
                                .id("classic-ndef-file")
                                .into_any_element(),
                        ),
                    })
                    .child(
                        Button::new("classic-ndef-write")
                            .primary()
                            .label(translator.text(Key::WriteNdef))
                            .disabled(busy || !writable || !formatted)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.write_ndef(window, cx)),
                            ),
                    ),
            );
        let export_card = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::ExportMessage),
                translator.text(Key::MessageBytes),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(
                        Input::new(&self.ndef_export_input)
                            .id("classic-ndef-export")
                            .into_any_element(),
                    )
                    .child(
                        Button::new("classic-ndef-export-action")
                            .label(translator.text(Key::Export))
                            .disabled(busy || self.ndef.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.export_ndef(cx))),
                    ),
            );
        let format_card = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::FormatCard),
                translator.text(Key::AdvancedActions),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(
                        Checkbox::new("classic-ndef-read-only")
                            .checked(self.format_read_only)
                            .label(translator.text(Key::FormatReadOnly))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_format_read_only(cx);
                            })),
                    )
                    .child(callout(
                        cx,
                        StatusKind::Warning,
                        translator.text(Key::FormatClassicNote),
                    ))
                    .child(
                        Button::new("classic-ndef-format")
                            .danger()
                            .label(translator.text(Key::FormatCard))
                            .disabled(busy || read_only || self.dump.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_format(window, cx);
                            })),
                    ),
            );
        page.child(
            h_flex()
                .items_start()
                .gap_3()
                .flex_1()
                .min_h_0()
                .child(left)
                .child(
                    v_flex()
                        .w(px(360.))
                        .flex_shrink_0()
                        .gap_3()
                        .child(write_card)
                        .child(export_card)
                        .child(format_card),
                ),
        )
        .into_any_element()
    }

    pub fn render_diff(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let header = page_header(
            cx,
            translator,
            Key::EyebrowClassicCompare,
            Key::DiffTitle,
            translator.text(Key::DiffSubtitle),
            vec![action(
                "classic-compare",
                translator.text(Key::Compare),
                false,
                true,
                cx.listener(|this, _, _, cx| this.compare_files(cx)),
            )],
        );
        let page = v_flex().size_full().child(header).child(
            h_flex()
                .gap_2()
                .items_center()
                .flex_wrap()
                .pb_3()
                .child(
                    Input::new(&self.diff_before_input)
                        .id("classic-diff-before")
                        .w(px(260.)),
                )
                .child(
                    Input::new(&self.diff_after_input)
                        .id("classic-diff-after")
                        .w(px(260.)),
                )
                .child(
                    Button::new("classic-compare-action")
                        .small()
                        .label(translator.text(Key::Compare))
                        .on_click(cx.listener(|this, _, _, cx| this.compare_files(cx))),
                ),
        );
        let Some(diff) = self.diff() else {
            return page
                .child(empty_state(
                    cx,
                    translator,
                    Key::NoDiffTitle,
                    Key::NoDiffBody,
                ))
                .into_any_element();
        };
        let changed: Vec<_> = diff
            .blocks
            .iter()
            .filter(|block| block.state != nfc_classic::DiffState::Equal)
            .collect();
        let mut table = v_flex().gap_1().p_2();
        for block in &changed {
            table = table.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .child(div().w(px(44.)).child(format!("{:02}", block.block)))
                    .child(div().w(px(44.)).child(block.sector.to_string()))
                    .child(div().w(px(70.)).child(match block.state {
                        nfc_classic::DiffState::Changed => {
                            pill(cx, StatusKind::Warning, translator.text(Key::StateChanged))
                        }
                        nfc_classic::DiffState::Unknown => {
                            pill(cx, StatusKind::Info, translator.text(Key::StateUnknown))
                        }
                        nfc_classic::DiffState::Equal => {
                            pill(cx, StatusKind::Success, translator.text(Key::StateEqual))
                        }
                    }))
                    .child(div().flex_1().child(diff_detail(cx, block))),
            );
        }
        let summary = v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(summary_tile(
                        cx,
                        translator.text(Key::ChangedBlocks),
                        diff.changed_blocks,
                        StatusKind::Warning,
                    ))
                    .child(summary_tile(
                        cx,
                        translator.text(Key::EqualBlocks),
                        diff.equal_blocks,
                        StatusKind::Success,
                    ))
                    .child(summary_tile(
                        cx,
                        translator.text(Key::UnknownBlocks),
                        diff.unknown_blocks,
                        StatusKind::Info,
                    ))
                    .child(summary_tile(
                        cx,
                        translator.text(Key::TrailerChanges),
                        diff.trailer_changes,
                        StatusKind::Warning,
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(translator.text(Key::DiffSummary)),
                    )
                    .child(div().flex_1())
                    .child(pill(
                        cx,
                        if diff.identity_changed {
                            StatusKind::Warning
                        } else {
                            StatusKind::Success
                        },
                        if diff.identity_changed {
                            translator.text(Key::IdentityChanged)
                        } else {
                            translator.text(Key::IdentityUnchanged)
                        },
                    )),
            )
            .child(
                v_flex()
                    .gap_1()
                    .p_4()
                    .child(kv_row(
                        cx,
                        translator.text(Key::LayoutLabel),
                        diff.layout.name(),
                    ))
                    .child(callout(
                        cx,
                        StatusKind::Info,
                        translator.text(Key::DiffDetail),
                    )),
            );
        page.child(
            h_flex()
                .items_start()
                .gap_3()
                .flex_1()
                .min_h_0()
                .child(
                    v_flex().flex_1().min_w_0().gap_3().child(summary).child(
                        v_flex()
                            .bg(cx.theme().popover)
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_lg()
                            .overflow_hidden()
                            .child(card_head(
                                cx,
                                translator.text(Key::ChangedBlocks),
                                translator.text(Key::OnlyDifferences),
                            ))
                            .child(
                                div()
                                    .id("classic-diff-scroll")
                                    .test_support()
                                    .max_h(px(420.))
                                    .overflow_y_scroll()
                                    .child(table),
                            ),
                    ),
                )
                .child(
                    v_flex()
                        .w(px(300.))
                        .flex_shrink_0()
                        .bg(cx.theme().popover)
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded_lg()
                        .child(card_head(
                            cx,
                            translator.text(Key::DiffSummary),
                            diff.layout.name(),
                        ))
                        .child(
                            v_flex()
                                .gap_1()
                                .p_4()
                                .children(self.previous_dump.as_ref().map(|before| {
                                    kv_row(
                                        cx,
                                        translator.text(Key::Before),
                                        before
                                            .card
                                            .as_ref()
                                            .map(|card| card.uid_hex())
                                            .unwrap_or_else(|| "—".to_owned()),
                                    )
                                }))
                                .children(self.dump.as_ref().map(|after| {
                                    kv_row(
                                        cx,
                                        translator.text(Key::After),
                                        after
                                            .card
                                            .as_ref()
                                            .map(|card| card.uid_hex())
                                            .unwrap_or_else(|| "—".to_owned()),
                                    )
                                })),
                        ),
                ),
        )
        .into_any_element()
    }

    pub fn render_keys(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let busy = self.is_busy(cx);
        let header = page_header(
            cx,
            translator,
            Key::EyebrowClassicKeys,
            Key::KeysTitle,
            translator.text(Key::KeysSubtitle),
            vec![action(
                "classic-keys-import-action",
                translator.text(Key::ImportKeys),
                busy,
                false,
                cx.listener(|this, _, _, cx| this.import_keys(cx)),
            )],
        );
        let mut rows = v_flex().gap_1().p_2();
        if self.keys.is_empty() {
            rows = rows.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::KeyStoreSub)),
            );
        } else {
            for (index, entry) in self.keys.entries().iter().enumerate() {
                let key = if self.reveal_keys {
                    entry.key.to_hex()
                } else {
                    translator.text(Key::KeyMasked).to_owned()
                };
                rows = rows.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .child(div().w(px(80.)).child(entry.sector_label()))
                        .child(div().w(px(52.)).child(entry.kind.label()))
                        .child(
                            div()
                                .flex_1()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(key),
                        )
                        .child(div().w(px(64.)).child(entry.source.label()))
                        .child(
                            Button::new(("classic-remove-key", index))
                                .xsmall()
                                .ghost()
                                .label(translator.text(Key::Remove))
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_key(index, cx);
                                })),
                        ),
                );
            }
        }
        let table = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(translator.text(Key::KeyStoreTitle)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(translator.text(Key::KeyStoreSub)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("classic-reveal-keys")
                            .xsmall()
                            .outline()
                            .label(if self.reveal_keys {
                                translator.text(Key::HideKeys)
                            } else {
                                translator.text(Key::RevealKeys)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_reveal(cx))),
                    )
                    .children(
                        [Key::KeyTypesAB, Key::KeyTypesA, Key::KeyTypesB]
                            .into_iter()
                            .enumerate()
                            .map(|(index, key)| {
                                format_button(
                                    ("classic-scan-types", index),
                                    translator.text(key),
                                    self.scan_types.key() == key,
                                    cx.listener(move |this, _, _, cx| {
                                        this.select_scan_types(index, cx);
                                    }),
                                )
                            }),
                    )
                    .child(
                        Button::new("classic-scan-keys")
                            .xsmall()
                            .label(translator.text(Key::ScanKeys))
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.scan_keys(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("classic-keys-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::KeyScanNote))
                    .child(div().flex_1())
                    .child(format!(
                        "{} {}",
                        self.keys.len(),
                        translator.text(Key::KeyEntryCount)
                    )),
            );
        let add_card = v_flex()
            .gap_2()
            .w(px(320.))
            .flex_shrink_0()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::AddKey),
                translator.text(Key::AddKeyNote),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(field(
                        cx,
                        translator.text(Key::FieldKey),
                        None,
                        Input::new(&self.key_hex_input)
                            .id("classic-key-hex")
                            .into_any_element(),
                    ))
                    .child(field(
                        cx,
                        translator.text(Key::FieldSector),
                        None,
                        Input::new(&self.key_sector_input)
                            .id("classic-key-sector")
                            .into_any_element(),
                    ))
                    .child(
                        h_flex().gap_1().children(
                            [Key::KeyTypeA, Key::KeyTypeB, Key::TypeAny]
                                .into_iter()
                                .enumerate()
                                .map(|(index, key)| {
                                    format_button(
                                        ("classic-key-kind", index),
                                        translator.text(key),
                                        self.key_kind == index,
                                        cx.listener(move |this, _, _, cx| {
                                            this.select_key_kind(index, cx);
                                        }),
                                    )
                                }),
                        ),
                    )
                    .child(
                        Button::new("classic-add-key")
                            .primary()
                            .label(translator.text(Key::AddKey))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.add_key(window, cx))),
                    )
                    .child(section_title(cx, translator.text(Key::ImportKeys)))
                    .child(
                        Input::new(&self.import_input)
                            .id("classic-import-path")
                            .into_any_element(),
                    )
                    .child(
                        Button::new("classic-import-keys")
                            .label(translator.text(Key::ImportKeys))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.import_keys(cx))),
                    ),
            );
        v_flex()
            .size_full()
            .child(header)
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .flex_1()
                    .min_h_0()
                    .child(table)
                    .child(add_card),
            )
            .into_any_element()
    }

    pub fn render_recovery(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let running = self.recovery.running;
        let busy = self.is_busy(cx);
        let header = page_header(
            cx,
            translator,
            Key::EyebrowClassicRecovery,
            Key::RecoveryTitle,
            translator.text(Key::RecoverySubtitle),
            vec![pill(
                cx,
                StatusKind::Success,
                translator.text(Key::RecoveryNoWrites),
            )],
        );
        let mut strategies = v_flex().gap_2().p_4();
        for (index, strategy) in STRATEGIES.iter().enumerate() {
            strategies = strategies.child(
                v_flex()
                    .gap_1()
                    .child(
                        Button::new(("recovery-strategy", index))
                            .w_full()
                            .when(index == self.recovery.strategy, |button| button.primary())
                            .when(index != self.recovery.strategy, |button| button.outline())
                            .label(translator.text(strategy_key(*strategy)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.recovery.strategy = index;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .px_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(translator.text(strategy_description_key(*strategy))),
                    ),
            );
        }
        let candidates = self.recovery_candidates_for_display(cx);
        let mut candidate_rows = v_flex().gap_1();
        for (kind, key) in &candidates {
            candidate_rows = candidate_rows.child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(cx.theme().secondary)
                    .child(pill(cx, StatusKind::Info, kind.label()))
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(key.to_hex()),
                    ),
            );
        }
        let target = v_flex()
            .gap_2()
            .p_4()
            .child(
                h_flex()
                    .gap_2()
                    .child(field(
                        cx,
                        translator.text(Key::FieldKnownBlock),
                        None,
                        Input::new(&self.recovery.known_block)
                            .id("recovery-known-block")
                            .into_any_element(),
                    ))
                    .child(field(
                        cx,
                        translator.text(Key::FieldTargetBlock),
                        None,
                        Input::new(&self.recovery.target_block)
                            .id("recovery-target-block")
                            .into_any_element(),
                    )),
            )
            .child(field(
                cx,
                translator.text(Key::FieldKnownKey),
                None,
                Input::new(&self.recovery.known_key)
                    .id("recovery-known-key")
                    .into_any_element(),
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(field(
                        cx,
                        translator.text(Key::FieldTargetType),
                        None,
                        h_flex()
                            .gap_1()
                            .child(format_button(
                                "recovery-type-a",
                                translator.text(Key::KeyTypeA),
                                self.recovery.target_type == 0,
                                cx.listener(|this, _, _, cx| {
                                    this.recovery.target_type = 0;
                                    cx.notify();
                                }),
                            ))
                            .child(format_button(
                                "recovery-type-b",
                                translator.text(Key::KeyTypeB),
                                self.recovery.target_type == 1,
                                cx.listener(|this, _, _, cx| {
                                    this.recovery.target_type = 1;
                                    cx.notify();
                                }),
                            ))
                            .into_any_element(),
                    ))
                    .child(field(
                        cx,
                        translator.text(Key::FieldMaxNonces),
                        None,
                        Input::new(&self.recovery.max_nonces)
                            .id("recovery-max-nonces")
                            .into_any_element(),
                    )),
            )
            .child(field(
                cx,
                translator.text(Key::FieldTables),
                None,
                text_value(
                    cx,
                    self.settings
                        .tables_dir
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| translator.text(Key::DefaultTables).to_owned()),
                ),
            ));

        let view = &self.recovery.view;
        let percent = view.percent;
        let mut stages = v_flex().gap_1();
        for (index, key) in [
            Key::StageAcquire,
            Key::StageFilter,
            Key::StageSolve,
            Key::StageVerify,
        ]
        .into_iter()
        .enumerate()
        {
            let active = matches!(
                (index, view.stage),
                (0, nfc_ui::RecoveryStage::Acquire)
                    | (1, nfc_ui::RecoveryStage::Filter)
                    | (2, nfc_ui::RecoveryStage::Solve)
                    | (3, nfc_ui::RecoveryStage::Verify)
            );
            stages = stages.child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .when(active, |row| row.text_color(cx.theme().foreground))
                    .when(!active, |row| row.text_color(cx.theme().muted_foreground))
                    .child(div().w(px(18.)).child(format!("{}", index + 1)))
                    .child(translator.text(key)),
            );
        }
        let mut result = v_flex().gap_1();
        if !self.recovery.verified.is_empty() {
            result = result.child(callout(
                cx,
                StatusKind::Success,
                translator.text(Key::VerifiedKey),
            ));
            for key in &self.recovery.verified {
                result = result.child(text_value(cx, key.clone()));
            }
            result = result.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::VerifiedKeyDetail)),
            );
        } else if view.failed {
            result = result.child(callout(cx, StatusKind::Error, view.detail.clone()));
        } else if view.cancelled {
            result = result.child(callout(
                cx,
                StatusKind::Warning,
                translator.text(Key::RecoveryCancelled),
            ));
        }
        if !self.recovery.profile.is_empty() {
            result = result.child(kv_row(
                cx,
                translator.text(Key::RecoveryProfile),
                self.recovery.profile.clone(),
            ));
        }
        if !self.recovery.note.is_empty() {
            result = result.child(kv_row(
                cx,
                translator.text(Key::RecoveryNote),
                self.recovery.note.clone(),
            ));
        }
        let run_card = v_flex()
            .gap_2()
            .w(px(360.))
            .flex_shrink_0()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::RunTitle),
                translator.text(Key::RunSub),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(stages)
                    .child(
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded_full()
                            .bg(cx.theme().secondary)
                            .child(
                                div()
                                    .h(px(6.))
                                    .w(px(percent as f32 * 2.6))
                                    .rounded_full()
                                    .bg(cx.theme().primary),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(translator.text(view.stage.key()))
                            .child(format!("{percent}%")),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("recovery-start")
                                    .primary()
                                    .label(translator.text(Key::StartRecovery))
                                    .disabled(busy || running)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_recovery(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("recovery-cancel")
                                    .label(translator.text(Key::CancelRecovery))
                                    .disabled(!running)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_recovery(cx);
                                    })),
                            ),
                    )
                    .child(callout_with_id(
                        cx,
                        StatusKind::Warning,
                        "recovery-scope-note",
                        translator.text(Key::RecoveryScopeNote),
                    ))
                    .child(result),
            );
        v_flex()
            .size_full()
            .child(header)
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .child(
                                v_flex()
                                    .bg(cx.theme().popover)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .rounded_lg()
                                    .overflow_hidden()
                                    .child(card_head(
                                        cx,
                                        translator.text(Key::RecoveryStrategy),
                                        translator.text(Key::RecoveryStrategySub),
                                    ))
                                    .child(strategies),
                            )
                            .child(
                                v_flex()
                                    .bg(cx.theme().popover)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .rounded_lg()
                                    .overflow_hidden()
                                    .child(card_head(
                                        cx,
                                        translator.text(Key::TargetTitle),
                                        translator.text(Key::TargetSub),
                                    ))
                                    .child(target),
                            )
                            .child(
                                v_flex()
                                    .bg(cx.theme().popover)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .rounded_lg()
                                    .overflow_hidden()
                                    .child(card_head(
                                        cx,
                                        translator.text(Key::CandidateKeys),
                                        format!(
                                            "{} {}",
                                            candidates.len(),
                                            translator.text(Key::CandidateCount)
                                        ),
                                    ))
                                    .child(
                                        div()
                                            .id("recovery-candidates")
                                            .max_h(px(200.))
                                            .overflow_y_scroll()
                                            .p_3()
                                            .child(candidate_rows),
                                    ),
                            ),
                    )
                    .child(run_card),
            )
            .into_any_element()
    }

    fn recovery_candidates_for_display(&self, cx: &App) -> Vec<(nfc_core::KeyType, nfc_core::Key)> {
        let mut candidates = Vec::new();
        for entry in self.keys.entries() {
            for key_type in [nfc_core::KeyType::A, nfc_core::KeyType::B] {
                if entry.kind.includes(key_type) {
                    candidates.push((key_type, entry.key));
                }
            }
        }
        if self.settings.use_common_keys {
            for key in nfc_app::classic::common_keys() {
                for key_type in [nfc_core::KeyType::A, nfc_core::KeyType::B] {
                    candidates.push((key_type, key));
                }
            }
        }
        let manual = self.recovery.known_key.read(cx).value().trim().to_owned();
        if let Ok(key) = nfc_core::Key::from_hex(&manual) {
            candidates.push((nfc_core::KeyType::A, key));
        }
        candidates.sort_by_key(|(kind, key)| (kind.label(), key.to_hex()));
        candidates.dedup();
        candidates
    }
}

fn action(
    id: impl Into<gpui_kit::ElementId>,
    label: &'static str,
    disabled: bool,
    primary: bool,
    handler: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> AnyElement {
    Button::new(id)
        .label(label)
        .when(primary, |button| button.primary())
        .disabled(disabled)
        .on_click(handler)
        .into_any_element()
}

fn format_button(
    id: impl Into<gpui_kit::ElementId>,
    label: &'static str,
    selected: bool,
    handler: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> AnyElement {
    Button::new(id)
        .xsmall()
        .when(selected, |button| button.primary())
        .when(!selected, |button| button.outline())
        .label(label)
        .on_click(handler)
        .into_any_element()
}

fn legend(cx: &App, kind: StatusKind, label: &'static str) -> AnyElement {
    let color = match kind {
        StatusKind::Info => cx.theme().muted_foreground,
        StatusKind::Success => cx.theme().success,
        StatusKind::Warning => cx.theme().warning,
        StatusKind::Error => cx.theme().danger,
    };
    h_flex()
        .gap_1()
        .child(div().size(px(5.)).rounded_full().bg(color))
        .child(label)
        .into_any_element()
}

fn summary_tile(cx: &App, label: &'static str, count: usize, kind: StatusKind) -> AnyElement {
    let color = match kind {
        StatusKind::Info => cx.theme().muted_foreground,
        StatusKind::Success => cx.theme().success,
        StatusKind::Warning => cx.theme().warning,
        StatusKind::Error => cx.theme().danger,
    };
    v_flex()
        .gap_0p5()
        .px_3()
        .py_2()
        .bg(cx.theme().popover)
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(count.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .into_any_element()
}

fn block_status(status: BlockStatus, translator: nfc_ui::Translator) -> &'static str {
    translator.text(match status {
        BlockStatus::Unread => Key::BlockStatusUnread,
        BlockStatus::Read => Key::BlockStatusRead,
        BlockStatus::AuthFailed => Key::BlockStatusAuthFailed,
        BlockStatus::ReadFailed => Key::BlockStatusReadFailed,
        BlockStatus::Synthetic => Key::BlockStatusSynthetic,
        BlockStatus::Verified => Key::BlockStatusVerified,
    })
}

fn access_line(group: usize, condition: AccessCondition, write: bool) -> String {
    if write {
        key_mask(if group == 3 {
            condition.full_trailer_write_keys()
        } else {
            condition.data_write_keys()
        })
        .to_owned()
    } else {
        key_mask(if group == 3 {
            condition.trailer_access().read_keys
        } else {
            condition.data_read_keys()
        })
        .to_owned()
    }
}

fn key_mask(mask: KeyMask) -> &'static str {
    match mask {
        KeyMask::ANY => "A / B",
        KeyMask::A => "A",
        KeyMask::B => "B",
        _ => "—",
    }
}

fn record_row(
    cx: &App,
    index: usize,
    record: &NdefRecord,
    translator: nfc_ui::Translator,
) -> AnyElement {
    let (kind, name, detail) = if let Ok(text) = TextRecord::from_record(record) {
        (
            translator.text(Key::RecordText),
            text.text.clone(),
            format!(
                "{} {} · {} {}",
                translator.text(Key::RecordLanguage),
                text.language,
                translator.text(Key::RecordPayload),
                hex_bytes(record.payload())
            ),
        )
    } else if let Ok(uri) = UriRecord::from_record(record) {
        (
            translator.text(Key::RecordUri),
            uri.uri.clone(),
            format!(
                "{} {}",
                translator.text(Key::RecordPayload),
                hex_bytes(record.payload())
            ),
        )
    } else {
        (
            translator.text(Key::RecordFile),
            hex_bytes(record.record_type()),
            format!("{:?}", record.tnf()),
        )
    };
    v_flex()
        .gap_1()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(pill(cx, StatusKind::Info, kind))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(name),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(translator.fmt1(Key::RecordIndex, index + 1)),
                ),
        )
        .child(
            div()
                .text_xs()
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(cx.theme().muted_foreground)
                .child(detail),
        )
        .into_any_element()
}

fn diff_detail(cx: &App, block: &nfc_classic::BlockDiff) -> AnyElement {
    if let Some(trailer) = &block.trailer {
        let mut parts = Vec::new();
        if trailer.key_a_changed {
            parts.push("Key A");
        }
        if trailer.key_b_changed {
            parts.push("Key B");
        }
        if trailer.access_changed {
            parts.push("access bits");
        }
        if trailer.gpb_changed {
            parts.push("GPB");
        }
        if !trailer.valid {
            parts.push("invalid trailer");
        }
        return text_value(
            cx,
            if parts.is_empty() {
                "trailer changed".to_owned()
            } else {
                parts.join(", ")
            },
        );
    }
    text_value(
        cx,
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
    )
}
