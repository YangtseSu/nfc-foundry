//! Rendering for the Type 2 / NTAG screens.

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
use nfc_type2::{Configuration, LockBits, NdefState, PageRole, Type2Image};
use nfc_ui::{
    Key, StatusKind, ascii, byte_grid, callout, card_head, empty_state, field, hex_bytes,
    inspector, kv_row, metric, page_header, pill, section_title, text_value,
};

use crate::model::{NdefSource, PageFilter, role_key};
use crate::workspace::Type2Workspace;

impl Type2Workspace {
    pub fn render_memory(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let subtitle = self
            .image
            .as_ref()
            .map(|image| {
                format!(
                    "{} · {} {} · {} {}",
                    image.layout.product.name(),
                    image.layout.page_count(),
                    translator.text(Key::PagesUnit),
                    image.layout.user_bytes(),
                    translator.text(Key::BytesUnit)
                )
            })
            .unwrap_or_else(|| translator.text(Key::Type2MemorySubtitle).to_owned());
        let header = page_header(
            cx,
            translator,
            Key::EyebrowType2Memory,
            Key::Type2MemoryTitle,
            subtitle,
            Vec::new(),
        );
        let mut page = v_flex().size_full().child(header);
        let Some(image) = &self.image else {
            return page
                .child(empty_state(
                    cx,
                    translator,
                    Key::NoCardTitle,
                    Key::NoCardBody,
                ))
                .into_any_element();
        };
        page = page
            .child(self.render_file_row(cx))
            .child(self.render_metrics(cx, image));
        let workbench = h_flex()
            .items_start()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(self.render_page_table(cx, image))
            .child(self.render_page_inspector(cx, image));
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
            .child(Input::new(&self.path_input).id("type2-path").w(px(240.)))
            .child(
                Button::new("type2-save")
                    .small()
                    .label(translator.text(Key::SaveDump))
                    .disabled(busy || self.image.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.save_dump(cx))),
            )
            .child(
                Button::new("type2-load")
                    .small()
                    .label(translator.text(Key::LoadDump))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.load_dump(cx))),
            )
            .child(
                Button::new("type2-restore")
                    .small()
                    .danger()
                    .label(translator.text(Key::RestoreTag))
                    .disabled(busy || self.loaded.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.restore_loaded(window, cx))),
            )
            .child(
                Checkbox::new("type2-restore-config")
                    .checked(self.restore_configuration)
                    .label(translator.text(Key::IncludeConfiguration))
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.restore_configuration = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("type2-restore-cc")
                    .checked(self.restore_cc)
                    .label(translator.text(Key::IncludeCapabilityContainer))
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.restore_cc = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("type2-restore-locks")
                    .checked(self.restore_locks)
                    .label(translator.text(Key::IncludeLocks))
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.restore_locks = *checked;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_metrics(&self, cx: &mut Context<Self>, image: &Type2Image) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let ndef = self.ndef.as_ref();
        let locked = LockBits::from_image(image)
            .map(|locks| locked_pages(image, &locks).len())
            .unwrap_or_default();
        h_flex()
            .gap_2()
            .flex_wrap()
            .pb_3()
            .child(metric(
                cx,
                translator.text(Key::TagLabel),
                image.layout.product.name(),
                format!(
                    "{} {} · {} {}",
                    image.layout.page_count(),
                    translator.text(Key::PagesUnit),
                    image.layout.user_bytes(),
                    translator.text(Key::BytesUnit)
                ),
            ))
            .child(metric(
                cx,
                translator.text(Key::PageReadability),
                format!("{}/{}", image.known_pages(), image.layout.page_count()),
                format!(
                    "{}/{} {}",
                    image.known_pages(),
                    image.layout.page_count(),
                    translator.text(Key::ReadabilityDetail)
                ),
            ))
            .child(metric(
                cx,
                translator.text(Key::NdefBytes),
                format!(
                    "{} / {}",
                    ndef.map_or(0, |ndef| ndef.message_bytes),
                    ndef.map_or(image.cc.ndef_capacity(), |ndef| ndef.capacity)
                ),
                translator.text(Key::BytesUnit),
            ))
            .child(metric(
                cx,
                translator.text(Key::ProtectionTitle),
                if locked == 0 {
                    translator.text(Key::UnlockedState).to_owned()
                } else {
                    format!("{locked} {}", translator.text(Key::LockedPagesCount))
                },
                if locked == 0 {
                    translator.text(Key::CfgLockClear).to_owned()
                } else {
                    translator.text(Key::LockedLegend).to_owned()
                },
            ))
            .into_any_element()
    }

    fn render_page_table(&self, cx: &mut Context<Self>, image: &Type2Image) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let locks = LockBits::from_image(image).ok();
        let query = self
            .page_search
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let filter = self.page_filter;
        let mut body = v_flex().gap_1().p_2();
        let mut shown = 0usize;
        for (page, data) in image.pages() {
            let role = image.role(page);
            if !filter.matches(role) {
                continue;
            }
            let data_text = data
                .map(|data| hex_bytes(data))
                .unwrap_or_else(|| "—".to_owned());
            if !query.is_empty() {
                let haystack = format!("{page} {} {data_text}", translator.text(role_key(role)))
                    .to_ascii_lowercase();
                if !haystack.contains(&query) {
                    continue;
                }
            }
            shown += 1;
            body = body.child(self.render_page_row(cx, image, page, role, data, locks.as_ref()));
        }
        if shown == 0 {
            body = body.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::SelectPage)),
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
                                    .child(translator.text(Key::TagMemory)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(translator.text(Key::TagMemorySub)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Input::new(&self.page_search)
                            .id("type2-page-search")
                            .small()
                            .w(px(110.)),
                    )
                    .children(
                        PageFilter::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(index, choice)| {
                                format_button(
                                    ("type2-filter", index),
                                    translator.text(choice.key()),
                                    choice == filter,
                                    cx.listener(move |this, _, _, cx| {
                                        this.select_filter(index, cx)
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
                    .child(div().w(px(44.)).child(translator.text(Key::PageLabel)))
                    .child(div().w(px(110.)).child(translator.text(Key::RoleLabel)))
                    .child(div().w(px(52.)).child(translator.text(Key::LockLabel)))
                    .child(div().flex_1().child(translator.text(Key::DataLabel))),
            )
            .child(
                div()
                    .id("type2-page-scroll")
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
                        translator.text(Key::WritableLegend),
                    ))
                    .child(legend(
                        cx,
                        StatusKind::Warning,
                        translator.text(Key::LockedLegend),
                    ))
                    .child(div().flex_1())
                    .child(format!("{shown} {}", translator.text(Key::PageCount))),
            )
            .into_any_element()
    }

    fn render_page_row(
        &self,
        cx: &mut Context<Self>,
        image: &Type2Image,
        page: u8,
        role: PageRole,
        data: Option<&[u8; 4]>,
        locks: Option<&LockBits>,
    ) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let selected = self.selected_page == Some(page);
        let locked = locks.is_some_and(|locks| locks.is_locked(&image.layout, page));
        let data_text = data
            .map(|data| hex_bytes(data))
            .unwrap_or_else(|| "—".to_owned());
        h_flex()
            .id(("type2-page", usize::from(page)))
            .test_support()
            .role(Role::Button)
            .aria_label(format!(
                "{} {page} · {}",
                translator.text(Key::PageLabel),
                translator.text(role_key(role))
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
            .child(div().w(px(44.)).child(format!("{page:03}")))
            .child(div().w(px(110.)).child(pill(
                cx,
                if role.is_advanced() {
                    StatusKind::Warning
                } else {
                    StatusKind::Info
                },
                translator.text(role_key(role)),
            )))
            .child(div().w(px(52.)).child(if locked {
                pill(cx, StatusKind::Error, translator.text(Key::LockedLegend))
            } else {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("—")
                    .into_any_element()
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(data_text),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_page(page, window, cx);
            }))
            .into_any_element()
    }

    fn render_page_inspector(&self, cx: &mut Context<Self>, image: &Type2Image) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let Some(page) = self.selected_page else {
            return div()
                .w(px(360.))
                .child(empty_state(
                    cx,
                    translator,
                    Key::SelectPage,
                    Key::TagMemorySub,
                ))
                .into_any_element();
        };
        let role = image.role(page);
        let data = image.page(page).copied();
        let locked = LockBits::from_image(image)
            .map(|locks| locks.is_locked(&image.layout, page))
            .unwrap_or(false);
        let advanced = role.is_advanced();
        let writable = data.is_some()
            && !locked
            && !matches!(role, PageRole::Manufacturer | PageRole::Reserved);
        let mut body = v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(pill(
                        cx,
                        StatusKind::Info,
                        format!("4 {}", translator.text(Key::BytesUnit)),
                    ))
                    .when(advanced, |body| {
                        body.child(pill(
                            cx,
                            StatusKind::Warning,
                            translator.text(Key::PageAdvanced),
                        ))
                    })
                    .when(locked, |body| {
                        body.child(pill(
                            cx,
                            StatusKind::Error,
                            translator.text(Key::LockedLegend),
                        ))
                    }),
            )
            .child(section_title(cx, translator.text(Key::InspectorHex)));
        if let Some(data) = data {
            body = body
                .child(byte_grid(cx, &data, 4))
                .child(section_title(cx, translator.text(Key::InspectorAscii)))
                .child(
                    div()
                        .rounded_sm()
                        .bg(cx.theme().secondary)
                        .p_2()
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(ascii(&data)),
                );
        } else {
            body = body.child(callout(
                cx,
                StatusKind::Warning,
                translator.text(Key::PageNotReadable),
            ));
        }
        if matches!(role, PageRole::Password | PageRole::PasswordAck) {
            body = body.child(callout(
                cx,
                StatusKind::Warning,
                translator.text(Key::PwdPackNote),
            ));
        }
        if advanced {
            body = body.child(callout(
                cx,
                StatusKind::Warning,
                translator.text(Key::AdvancedPageNote),
            ));
        }
        if locked {
            body = body.child(callout(
                cx,
                StatusKind::Error,
                translator.text(Key::LockedPageNote),
            ));
        }
        let busy = self.is_busy(cx);
        let actions = if writable {
            v_flex()
                .gap_2()
                .child(field(
                    cx,
                    translator.text(Key::NewPageData),
                    None,
                    Input::new(&self.page_input)
                        .id("type2-page-data")
                        .into_any_element(),
                ))
                .child(
                    Button::new("type2-write-page")
                        .when(advanced, |button| button.danger())
                        .when(!advanced, |button| button.primary())
                        .label(if advanced {
                            translator.text(Key::WriteAdvancedPage)
                        } else {
                            translator.text(Key::WritePage)
                        })
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.write_selected_page(window, cx);
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(translator.text(Key::ReadBackPlan)),
                )
                .into_any_element()
        } else {
            callout(cx, StatusKind::Error, translator.text(Key::PageNotReadable))
        };
        div()
            .id("type2-inspector")
            .test_support()
            .w(px(360.))
            .h_full()
            .flex_shrink_0()
            .role(Role::Group)
            .aria_label(format!("Page {page} · {}", translator.text(role_key(role))))
            .child(inspector(
                cx,
                translator.fmt1(Key::PageKicker, page),
                translator.text(role_key(role)),
                Some(pill(
                    cx,
                    if advanced {
                        StatusKind::Warning
                    } else {
                        StatusKind::Success
                    },
                    if advanced {
                        translator.text(Key::PageAdvanced)
                    } else {
                        translator.text(Key::PageWritable)
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
            Key::EyebrowType2Ndef,
            Key::Type2NdefTitle,
            translator.text(Key::Type2NdefSubtitle),
            vec![],
        );
        let page = v_flex()
            .size_full()
            .child(header)
            .child(self.render_file_row(cx));
        let Some(image) = &self.image else {
            return page
                .child(empty_state(
                    cx,
                    translator,
                    Key::NoCardTitle,
                    Key::NoCardBody,
                ))
                .into_any_element();
        };
        let ndef = self.ndef.as_ref();
        let read_only = ndef.is_some_and(|ndef| ndef.read_only);
        let writable = !read_only && image.cc.writable();
        let (state_label, used, capacity) = match ndef.map(|ndef| &ndef.state) {
            Some(NdefState::Message(message)) => (
                format!(
                    "{} {}",
                    message.records().len(),
                    translator.text(Key::RecordsUnit)
                ),
                ndef.map_or(0, |ndef| ndef.message_bytes),
                ndef.map_or(0, |ndef| ndef.capacity),
            ),
            Some(NdefState::Empty) => (
                translator.text(Key::NdefEmpty).to_owned(),
                0,
                ndef.map_or(0, |ndef| ndef.capacity),
            ),
            Some(NdefState::Unformatted) => (
                translator.text(Key::NdefUnformatted).to_owned(),
                0,
                ndef.map_or(0, |ndef| ndef.capacity),
            ),
            None => (translator.text(Key::NdefInvalid).to_owned(), 0, 0),
        };
        let percent = used
            .saturating_mul(100)
            .checked_div(capacity.max(1))
            .unwrap_or(0);
        let mut records = v_flex().gap_2();
        if let Some(message) = ndef.and_then(nfc_type2::Type2Ndef::message) {
            for (index, record) in message.records().iter().enumerate() {
                records = records.child(record_row(cx, index, record, translator));
            }
        }
        let capacity_card = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::CapacityContainer),
                format!("CC at page 3 · {}", image.layout.product.name()),
            ))
            .child(
                v_flex()
                    .gap_1()
                    .p_4()
                    .child(kv_row(
                        cx,
                        translator.text(Key::CcBytes),
                        hex_bytes(&image.cc.to_bytes()),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::MappingVersion),
                        format!("{}.{}", image.cc.major_version, image.cc.minor_version),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::DataAreaLabel),
                        format!(
                            "page {} · {} {}",
                            image.layout.user_first,
                            image.cc.size,
                            translator.text(Key::BytesUnit)
                        ),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::AccessLabel),
                        if image.cc.writable() {
                            translator.text(Key::NdefReadWrite)
                        } else {
                            translator.text(Key::NdefReadOnly)
                        },
                    )),
            );
        let message_card = v_flex()
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
                        } else {
                            translator.text(Key::NdefReadWrite)
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
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(state_label),
                    )
                    .child(records)
                    .child(callout(cx, StatusKind::Info, translator.text(Key::TlvInfo))),
            );
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
                translator.text(Key::WriteNdefType2Note),
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
                                    ("type2-ndef-source", index),
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
                                    .id("type2-ndef-text")
                                    .into_any_element(),
                            ))
                            .child(field(
                                cx,
                                translator.text(Key::FieldLanguage),
                                None,
                                Input::new(&self.ndef_language_input)
                                    .id("type2-ndef-language")
                                    .into_any_element(),
                            ))
                            .into_any_element(),
                        NdefSource::Uri => field(
                            cx,
                            translator.text(Key::FieldValue),
                            None,
                            Input::new(&self.ndef_value_input)
                                .id("type2-ndef-uri")
                                .into_any_element(),
                        ),
                        NdefSource::File => field(
                            cx,
                            translator.text(Key::FieldPath),
                            None,
                            Input::new(&self.ndef_file_input)
                                .id("type2-ndef-file")
                                .into_any_element(),
                        ),
                    })
                    .child(
                        Button::new("type2-ndef-write")
                            .primary()
                            .label(translator.text(Key::WriteNdef))
                            .disabled(busy || !writable)
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
                            .id("type2-ndef-export")
                            .into_any_element(),
                    )
                    .child(
                        Button::new("type2-ndef-export-action")
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
                translator.text(Key::FormatTag),
                translator.text(Key::AdvancedActions),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(callout(
                        cx,
                        StatusKind::Warning,
                        translator.text(Key::FormatTagNote),
                    ))
                    .child(
                        Button::new("type2-ndef-format")
                            .danger()
                            .label(translator.text(Key::FormatTag))
                            .disabled(busy || read_only || !writable)
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
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_3()
                        .child(message_card)
                        .child(capacity_card),
                )
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

    pub fn render_protection(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let busy = self.is_busy(cx);
        let header = page_header(
            cx,
            translator,
            Key::EyebrowType2Protection,
            Key::ProtectionTitle,
            translator.text(Key::AdvancedActionPlanSub),
            vec![pill(
                cx,
                StatusKind::Warning,
                translator.text(Key::TypedConfirmation),
            )],
        );
        let page = v_flex().size_full().child(header);
        let Some(image) = &self.image else {
            return page
                .child(empty_state(
                    cx,
                    translator,
                    Key::NoCardTitle,
                    Key::NoCardBody,
                ))
                .into_any_element();
        };
        let locks = LockBits::from_image(image).ok();
        let locked = locks
            .as_ref()
            .map(|locks| locked_pages(image, locks))
            .unwrap_or_default();
        let configuration = Configuration::from_image(image).ok();
        let lock_card = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::LockStateTitle),
                translator.text(Key::LockStateSub),
            ))
            .child(
                v_flex()
                    .gap_1()
                    .p_4()
                    .child(kv_row(
                        cx,
                        translator.text(Key::StaticLock),
                        locks
                            .as_ref()
                            .map(|locks| hex_bytes(&locks.static_lock))
                            .unwrap_or_else(|| "—".to_owned()),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::DynamicLock),
                        locks
                            .as_ref()
                            .map(|locks| hex_bytes(&locks.dynamic_lock))
                            .unwrap_or_else(|| "—".to_owned()),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::LockedPagesLabel),
                        if locked.is_empty() {
                            translator.text(Key::NoLockedPages).to_owned()
                        } else {
                            locked
                                .iter()
                                .map(u8::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::WritableUserPages),
                        format!("{}..={}", image.layout.user_first, image.layout.user_last),
                    )),
            );
        let lock_form = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::LockPagesTitle),
                translator.text(Key::LockPagesSub),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(field(
                        cx,
                        translator.text(Key::FieldPages),
                        None,
                        Input::new(&self.lock_pages_input)
                            .id("type2-lock-pages")
                            .into_any_element(),
                    ))
                    .child(callout(
                        cx,
                        StatusKind::Error,
                        translator.text(Key::LockNote),
                    ))
                    .child(
                        Button::new("type2-apply-locks")
                            .danger()
                            .label(translator.text(Key::ApplyLocks))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_lock(window, cx);
                            })),
                    ),
            );
        let configuration_card = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::ConfigurationTitle),
                translator.text(Key::ConfigurationSub),
            ))
            .child(
                v_flex()
                    .gap_1()
                    .p_4()
                    .child(kv_row(
                        cx,
                        translator.text(Key::FieldAuth0),
                        configuration
                            .as_ref()
                            .map(|configuration| format!("{:02X}", configuration.auth0))
                            .unwrap_or_else(|| "—".to_owned()),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::FieldAccess),
                        configuration
                            .as_ref()
                            .map(|configuration| format!("{:02X}", configuration.access))
                            .unwrap_or_else(|| "—".to_owned()),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::CfgLockLabel),
                        configuration
                            .as_ref()
                            .map(|configuration| {
                                if configuration.cfg_locked() {
                                    translator.text(Key::LockedLegend)
                                } else {
                                    translator.text(Key::UnlockedState)
                                }
                            })
                            .unwrap_or("—"),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::AuthLimLabel),
                        configuration
                            .as_ref()
                            .map(|configuration| configuration.auth_lim().to_string())
                            .unwrap_or_else(|| "—".to_owned()),
                    )),
            );
        let password_card = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::PasswordConfigTitle),
                translator.text(Key::PasswordConfigSub),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(field(
                                cx,
                                translator.text(Key::FieldAuth0),
                                None,
                                Input::new(&self.auth0_input)
                                    .id("type2-auth0")
                                    .into_any_element(),
                            ))
                            .child(field(
                                cx,
                                translator.text(Key::FieldAccess),
                                None,
                                Input::new(&self.access_input)
                                    .id("type2-access")
                                    .into_any_element(),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(field(
                                cx,
                                translator.text(Key::FieldPassword),
                                None,
                                Input::new(&self.pwd_input)
                                    .id("type2-pwd")
                                    .into_any_element(),
                            ))
                            .child(field(
                                cx,
                                translator.text(Key::FieldPack),
                                None,
                                Input::new(&self.pack_input)
                                    .id("type2-pack")
                                    .into_any_element(),
                            )),
                    )
                    .child(callout(
                        cx,
                        StatusKind::Warning,
                        translator.text(Key::PasswordNote),
                    ))
                    .child(
                        Button::new("type2-set-password")
                            .danger()
                            .label(translator.text(Key::SetPassword))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_password(window, cx);
                            })),
                    ),
            );
        let action_plan = v_flex()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
            .child(card_head(
                cx,
                translator.text(Key::AdvancedActionPlan),
                translator.text(Key::AdvancedActionPlanSub),
            ))
            .child(
                v_flex()
                    .gap_1()
                    .p_4()
                    .child(kv_row(
                        cx,
                        translator.text(Key::ExactTarget),
                        if locked.is_empty() {
                            translator.text(Key::NoLockedPages).to_owned()
                        } else {
                            format!("{}..={}", image.layout.user_first, image.layout.user_last)
                        },
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::OldToNew),
                        translator.text(Key::UnlockedState),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::BackupLabel),
                        translator.text(Key::BackupBeforeChange),
                    ))
                    .child(kv_row(
                        cx,
                        translator.text(Key::VerificationLabel),
                        translator.text(Key::ReadBackPlan),
                    ))
                    .child(callout(
                        cx,
                        StatusKind::Info,
                        translator.text(Key::AdvancedActionNote),
                    )),
            );
        page.child(
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
                        .child(lock_card)
                        .child(lock_form)
                        .child(action_plan),
                )
                .child(
                    v_flex()
                        .w(px(360.))
                        .flex_shrink_0()
                        .gap_3()
                        .child(configuration_card)
                        .child(password_card),
                ),
        )
        .into_any_element()
    }
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

fn locked_pages(image: &Type2Image, locks: &LockBits) -> Vec<u8> {
    (0..image.layout.total_pages)
        .filter(|page| locks.is_locked(&image.layout, *page))
        .collect()
}

fn record_row(
    cx: &App,
    index: usize,
    record: &nfc_ndef::NdefRecord,
    translator: nfc_ui::Translator,
) -> AnyElement {
    let (kind, name) = if let Ok(text) = nfc_ndef::TextRecord::from_record(record) {
        (translator.text(Key::RecordText), text.text.clone())
    } else if let Ok(uri) = nfc_ndef::UriRecord::from_record(record) {
        (translator.text(Key::RecordUri), uri.uri.clone())
    } else {
        (
            translator.text(Key::RecordFile),
            hex_bytes(record.record_type()),
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
        .child(text_value(cx, hex_bytes(record.payload())))
        .into_any_element()
}
