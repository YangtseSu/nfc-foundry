//! Language catalogs and display-time translation for the workbench.
//!
//! English is the base language and the fallback. Every [`Key`] has an
//! explicit English and Simplified Chinese value; `Key::ALL` is used by the
//! tests that reject empty or duplicate entries.
//!
//! Views keep their English source strings. [`translate`] maps a displayed
//! line to the current language, so static labels, dynamic status lines, and
//! table cells are all localized at the same point. Protocol names, hex, UIDs,
//! paths, and counts pass through unchanged because no catalog entry matches
//! them.

use std::cell::Cell;
#[cfg(test)]
use std::collections::BTreeSet;
use std::sync::LazyLock;

use adw::prelude::*;
use serde::{Deserialize, Serialize};

macro_rules! keys {
    ($($key:ident => ($english:literal, $chinese:literal)),+ $(,)?) => {
        /// One localized workbench string.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[allow(clippy::enum_variant_names)]
        pub enum Key {
            $($key),+
        }

        impl Key {
            /// Every catalog key, used by the completeness tests.
            pub const ALL: &'static [Self] = &[$(Self::$key),+];
        }

        const fn english(key: Key) -> &'static str {
            match key {
                $(Key::$key => $english),+
            }
        }

        const fn chinese(key: Key) -> &'static str {
            match key {
                $(Key::$key => $chinese),+
            }
        }
    };
}

/// The two workbench languages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// English, the base language and fallback.
    #[default]
    En,
    /// Simplified Chinese.
    ZhCn,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::En, Self::ZhCn];

    /// Stable preference-file code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }

    /// Parse a stored preference code. Unknown codes are rejected so a typo
    /// cannot silently change the language.
    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en-gb" => Some(Self::En),
            "zh" | "zh-cn" | "zh-hans" | "zh-sg" => Some(Self::ZhCn),
            _ => None,
        }
    }
}

/// Resolve one locale value to a supported language. A `zh` primary tag
/// selects `zh-CN`; every other value resolves to English.
pub fn locale_language(value: &str) -> Option<Language> {
    let tag = value.split(['.', '@']).next().unwrap_or_default().trim();
    if tag.is_empty() {
        return None;
    }
    let primary = tag.split(['_', '-']).next().unwrap_or_default();
    if primary.eq_ignore_ascii_case("zh") {
        Some(Language::ZhCn)
    } else {
        Some(Language::En)
    }
}

/// Detect a language from a saved preference, then `LC_ALL`, `LC_MESSAGES`,
/// and `LANG`, then English.
pub fn detect_language(saved: Option<Language>, env: impl Fn(&str) -> Option<String>) -> Language {
    if let Some(saved) = saved {
        return saved;
    }
    for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env(variable)
            && let Some(language) = locale_language(&value)
        {
            return language;
        }
    }
    Language::En
}

/// The translator for one explicit language, independent of the process
/// setting. Tests use it directly; the UI uses [`current_language`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Translator {
    language: Language,
}

impl Translator {
    pub const fn new(language: Language) -> Self {
        Self { language }
    }

