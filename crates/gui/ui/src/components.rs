//! Shared screen primitives.
//!
//! These helpers keep spacing, hierarchy, and state colors consistent across
//! the workspace crates. They are presentation only: no helper performs I/O or
//! owns application state.

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, IntoElement, SharedString, TestSupportExt as _, div, px,
};

use crate::i18n::{Key, Translator};
use crate::session::StatusKind;

/// Space-separated uppercase hexadecimal.
pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Printable ASCII with dots for non-printable bytes.
pub fn ascii(bytes: &[u8]) -> String {
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

/// Eyebrow, title, subtitle, and right-aligned screen actions.
pub fn page_header(
    cx: &App,
    translator: Translator,
    eyebrow: Key,
    title: Key,
    subtitle: impl Into<SharedString>,
    actions: Vec<AnyElement>,
) -> AnyElement {
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .mb_4()
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(cx.theme().muted_foreground)
                                .child(translator.text(eyebrow).to_uppercase()),
                        ),
                )
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::BOLD)
                        .child(translator.text(title)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(subtitle.into()),
                ),
        )
        .child(h_flex().gap_2().flex_wrap().justify_end().children(actions))
        .into_any_element()
}

/// A titled panel.
pub fn card(
    cx: &App,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    body: AnyElement,
) -> AnyElement {
    v_flex()
        .bg(cx.theme().popover)
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .overflow_hidden()
        .child(card_head(cx, title, subtitle))
        .child(card_body(body))
        .into_any_element()
}

pub fn card_head(
    cx: &App,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
) -> AnyElement {
    v_flex()
        .gap_0p5()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(subtitle.into()),
        )
        .into_any_element()
}

pub fn card_body(body: impl IntoElement) -> AnyElement {
    div().p_4().child(body).into_any_element()
}

/// Right inspector with a fixed action footer.
pub fn inspector(
    cx: &App,
    kicker: impl Into<SharedString>,
    title: impl Into<SharedString>,
    badge: Option<AnyElement>,
    body: AnyElement,
    actions: Option<AnyElement>,
) -> AnyElement {
    let mut head = h_flex()
        .items_start()
        .gap_3()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(kicker.into()),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                ),
        );
    if let Some(badge) = badge {
        head = head.child(div().ml_auto().child(badge));
    }
    v_flex()
        .h_full()
        .bg(cx.theme().popover)
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .overflow_hidden()
        .child(head)
        .child(
            div()
                .id("inspector-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .child(body),
        )
        .when_some(actions, |this, actions| {
            this.child(
                v_flex()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .px_4()
                    .py_3()
                    .bg(cx.theme().popover)
                    .child(actions),
            )
        })
        .into_any_element()
}

/// Metric tile.
pub fn metric(
    cx: &App,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    sub: impl Into<SharedString>,
) -> AnyElement {
    v_flex()
        .gap_0p5()
        .p_3()
        .bg(cx.theme().popover)
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .child(value.into()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(sub.into()),
        )
        .into_any_element()
}

/// Small status pill.
pub fn pill(cx: &App, kind: StatusKind, text: impl Into<SharedString>) -> AnyElement {
    let color = match kind {
        StatusKind::Info => cx.theme().muted_foreground,
        StatusKind::Success => cx.theme().success,
        StatusKind::Warning => cx.theme().warning,
        StatusKind::Error => cx.theme().danger,
    };
    h_flex()
        .gap_1()
        .h(px(19.))
        .px_1p5()
        .rounded_sm()
        .bg(color.opacity(0.12))
        .border_1()
        .border_color(color.opacity(0.28))
        .text_xs()
        .text_color(color)
        .child(div().size(px(4.)).rounded_full().bg(color))
        .child(text.into())
        .into_any_element()
}

/// Inline callout with an icon and a message.
pub fn callout(cx: &App, kind: StatusKind, text: impl Into<SharedString>) -> AnyElement {
    callout_inner(cx, kind, None, text.into())
}

/// Callout with a stable test/observation id.
pub fn callout_with_id(
    cx: &App,
    kind: StatusKind,
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
) -> AnyElement {
    callout_inner(cx, kind, Some(id.into()), text.into())
}

fn callout_inner(
    cx: &App,
    kind: StatusKind,
    id: Option<ElementId>,
    text: SharedString,
) -> AnyElement {
    let (color, icon) = match kind {
        StatusKind::Info => (cx.theme().info, IconName::Info),
        StatusKind::Success => (cx.theme().success, IconName::CircleCheck),
        StatusKind::Warning => (cx.theme().warning, IconName::TriangleAlert),
        StatusKind::Error => (cx.theme().danger, IconName::CircleAlert),
    };
    let container = h_flex()
        .items_start()
        .gap_2()
        .min_w_0()
        .p_2()
        .rounded_md()
        .bg(color.opacity(0.08))
        .border_1()
        .border_color(color.opacity(0.22))
        .child(Icon::new(icon).xsmall().flex_shrink_0().text_color(color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(text),
        );
    match id {
        Some(id) => container.id(id).test_support().into_any_element(),
        None => container.into_any_element(),
    }
}

/// Centered empty state.
pub fn empty_state(cx: &App, translator: Translator, title: Key, body: Key) -> AnyElement {
    v_flex()
        .items_center()
        .justify_center()
        .gap_2()
        .p_8()
        .flex_1()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(translator.text(title)),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(translator.text(body)),
        )
        .into_any_element()
}

pub fn section_title(cx: &App, text: impl Into<SharedString>) -> AnyElement {
    div()
        .mt_3()
        .mb_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// Labelled form field.
pub fn field(
    cx: &App,
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
    control: AnyElement,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child(label.into()),
        )
        .child(control)
        .when_some(hint, |this, hint| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(hint),
            )
        })
        .into_any_element()
}

/// Key/value row for inspector facts.
pub fn kv_row(
    cx: &App,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
) -> AnyElement {
    h_flex()
        .justify_between()
        .gap_3()
        .py_0p5()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(
            div()
                .text_xs()
                .font_family(cx.theme().mono_font_family.clone())
                .child(value.into()),
        )
        .into_any_element()
}

/// Fixed-width byte cells for a block or page inspector.
pub fn byte_grid(cx: &App, bytes: &[u8], columns: usize) -> AnyElement {
    let mut grid = v_flex().gap_1();
    for chunk in bytes.chunks(columns) {
        let mut row = h_flex().gap_1();
        for byte in chunk {
            row = row.child(
                div()
                    .w(px(32.))
                    .h(px(24.))
                    .rounded_sm()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(format!("{byte:02X}")),
            );
        }
        grid = grid.child(row);
    }
    grid.into_any_element()
}

/// Monospaced value text.
pub fn text_value(cx: &App, text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .child(text.into())
        .into_any_element()
}