    /// Localized string for a key.
    pub fn localized(self, key: Key) -> &'static str {
        match self.language {
            Language::En => english(key),
            Language::ZhCn if chinese(key).is_empty() => english(key),
            Language::ZhCn => chinese(key),
        }
    }

    /// Translate one displayed line, including catalog patterns with `{}`.
    #[cfg(test)]
    pub fn translate(self, text: &str) -> String {
        text.split('\n')
            .map(|line| translate_with(self.language, line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

thread_local! {
    /// Workbench language for the UI thread. The GTK main context owns every
    /// display update, so a thread-local value keeps parallel tests isolated
    /// and leaves worker threads free of shared mutable state.
    static CURRENT: Cell<Language> = const { Cell::new(Language::En) };
}

/// Current workbench language.
pub fn current_language() -> Language {
    CURRENT.with(Cell::get)
}

/// Select the workbench language for subsequent display updates.
pub fn set_current_language(language: Language) {
    CURRENT.with(|current| current.set(language));
}

/// Translate every line of a multi-line label, preserving the line breaks.
pub fn translate_lines(text: &str) -> String {
    let language = current_language();
    text.split('\n')
        .map(|line| translate_with(language, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn translate_with(language: Language, text: &str) -> String {
    translate_depth(language, text, 0)
}

fn translate_depth(language: Language, text: &str, depth: u8) -> String {
    if depth > 3 {
        return text.to_owned();
    }
    let pairs = catalog();
    match language {
        Language::En => {
            for (english, chinese) in pairs {
                if let Some(value) = apply_pattern(chinese, english, text, language, depth) {
                    return value;
                }
            }
            text.to_owned()
        }
        Language::ZhCn => {
            for (english, chinese) in pairs {
                if let Some(value) = apply_pattern(english, chinese, text, language, depth) {
                    return value;
                }
            }
            #[cfg(test)]
            note_missing(text);
            text.to_owned()
        }
    }
}

/// Sorted `(English, Chinese)` pairs, longest source first so a specific
/// pattern wins over a shorter prefix.
fn catalog() -> &'static [(&'static str, &'static str)] {
    static PAIRS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
        let mut pairs = Key::ALL
            .iter()
            .map(|key| (english(*key), chinese(*key)))
            .collect::<Vec<_>>();
        pairs.sort_by_key(|(english, _)| std::cmp::Reverse(english.len()));
        pairs
    });
    &PAIRS
}

/// Split a catalog pattern into literal segments around `{...}`
/// placeholders. The placeholder name or format specifier is irrelevant to
/// display matching; only the number and position matter.
fn pattern_parts(pattern: &str) -> (Vec<&str>, usize) {
    let mut parts = Vec::new();
    let mut count = 0;
    let mut rest = pattern;
    loop {
        let Some(start) = rest.find('{') else {
            parts.push(rest);
            break;
        };
        let Some(offset) = rest[start..].find('}') else {
            parts.push(rest);
            break;
        };
        parts.push(&rest[..start]);
        count += 1;
        rest = &rest[start + offset + 1..];
    }
    (parts, count)
}

/// Match `source` against `text`, substituting captured values into `target`.
fn apply_pattern(
    source: &str,
    target: &str,
    text: &str,
    language: Language,
    depth: u8,
) -> Option<String> {
    let (parts, count) = pattern_parts(source);
    if count == 0 {
        return (source == text).then(|| target.to_owned());
    }
    let (_, target_count) = pattern_parts(target);
    if count != target_count {
        return None;
    }
    let mut rest = text;
    let mut captures = Vec::with_capacity(count);
    if let Some(prefix) = parts.first().filter(|part| !part.is_empty()) {
        rest = rest.strip_prefix(prefix)?;
    }
    for (index, part) in parts.iter().enumerate().skip(1) {
        let part = *part;
        if index == count {
            if part.is_empty() {
                captures.push(rest.to_owned());
            } else {
                captures.push(rest.strip_suffix(part)?.to_owned());
            }
            rest = "";
        } else {
            let position = rest.find(part)?;
            captures.push(rest[..position].to_owned());
            rest = &rest[position + part.len()..];
        }
    }
    if !rest.is_empty() {
        return None;
    }
    let (target_parts, _) = pattern_parts(target);
    let mut out = String::new();
    out.push_str(target_parts.first()?);
    for (index, capture) in captures.iter().enumerate() {
        out.push_str(&translate_depth(language, capture, depth + 1));
        out.push_str(target_parts[index + 1]);
    }
    Some(out)
}

const LABEL_SOURCE: &str = "label";
const PLACEHOLDER_SOURCE: &str = "placeholder";
const TITLE_SOURCE: &str = "title";
const SUBTITLE_SOURCE: &str = "subtitle";
const TOOLTIP_SOURCE: &str = "tooltip";
const ALERT_HEADING_SOURCE: &str = "heading";
const ALERT_BODY_SOURCE: &str = "body";

struct SourceEntry {
    object: glib::WeakRef<glib::Object>,
    key: &'static str,
    text: String,
}

struct ModelEntry {
    object: glib::WeakRef<glib::Object>,
    items: Vec<String>,
}

thread_local! {
    static SOURCES: std::cell::RefCell<Vec<SourceEntry>> = const { std::cell::RefCell::new(Vec::new()) };
    static MODELS: std::cell::RefCell<Vec<ModelEntry>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn same_object(object: &impl IsA<glib::Object>, weak: &glib::WeakRef<glib::Object>) -> bool {
    weak.upgrade().is_some_and(|stored| {
        std::ptr::eq(stored.as_ptr() as *const (), object.as_ptr() as *const ())
    })
}

fn stored_source(object: &impl IsA<glib::Object>, key: &str) -> Option<String> {
    SOURCES.with(|sources| {
        let mut sources = sources.borrow_mut();
        sources.retain(|entry| entry.object.upgrade().is_some());
        sources
            .iter()
            .find(|entry| entry.key == key && same_object(object, &entry.object))
            .map(|entry| entry.text.clone())
    })
}

fn store_source(object: &impl IsA<glib::Object>, key: &'static str, text: &str) {
    SOURCES.with(|sources| {
        let mut sources = sources.borrow_mut();
        sources.retain(|entry| entry.object.upgrade().is_some());
        if let Some(entry) = sources
            .iter_mut()
            .find(|entry| entry.key == key && same_object(object, &entry.object))
        {
            entry.text = text.to_owned();
        } else {
            sources.push(SourceEntry {
                object: object.upcast_ref::<glib::Object>().downgrade(),
                key,
                text: text.to_owned(),
            });
        }
    });
}

fn stored_model(object: &impl IsA<glib::Object>) -> Option<Vec<String>> {
    MODELS.with(|models| {
        let mut models = models.borrow_mut();
        models.retain(|entry| entry.object.upgrade().is_some());
        models
            .iter()
            .find(|entry| same_object(object, &entry.object))
            .map(|entry| entry.items.clone())
    })
}

fn store_model(object: &impl IsA<glib::Object>, items: Vec<String>) {
    MODELS.with(|models| {
        let mut models = models.borrow_mut();
        models.retain(|entry| entry.object.upgrade().is_some());
        if let Some(entry) = models
            .iter_mut()
            .find(|entry| same_object(object, &entry.object))
        {
            entry.items = items;
        } else {
            models.push(ModelEntry {
                object: object.upcast_ref::<glib::Object>().downgrade(),
                items,
            });
        }
    });
}

/// Return the English source for one property, recording it on first use so a
/// language switch never has to reverse-translate an ambiguous Chinese value.
fn source_or_record(object: &impl IsA<glib::Object>, key: &'static str, current: &str) -> String {
    match stored_source(object, key) {
        Some(source) => source,
        None => {
            store_source(object, key, current);
            current.to_owned()
        }
    }
}

fn localized_property(object: &impl IsA<glib::Object>, key: &'static str, current: &str) -> String {
    translate_lines(&source_or_record(object, key, current))
}

/// Set a new English source and return its projection. A dynamic label may
/// change many times; the newest source replaces the previous one.
fn updated_property(object: &impl IsA<glib::Object>, key: &'static str, text: &str) -> String {
    store_source(object, key, text);
    translate_lines(text)
}

/// Translate a widget label as it is set. Views keep writing English source
/// strings; this converts them for the current language at the display edge
/// and records the English source for later language switches.
pub trait Localized {
    fn set_localized(&self, text: &str);
}

impl Localized for gtk::Label {
    fn set_localized(&self, text: &str) {
        self.set_label(&updated_property(self, LABEL_SOURCE, text));
    }
}

impl Localized for gtk::Button {
    fn set_localized(&self, text: &str) {
        self.set_label(&updated_property(self, LABEL_SOURCE, text));
    }
}

impl Localized for gtk::MenuButton {
    fn set_localized(&self, text: &str) {
        self.set_label(&updated_property(self, LABEL_SOURCE, text));
    }
}

/// Relabel an already-built widget tree in the current language. Source
/// strings are stored per property, so switching back restores the exact
/// English copy instead of a reverse-mapped synonym.
pub fn localize_widget_tree(widget: &impl IsA<gtk::Widget>) {
    let widget = widget.as_ref();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        let text = label.label();
        label.set_label(&localized_property(label, LABEL_SOURCE, &text));
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && let Some(text) = button.label()
    {
        button.set_label(&localized_property(button, LABEL_SOURCE, &text));
    }
    if let Some(button) = widget.downcast_ref::<gtk::MenuButton>()
        && let Some(text) = button.label()
    {
        button.set_label(&localized_property(button, LABEL_SOURCE, &text));
    }
    if let Some(button) = widget.downcast_ref::<gtk::CheckButton>()
        && let Some(text) = button.label()
    {
        button.set_label(Some(&localized_property(button, LABEL_SOURCE, &text)));
    }
    if let Some(entry) = widget.downcast_ref::<gtk::Entry>()
        && let Some(text) = entry.placeholder_text()
    {
        entry.set_placeholder_text(Some(&localized_property(entry, PLACEHOLDER_SOURCE, &text)));
    }
    if let Some(window) = widget.downcast_ref::<gtk::Window>()
        && let Some(title) = window.title()
    {
        window.set_title(Some(&localized_property(window, TITLE_SOURCE, &title)));
    }
    if let Some(title) = widget.downcast_ref::<adw::WindowTitle>() {
        let text = title.title();
        title.set_title(&localized_property(title, TITLE_SOURCE, &text));
        let text = title.subtitle();
        if !text.is_empty() {
            title.set_subtitle(&localized_property(title, SUBTITLE_SOURCE, &text));
        }
    }
    if let Some(dropdown) = widget.downcast_ref::<gtk::DropDown>()
        && let Some(model) = dropdown.model()
        && let Some(strings) = model.downcast_ref::<gtk::StringList>()
    {
        let current = (0..strings.n_items())
            .filter_map(|index| strings.item(index))
            .filter_map(|item| item.downcast::<gtk::StringObject>().ok())
            .map(|item| item.string().to_string())
            .collect::<Vec<_>>();
        let source = match stored_model(dropdown) {
            Some(source) if source.len() == current.len() => source,
            _ => {
                store_model(dropdown, current.clone());
                current
            }
        };
        let values = source
            .iter()
            .map(|text| translate_lines(text))
            .collect::<Vec<_>>();
        let values = values.iter().map(String::as_str).collect::<Vec<_>>();
        strings.splice(0, strings.n_items(), &values);
    }
    if let Some(stack) = widget.downcast_ref::<gtk::Stack>() {
        let pages = stack.pages();
        for index in 0..pages.n_items() {
            let Some(page) = pages.item(index).and_downcast::<gtk::StackPage>() else {
                continue;
            };
            if let Some(title) = page.title() {
                page.set_title(&localized_property(&page, TITLE_SOURCE, &title));
            }
        }
    }
    if let Some(columns) = widget.downcast_ref::<gtk::ColumnView>() {
        let columns = columns.columns();
        for index in 0..columns.n_items() {
            let Some(column) = columns.item(index).and_downcast::<gtk::ColumnViewColumn>() else {
                continue;
            };
            if let Some(title) = column.title() {
                column.set_title(Some(&localized_property(&column, TITLE_SOURCE, &title)));
            }
        }
    }
    if let Some(tooltip) = widget.tooltip_text() {
        widget.set_tooltip_text(Some(&localized_property(widget, TOOLTIP_SOURCE, &tooltip)));
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        localize_widget_tree(&current);
        child = current.next_sibling();
    }
}

/// Localize an alert dialog before it is presented. Response labels are part
/// of the dialog's widget tree and are covered by the same pass.
pub fn localize_alert(dialog: &adw::AlertDialog) {
    if let Some(heading) = dialog.heading() {
        dialog.set_heading(Some(&localized_property(
            dialog,
            ALERT_HEADING_SOURCE,
            &heading,
        )));
    }
    let body = dialog.body();
    dialog.set_body(&localized_property(dialog, ALERT_BODY_SOURCE, &body));
    localize_widget_tree(dialog);
}

/// Every label-like string currently in a widget tree. Used by the gated GTK
/// test to prove that no English chrome survives a Chinese switch.
#[cfg(test)]
pub fn visible_texts(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    collect_texts(widget, false)
}

#[cfg(test)]
fn collect_texts(widget: &impl IsA<gtk::Widget>, include_dialog: bool) -> Vec<String> {
    fn walk(widget: &gtk::Widget, texts: &mut Vec<String>, include_dialog: bool) {
        if !widget.is_visible() {
            return;
        }
        if !include_dialog && widget.downcast_ref::<adw::AlertDialog>().is_some() {
            return;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            texts.push(label.label().to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && let Some(text) = button.label()
        {
            texts.push(text.to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::MenuButton>()
            && let Some(text) = button.label()
        {
            texts.push(text.to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::CheckButton>()
            && let Some(text) = button.label()
        {
            texts.push(text.to_string());
        }
        if let Some(entry) = widget.downcast_ref::<gtk::Entry>()
            && let Some(text) = entry.placeholder_text()
        {
            texts.push(text.to_string());
        }
        if let Some(stack) = widget.downcast_ref::<gtk::Stack>() {
            let pages = stack.pages();
            for index in 0..pages.n_items() {
                if let Some(page) = pages.item(index).and_downcast::<gtk::StackPage>()
                    && let Some(title) = page.title()
                {
                    texts.push(title.to_string());
                }
            }
        }
        if let Some(columns) = widget.downcast_ref::<gtk::ColumnView>() {
            let columns = columns.columns();
            for index in 0..columns.n_items() {
                if let Some(column) = columns.item(index).and_downcast::<gtk::ColumnViewColumn>()
                    && let Some(title) = column.title()
                {
                    texts.push(title.to_string());
                }
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            walk(&current, texts, include_dialog);
            child = current.next_sibling();
        }
    }
    let mut texts = Vec::new();
    walk(widget.as_ref(), &mut texts, include_dialog);
    texts
}

#[cfg(test)]
thread_local! {
    static MISSING: std::cell::RefCell<BTreeSet<String>> = const { std::cell::RefCell::new(BTreeSet::new()) };
}

#[cfg(test)]
fn note_missing(text: &str) {
    if !text
        .chars()
        .any(|character| character.is_ascii_alphabetic())
    {
        return;
    }
    MISSING.with(|missing| {
        missing.borrow_mut().insert(text.to_owned());
    });
}

#[cfg(test)]
fn take_missing() -> BTreeSet<String> {
    MISSING.with(|missing| std::mem::take(&mut *missing.borrow_mut()))
}

keys! {
    // Shell and navigation.
    AppSubtitle => ("reader workbench", "读卡工作台"),
    RailReader => ("READER", "读卡器"),
    RailCard => ("CARD", "卡片"),
    RailKeys => ("SECTOR KEYS", "扇区密钥"),
    RailSelected => ("SELECTED", "选中项"),
    RefreshReaders => ("Refresh readers", "刷新读卡器"),
    MainMenu => ("Main menu", "主菜单"),
    BackAction => ("Back", "返回"),
    DumpSection => ("Dump", "转储"),
    DumpsPage => ("Dumps", "转储"),
    SystemSection => ("System", "系统"),
    AboutNfcFoundry => ("About NFC Foundry", "关于 NFC Foundry"),
    NoReaderSelected => ("No reader selected", "未选择读卡器"),
    NoCard => ("No card", "无卡片"),
    SelectReaderAndRead => ("Select a reader and read a card.", "选择读卡器并读取卡片。"),
    NoBlockSelected => ("No block selected", "未选择数据块"),
    Ready => ("Ready", "就绪"),
    ReadCard => ("Read card", "读取卡片"),
    RefreshCard => ("Refresh card", "刷新卡片"),
    DesfireInventory => ("Inventory", "清单"),
    DesfireReadTab => ("Read", "读取"),
    DesfireReadFile => ("Read file", "读取文件"),
    DesfireAid => ("Application ID", "应用 ID"),
    DesfireFileId => ("File ID", "文件 ID"),
    DesfireKey => ("Legacy key (optional)", "传统密钥（可选）"),
    DesfireKeyNumber => ("Key number", "密钥编号"),
    DesfireOffset => ("Offset", "偏移"),
    DesfireLength => ("Length (optional)", "长度（可选）"),
    DesfireHint => (
        "Read the card, then enter an application ID and file ID.",
        "读取卡片，然后输入应用 ID 和文件 ID。"
    ),
    DesfireNoInventory => (
        "Read the card to list DESFire applications.",
        "读取卡片以列出 DESFire 应用。"
    ),
    Actions => ("Actions", "操作"),
    CardPage => ("Card", "卡片"),
    Type2Page => ("Type 2", "Type 2"),
    NdefPage => ("NDEF", "NDEF"),
    KeysPage => ("Keys", "密钥"),
    KeyscanPage => ("Key scan", "密钥扫描"),
    RecoveryPage => ("Recovery", "密钥恢复"),
    DiffPage => ("Diff", "比较"),
    RestorePage => ("Restore", "恢复写入"),
    DiagnosticsPage => ("Diagnostics", "诊断"),
    ActivityPage => ("Activity", "活动"),
    SettingsPage => ("Settings", "设置"),
    NoCardLoaded => ("No card loaded", "未加载卡片"),
    EmptyCardHint => (
        "Select a reader and press Read card.",
        "选择读卡器并按下“读取卡片”。"
    ),
    EmptyCardHint2 => (
        "The block table shows the real card image.",
        "数据块表显示真实卡片镜像。"
    ),
    BlockInspector => ("BLOCK INSPECTOR", "数据块检查器"),
    NoBlock => ("No block", "未选择数据块"),
    ReadCardToInspectBlock => ("Read a card to inspect a block.", "读取卡片后可检查数据块。"),
    BlockNotRead => ("block not read", "数据块未读取"),
    ReadBlock => ("Read block", "读取数据块"),
    WriteBlock => ("Write block", "写入数据块"),
    WriteNote => (
        "Writes are confirmed, backed up, and read back. Block 0 stays read-only.",
        "写入前确认并备份，写入后回读校验。块 0 保持只读。"
    ),

    // Classic NDEF.
    ComposeMessage => ("Compose message", "编辑消息"),
    Record => ("Record", "记录"),
    LanguageField => ("Language", "语言"),
    TextRecord => ("Text", "文本"),
    UriRecord => ("URI", "URI"),
    RawNdefFile => ("Raw NDEF file", "原始 NDEF 文件"),
    ChooseNdefFile => ("Choose NDEF file…", "选择 NDEF 文件…"),
    NoFileChosen => ("No file chosen", "未选择文件"),
    NoMessageComposed => ("No message composed", "尚未编辑消息"),
    WriteNdef => ("Write NDEF", "写入 NDEF"),
    FormatNdefAreaAction => ("Format NDEF area…", "格式化 NDEF 区域…"),
    FormatNdefArea => ("Format NDEF area", "格式化 NDEF 区域"),
    LeaveReadOnly => ("Leave the NDEF area read-only", "将 NDEF 区域保持只读"),
    NdefUnavailable => ("NDEF unavailable", "NDEF 不可用"),
    NotNdefFormatted => ("Not NDEF formatted", "未格式化为 NDEF"),
    NdefEmptyMessage => ("NDEF formatted · empty message", "NDEF 已格式化 · 空消息"),
    NdefOneRecord => ("NDEF · 1 record", "NDEF · 1 条记录"),
    NdefMultipleRecords => ("NDEF · multiple records", "NDEF · 多条记录"),
    NdefUnavailableFacts => ("The NDEF facts are unavailable.", "NDEF 信息不可用。"),
    NoMadMapping => (
        "No MAD or NFC sector mapping was found on this card.",
        "此卡上未找到 MAD 或 NFC 扇区映射。"
    ),
    FormatNdefBeforeWrite => (
        "Format the NDEF area before writing; Format creates the MAD and NFC sectors.",
        "写入前需格式化 NDEF 区域；格式化会创建 MAD 和 NFC 扇区。"
    ),
    FormatNdefDataBeforeWrite => (
        "Format the NDEF data area before writing; Format creates an empty NDEF Message TLV.",
        "写入前需格式化 NDEF 数据区域；格式化会创建空的 NDEF 消息 TLV。"
    ),
    CapacityEmpty => ("Capacity {} bytes · message empty", "容量 {} 字节 · 消息为空"),
    CapacityMessage => ("Capacity {} bytes · message {} bytes", "容量 {} 字节 · 消息 {} 字节"),
    ReadLiveCardToWrite => ("Read a live card to write or format.", "读取在线卡片后才能写入或格式化。"),
    ReadLiveType2ToWrite => (
        "Read a live Type 2 tag to write or format.",
        "读取在线 Type 2 标签后才能写入或格式化。"
    ),
    OperationRunning => ("An operation is running.", "有操作正在运行。"),
    ImagePartialReadWhole => ("The image is partial; read the whole card first.", "镜像不完整；请先完整读取卡片。"),
    NdefStateUnavailable => ("NDEF state is unavailable.", "NDEF 状态不可用。"),
    NdefStateUnavailableReadAgain => (
        "The NDEF state is unavailable; read the card again",
        "NDEF 状态不可用；请重新读取卡片"
    ),
    FormatRequiredBeforeWriting => (
        "Not NDEF formatted · Format is required before writing.",
        "未格式化为 NDEF · 写入前需要执行格式化。"
    ),
    NotNdefFormattedFormatFirst => (
        "Not NDEF formatted; format the NDEF area first",
        "未格式化为 NDEF；请先格式化 NDEF 区域"
    ),
    NotNdefFormattedFormatDataFirst => (
        "Not NDEF formatted; format the data area first",
        "未格式化为 NDEF；请先格式化数据区域"
    ),
    NdefAreaReadOnlyWritingDisabled => (
        "The NDEF area is read-only; writing is disabled",
        "NDEF 区域为只读；写入已禁用"
    ),
    NdefAreaReadOnlyDisable => (
        "The NDEF area is read-only; writing is disabled.",
        "NDEF 区域为只读；写入已禁用。"
    ),
    NfcMappingNotWritable => ("The NFC mapping is not writable; format the card.", "NFC 映射不可写；请格式化卡片。"),
    AlreadyStoresMessage => (
        "The NDEF area already stores this message.",
        "NDEF 区域已存储此消息。"
    ),
    AlreadyStoresMessageNoDot => (
        "The NDEF area already stores this message",
        "NDEF 区域已存储此消息"
    ),
    ReadyToWriteBytes => ("Ready to write {} bytes.", "可以写入 {} 字节。"),
    CapacityUnavailable => ("Capacity is unavailable; read the card again.", "容量不可用；请重新读取卡片。"),
    CapacityUnavailableTag => ("Capacity is unavailable; read the tag again.", "容量不可用；请重新读取标签。"),
    MessageBytes => ("Message {} bytes", "消息 {} 字节"),
    MessageBytesCapacity => ("Message {} bytes · capacity {} bytes", "消息 {} 字节 · 容量 {} 字节"),
    RecordKicker => ("RECORD {}", "记录 {}"),
    RecordTextLanguage => ("Text · {}", "文本 · {}"),
    RecordTextInvalid => ("Text (invalid)", "文本（无效）"),
    RecordUriInvalid => ("URI (invalid)", "URI（无效）"),
    RecordEmpty => ("Empty", "空"),
    RecordWellKnown => ("Well-known · {}", "已知类型 · {}"),
    RecordMedia => ("Media · {}", "媒体 · {}"),
    RecordAbsoluteUri => ("Absolute URI · {}", "绝对 URI · {}"),
    RecordExternal => ("External · {}", "外部类型 · {}"),
    RecordUnknown => ("Unknown", "未知"),
    RecordChunk => ("Chunk", "分块"),
    CannotEncode => ("Cannot encode the message: {}", "无法编码消息：{}"),
    NdefNeedsLiveCard => ("NDEF writes need a live selected card", "NDEF 写入需要在线选中的卡片"),
    NdefNeedsCompleteRead => ("NDEF writes need a complete card read", "NDEF 写入需要完整读取卡片"),
    MadHasNoNfcSectors => ("The MAD has no NFC sectors; format the NDEF area first", "MAD 中没有 NFC 扇区；请先格式化 NDEF 区域"),
    WriteNdefQuestion => ("Write NDEF message?", "写入 NDEF 消息？"),
    WritingNdef => ("Writing NDEF message…", "正在写入 NDEF 消息…"),
    NdefWrittenBlocks => ("NDEF written · {} block(s)", "NDEF 已写入 · {} 个数据块"),
    NdefWrittenPages => ("NDEF written · {} page(s)", "NDEF 已写入 · {} 页"),
    NdefWrittenVerified => ("NDEF written and read back", "NDEF 已写入并回读校验"),
    NdefWriteFailed => ("NDEF write failed", "NDEF 写入失败"),
    NdefFormatNeedsLiveCard => ("NDEF format needs a live selected card", "NDEF 格式化需要在线选中的卡片"),
    ReadCardBeforeFormat => ("Read a card before formatting its NDEF area", "格式化 NDEF 区域前请先读取卡片"),
    NdefFormatNeedsCompleteRead => ("NDEF format needs a complete card read", "NDEF 格式化需要完整读取卡片"),
    NfcSectorsSummary => ("NFC sectors {} · {} data blocks · {} bytes", "NFC 扇区 {} · {} 个数据块 · {} 字节"),
    MadAndNfcSectors => ("the MAD and every NFC sector", "MAD 和每个 NFC 扇区"),
    FormatNdefQuestion => ("Format the NDEF area?", "格式化 NDEF 区域？"),
    FormattingNdefArea => ("Formatting the NDEF area…", "正在格式化 NDEF 区域…"),
    NdefFormattedBlocks => (
        "NDEF formatted · {} block(s), {} trailer(s)",
        "NDEF 已格式化 · {} 个数据块，{} 个扇区尾块"
    ),
    NdefFormatVerified => ("NDEF area formatted and read back", "NDEF 区域已格式化并回读校验"),
    NdefFormatFailed => ("NDEF format failed", "NDEF 格式化失败"),
    MadSectors => ("MAD sectors: {}", "MAD 扇区：{}"),
    MappingVersion => ("Mapping version {}.{}", "映射版本 {}.{}"),
    MappingNone => ("none", "无"),
    MappingReadOnly => ("read-only", "只读"),
    MappingReadWrite => ("read/write", "读写"),
    MappingNotWritable => ("not writable", "不可写"),
    CurrentNdefUnformatted => ("Current NDEF state: unformatted", "当前 NDEF 状态：未格式化"),
    CurrentNdefEmpty => ("Current NDEF state: empty message", "当前 NDEF 状态：空消息"),
    CurrentNdefRecords => (
        "Current NDEF state: {} record(s), {} bytes",
        "当前 NDEF 状态：{} 条记录，{} 字节"
    ),
    NdefWriteDialogBody => (
        "{}\nMessage {} bytes · capacity {} bytes\nNew bytes  {}\n\n{}\n\nA project backup is written before the write, and the NDEF Message TLV is read back afterwards.",
        "{}\n消息 {} 字节 · 容量 {} 字节\n新字节  {}\n\n{}\n\n写入前会保存项目备份，写入后回读 NDEF 消息 TLV。"
    ),
    RawNdefLoaded => ("Raw NDEF message loaded", "原始 NDEF 消息已加载"),
    ExportMessage => ("Export message…", "导出消息…"),
    ExportNdefMessage => ("Export NDEF message", "导出 NDEF 消息"),
    NoNdefMessageToExport => ("No NDEF message to export", "没有可导出的 NDEF 消息"),
    NdefMessageExported => ("NDEF message exported", "NDEF 消息已导出"),
    LoadRawNdef => ("Load raw NDEF message", "加载原始 NDEF 消息"),
    LocalFileOnly => ("The selected location is not a local file", "所选位置不是本地文件"),
    LocalDirectoryOnly => ("The selected location is not a local directory", "所选位置不是本地目录"),

    // Type 2 memory.
    Type2Title => ("Type 2 / NTAG", "Type 2 / NTAG"),
    Type2SummaryIdle => ("Read a Type 2 tag to inspect its pages.", "读取 Type 2 标签后可检查其页。"),
    AdvancedMode => ("Advanced mode", "高级模式"),
    SaveDump => ("Save dump…", "保存转储…"),
    LoadDump => ("Load dump…", "加载转储…"),
    PageColumn => ("Page", "页"),
    RoleColumn => ("Role", "角色"),
    LockColumn => ("Lock", "锁定"),
    DataColumn => ("Data", "数据"),
    Type2EmptyHint => ("Read a Type 2 tag to list its pages.", "读取 Type 2 标签后可列出其页。"),
    Type2EmptyHint2 => ("Partial reads keep unknown pages as —.", "部分读取时未知页显示为 —。"),
    RestoreProjectImage => ("Restore project image", "恢复写入项目镜像"),
    ChooseProjectImage => ("Choose project image…", "选择项目镜像…"),
    NoRestoreImage => ("No restore image loaded", "未加载恢复镜像"),
    IncludeUserMemory => ("User memory", "用户存储"),
    IncludeConfigurationPages => ("Configuration pages", "配置页"),
    IncludeCapabilityContainer => ("Capability container", "容量容器"),
    IncludeLockBytes => ("Lock bytes", "锁定位"),
    RestoreImageAction => ("Restore image…", "恢复写入镜像…"),
    RestoreImageActionShort => ("Restore image", "恢复写入镜像"),
    RestoreSelectiveNote => (
        "Restore writes only the selected page classes after the confirmation dialog.",
        "恢复写入仅在输入确认短语后写入所选页类别。"
    ),
    MemoryTab => ("Memory", "存储"),
    ProtectionTab => ("Protection", "保护"),
    ConfigurationTitle => ("Configuration", "配置"),
    ReadType2TagPeriod => ("Read a Type 2 tag.", "请读取 Type 2 标签。"),
    OneWayLocks => ("One-way locks", "单向锁定"),
    PagesField => ("Pages", "页"),
    ApplyLockBits => ("Apply lock bits…", "应用锁定位…"),
    ApplyLockBitsShort => ("Apply lock bits", "应用锁定位"),
    LockPreviewHint => ("Enter pages to preview the lock bytes.", "输入页号以预览锁定位。"),
    PasswordAndAccess => ("Password and access", "密码与访问"),
    ProtectedAccess => ("Protected access", "受保护访问"),
    AccessPwd => ("Access PWD", "访问 PWD"),
    AccessPack => ("Access PACK", "访问 PACK"),
    ProtectedAccessNote => (
        "Used for reads and writes on a tag protected by AUTH0, and as the current credentials when changing the configuration. Cleared after each operation and never saved.",
        "用于受 AUTH0 保护标签的读写，以及更改配置时的当前凭据。每次操作后清除，绝不保存。"
    ),
    CurrentCredentialsRequired => (
        "This configuration is protected; enter the current access password and PACK first.",
        "此配置受保护；请先输入当前访问密码和 PACK。"
    ),
    PasswordReplaced => (
        "PWD and PACK are replaced with the new values.",
        "PWD 和 PACK 将替换为新值。"
    ),
    PasswordProtectionStartsPage => (
        "Password protection starts at page {}; enter the access password and PACK in the Protection tab.",
        "密码保护从第 {} 页开始；请在保护页输入访问密码和 PACK。"
    ),
    ApplyPasswordConfiguration => ("Apply password configuration…", "应用密码配置…"),
    ApplyConfiguration => ("Apply configuration", "应用配置"),
    PasswordNote => (
        "PWD and PACK are write-only; the service verifies PACK with PWD_AUTH after the write.",
        "PWD 和 PACK 只可写；服务会在写入后用 PWD_AUTH 校验 PACK。"
    ),
    PageInspector => ("PAGE INSPECTOR", "页检查器"),
    NoPage => ("No page", "未选择页"),
    ReadType2ToInspectPage => ("Read a Type 2 tag to inspect a page.", "读取 Type 2 标签后可检查页。"),
    SelectPageInTable => ("Select a page in the table.", "请在表中选择一页。"),
    Hex8Placeholder => ("8 hex digits", "8 位十六进制"),
    WritePage => ("Write page…", "写入页…"),
    WritePageShort => ("Write page", "写入页"),
    WriteAdvancedPageQuestion => ("Write an Advanced Type 2 page?", "写入高级 Type 2 页？"),
    PageWriteNote => (
        "User pages are backed up and read back. Advanced pages need Advanced mode and a confirmation dialog.",
        "用户页写入前备份并写入后回读。高级页需要高级模式和输入确认短语。"
    ),
    PageNotReadableReadAgain => ("This page was not readable; read the tag again.", "此页不可读；请重新读取标签。"),
    ReadLiveCardToWritePage => ("Read a live card to write this page.", "读取在线卡片后才能写入此页。"),
    PageAlreadyContains => ("The page already contains these bytes.", "此页已包含这些字节。"),
    PageAlreadyContainsNoDot => ("The page already contains these bytes", "此页已包含这些字节"),
    ReadyToWritePage => ("Ready to write page {}.", "可以写入第 {} 页。"),
    Type2WritesNeedLive => ("Type 2 writes need a live selected card", "Type 2 写入需要在线选中的卡片"),
    ReadType2First => ("Read a Type 2 tag first", "请先读取 Type 2 标签"),
    ReadType2BeforeRestore => ("Read a Type 2 tag before restoring", "恢复写入前请先读取 Type 2 标签"),
    SelectPageFirst => ("Select a page first", "请先选择一页"),
    SelectedPageNotReadable => ("The selected page is not readable", "所选页不可读"),
    Type2WriteDialogBody => (
        "{} · UID {}\nPage {:03} · {}\n\nOld  {}\nNew  {}\n\nA project backup is written before the write, and the page is read back afterwards.",
        "{} · UID {}\n第 {:03} 页 · {}\n\n旧值  {}\n新值  {}\n\n写入前会保存项目备份，写入后回读该页。"
    ),
    WritePageQuestion => ("Write page {}?", "写入第 {} 页？"),
    WritingPage => ("Writing page {page:03}…", "正在写入第 {page:03} 页…"),
    Type2PageWritten => ("Type 2 page {page:03} written", "Type 2 第 {page:03} 页已写入"),
    PageWrittenReadBack => ("Page {page:03} written and read back", "第 {page:03} 页已写入并回读校验"),
    Type2WriteFailed => ("Type 2 write failed", "Type 2 写入失败"),
    LockingNeedsLive => ("Locking needs a live selected card", "锁定需要在线选中的卡片"),
    EnableAdvancedBeforeLocking => ("Enable Advanced mode before locking pages", "锁定页面前请启用高级模式"),
    PagesAlreadyLocked => ("The requested pages are already locked", "所选页已锁定"),
    PagesAlreadyLockedDot => ("The requested pages are already locked.", "所选页已锁定。"),
    ApplyLocksQuestion => ("Apply one-way lock bits?", "应用单向锁定位？"),
    WritingLockBits => ("Writing Type 2 lock bits…", "正在写入 Type 2 锁定位…"),
    LockBitsAlreadySet => ("Lock bits already set", "锁定位已设置"),
    LockBitsWritten => ("Lock bits written", "锁定位已写入"),
    LockBitsVerified => ("Lock bits written and read back", "锁定位已写入并回读校验"),
    Type2LockFailed => ("Type 2 lock failed", "Type 2 锁定失败"),
    PasswordConfigNeedsLive => ("Password configuration needs a live selected card", "密码配置需要在线选中的卡片"),
    EnableAdvancedBeforePassword => ("Enable Advanced mode before changing the password", "更改密码前请启用高级模式"),
    Auth0Error => ("AUTH0: {error}", "AUTH0：{error}"),
    ConfigurationUnavailable => ("configuration unavailable", "配置不可用"),
    ChangePasswordQuestion => ("Change the Type 2 password configuration?", "更改 Type 2 密码配置？"),
    WritingPasswordConfig => ("Writing password configuration…", "正在写入密码配置…"),
    PasswordConfigVerified => ("Password configuration written and verified", "密码配置已写入并校验"),
    Type2PasswordFailed => ("Type 2 password failed", "Type 2 密码配置失败"),
    Type2UidChecking => ("Checking the Type 2 UID plan…", "正在检查 Type 2 UID 方案…"),
    Type2UidChanging => ("Changing the Type 2 UID…", "正在更改 Type 2 UID…"),
    Type2UidChanged => ("Type 2 UID changed to {}", "Type 2 UID 已更改为 {}"),
    Type2UidFailed => ("Type 2 UID change failed", "Type 2 UID 更改失败"),
    ChangeType2UidQuestion => ("Change the Type 2 UID?", "更改 Type 2 UID？"),
    ChangeUidAction => ("Change UID", "更改 UID"),
    ReadType2ToChangeUid => (
        "Read a Type 2 tag to change its UID.",
        "读取 Type 2 标签后可更改其 UID。"
    ),
    UidLengthUnsupported => (
        "This UID length has no supported Type 2 UID workflow.",
        "此 UID 长度没有受支持的 Type 2 UID 流程。"
    ),
    EnableAdvancedForUid => ("Enable Advanced mode to change the UID.", "启用高级模式后可更改 UID。"),
    Type2UidNote => (
        "Only a UID-writable NTAG213 clone accepts this. A full backup is written first; page 0/1/2 are read back and reselected, and the original UID is restored on failure.",
        "仅可写 UID 的 NTAG213 克隆卡支持此操作。写入前会保存完整备份；第 0/1/2 页会回读并重选卡片，失败时会写回原 UID。"
    ),
    Type2UidDialogBody => (
        "{} · UID {}\n\npage 0: {} -> {}\npage 1: {} -> {}\npage 2: {} -> {}\n\nA project backup is written first. Pages 0/1/2 are read back and the card is reselected. If anything fails, the original UID is written back.",
        "{} · UID {}\n\n第 0 页：{} -> {}\n第 1 页：{} -> {}\n第 2 页：{} -> {}\n\n写入前会保存项目备份。第 0/1/2 页会回读并重选卡片；任何步骤失败都会写回原 UID。"
    ),

    ReadType2ToInspectPassword => (
        "Read a Type 2 tag to inspect its password configuration.",
        "读取 Type 2 标签后可检查其密码配置。"
    ),
    NoReadableConfiguration => (
        "This tag has no readable NTAG configuration pages.",
        "此标签没有可读的 NTAG 配置页。"
    ),
    EnableAdvancedForProtection => (
        "Enable Advanced mode to change locks, configuration, or the password.",
        "启用高级模式后可更改锁定、配置或密码。"
    ),
    EnableAdvancedForLocks => ("Enable Advanced mode to change lock bits.", "启用高级模式后可更改锁定位。"),
    PasswordProtectionCoversLockPages => (
        "Password protection covers the lock pages; enter the access password and PACK first.",
        "密码保护覆盖锁定位页；请先输入访问密码和 PACK。"
    ),
    LockPreview => (
        "Pages {}\nStatic lock {} → {}\nDynamic lock {} → {}\nOne-way: set bits cannot be cleared. The service backs up the image and reads every lock page back.",
        "页 {}\n静态锁定 {} → {}\n动态锁定 {} → {}\n单向：已设置的位不能清除。服务会备份镜像并回读每个锁定页。"
    ),
    ReadLiveCardToChangeLocks => ("Read a live card to change lock bits.", "读取在线卡片后才能更改锁定位。"),
    RestoringNeedsLive => ("Restoring needs a live selected card", "恢复写入需要在线选中的卡片"),
    ChooseType2ImageFirst => ("Choose a Type 2 project image first", "请先选择 Type 2 项目镜像"),
    ImageCardMismatch => ("The project image is for {} but the card is {}", "项目镜像适用于 {}，但当前卡片是 {}"),
    CcMismatch => (
        "The project image capability container does not match the card",
        "项目镜像的容量容器与卡片不匹配"
    ),
    SelectPageClass => ("Select at least one page class to restore", "请至少选择一种要恢复写入的页类别"),
    ClassUserMemory => ("user memory", "用户存储"),
    ClassConfiguration => ("configuration pages", "配置页"),
    ClassCapability => ("capability container", "容量容器"),
    ClassLocks => ("lock bytes", "锁定位"),
    Type2RestoreDialogBody => (
        "Target {} · UID {}\nSource {} · UID {}\nRestore {}\n\nA project backup of the live tag is written before the restore, and every written page is read back. Manufacturer and UID pages are never written.",
        "目标 {} · UID {}\n来源 {} · UID {}\n恢复写入 {}\n\n恢复写入前会保存在线标签的项目备份，并回读每一页。厂商页和 UID 页永不写入。"
    ),
    RestoreType2Question => ("Restore the Type 2 image?", "恢复写入 Type 2 镜像？"),
    RestoringType2Image => ("Restoring the Type 2 image…", "正在恢复写入 Type 2 镜像…"),
    RestoreCompleteWritten => (
        "Restore complete · {} written, {} unchanged",
        "恢复写入完成 · 已写入 {}，未变 {}"
    ),
    Type2RestoredVerified => ("Type 2 image restored and read back", "Type 2 镜像已恢复写入并回读校验"),
    Type2RestoreFailed => ("Type 2 restore failed", "Type 2 恢复写入失败"),
    RestoreBackupNote => (
        "The live card is backed up before the restore and every written page is read back.",
        "恢复写入前会备份在线卡片，并回读每个写入页。"
    ),
    NoType2ImageToSave => ("No Type 2 image to save", "没有可保存的 Type 2 镜像"),
    SaveType2Image => ("Save Type 2 project image", "保存 Type 2 项目镜像"),
    ChooseType2Image => ("Choose Type 2 project image", "选择 Type 2 项目镜像"),
    Type2ImageLoadedInspection => (
        "Type 2 image loaded; writes are disabled without a live card",
        "Type 2 镜像已加载；无在线卡片时写入已禁用"
    ),
    Type2NdefCapacity => (
        "CC {} · data area {} bytes · capacity {} bytes",
        "CC {} · 数据区域 {} 字节 · 容量 {} 字节"
    ),
    CcReadOnly => ("capability container read-only", "容量容器只读"),
    CcWritable => ("capability container writable", "容量容器可写"),
    LockedPages => ("{} · locked pages: {}", "{} · 已锁定页：{}"),
    PasswordProtectionFromPage => (
        "Password protection covers the data area from page {page}; enter the access password and PACK in the Protection tab.",
        "密码保护从第 {page} 页起覆盖数据区域；请在保护页输入访问密码和 PACK。"
    ),
    NdefAreaSize => ("{} bytes", "{} 字节"),
    NdefFormatDataQuestion => ("Format the NDEF data area?", "格式化 NDEF 数据区域？"),
    NdefFormatPagesBody => (
        "{} · UID {}\n\nPages {page_range} ({area_size})\n\nThe NDEF data area is replaced with an empty NDEF Message TLV; proprietary TLVs are removed. A project backup is written first, and every written page is read back.",
        "{} · UID {}\n\n页 {page_range}（{area_size}）\n\nNDEF 数据区域将替换为空 NDEF 消息 TLV；专有 TLV 会被移除。写入前会保存项目备份，并回读每个写入页。"
    ),
    FormattingNdefDataArea => ("Formatting the NDEF data area…", "正在格式化 NDEF 数据区域…"),
    NdefFormattedPages => ("NDEF formatted · {} page(s)", "NDEF 已格式化 · {} 页"),
    NdefDataFormattedVerified => ("NDEF data area formatted and read back", "NDEF 数据区域已格式化并回读校验"),
    NtAGConfigNotReadable => (
        "The NTAG configuration pages are not readable; protection state is unknown.",
        "NTAG 配置页不可读；保护状态未知。"
    ),
    NdefAreaNotReadable => ("The NDEF data area is not completely readable.", "NDEF 数据区域无法完整读取。"),
    CcMarksReadOnly => (
        "The capability container marks the NDEF data area read-only.",
        "容量容器将 NDEF 数据区域标记为只读。"
    ),
    PagesLockedReadOnly => ("Pages {} are locked read-only.", "页 {} 已锁定为只读。"),
    PasswordProtectionFromPageFirst => (
        "Password protection covers the data area from page {page}; enter the access password and PACK in the Protection tab.",
        "密码保护从第 {page} 页起覆盖数据区域；请在保护页输入访问密码和 PACK。"
    ),
    NdefNoCardHint => (
        "Read a card to inspect its MAD mapping and NDEF message.",
        "读取卡片后可检查其 MAD 映射和 NDEF 消息。"
    ),

    // Keys.
    SectorKeysTitle => ("Sector keys", "扇区密钥"),
    KeysNote => (
        "The JSON store keeps sector, A/B, and provenance. Built-in common keys are candidates and are not stored; Save and Export apply to stored keys only. Proxmark3 .dic export is lossy: unique uppercase keys in candidate order.",
        "JSON 存储保留扇区、A/B 和来源。内置常用密钥仅作为候选项，不保存在库中；保存和导出仅作用于已保存密钥。Proxmark3 .dic 导出是有损的：按候选顺序输出唯一的大写密钥。"
    ),
    SectorColumn => ("Sector", "扇区"),
    KeySector => ("Sector {}", "扇区 {}"),
    KeySourceManual => ("manual", "手动"),
    KeySourceFile => ("key file", "密钥文件"),
    KeySourceScan => ("scan", "扫描"),
    KeySourceRecovery => ("recovery", "恢复"),
    KeySourceCommon => ("common", "常用"),
    TypeColumn => ("Type", "类型"),
    KeyColumn => ("Key", "密钥"),
    SourceColumn => ("Source", "来源"),
    Hex12Placeholder => ("12 hex digits", "12 位十六进制"),
    AllSectorsPlaceholder => ("all sectors", "全部扇区"),
    AnyKind => ("any", "任意"),
    AddKey => ("Add key", "添加密钥"),
    ImportKeys => ("Import .dic / JSON…", "导入 .dic / JSON…"),
    SaveStore => ("Save store", "保存密钥库"),
    ExportDic => ("Export .dic…", "导出 .dic…"),
    KeyCount => ("{} key(s) · {}", "{} 个密钥 · {}"),
    NoStorePath => ("no store path", "无密钥库路径"),
    CommonKeysCount => ("{} common", "{} 个常用密钥"),
    CommonKeysOff => ("common keys off", "常用密钥已关闭"),
    CandidateKeys => ("{} saved · {}\nA + B candidates", "已保存 {} · {}\nA + B 候选"),
    CandidateLine => ("A + B candidates", "A + B 候选"),
    SectorMustBeNumber => ("Sector must be a number or empty for all sectors", "扇区必须为数字，或留空表示全部扇区"),
    KeyAlreadyPresent => ("That key entry is already present", "该密钥条目已存在"),
    NoKeyStorePath => ("No key store path; set XDG_CONFIG_HOME or HOME", "无密钥库路径；请设置 XDG_CONFIG_HOME 或 HOME"),
    ImportClassicKeys => ("Import Classic keys", "导入 Classic 密钥"),
    ImportedKeys => ("Imported {} key(s)", "已导入 {} 个密钥"),
    ExportProxmarkKeys => ("Export Proxmark3 key dictionary", "导出 Proxmark3 密钥字典"),
    ExportDicNote => (
        "Exported unique keys; sector, A/B, and provenance are not stored in .dic",
        "已导出唯一密钥；扇区、A/B 和来源不会存入 .dic"
    ),

    // Diff and restore.
    DiffTitle => ("Dump diff", "转储比较"),
    DiffNote => (
        "Compare two images without a reader. A block with unknown bytes is classified unknown, never equal.",
        "无需读卡器即可比较两个镜像。含未知字节的数据块归类为未知，绝不视为相同。"
    ),
    BeforeNoImage => ("Before: no image loaded", "之前：未加载镜像"),
    AfterNoImage => ("After: no image loaded", "之后：未加载镜像"),
    LoadBefore => ("Load before…", "加载之前镜像…"),
    LoadAfter => ("Load after…", "加载之后镜像…"),
    Compare => ("Compare", "比较"),
    LoadBothThenCompare => ("Load both images, then compare.", "加载两个镜像后进行比较。"),
    BlockColumn => ("Block", "数据块"),
    BlocksPage => ("Blocks", "数据块"),
    StateColumn => ("State", "状态"),
    DetailColumn => ("Detail", "详情"),
    DiffSummary => (
        "{} · {} changed · {} equal · {} unknown · {} trailer change(s)",
        "{} · {} 个变化 · {} 个相同 · {} 个未知 · {} 个扇区尾块变化"
    ),
    IdentityChanged => (" · card identity changed", " · 卡片标识已改变"),
    LoadBothBeforeCompare => ("Load both images before comparing", "比较前请先加载两个镜像"),
    LoadClassicDumpDiff => ("Load Classic dump for diff", "加载用于比较的 Classic 转储"),
    DumpReadableSummary => ("{}\n{} · {}/{} blocks readable{}", "{}\n{} · {}/{} 个数据块可读{}"),
    WarningSuffix => (" · {} warning(s)", " · {} 个警告"),
    RestoreClassicTitle => ("Restore a Classic dump", "恢复写入 Classic 转储"),
    NoImageLoaded => ("No image loaded", "未加载镜像"),
    ContinueOnError => ("Continue after a failed block", "数据块失败后继续"),
    RestoreToCard => ("Restore to card", "恢复写入卡片"),
    RestoreNote => (
        "Restore backs up the current card first, writes data blocks before sector trailers, and reads every block back. Block 0 is written only when a target UID is supplied, after the restore is verified. A card with a different identity from the image is refused before the backup.",
        "恢复写入会先备份当前卡片，先写数据块再写扇区尾块，并回读每个数据块。仅当指定目标 UID 时才在恢复验证后写入块 0。与镜像标识不同的卡片会在备份前被拒绝。"
    ),
    RestoreResultColumn => ("Result", "结果"),
    LoadClassicDumpRestore => ("Load Classic dump to restore", "加载用于恢复写入的 Classic 转储"),
    RestoreDumpSummary => (
        "{} · {} · UID {}\n{}/{} blocks known · {} of {} sectors readable{}",
        "{} · {} · UID {}\n{}/{} 个数据块已知 · {}/{} 个扇区可读{}"
    ),
    UnknownValue => ("unknown", "未知"),
    RestoreNeedsLive => ("Restore needs a live selected card", "恢复写入需要在线选中的卡片"),
    LoadDumpBeforeRestore => ("Load a dump before restoring", "恢复写入前请先加载转储"),
    RestoreDumpQuestion => ("Restore dump to the live card?", "将转储恢复写入在线卡片？"),
    RestoreAction => ("Restore", "恢复写入"),
    RestoreDumpBody => (
        "Load: {}\nImage: {} · {} of {} blocks known\n\nA project backup is written before the write. Data blocks are written before sector trailers, and every block is read back. Block 0 is never written.",
        "加载：{}\n镜像：{} · {}/{} 个数据块已知\n\n写入前会保存项目备份。先写数据块再写扇区尾块，并回读每个数据块。块 0 永不写入。"
    ),
    RestoringDump => ("Restoring dump…", "正在恢复写入转储…"),
    RestoreResult => (
        "Restored {} block(s), {} trailer(s); {} unchanged, {} unknown skipped.\nBackup: {}",
        "已恢复写入 {} 个数据块、{} 个扇区尾块；{} 个未变，跳过 {} 个未知数据块。\n备份：{}"
    ),
    WarnPrefix => ("Warning: ", "警告："),
    FailurePrefix => ("Failure: ", "失败："),
    RestoreCompletePath => ("Restore complete · {}", "恢复写入完成 · {}"),
    RestoreCompleteVerified => ("Restore complete and read back", "恢复写入完成并回读校验"),
    RestoreFinishedFailures => (
        "Restore finished with failures; see the Restore screen",
        "恢复写入完成但有失败；请查看恢复写入页面"
    ),
    RestoreFailed => ("Restore failed", "恢复写入失败"),
    NoDumpToSave => ("No dump to save", "没有可保存的转储"),
    CompleteImageNote => (
        "Complete image. The configured default is preselected; every format can represent it.",
        "完整镜像。已预选配置的默认格式；所有格式均可表示。"
    ),
    PartialImageNote => (
        "Partial image. Only the NFC Foundry project format can preserve unknown bytes; raw, Proxmark3 JSON, and MCT are disabled.",
        "不完整镜像。只有 NFC Foundry 项目格式能保留未知字节；原始、Proxmark3 JSON 和 MCT 已禁用。"
    ),
    SaveClassicDump => ("Save Classic dump", "保存 Classic 转储"),
    CompleteImageRequired => (
        "A complete image is required; unknown bytes cannot be represented.",
        "需要完整镜像；无法表示未知字节。"
    ),
    ChooseFile => ("Choose file…", "选择文件…"),
    SavedPathFormat => ("Saved {} ({})", "已保存 {}（{}）"),
    LoadDumpTitle => ("Load dump", "加载转储"),
    Type2InspectionOnly => ("Loaded {} · Type 2 project · inspection only", "已加载 {} · Type 2 项目 · 仅检查"),
    DumpLoadedInspection => (
        "Dump loaded; writes are disabled without a live card",
        "转储已加载；无在线卡片时写入已禁用"
    ),
    LoadedPathFormat => ("Loaded {} · {}", "已加载 {} · {}"),
    FormatAuto => ("Automatic (raw or project)", "自动（原始或项目）"),
    FormatRaw => ("Proxmark3 raw (.bin)", "Proxmark3 原始 (.bin)"),
    FormatProxmark => ("Proxmark3 JSON (.mfc.json)", "Proxmark3 JSON (.mfc.json)"),
    FormatMct => ("MCT text (.mct)", "MCT 文本 (.mct)"),
    FormatProject => ("NFC Foundry project (.nfc-foundry.json)", "NFC Foundry 项目 (.nfc-foundry.json)"),

    // Key scan.
    KeyscanTitle => ("Key scan", "密钥扫描"),
    KeyscanNote => (
        "Authenticate candidate keys per sector and verify a hit by reading the sector trailer. The scan never writes the card. Discovered keys are saved only when you choose to merge them.",
        "逐扇区认证候选密钥，并通过读取扇区尾块验证命中。扫描绝不写卡。发现密钥只在你选择合并时保存。"
    ),
    IncludeCommonKeys => ("Include common keys", "包含常用密钥"),
    KeyTypesBoth => ("A and B", "A 和 B"),
    KeyTypesA => ("A only", "仅 A"),
    KeyTypesB => ("B only", "仅 B"),
    DictionaryPlaceholder => ("Optional Proxmark3 .dic dictionary", "可选的 Proxmark3 .dic 字典"),
    Choose => ("Choose…", "选择…"),
    WifiSsid => ("SSID", "SSID"),
    StartScan => ("Start scan", "开始扫描"),
    Cancel => ("Cancel", "取消"),
    NoScanRunning => ("No scan running.", "没有正在运行的扫描。"),
    ScanIdle => ("Scan a MIFARE Classic card to list per-sector keys.", "扫描 MIFARE Classic 卡片以列出各扇区密钥。"),
    KeyAColumn => ("Key A", "Key A"),
    KeyBColumn => ("Key B", "Key B"),
    KeyscanEmpty => ("No scan result yet.", "尚无扫描结果。"),
    KeyscanEmpty2 => ("Choose candidates and start a scan.", "选择候选密钥并开始扫描。"),
    SaveDiscoveredKeys => ("Save discovered keys", "保存发现的密钥"),
    AuthenticatingCandidates => ("Authenticating candidates…", "正在认证候选密钥…"),
    ScanningEverySector => ("Scanning every sector.", "正在扫描全部扇区。"),
    ScanningKeys => ("Scanning keys…", "正在扫描密钥…"),
    ScanComplete => ("Scan complete.", "扫描完成。"),
    ScanCancelled => ("Scan cancelled.", "扫描已取消。"),
    ScanCancelledKeysKept => (
        "Scan cancelled; confirmed keys kept.",
        "扫描已取消；已确认密钥已保留。"
    ),
    CancelledBeforeExchange => ("Cancelled before the next reader exchange.", "已在下次读卡器交换前取消。"),
    ScanFailed => ("Scan failed.", "扫描失败。"),
    CancellingAfterExchange => ("Cancelling after the current exchange…", "将在当前交换后取消…"),
    ScanProgress => (
        "Sector {}/{} · {} attempts · {} verified",
        "扇区 {}/{} · {} 次尝试 · {} 次校验"
    ),
    KeyscanSummary => (
        "{} sector(s) with a key · {} attempts · {} verified trailer reads",
        "{} 个扇区找到密钥 · {} 次尝试 · {} 次扇区尾块读取校验"
    ),
    KeyScanComplete => ("Key scan complete · {} sector(s)", "密钥扫描完成 · {} 个扇区"),
    NoCandidateAuthenticated => ("No candidate key authenticated a sector", "没有候选密钥通过扇区认证"),
    FoundKeysIn => ("Found keys in {} sector(s)", "在 {} 个扇区中找到密钥"),
    KeyScanCancelled => ("Key scan cancelled", "密钥扫描已取消"),
    KeyScanFailed => ("Key scan failed", "密钥扫描失败"),
    RunScanFirst => ("Run a key scan first", "请先运行密钥扫描"),
    SetKeyStoreFirst => ("Set a key store path in Settings first", "请先在设置中指定密钥库路径"),
    AllKeysAlreadyStored => ("Every discovered key is already stored", "所有发现的密钥均已保存"),
    MergedKeys => ("Merged {} discovered key(s)", "已合并 {} 个发现的密钥"),
    ExportScannedKeys => ("Export scanned keys", "导出扫描到的密钥"),
    ExportedKeys => ("Exported {} key(s)", "已导出 {} 个密钥"),
    ChooseClassicDictionary => ("Choose a Classic key dictionary", "选择 Classic 密钥字典"),

    // Recovery.
    RecoveryTitle => ("Recovery", "密钥恢复"),
    RecoveryNote => (
        "Native Crypto1 recovery is read/authenticate-only. A recovered key is accepted only after the service authenticates it again. Cancelling stops before the next reader exchange.",
        "原生 Crypto1 恢复只读并仅做认证。恢复出的密钥只有在服务重新认证通过后才会被接受。取消会在下次读卡器交换前停止。"
    ),
    StrategyAuto => ("Auto", "自动"),
    StrategyDarkside => ("Darkside", "Darkside"),
    StrategyNested => ("Nested", "Nested"),
    StrategyStaticNested => ("Static nested", "静态 Nested"),
    StrategyHardnested => ("Hardnested", "Hardnested"),
    StrategyFm11rf08s => ("FM11RF08S", "FM11RF08S"),
    StrategyField => ("Strategy", "策略"),
    KnownBlockType => ("Known block / type", "已知块 / 类型"),
    TargetBlockType => ("Target block / type", "目标块 / 类型"),
    TablesDirectory => ("Tables directory", "表目录"),
    MaxNonces => ("Max nonces", "最大 nonce 数"),
    TablesPlaceholder => ("Installed table directory", "已安装的表目录"),
    MaxNoncesNote => (
        "Hardnested acquisition cap; zero uses the engine default (4000 frames).",
        "Hardnested 采集上限；0 使用引擎默认值（4000 帧）。"
    ),
    StartRecovery => ("Start recovery", "开始恢复"),
    NoRecoveryRunning => ("No recovery running.", "没有正在运行的恢复。"),
    ElapsedDash => ("Elapsed —", "耗时 —"),
    RecoveryAutoSummary => (
        "Auto scans every sector with the key store and common keys, takes a darkside seed when nothing matches, then recovers the remaining keys with nested or hardnested.",
        "自动模式会用密钥库和常见密钥扫描所有扇区；全部未命中时先用 Darkside 取种子密钥，再用 Nested 或 Hardnested 恢复剩余密钥。"
    ),
    NoRecoveredKey => ("No recovered key.", "尚无恢复的密钥。"),
    SaveVerifiedKey => ("Save verified keys", "保存已验证密钥"),
    AdvancedStrategyOptions => ("Advanced strategy options", "高级策略选项"),
    RecoveryLog => ("Recovery log", "恢复日志"),
    RecoveryRunning => ("Recovery running.", "恢复正在运行。"),
    Starting => ("Starting…", "正在启动…"),
    ElapsedZero => ("Elapsed 0:00", "耗时 0:00"),
    RecoveringKey => ("Recovering a key…", "正在恢复密钥…"),
    Acquiring => (
        "Acquiring · {} frames · {} distinct nonces · {}/256 first bytes · {} states",
        "采集中 · {} 帧 · {} 个不同 nonce · {}/256 个首字节 · {} 个状态"
    ),
    Filtering => ("Filtering · {} expected states", "筛选中 · 预计 {} 个状态"),
    BruteForce => (
        "Brute force · {}% ({}/{}) · {} expected states",
        "暴力搜索 · {}%（{}/{}）· 预计 {} 个状态"
    ),
    VerifiedKeyLine => ("{} · {} · verified", "{} · {} · 已验证"),
    RecoveryComplete => ("Recovery complete.", "恢复完成。"),
    RecoveryCompleteKeys => ("Recovery complete · {} key(s) verified", "恢复完成 · {} 个密钥已验证"),
    RecoveredKeyVerified => ("Recovered key verified by the service", "恢复的密钥已由服务验证"),
    RecoveryCancelledDot => ("Recovery cancelled.", "恢复已取消。"),
    RecoveryCancelled => ("Recovery cancelled", "恢复已取消"),
    RecoveryStopped => ("Recovery stopped.", "恢复已停止。"),
    RecoveryLimitationSummary => (
        "The engine reported a limitation instead of a generic failure.",
        "引擎报告了限制，而不是一般性失败。"
    ),
    RecoveryLimitation => ("Recovery limitation", "恢复限制"),
    RecoveryLimitationReported => (
        "Recovery limitation reported by the engine",
        "引擎报告了恢复限制"
    ),
    RecoveryFailed => ("Recovery failed", "恢复失败"),
    NoRecoveredKeyToSave => ("No recovered key to save", "没有可保存的恢复密钥"),
    RecoveredKeyAlreadyStored => ("The recovered key is already in the store", "恢复的密钥已在密钥库中"),
    SavedRecoveredKeys => ("Saved {} recovered key(s)", "已保存 {} 个恢复的密钥"),
    ChooseTablesDirectory => ("Choose the hardnested table directory", "选择 hardnested 表目录"),
    KnownBlockLabel => ("known block", "已知块"),
    TargetBlockLabel => ("target block", "目标块"),
    NumberRange => ("{label} must be a number in 0..=255", "{label} 必须是 0..=255 的数字"),
    LabelRange => ("{label} must be in 0..=255", "{label} 必须在 0..=255 范围内"),
    MaxNoncesPositive => ("Max nonces must be zero or positive", "最大 nonce 数必须为零或正数"),
    RecoveryElapsed => ("Elapsed {}", "耗时 {}"),

    // Diagnostics.
    DiagnosticsTitle => ("Diagnostics", "诊断"),
    DiagnosticsNote => (
        "Build and resource facts for this installation. Diagnostics reads the filesystem and enumerates readers; it never reads a card and never reports key material.",
        "此安装的构建与资源信息。诊断会读取文件系统并枚举读卡器；它绝不读卡，也不报告密钥材料。"
    ),
    DiagnosticsVersion => ("NFC Foundry {} · nfctl/service {}", "NFC Foundry {} · nfctl/服务 {}"),
    SupportedReaders => (
        "Supported readers: {}<port> · {}<bus>:<address>",
        "支持的读卡器：{}<port> · {}<bus>:<address>"
    ),
    HardnestedTablesPath => ("Hardnested tables: {}", "Hardnested 表：{}"),
    HardnestedNotResolved => ("Hardnested tables: not resolved ({})", "Hardnested 表：未解析（{}）"),
    ReadersNone => ("Readers: none found", "读卡器：未找到"),
    ReadersCount => ("Readers: {}", "读卡器：{}"),
    BuildLabel => ("Build: {} · {} · {}", "构建：{} · {} · {}"),
    BuildDebug => ("debug", "调试"),
    BuildRelease => ("release", "发布"),

    // Activity.
    ActivityTitle => ("Activity", "活动"),
    ActivityNote => (
        "Session-only record of operations, results, warnings, and backup paths. Key material and NDEF payloads are not logged. Clearing the list does not affect stored files.",
        "仅本次会话的操作、结果、警告和备份路径记录。不会记录密钥材料和 NDEF 负载。清除列表不会影响已保存文件。"
    ),
    TimeColumn => ("Time", "时间"),
    OperationColumn => ("Operation", "操作"),
    ClearActivity => ("Clear activity", "清除活动"),

    // Settings.
    SettingsTitle => ("Settings", "设置"),
    SettingsNote => (
        "Preferences are written atomically with mode 0600 under the XDG configuration directory. Advanced mode and recovery cancellation stay session-only.",
        "偏好设置以 0600 权限原子写入 XDG 配置目录。高级模式和恢复取消仅限当前会话。"
    ),
    KeyStoreField => ("Key store", "密钥库"),
    BackupDirectory => ("Backup directory", "备份目录"),
    HardnestedTables => ("Hardnested tables", "Hardnested 表"),
    UseCommonKeys => ("Use common keys", "使用常用密钥"),
    DefaultDumpFormat => ("Default dump format", "默认转储格式"),
    DefaultKeyStore => ("Default key store", "默认密钥库"),
    DefaultBackupDirectory => ("Default backup directory", "默认备份目录"),
    SavePreferences => ("Save preferences", "保存偏好设置"),
    PreferenceFile => ("Preference file: {}", "偏好设置文件：{}"),
    ConfigUnavailable => ("unavailable (no XDG config directory)", "不可用（无 XDG 配置目录）"),
    NotLoaded => ("Not loaded: {}", "未加载：{}"),
    NotSaved => ("Not saved: {}", "未保存：{}"),
    SavedButKeyStoreFailed => ("Saved preferences but the key store failed: {}", "偏好设置已保存，但密钥库加载失败：{}"),
    PreferencesSaved => ("Preferences saved", "偏好设置已保存"),
    NoConfigPath => ("No configuration path; set XDG_CONFIG_HOME or HOME", "无配置路径；请设置 XDG_CONFIG_HOME 或 HOME"),
    ChooseKeyStore => ("Choose the Classic key store", "选择 Classic 密钥库"),
    ChooseBackupDirectory => ("Choose the backup directory", "选择备份目录"),

    // Generic actions and status.
    LanguageSetting => ("Language", "语言"),
    English => ("English", "English"),
    Chinese => ("简体中文", "简体中文"),
    Save => ("Save", "保存"),
    Close => ("Close", "关闭"),
    ContinueAction => ("Continue", "继续"),
    Text => ("Text", "文本"),
    Sector => ("Sector", "扇区"),
    FingerprintValue => ("Value", "值"),
    ReaderLabel => ("Reader", "读卡器"),
    ReaderWorkbench => ("reader workbench", "读卡工作台"),
    ReaderChanged => ("Reader changed · read the card again", "读卡器已更改 · 请重新读取卡片"),
    ReaderConnecting => ("Connecting…", "连接中…"),
    ReaderPollingCard => ("Polling · card present", "轮询中 · 卡片在位"),
    ReaderPollingNoCard => ("Polling · no card", "轮询中 · 无卡片"),
    ReaderBusy => ("Busy", "忙"),
    ReaderRecovering => ("Recovering", "恢复中"),
    ReaderRecoveringDetail => ("Recovering: {}", "恢复中：{}"),
    ReaderError => ("Error", "错误"),
    ReaderErrorDetail => ("Error: {}", "错误：{}"),
    ReaderCapabilities => ("Capabilities", "读卡器能力"),
    ReaderNotOpened => ("not opened", "未打开"),
    ReaderCapsFull => (
        "MIFARE Classic read/write · Type 2 read/write · raw bytes · raw bits · UID write · recovery",
        "MIFARE Classic 读写 · Type 2 读写 · 原始字节 · 原始位 · UID 写入 · 恢复"
    ),
    ReaderCapsNoRecovery => (
        "MIFARE Classic read/write · Type 2 read/write · raw bytes · raw bits · UID write",
        "MIFARE Classic 读写 · Type 2 读写 · 原始字节 · 原始位 · UID 写入"
    ),
    CardRemovedReadAgain => ("Card removed · read the card again", "卡片已移除 · 请重新读取卡片"),
    DifferentCardReadAgain => (
        "A different card is present · read it again",
        "已更换卡片 · 请重新读取"
    ),
    ReaderUnavailableReadAgain => (
        "Reader unavailable · read the card again",
        "读卡器不可用 · 请重新读取卡片"
    ),
    ReaderErrorReadAgain => ("Reader error · read the card again", "读卡器错误 · 请重新读取卡片"),
    LogMonitor => ("monitor · {message}", "监控 · {message}"),
    NoSupportedReader => ("No supported reader found", "未找到受支持的读卡器"),
    NoReaderFound => ("No reader found", "未找到读卡器"),
    InspectionOnlyReadAgain => ("Inspection only · read the card again", "仅检查 · 请重新读取卡片"),
    InspectionOnlyReadAgainToWrite => ("Inspection only · read again to write", "仅检查 · 重新读取后可写入"),
    ReadingCard => ("Reading card…", "正在读取卡片…"),
    ReadCompleteBlocks => ("Read complete · {} blocks", "读取完成 · {} 个数据块"),
    ReadCompletePages => ("Read complete · {}/{} pages", "读取完成 · {}/{} 页"),
    ReadFailed => ("Read failed", "读取失败"),
    CardReadComplete => ("Card read complete", "卡片读取完成"),
    CardReadWarnings => (
        "Card read with {} warning(s); see the activity log",
        "卡片读取完成，有 {} 个警告；请查看活动日志"
    ),
    Type2ReadComplete => ("Type 2 tag read complete", "Type 2 标签读取完成"),
    Type2ReadWarnings => (
        "Type 2 read with {} warning(s); see the activity log",
        "Type 2 读取完成，有 {} 个警告；请查看活动日志"
    ),
    UnsupportedReadResult => ("unsupported read result", "不支持的读取结果"),
    ReadingBlock => ("Reading block {block:03}…", "正在读取数据块 {block:03}…"),
    BlockRead => ("Block {block:03} read", "数据块 {block:03} 已读取"),
    BlockReadFailed => ("Block read failed", "数据块读取失败"),
    BlockZeroReadOnly => ("Block 0 is read-only", "块 0 为只读"),
    BlockNotInDump => ("The selected block is not in the current dump", "所选数据块不在当前转储中"),
    BlockAlreadyContains => ("The block already contains these bytes", "数据块已包含这些字节"),
    StageEdit => ("Stage edit", "暂存编辑"),
    WriteChangesAction => ("Write changes…", "写入更改…"),
    UndoEdit => ("Undo edit", "撤销编辑"),
    RevertEdits => ("Revert edits", "还原编辑"),
    WorkbenchNote => (
        "Edits change the working image only. Write changes confirms a preflight plan, backs up the card, and reads every block back. Block 0 stays read-only.",
        "编辑只修改工作镜像。写入更改会确认预检计划、备份卡片并回读每个数据块。块 0 保持只读。"
    ),
    AllKnownBlocks => ("All known blocks", "全部已知块"),
    ChangesOnly => ("Changes only", "仅更改块"),
    SelectedBlocks => ("Selected blocks", "选定块"),
    BlocksPlaceholder => ("Blocks, e.g. 1,4,5-7", "块号，例如 1,4,5-7"),
    StagedBlock => ("Block {block:03} staged", "数据块 {block:03} 已暂存"),
    EditUndone => ("Edit undone", "已撤销编辑"),
    EditsReverted => ("Edits reverted", "已还原编辑"),
    BuildingWritePlan => ("Building the write plan…", "正在生成写入计划…"),
    WritingPlannedBlocks => ("Writing the planned blocks…", "正在写入计划的块…"),
    NoWritesPlanned => ("No writes planned", "没有计划写入"),
    CardAlreadyMatches => ("The card already matches the selected blocks", "卡片已与所选块一致"),
    WritePreflightFailed => ("Write preflight failed", "写入预检失败"),
    WriteChangesNeedsLive => ("Writing changes needs a live selected card", "写入更改需要在线选中的卡片"),
    NoWorkbenchImage => ("No workbench image is loaded", "未加载工作镜像"),
    NoEditsToWrite => ("There are no edits to write", "没有可写入的编辑"),
    WriteChangesQuestion => ("Write the edited blocks?", "写入已编辑的数据块？"),
    WriteChangesActionLabel => ("Write changes", "写入更改"),
    OperationPlan => ("plan", "计划"),
    OperationWriteChanges => ("write changes", "写入更改"),
    PlanCounts => ("{} block(s), {} trailer(s)", "{} 个数据块、{} 个扇区尾块"),
    PlanSelectionAll => ("all known blocks", "全部已知块"),
    PlanSelectionChanges => ("changed blocks only", "仅更改块"),
    PlanSelectionBlocks => ("{} selected block(s)", "{} 个选定块"),
    WriteWith => ("write with {}", "使用 {} 写入"),
    KeyAOrB => ("Key A or B", "密钥 A 或 B"),
    KeyAType => ("Key A", "密钥 A"),
    KeyBType => ("Key B", "密钥 B"),
    NoWriteKey => ("no key", "无可用密钥"),
    PlanSummaryFirst => ("{} · UID {}", "{} · UID {}"),
    PlanSummaryBlocks => ("{} · {} block(s), {} trailer(s)", "{} · {} 个数据块、{} 个扇区尾块"),
    PlanSummarySkipped => ("{} unchanged, {} unknown skipped · plan expires at {}", "{} 个未变，跳过 {} 个未知数据块 · 计划到期于 {}"),
    PlanBackupNote => (
        "A project backup is written before the write, and every block is read back. Block 0 is never written.",
        "写入前会保存项目备份，并回读每个数据块。块 0 永不写入。"
    ),
    PlanBlockLine => ("Block {block:03} · sector {sector:02} · {role} · {access}", "数据块 {block:03} · 扇区 {sector:02} · {role} · {access}"),
    PlanOldLine => ("  old {}", "  旧值 {}"),
    PlanNewLine => ("  new {}", "  新值 {}"),
    PlanMoreBlocks => ("… and {} more block(s)", "… 还有 {} 个数据块"),
    ChangesWrittenStatus => ("Changes written · {} block(s)", "更改已写入 · {} 个数据块"),
    ChangesWrittenToast => ("Changes written and read back", "更改已写入并回读校验"),
    WriteFinishedFailures => ("Write finished with failures; see the activity log", "写入完成但有失败；请查看活动日志"),
    WriteBlockQuestion => ("Write block {}?", "写入数据块 {}？"),
    SectorTrailer => ("sector trailer", "扇区尾块"),
    DataBlock => ("data block", "数据块"),
    ManufacturerBlock => ("manufacturer", "厂商块"),
    WriteBlockBody => (
        "Sector {sector:02} · {role}\n\nOld  {old}\nNew  {new}\n\nA project backup is written before the write, and the block is read back afterwards.",
        "扇区 {sector:02} · {role}\n\n旧值  {old}\n新值  {new}\n\n写入前会保存项目备份，写入后回读该数据块。"
    ),
    WriteTrailer => ("Write trailer", "写入扇区尾块"),
    WritingBlock => ("Writing block {block:03}…", "正在写入数据块 {block:03}…"),
    BlockWritten => ("Block {block:03} written", "数据块 {block:03} 已写入"),
    BlockWrittenVerified => ("Block {block:03} written and read back", "数据块 {block:03} 已写入并回读校验"),
    NoChange => ("No change", "无变化"),
    WriteFailed => ("Write failed", "写入失败"),
    WriteNdefAction => ("Write NDEF", "写入 NDEF"),
    FormatNdef => ("Format NDEF", "格式化 NDEF"),
    CancelAction => ("Cancel", "取消"),
    RestoreActionLabel => ("Restore", "恢复写入"),
    SaveAction => ("Save", "保存"),
    ReaderServicePoisoned => ("Reader service lock was poisoned", "读卡器服务锁已损坏"),
    NoReaderSelectedError => ("No reader selected", "未选择读卡器"),
    ReaderListFailed => ("Reader list failed", "读卡器列表读取失败"),
    ReaderListRefreshed => ("Reader list refreshed", "读卡器列表已刷新"),
    RefreshNoReader => ("refresh · no supported reader", "刷新 · 无受支持的读卡器"),
    ReaderSelectedLog => ("reader selected · {}", "已选择读卡器 · {}"),
    SavedPath => ("Saved {}", "已保存 {}"),
    CouldNotWrite => ("Could not write {}: {}", "无法写入 {}：{}"),
    CouldNotRead => ("Could not read {}: {}", "无法读取 {}：{}"),
    ReadComplete => ("Read complete", "读取完成"),
    ValueLabel => ("Value", "值"),
    NoBackup => ("no backup", "无备份"),
    NoBackupNeeded => ("no backup needed", "无需备份"),
    UnknownPage => ("unknown", "未知"),
    NoneValue => ("none", "无"),
    Yes => ("yes", "是"),
    No => ("no", "否"),
    Read => ("read", "已读取"),
    Unread => ("unread", "未读取"),
    NoKey => ("no key", "无密钥"),
    ReadFailedShort => ("read failed", "读取失败"),
    Synthetic => ("synthetic", "合成"),
    VerifiedState => ("verified", "已验证"),
    ChangedState => ("changed", "已变化"),
    EqualState => ("equal", "相同"),
    UnknownState => ("unknown", "未知"),
    StaticLock => ("static", "静态"),
    DynamicLock => ("dynamic", "动态"),
    AuthProtected => ("auth", "需认证"),
    LockPage => ("lock page", "锁定页"),
    Locked => ("locked", "已锁定"),
    Writable => ("writable", "可写"),
    WriteOnly => ("write-only", "只写"),
    RoleManufacturer => ("Manufacturer", "厂商"),
    RoleReserved => ("Reserved", "保留"),
    RoleCapability => ("Capability container", "容量容器"),
    RoleUserMemory => ("User memory", "用户存储"),
    RoleStaticLock => ("Static lock", "静态锁定"),
    RoleDynamicLock => ("Dynamic lock", "动态锁定"),
    RoleConfiguration => ("Configuration", "配置"),
    RolePassword => ("Password", "密码"),
    RolePasswordAck => ("Password ack", "密码确认"),
    ConfigUnformatted => ("unformatted", "未格式化"),
    ConfigReadOnly => ("read-only", "只读"),
    BlockMeta => ("Sector {} · {} · {}", "扇区 {} · {} · {}"),
    SectorRole => ("Sector {} · {}", "扇区 {} · {}"),
    PairLine => ("{} · {}", "{} · {}"),
    LogReaderSelectedOperation => ("reader selected", "已选择读卡器"),
    StatusColumn => ("Status", "状态"),
    Hex4Placeholder => ("4 hex digits", "4 位十六进制"),
    WriteBodyNote => (
        "A project backup is written before the write, and the block is read back afterwards.",
        "写入前会保存项目备份，写入后回读该数据块。"
    ),
    CardIdentityDetail => ("the card identity changed", "卡片标识已改变"),
    OldValueLine => ("Old  {}", "旧值  {}"),
    NewValueLine => ("New  {}", "新值  {}"),
    NewBytesLine => ("New bytes  {}", "新字节  {}"),
    RestoreClassLine => ("Restore {}", "恢复写入 {}"),
    ReadOnlyAfterFormatLine => ("Read-only after format: {}", "格式化后只读：{}"),
    PagesLine => ("Pages {}", "页 {}"),
    StaticLockLine => ("Static lock {} → {}", "静态锁定 {} → {}"),
    DynamicLockLine => ("Dynamic lock {} → {}", "动态锁定 {} → {}"),
    PagesRangeLine => ("Pages {}..={} ({} bytes)", "页 {}..={}（{} 字节）"),
    LockOneWayLine => (
        "Lock bits are one-way; set bits can never be cleared. A project backup is written first, and the lock pages are read back.",
        "锁定位是单向的；已设置的位永远不能清除。写入前会保存项目备份，并回读锁定页。"
    ),
    ProjectBackupWriteLine => (
        "A project backup is written before the write. Data blocks are written before sector trailers, and every block is read back. Block 0 is never written.",
        "写入前会保存项目备份。先写数据块再写扇区尾块，并回读每个数据块。块 0 永不写入。"
    ),
    ProjectBackupPageLine => (
        "A project backup is written before the write, and the page is read back afterwards.",
        "写入前会保存项目备份，写入后回读该页。"
    ),
    ProjectBackupFormatLine => (
        "A project backup is written before the format, and every written block is read back afterwards.",
        "格式化前会保存项目备份，并回读每个写入的数据块。"
    ),
    PageTitle => ("Page {}", "第 {} 页"),
    BlockTitle => ("Block {}", "数据块 {}"),

    // Settings field values and dialog labels.
    DialogConfirm => ("Confirm", "确认"),
    CancelResponse => ("Cancel", "取消"),
    SaveResponse => ("Choose file…", "选择文件…"),

    // Activity/log detail prefixes.
    LogWarning => ("warning · {message}", "警告 · {message}"),
    LogError => ("error · {message}", "错误 · {message}"),
    LogFailure => ("failure · {message}", "失败 · {message}"),
    LogRead => ("read · {message}", "读取 · {message}"),
    LogWrite => ("write · {message}", "写入 · {message}"),
    LogLoad => ("load · {message}", "加载 · {message}"),
    LogSave => ("save · {message}", "保存 · {message}"),
    LogDiff => ("diff · {message}", "比较 · {message}"),
    LogRestore => ("restore · {message}", "恢复写入 · {message}"),
    LogRefresh => ("refresh · {message}", "刷新 · {message}"),
    LogKeys => ("keys · {message}", "密钥 · {message}"),
    LogKeyscan => ("keyscan · {message}", "密钥扫描 · {message}"),
    LogRecovery => ("recovery · {message}", "恢复 · {message}"),
    LogSettings => ("settings · {message}", "设置 · {message}"),
    LogReaderSelected => ("reader selected · {message}", "已选择读卡器 · {message}"),
    LogType2Dump => ("type2 dump · {message}", "Type 2 转储 · {message}"),
    LogType2Restore => ("type2 restore · {message}", "Type 2 恢复写入 · {message}"),
    LogType2Write => ("type2 write · {message}", "Type 2 写入 · {message}"),
    LogType2Lock => ("type2 lock · {message}", "Type 2 锁定 · {message}"),
    LogType2Password => ("type2 password · {message}", "Type 2 密码 · {message}"),
    LogNdefWrite => ("ndef write · {message}", "NDEF 写入 · {message}"),
    LogNdefFormat => ("ndef format · {message}", "NDEF 格式化 · {message}"),
    LogType2NdefWrite => ("type2 ndef write · {message}", "Type 2 NDEF 写入 · {message}"),
    LogType2NdefFormat => ("type2 ndef format · {message}", "Type 2 NDEF 格式化 · {message}"),
    LogReadBlock => ("read block · {message}", "读取数据块 · {message}"),
    LogInspectionOnly => ("inspection only · {message}", "仅检查 · {message}"),
    LogCancelled => ("cancelled", "已取消"),

    // Block inspector decoded details.
    InspectorPartial => ("Some bytes were not read; unknown bytes are never treated as zero.", "部分字节未读取；未知字节绝不视为零。"),
    BccNotChecked => ("BCC: not checked", "BCC：未检查"),
    BccValid => ("BCC {:02X} · valid", "BCC {:02X} · 有效"),
    BccNoIdentity => ("BCC: no card identity; cannot check", "BCC：无卡片标识；无法检查"),
    BccNotApplicable => ("BCC: not applicable for this UID length", "BCC：此 UID 长度不适用"),
    BccMismatch => (
        "BCC {actual:02X} does not match the UID (expected {expected:02X})",
        "BCC {actual:02X} 与 UID 不匹配（应为 {expected:02X}）"
    ),
    SectorZeroAccess => (
        "Sector 0 access bits {} · group 0 · {} · {}",
        "扇区 0 访问位 {} · 组 0 · {} · {}"
    ),
    AccessGroup => ("Access group {group} · {} · {}", "访问组 {group} · {} · {}"),
    ValueBlockLine => ("Value block · value {} · address {}", "值块 · 值 {} · 地址 {}"),
    TrailerAccessHeader => (
        "Sector {sector} trailer · access bytes {} {}",
        "扇区 {sector} 尾块 · 访问字节 {} {}"
    ),
    TrailerGroup => ("Group {group} · {} · {} · {}", "组 {group} · {} · {} · {}"),
    TrailerGroup3 => (
        "Group 3 · {} · {} · Key A write {} · access write {} · Key B write {} · Key B readable {}",
        "组 3 · {} · {} · Key A 写 {} · 访问位写 {} · Key B 写 {} · Key B 可读 {}"
    ),
    TrailerKeys => ("Key A {} · Key B {} · GPB {:02X}", "Key A {} · Key B {} · GPB {:02X}"),
    ConditionCodes => ("groups {}", "组 {}"),
    DataPermissions => (
        "read {} · write {} · increment {} · decrement {}",
        "读 {} · 写 {} · 加 {} · 减 {}"
    ),
    GroupBlocksRange => ("blocks {}-{}", "数据块 {}-{}"),
    GroupBlockSingle => ("block {}", "数据块 {}"),
    DiffKeyAChanged => ("Key A {} → {}", "Key A {} → {}"),
    DiffKeyBChanged => ("Key B {} → {}", "Key B {} → {}"),
    DiffAccessChanged => ("access {} → {}", "访问位 {} → {}"),
    DiffGpbChanged => ("GPB {:02X} → {:02X}", "GPB {:02X} → {:02X}"),
    DiffUnknownBytes => ("unknown bytes on one or both sides", "一侧或两侧存在未知字节"),
    DiffMore => ("+{} more", "另有 {} 项"),
    SectorsBlocks => ("{} sectors · {} blocks", "{} 个扇区 · {} 个数据块"),
    BlocksReadable => ("{}/{} blocks readable{}", "{}/{} 个数据块可读{}"),
    PagesReadable => ("{}/{} pages readable{}", "{}/{} 页可读{}"),
    PagesCount => ("{} pages", "{} 页"),
    BlocksCount => ("{} blocks", "{} 个数据块"),
    SavedCount => ("{} saved", "已保存 {}"),
    Type2CardDetail => ("{} pages · CC {} · NDEF {}", "{} 页 · CC {} · NDEF {}"),
    Type2SummaryLine => (
        "{}/{} pages readable · CC {} · NDEF {}",
        "{}/{} 页可读 · CC {} · NDEF {}"
    ),

    // Inspector and Type 2 decoded details.
    Type2UidLow => ("UID bytes 0..2: {}", "UID 字节 0..2：{}"),
    Type2UidHigh => ("UID bytes 3..6: {}", "UID 字节 3..6：{}"),
    Bcc0 => ("BCC0 check byte: {}", "BCC0 校验字节：{}"),
    ManufacturerReadOnly => ("Manufacturer and UID pages are read-only.", "厂商页和 UID 页为只读。"),
    StaticLockBytes => ("Static lock bytes: {}", "静态锁定位：{}"),
    StaticallyLockedPages => ("Statically locked pages: {}", "静态锁定页：{}"),
    StaticLockNotReadable => ("Static lock bytes are not readable.", "静态锁定位不可读。"),
    LockOneWayPage => (
        "Lock bits are one-way: a set bit can never be cleared by a page write.",
        "锁定位是单向的：已设置的位永远不能被页写入清除。"
    ),
    CcVersion => ("CC {} · version {}.{}", "CC {} · 版本 {}.{}"),
    CcDataArea => ("Data area {} bytes · NDEF capacity {} bytes", "数据区域 {} 字节 · NDEF 容量 {} 字节"),
    CcAccess => ("Read access {} · write access {} · {}", "读访问 {} · 写访问 {} · {}"),
    DataAreaWritable => ("data area writable", "数据区域可写"),
    DataAreaReadOnly => ("data area read-only", "数据区域只读"),
    InvalidCc => ("Invalid capability container: {}", "容量容器无效：{}"),
    CcNotReadable => ("The capability container is not readable.", "容量容器不可读。"),
    NdefOffset => ("NDEF data area offset {} of {} bytes", "NDEF 数据区域偏移 {}，共 {} 字节"),
    OutsideCc => ("Outside the CC data area.", "位于 CC 数据区域之外。"),
    DynamicLockBytes => ("Dynamic lock bytes: {}", "动态锁定位：{}"),
    LockPageGranularity => (
        "Lock bits are one-way and page granularity follows the product.",
        "锁定位是单向的，页粒度由产品决定。"
    ),
    ConfigNotReadable => ("The NTAG configuration pages are not readable.", "NTAG 配置页不可读。"),
    PwdWriteOnly => (
        "PWD is write-only: reads return zero or undefined data. Use the Protection form to program PWD, PACK, AUTH0, and ACCESS together.",
        "PWD 只可写：读取返回零或未定义数据。请使用保护表单一起编程 PWD、PACK、AUTH0 和 ACCESS。"
    ),
    PackWriteOnly => (
        "PACK is write-only: the value is verified with PWD_AUTH, not by reading the page.",
        "PACK 只可写：该值通过 PWD_AUTH 校验，而不是读取该页。"
    ),
    ReservedNoWrite => ("Reserved page; no write path.", "保留页；无写入路径。"),
    PasswordStartsPage => (
        "Password protection starts at page {}; enter the access password and PACK in the Protection tab.",
        "密码保护从第 {} 页开始；请在保护页输入访问密码和 PACK。"
    ),
    WriteReady => ("Write: ready.", "写入：就绪。"),
    WriteLabel => ("Write: {}", "写入：{}"),
    MirrorLine => ("MIRROR {} · mirror page {}", "MIRROR {} · 镜像页 {}"),
    AuthLine => ("AUTH0 {} · ACCESS {} · AUTH_LIM {}", "AUTH0 {} · ACCESS {} · AUTH_LIM {}"),
    CfgLine => ("CFGLCK {} · PROT {} · protected from {}", "CFGLCK {} · PROT {} · 保护起始 {}"),
    OnOffOn => ("on", "开"),
    OnOffOff => ("off", "关"),
    Never => ("never", "从不"),
    PwdNeverReadable => ("PWD and PACK are never readable.", "PWD 和 PACK 永不可读。"),
    AuthLabel => ("auth", "需认证"),
    EmptyState => ("empty", "空"),
    RecordsCount => ("{} record(s)", "{} 条记录"),
    ReversedRange => ("page range {} is reversed", "页范围 {} 顺序颠倒"),
    EnterOnePage => ("Enter at least one page.", "请至少输入一页。"),
    NotPageNumber => ("{} is not a page number", "{} 不是页号"),
    NotByteValue => ("{} is not a byte value (0..=255)", "{} 不是字节值（0..=255）"),
    HexDigitsGot => ("{} is {} hexadecimal digits, got {}", "{} 应为 {} 位十六进制，实际为 {}"),
    NotHexDigit => ("{} is not a hexadecimal digit", "{} 不是十六进制数字"),
    ExpectedHex32 => ("expected 32 hexadecimal digits, got {}", "应为 32 位十六进制，实际为 {}"),
    InvalidHexByte => ("invalid hexadecimal byte {}", "无效的十六进制字节 {}"),
    DraftText => ("Text · language {}", "文本 · 语言 {}"),
    DraftUri => ("URI · {}", "URI · {}"),
    DraftRaw => ("Raw NDEF · {} record(s) · {}", "原始 NDEF · {} 条记录 · {}"),
    DraftRawNone => ("Raw NDEF · no file chosen", "原始 NDEF · 未选择文件"),
    ChooseRawFirst => ("choose a raw NDEF file first", "请先选择原始 NDEF 文件"),
    CouldNotReadLower => ("could not read {}: {}", "无法读取 {}：{}"),
    InvalidNdef => ("{} is not a valid NDEF message: {}", "{} 不是有效的 NDEF 消息：{}"),
    CapacityNeeds => (
        "The message needs {} bytes; the NDEF area holds at most {}.",
        "消息需要 {} 字节；NDEF 区域最多容纳 {} 字节。"
    ),
    DictionaryEmpty => ("The dictionary {} contains no keys", "字典 {} 不包含密钥"),
    AdvancedRequiredPage => ("Advanced mode is required for this page.", "此页需要高级模式。"),
    TargetLine => ("Target {} · UID {}", "目标 {} · UID {}"),
    SourceLine => ("Source {} · UID {}", "来源 {} · UID {}"),
    LiveBackupLine => (
        "A project backup of the live tag is written before the restore, and every written page is read back. Manufacturer and UID pages are never written.",
        "恢复写入前会保存在线标签的项目备份，并回读每一页。厂商页和 UID 页永不写入。"
    ),
    PageLine => ("Page {} · {}", "第 {} 页 · {}"),
    NdefDataReplaced => (
        "The NDEF data area is replaced with an empty NDEF Message TLV; proprietary TLVs are removed. A project backup is written first, and every written page is read back.",
        "NDEF 数据区域将替换为空 NDEF 消息 TLV；专有 TLV 会被移除。写入前会保存项目备份，并回读每个写入页。"
    ),
    FormatReplaces => (
        "Format replaces the MAD, the NDEF Message TLV, sector keys, and access bits on every NFC sector.",
        "格式化会替换每个 NFC 扇区的 MAD、NDEF 消息 TLV、扇区密钥和访问位。"
    ),
    ClassicUidWrite => ("Write UID…", "写入 UID…"),
    ClassicUidPlaceholder => ("Target UID, e.g. 04A2248A", "目标 UID，例如 04A2248A"),
    ClassicUidNote => (
        "UID writes need Advanced mode, a magic CUID/Gen2 Classic 1K card, a sector-0 backup, and a confirmation dialog. An ordinary card refuses the write unchanged.",
        "UID 写入需要高级模式、魔术 CUID/Gen2 Classic 1K 卡、扇区 0 备份以及精确输入确认。普通卡会拒绝写入且数据不变。"
    ),
    ClassicUidAdvancedRequired => (
        "Enable Advanced mode before changing the UID",
        "更改 UID 前请启用高级模式"
    ),
    ClassicUidNeedsLive => (
        "A UID write needs a live selected card",
        "UID 写入需要已连接的选中卡片"
    ),
    ClassicUidReadFirst => ("Read a card before changing its UID", "更改 UID 前请先读取卡片"),
    ClassicUidRequiresCard => (
        "UID writes require a four-byte UID MIFARE Classic 1K card",
        "UID 写入需要四字节 UID 的 MIFARE Classic 1K 卡"
    ),
    ClassicUidSame => ("The target UID already matches the card", "目标 UID 已与卡片一致"),
    ClassicUidPlanReading => (
        "Reading sector 0 for the UID plan…",
        "正在读取扇区 0 以生成 UID 计划…"
    ),
    ClassicUidPlanFailed => ("UID plan failed", "UID 计划失败"),
    ClassicUidDialogHeading => ("Write manufacturer block 0?", "写入厂商块 0？"),
    ClassicUidDialogConfirm => ("Write UID", "写入 UID"),
    ClassicUidWriting => ("Writing manufacturer block 0…", "正在写入厂商块 0…"),
    ClassicUidNotNeeded => ("No UID write was needed", "无需写入 UID"),
    ClassicUidChanged => ("UID changed to {}", "UID 已更改为 {}"),
    ClassicUidReread => ("UID changed · re-reading the card", "UID 已更改 · 正在重新读取卡片"),
    ClassicUidFailed => ("UID write failed", "UID 写入失败"),
    RestoreUidLabel => ("Target UID after restore (optional)", "恢复后目标 UID（可选）"),
    RestoreUidPlaceholder => ("Target UID, e.g. DEADBEEF", "目标 UID，例如 DEADBEEF"),
    RestoreUidNeedsAdvanced => (
        "Enable Advanced mode to write a target UID",
        "写入目标 UID 前请启用高级模式"
    ),
    RestoreUidNeedsFull => (
        "A UID target needs a full restore; remove the block selection or changes-only mode.",
        "写入目标 UID 需要完整恢复；请移除块选择或仅变更模式。"
    ),
    RestoreUidPlan => (
        "UID target {} · current UID {} · post-restore key {}",
        "UID 目标 {} · 当前 UID {} · 恢复后密钥 {}"
    ),
    RestoreUidAfterRestore => (
        "The UID is written only after every restored block and sector 0 are read back and verified.",
        "仅在每个恢复块和扇区 0 均回读验证后才会写入 UID。"
    ),
    RestoreUidDialogHeading => (
        "Restore the plan and write the target UID?",
        "恢复计划并写入目标 UID？"
    ),
    RestoreUidDialogConfirm => ("Restore and write UID", "恢复并写入 UID"),
    RestoreUidChanged => ("UID changed to {} · re-reading the card", "UID 已更改为 {} · 正在重新读取卡片"),
    RestoreUidFailed => (
        "Dump restore finished; UID write failed",
        "转储恢复已完成；UID 写入失败"
    ),
    RestoreUidNotAttempted => ("UID was not written: {}", "未写入 UID：{}"),

    // Card details and health.
    DetailsPage => ("Details", "详情"),
    DetailsTitle => ("Card details and health", "卡片详情与健康"),
    DetailsNote => (
        "Identity, image completeness, validation findings, and verified key coverage for the loaded image. Values come from the same dump and key store the block table uses; no card is read and no key material is shown.",
        "当前已加载镜像的标识、完整性、校验结果和已验证密钥覆盖。数值来自与数据块表相同的转储和密钥库；不读取卡片，也不显示密钥内容。"
    ),
    NoCardImage => ("No card image loaded", "未加载卡片镜像"),
    IdentitySection => ("Identity", "标识"),
    HealthCardLine => ("Card: {} · UID {} ({} bytes)", "卡片：{} · UID {}（{} 字节）"),
    HealthAtqaLine => (
        "ATQA {:02X}{:02X} · SAK {:02X} · inferred {}",
        "ATQA {:02X}{:02X} · SAK {:02X} · 推断 {}"
    ),
    CardIdentityMissing => ("Card identity: not recorded", "卡片标识：未记录"),
    ImageSection => ("Image", "镜像"),
    HealthLayoutLine => ("Layout: {}", "布局：{}"),
    HealthSourceLine => ("Source: {}", "来源：{}"),
    HealthStateLine => ("State: {}", "状态：{}"),
    HealthLiveState => ("live · writes enabled", "在线 · 可写入"),
    HealthReadAtLine => ("Read at: unix {}..={}", "读取时间：Unix {}..={}"),
    HealthReadAtMissing => ("Read at: not recorded", "读取时间：未记录"),
    CompletenessSection => ("Completeness", "完整性"),
    HealthImageLine => ("Image: {}", "镜像：{}"),
    ImageComplete => ("complete", "完整"),
    ImageIncomplete => ("incomplete", "不完整"),
    HealthBlocksLine => (
        "Blocks: {}/{} readable · sectors: {}/{}",
        "数据块：{}/{} 可读 · 扇区：{}/{}"
    ),
    HealthSkippedSectors => ("Skipped sectors: {}", "跳过的扇区：{}"),
    KeysSection => ("Keys", "密钥"),
    HealthVerifiedLine => (
        "Verified: {}/{} sectors · A {} · B {} · both {}",
        "已验证：{}/{} 个扇区 · A {} · B {} · 两者 {}"
    ),
    HealthDumpKeys => ("Dump keys: {}", "转储密钥：{}"),
    WorkbenchSection => ("Workbench", "工作区"),
    HealthDirtyNone => ("Dirty: none", "未保存更改：无"),
    HealthDirtyLine => (
        "Dirty: {} block(s) · {}",
        "未保存更改：{} 个数据块 · {}"
    ),
    ValidationSection => ("Validation", "校验"),
    ValidationOk => ("Validation: ok", "校验：通过"),
    HealthErrorLine => ("Error: {}", "错误：{}"),
    HealthWarningLine => ("Warning: {}", "警告：{}"),
    ReaderLine => ("Reader: {}", "读卡器：{}"),

    // Key catalog management.
    DeleteSelected => ("Delete selected", "删除所选"),
    BuiltInSource => ("built-in", "内置"),
    KeyKindBoth => ("A+B", "A+B（双向）"),
    StoredKeyCount => (
        "{} stored key(s) · {} built-in · {}",
        "{} 个已保存密钥 · {} 个内置 · {}"
    ),
    CandidateBuiltInCount => (
        "{} saved · {} built-in",
        "已保存 {} · {} 个内置"
    ),
    BuiltInKeysOff => ("built-in keys off", "内置密钥已关闭"),
    ImportReportCounts => (
        "Import: {} added · {} duplicate(s) · {} ignored · {} issue(s)",
        "导入：新增 {} · {} 个重复 · {} 个忽略 · {} 个问题"
    ),
    ImportIssueLine => ("Line {}: {}", "第 {} 行：{}"),
    ImportedKeysDetail => (
        "Imported {} key(s) · {} ignored · {} issue(s)",
        "已导入 {} 个密钥 · 忽略 {} 个 · {} 个问题"
    ),
    BuiltInNotDeletable => (
        "Built-in common keys are not stored; there is nothing to delete",
        "内置常用密钥不保存在库中，无需删除"
    ),
    KeyDeleted => ("Deleted the selected key", "已删除所选密钥"),
    SelectStoredKey => ("Select a stored key first", "请先选择一个已保存的密钥"),
    KeyEntryMissing => ("The selected key entry no longer exists", "所选密钥条目已不存在"),

    // Diagnostics details.
    BuildDateLine => ("Build date: {}", "构建日期：{}"),
    AppCommitLine => ("NFC Foundry {} · commit {}", "NFC Foundry {} · 提交 {}"),
    PlatformLine => ("Platform: {} · {}", "平台：{} · {}"),
    SupportedReaderLine => ("Supported readers: {}", "支持的读卡器：{}"),
    FirmwareLine => ("Firmware: {}", "固件：{}"),
    FirmwareNotOpened => ("Firmware: reader not opened", "固件：读卡器未打开"),
    ReaderCapabilitiesLine => ("Reader capabilities: {}", "读卡器能力：{}"),
    ReadersUnavailable => ("Readers: unavailable ({})", "读卡器：不可用（{}）"),
    ReaderSelectedLine => ("Selected: {} · {}", "已选择：{} · {}"),
    ReaderStatusConnecting => ("connecting", "连接中"),
    ReaderStatusPolling => ("polling", "轮询中"),
    ReaderStatusBusy => ("busy", "忙"),
    ReaderStatusRecovering => ("recovering", "恢复中"),
    ReaderStatusError => ("error", "错误"),
    MockReaderFirmware => ("mock reader firmware", "模拟读卡器固件"),
    RawBytes => ("raw bytes", "原始字节"),
    RawBits => ("raw bits", "原始位"),
    CapabilitiesNotOpened => ("not opened", "未打开"),
    CardRailLine => ("Card: {} · UID {}{}", "卡片：{} · UID {}{}"),
    CardRailNone => ("Card: none", "卡片：无"),
    UnknownCardType => ("unknown card", "未知卡片"),
    IdentityOnlySuffix => (" · identity only", " · 仅标识"),
    NoValueRecorded => ("not recorded", "未记录"),

    // Recovery authorization.
    RecoveryAuthHeading => (
        "Start MIFARE Classic recovery?",
        "开始 MIFARE Classic 密钥恢复？"
    ),
    RecoveryAuthStrategy => ("Strategy: {}", "策略：{}"),
    RecoveryAuthTarget => ("Target: {}", "目标：{}"),
    RecoveryAuthBody => (
        "Recovery authenticates and reads; it never writes the card. The selected reader stays busy until the attack stops or is cancelled.",
        "恢复仅做认证和读取，绝不写卡。攻击停止或取消前，所选读卡器保持占用。"
    ),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_both_languages() {
        let mut seen = BTreeSet::new();
        for key in Key::ALL {
            assert!(seen.insert(format!("{key:?}")), "duplicate key {key:?}");
            let en = english(*key);
            let zh = chinese(*key);
            assert!(!en.trim().is_empty(), "empty English value for {key:?}");
            assert!(!zh.trim().is_empty(), "empty Chinese value for {key:?}");
            assert_eq!(
                pattern_parts(en).1,
                pattern_parts(zh).1,
                "placeholder count differs for {key:?}"
            );
        }
    }

    #[test]
    fn english_is_the_base_and_chinese_switches() {
        let en = Translator::new(Language::En);
        let zh = Translator::new(Language::ZhCn);
        assert_eq!(en.localized(Key::ReadCard), "Read card");
        assert_eq!(zh.localized(Key::ReadCard), "读取卡片");
        assert_eq!(en.localized(Key::AppSubtitle), "reader workbench");
    }

    #[test]
    fn header_subtitle_round_trips_in_chinese() {
        let zh = Translator::new(Language::ZhCn);
        assert_eq!(zh.localized(Key::AppSubtitle), "读卡工作台");
        assert_eq!(zh.translate("读卡工作台"), "读卡工作台");
        assert_eq!(zh.translate("reader workbench"), "读卡工作台");
    }

    #[test]
    fn exact_and_pattern_lines_translate_both_ways() {
        let en = Translator::new(Language::En);
        let zh = Translator::new(Language::ZhCn);
        assert_eq!(zh.translate("Ready"), "就绪");
        assert_eq!(
            zh.translate("Read complete · 64 blocks"),
            "读取完成 · 64 个数据块"
        );
        assert_eq!(
            en.translate("读取完成 · 64 个数据块"),
            "Read complete · 64 blocks"
        );
        assert_eq!(zh.translate("UID 04 A2 24 8A"), "UID 04 A2 24 8A");
        assert_eq!(zh.translate("MIFARE Classic 1K"), "MIFARE Classic 1K");
    }

    #[test]
    fn multi_line_translation_keeps_breaks() {
        let zh = Translator::new(Language::ZhCn);
        assert_eq!(
            zh.translate("Select a reader and read a card.\nNo card"),
            "选择读卡器并读取卡片。\n无卡片"
        );
    }

    #[test]
    fn locale_detection_prefers_saved_then_lc_all_then_lang() {
        let env = |name: &str| match name {
            "LC_ALL" => Some("zh_CN.UTF-8".to_owned()),
            "LANG" => Some("en_US.UTF-8".to_owned()),
            _ => None,
        };
        assert_eq!(detect_language(Some(Language::En), env), Language::En);
        assert_eq!(detect_language(None, env), Language::ZhCn);
        assert_eq!(
            detect_language(None, |name| match name {
                "LANG" => Some("en_GB.UTF-8".to_owned()),
                _ => None,
            }),
            Language::En
        );
        assert_eq!(detect_language(None, |_| None), Language::En);
    }

    #[test]
    fn locale_values_resolve_to_supported_languages() {
        assert_eq!(locale_language("zh_CN.UTF-8"), Some(Language::ZhCn));
        assert_eq!(locale_language("zh-Hans-CN"), Some(Language::ZhCn));
        assert_eq!(locale_language("de_DE.UTF-8"), Some(Language::En));
        assert_eq!(locale_language("C.UTF-8"), Some(Language::En));
        assert_eq!(locale_language(""), None);
    }

    #[test]
    fn preference_codes_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_code(language.code()), Some(language));
        }
        assert_eq!(Language::from_code("fr"), None);
    }

    #[test]
    fn translated_ui_strings_have_no_missing_entries() {
        set_current_language(Language::En);
        let _ = take_missing();
        set_current_language(Language::ZhCn);
        for text in [
            "Ready",
            "No card",
            "Select a reader and read a card.",
            "Read complete · 64 blocks",
            "Block 004",
            "Sector 01 · data block · read",
        ] {
            let translated = translate_lines(text);
            assert_ne!(translated, text, "missing translation for {text}");
        }
        let missing = take_missing();
        set_current_language(Language::En);
        assert!(missing.is_empty(), "missing catalog entries: {missing:?}");
    }
}
