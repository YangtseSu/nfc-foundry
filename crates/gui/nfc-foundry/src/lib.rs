//! NFC Foundry persistent workbench shell.
//!
//! The shell owns navigation, reader selection, card identity, language,
//! activity, and the status bar. Feature crates render the page content.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, Theme, TitleBar,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Decorations, Entity, FontWeight, IntoElement, Render,
    Role, Task, TestSupportExt as _, Window, deferred, div, px, rgb,
};
use nfc_app::Service;
use nfc_app::classic::{BackupStore, KeyStore};
use nfc_classic::DumpFormat;
use nfc_classic_gui::{
    ClassicSettings, ClassicWorkspace, DUMP_FORMATS, dump_format_index, dump_format_key,
};
use nfc_type2_gui::{Type2Settings, Type2Workspace};
use nfc_ui::{
    ActivityEntry, Key, Language, Session, StatusKind, StatusMessage, Translator, UiPreferences,
    card_type_name, default_language, detect_language, preferences_path,
};

/// Card domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Classic,
    Type2,
}

/// One workbench page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    ClassicBlocks,
    ClassicNdef,
    ClassicDiff,
    ClassicKeys,
    ClassicRecovery,
    Type2Memory,
    Type2Ndef,
    Type2Protection,
    Diagnostics,
    Activity,
    Settings,
}

impl Screen {
    pub const CLASSIC: [Self; 5] = [
        Self::ClassicBlocks,
        Self::ClassicNdef,
        Self::ClassicDiff,
        Self::ClassicKeys,
        Self::ClassicRecovery,
    ];
    pub const TYPE2: [Self; 3] = [Self::Type2Memory, Self::Type2Ndef, Self::Type2Protection];

    pub const fn domain(self) -> Option<Domain> {
        match self {
            Self::ClassicBlocks
            | Self::ClassicNdef
            | Self::ClassicDiff
            | Self::ClassicKeys
            | Self::ClassicRecovery => Some(Domain::Classic),
            Self::Type2Memory | Self::Type2Ndef | Self::Type2Protection => Some(Domain::Type2),
            Self::Diagnostics | Self::Activity | Self::Settings => None,
        }
    }

    pub const fn label(self) -> Key {
        match self {
            Self::ClassicBlocks => Key::ScreenBlocks,
            Self::ClassicNdef => Key::ScreenClassicNdef,
            Self::ClassicDiff => Key::ScreenDiff,
            Self::ClassicKeys => Key::ScreenKeys,
            Self::ClassicRecovery => Key::ScreenRecovery,
            Self::Type2Memory => Key::ScreenMemory,
            Self::Type2Ndef => Key::ScreenType2Ndef,
            Self::Type2Protection => Key::ScreenProtection,
            Self::Diagnostics => Key::ScreenDiagnostics,
            Self::Activity => Key::ScreenActivity,
            Self::Settings => Key::ScreenSettings,
        }
    }

    /// Stable test and accessibility identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::ClassicBlocks => "nav-blocks",
            Self::ClassicNdef => "nav-classic-ndef",
            Self::ClassicDiff => "nav-diff",
            Self::ClassicKeys => "nav-keys",
            Self::ClassicRecovery => "nav-recovery",
            Self::Type2Memory => "nav-memory",
            Self::Type2Ndef => "nav-type2-ndef",
            Self::Type2Protection => "nav-protection",
            Self::Diagnostics => "nav-diagnostics",
            Self::Activity => "nav-activity",
            Self::Settings => "nav-settings",
        }
    }

    const fn icon(self) -> IconName {
        match self {
            Self::ClassicBlocks => IconName::Grid3x3,
            Self::ClassicNdef => IconName::Tag,
            Self::ClassicDiff => IconName::Diff,
            Self::ClassicKeys => IconName::Key,
            Self::ClassicRecovery => IconName::Radar,
            Self::Type2Memory => IconName::MemoryStick,
            Self::Type2Ndef => IconName::Tag,
            Self::Type2Protection => IconName::ShieldCheck,
            Self::Diagnostics => IconName::SlidersHorizontal,
            Self::Activity => IconName::Clock,
            Self::Settings => IconName::Settings,
        }
    }
}

/// Workbench configuration loaded from the preference file and defaults.
#[derive(Clone, Debug)]
pub struct ShellConfig {
    pub key_store_path: Option<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub hardnested_tables: Option<PathBuf>,
    pub default_dump_format: DumpFormat,
    pub use_common_keys: bool,
    pub preferences: UiPreferences,
    pub preferences_path: Option<PathBuf>,
}

impl ShellConfig {
    pub fn from_preferences(preferences: UiPreferences, preferences_path: Option<PathBuf>) -> Self {
        let key_store_path = preferences
            .key_store
            .clone()
            .or_else(KeyStore::default_path);
        let default_dump_format = preferences
            .default_dump_format
            .as_deref()
            .and_then(|name| DumpFormat::parse(name).ok())
            .unwrap_or(DumpFormat::Project);
        Self {
            key_store_path,
            backup_dir: preferences.backup_dir.clone(),
            hardnested_tables: preferences.hardnested_tables.clone(),
            default_dump_format,
            use_common_keys: preferences.use_common_keys.unwrap_or(true),
            preferences,
            preferences_path,
        }
    }

    fn backup_store(&self) -> BackupStore {
        match &self.backup_dir {
            Some(directory) => BackupStore::new(directory),
            None => BackupStore::default(),
        }
    }

    fn classic_settings(&self) -> ClassicSettings {
        ClassicSettings {
            key_store_path: self.key_store_path.clone(),
            backup: self.backup_store(),
            tables_dir: self.hardnested_tables.clone(),
            default_dump_format: self.default_dump_format,
            use_common_keys: self.use_common_keys,
        }
    }
}

/// The persistent application shell.
pub struct AppShell {
    service: Arc<Mutex<Service>>,
    session: Entity<Session>,
    classic: Entity<ClassicWorkspace>,
    type2: Entity<Type2Workspace>,
    screen: Screen,
    domain: Domain,
    config: ShellConfig,
    reader_menu_open: bool,
    key_store_input: Entity<InputState>,
    backup_input: Entity<InputState>,
    tables_input: Entity<InputState>,
    settings_format: usize,
    settings_common_keys: bool,
    watch_enabled: bool,
    watch_task: Option<Task<()>>,
}

impl AppShell {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        service: Arc<Mutex<Service>>,
        keys: KeyStore,
        config: ShellConfig,
        translator: Translator,
    ) -> Self {
        let backup = config.backup_store();
        let classic_settings = config.classic_settings();
        let session = cx.new(|_| Session::new(translator));
        let classic = cx.new(|cx| {
            ClassicWorkspace::new(
                window,
                cx,
                service.clone(),
                keys,
                classic_settings,
                session.clone(),
            )
        });
        let type2 = cx.new(|cx| {
            Type2Workspace::new(
                window,
                cx,
                service.clone(),
                Type2Settings { backup },
                session.clone(),
            )
        });
        let key_store_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/path/to/classic-keys.json")
                .default_value(path_text(config.key_store_path.as_deref()))
        });
        let backup_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/path/to/backups")
                .default_value(path_text(config.backup_dir.as_deref()))
        });
        let tables_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/path/to/hardnested_tables")
                .default_value(path_text(config.hardnested_tables.as_deref()))
        });
        let settings_format = dump_format_index(config.default_dump_format);
        let settings_common_keys = config.use_common_keys;
        let mut shell = Self {
            service,
            session: session.clone(),
            classic,
            type2,
            screen: Screen::ClassicBlocks,
            domain: Domain::Classic,
            config,
            reader_menu_open: false,
            key_store_input,
            backup_input,
            tables_input,
            settings_format,
            settings_common_keys,
            watch_enabled: false,
            watch_task: None,
        };
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        cx.observe(&shell.classic, |_, _, cx| cx.notify()).detach();
        cx.observe(&shell.type2, |_, _, cx| cx.notify()).detach();
        shell.refresh_devices(cx);
        shell
    }

    /// Current page.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Current card domain.
    pub fn domain(&self) -> Domain {
        self.domain
    }

    /// Session entity shared with the workspace crates.
    pub fn session(&self) -> &Entity<Session> {
        &self.session
    }

    /// Classic screens.
    pub fn classic(&self) -> &Entity<ClassicWorkspace> {
        &self.classic
    }

    /// Type 2 screens.
    pub fn type2(&self) -> &Entity<Type2Workspace> {
        &self.type2
    }

    /// Active shell configuration.
    pub fn config(&self) -> &ShellConfig {
        &self.config
    }

    /// Whether presence watch is on.
    pub fn watch_enabled(&self) -> bool {
        self.watch_enabled
    }

    /// Navigate to a screen.
    pub fn set_screen(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.screen = screen;
        if let Some(domain) = screen.domain() {
            self.domain = domain;
        }
        self.reader_menu_open = false;
        cx.notify();
    }

    /// Refresh the reader list and drop stale card images.
    pub fn refresh_devices(&mut self, cx: &mut Context<Self>) {
        let mut failure = None;
        let (devices, selected) = {
            let mut service = lock_service(&self.service);
            if let Err(error) = service.refresh_devices() {
                failure = Some(error);
            }
            let mut selected = service.selected_device().map(str::to_owned);
            if failure.is_none()
                && selected.is_none()
                && let Some(first) = service
                    .devices()
                    .first()
                    .map(|device| device.conn_string.clone())
            {
                match service.select_device(&first) {
                    Ok(()) => selected = Some(first),
                    Err(error) => failure = Some(error),
                }
            }
            let devices = service
                .devices()
                .iter()
                .map(|device| nfc_ui::ReaderSummary {
                    name: device.name.clone(),
                    conn_string: device.conn_string.clone(),
                })
                .collect::<Vec<_>>();
            (devices, selected)
        };
        self.session.update(cx, |session, cx| {
            session.set_devices(devices, selected);
            session.reset_cards();
            if let Some(error) = &failure {
                session.set_error(error);
            } else {
                session.set_status(StatusMessage::info(Key::StatusReaderRefreshed));
            }
            cx.notify();
        });
        self.classic
            .update(cx, |workspace, cx| workspace.reset_card(cx));
        self.type2
            .update(cx, |workspace, cx| workspace.reset_card(cx));
        cx.notify();
    }

    /// Select a reader by index in the session list.
    pub fn select_reader(&mut self, index: usize, cx: &mut Context<Self>) {
        let device = self.session.read(cx).devices.get(index).cloned();
        let Some(device) = device else {
            return;
        };
        let result = lock_service(&self.service).select_device(&device.conn_string);
        self.session.update(cx, |session, _cx| match result {
            Ok(()) => {
                session.selected_device = Some(device.conn_string.clone());
                session.reset_cards();
                session.set_status(StatusMessage::new(
                    StatusKind::Info,
                    Key::StatusReaderSelected,
                    Some(device.name.clone()),
                ));
                session.record(ActivityEntry::new(
                    StatusKind::Info,
                    Key::StatusReaderSelected,
                    device.name.clone(),
                ));
            }
            Err(error) => session.set_error(&error),
        });
        self.classic
            .update(cx, |workspace, cx| workspace.reset_card(cx));
        self.type2
            .update(cx, |workspace, cx| workspace.reset_card(cx));
        self.reader_menu_open = false;
        cx.notify();
    }

    /// Switch the interface language and persist it.
    pub fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        let translator = Translator::new(language);
        self.session.update(cx, |session, cx| {
            session.set_language(translator);
            cx.notify();
        });
        self.config.preferences.language = Some(language.code().to_owned());
        self.persist_preferences(cx);
        self.classic.update(cx, |_, cx| cx.notify());
        self.type2.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn persist_preferences(&mut self, cx: &mut Context<Self>) {
        let Some(path) = &self.config.preferences_path else {
            return;
        };
        let preferences = &mut self.config.preferences;
        preferences.key_store = self.config.key_store_path.clone();
        preferences.backup_dir = self.config.backup_dir.clone();
        preferences.hardnested_tables = self.config.hardnested_tables.clone();
        preferences.default_dump_format = Some(self.config.default_dump_format.name().to_owned());
        preferences.use_common_keys = Some(self.config.use_common_keys);
        let result = preferences.save(path);
        if let Err(error) = result {
            self.session.update(cx, |session, cx| {
                session.set_status(StatusMessage::error(
                    Key::StatusSettingsSaveFailed,
                    error.to_string(),
                ));
                cx.notify();
            });
        }
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let key_store_path = nonempty_path(self.key_store_input.read(cx).value().as_ref());
        let keys = match &key_store_path {
            Some(path) => match KeyStore::load_or_default(path) {
                Ok(keys) => keys,
                Err(error) => {
                    self.session.update(cx, |session, cx| {
                        session.set_status(StatusMessage::from_error(&error));
                        cx.notify();
                    });
                    return;
                }
            },
            None => KeyStore::new(),
        };
        let backup_dir = nonempty_path(self.backup_input.read(cx).value().as_ref());
        let tables = nonempty_path(self.tables_input.read(cx).value().as_ref());
        let format = DUMP_FORMATS[self.settings_format.min(DUMP_FORMATS.len() - 1)];
        self.config.key_store_path = key_store_path.clone();
        self.config.backup_dir = backup_dir.clone();
        self.config.hardnested_tables = tables.clone();
        self.config.default_dump_format = format;
        self.config.use_common_keys = self.settings_common_keys;
        let backup = self.config.backup_store();
        let classic_settings = self.config.classic_settings();
        self.classic.update(cx, |workspace, cx| {
            workspace.set_keys(keys, key_store_path, cx);
            workspace.set_settings(classic_settings, cx);
        });
        self.type2.update(cx, |workspace, cx| {
            workspace.set_settings(Type2Settings { backup }, cx)
        });
        self.persist_preferences(cx);
        self.session.update(cx, |session, cx| {
            session.set_status(StatusMessage::success(Key::StatusSettingsSaved));
            session.record(ActivityEntry::new(
                StatusKind::Success,
                Key::SaveSettings,
                "ui.json",
            ));
            cx.notify();
        });
        cx.notify();
    }

    fn reset_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key_store_input.update(cx, |state, cx| {
            state.set_value(path_text(KeyStore::default_path().as_deref()), window, cx);
        });
        self.backup_input.update(cx, |state, cx| {
            state.set_value(
                path_text(Some(BackupStore::default_directory().as_path())),
                window,
                cx,
            );
        });
        self.tables_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.settings_format = dump_format_index(DumpFormat::Project);
        self.settings_common_keys = true;
        self.save_settings(cx);
    }

    fn read_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.domain {
            Domain::Classic => self
                .classic
                .update(cx, |workspace, cx| workspace.read_card(window, cx)),
            Domain::Type2 => self
                .type2
                .update(cx, |workspace, cx| workspace.read_card(window, cx)),
        }
    }

    fn toggle_watch(&mut self, cx: &mut Context<Self>) {
        self.watch_enabled = !self.watch_enabled;
        if !self.watch_enabled {
            self.watch_task = None;
            self.session.update(cx, |session, cx| {
                session.watching = false;
                cx.notify();
            });
            return;
        }
        self.session.update(cx, |session, cx| {
            session.watching = true;
            cx.notify();
        });
        let service = Arc::clone(&self.service);
        let session = self.session.clone();
        self.watch_task = Some(cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let result = {
                    let mut service = lock_service(&service);
                    service.scan()
                };
                let keep_going = session.update(cx, |session, cx| {
                    if !session.watching {
                        return false;
                    }
                    match result {
                        Ok(card) => {
                            let name = card_type_name(card.card_type);
                            session.set_status(StatusMessage::new(
                                StatusKind::Info,
                                Key::CardPresent,
                                Some(format!("{name} · {}", card.uid_hex())),
                            ));
                        }
                        Err(_) => {
                            session.set_status(StatusMessage::info(Key::CardAbsent));
                        }
                    }
                    cx.notify();
                    true
                });
                if !keep_going {
                    break;
                }
            }
        }));
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let translator = session.translator;
        let selected = session.selected_reader().cloned();
        v_flex()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .px_4()
                    .pt_5()
                    .pb_3()
                    .child(
                        div()
                            .size(px(34.))
                            .rounded_lg()
                            .bg(cx.theme().sidebar_accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Cpu).small()),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child(translator.text(Key::AppName)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().sidebar_accent_foreground)
                                    .child(translator.text(Key::AppSubtitle)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .mx_3()
                    .mb_3()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().sidebar_accent)
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(div().size(px(7.)).rounded_full().bg(if selected.is_some() {
                                cx.theme().success
                            } else {
                                cx.theme().muted_foreground
                            }))
                            .child(
                                div().text_xs().font_weight(FontWeight::SEMIBOLD).child(
                                    selected
                                        .as_ref()
                                        .map(|device| device.name.clone())
                                        .unwrap_or_else(|| {
                                            translator.text(Key::NoReaderSelected).to_owned()
                                        }),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().sidebar_accent_foreground)
                            .truncate()
                            .child(
                                selected
                                    .as_ref()
                                    .map(|device| device.conn_string.clone())
                                    .unwrap_or_else(|| {
                                        translator.text(Key::ReaderDisconnected).to_owned()
                                    }),
                            ),
                    )
                    .child(
                        Button::new("sidebar-change-reader")
                            .xsmall()
                            .outline()
                            .w_full()
                            .label(translator.text(Key::ChangeReader))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reader_menu_open = !this.reader_menu_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id("sidebar-nav")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_3()
                    .px_2()
                    .pb_3()
                    .child(self.render_nav_group(
                        cx,
                        translator.text(Key::NavClassic),
                        &Screen::CLASSIC,
                    ))
                    .child(self.render_nav_group(
                        cx,
                        translator.text(Key::NavType2),
                        &Screen::TYPE2,
                    ))
                    .child(self.render_nav_group(
                        cx,
                        translator.text(Key::NavSystem),
                        &[Screen::Diagnostics, Screen::Activity, Screen::Settings],
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .text_xs()
                    .text_color(cx.theme().sidebar_accent_foreground)
                    .child("Linux · amd64")
                    .child(div().flex_1())
                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
            )
            .into_any_element()
    }

    fn render_nav_group(
        &self,
        cx: &mut Context<Self>,
        label: &str,
        screens: &[Screen],
    ) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let mut group = v_flex().gap_0p5().child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().sidebar_accent_foreground)
                .child(label.to_uppercase()),
        );
        for &screen in screens {
            let active = self.screen == screen;
            let badge = self.nav_badge(screen, cx);
            let icon = screen.icon();
            let mut item = h_flex()
                .id(screen.id())
                .test_support()
                .role(Role::Button)
                .aria_label(translator.text(screen.label()))
                .gap_2()
                .items_center()
                .h(px(34.))
                .px_2()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(active, |item| {
                    item.bg(cx.theme().sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!active, |item| {
                    item.text_color(cx.theme().sidebar_foreground)
                        .hover(|item| item.bg(cx.theme().sidebar_accent))
                })
                .child(Icon::new(icon).xsmall())
                .child(translator.text(screen.label()))
                .child(div().flex_1());
            if let Some(badge) = badge {
                item = item.child(
                    div()
                        .px_1()
                        .rounded_sm()
                        .bg(cx.theme().sidebar_accent)
                        .text_xs()
                        .child(badge),
                );
            }
            group = group.child(item.on_click(cx.listener(move |this, _, _, cx| {
                this.set_screen(screen, cx);
            })));
        }
        group.into_any_element()
    }

    fn nav_badge(&self, screen: Screen, cx: &App) -> Option<String> {
        match screen {
            Screen::ClassicBlocks => {
                let count = self.classic.read(cx).block_count();
                (count > 0).then(|| count.to_string())
            }
            Screen::ClassicNdef => {
                let count = self.classic.read(cx).ndef_record_count();
                (count > 0).then(|| count.to_string())
            }
            Screen::ClassicDiff => self
                .classic
                .read(cx)
                .diff()
                .map(|diff| diff.changed_blocks.to_string()),
            Screen::ClassicKeys => {
                let count = self.classic.read(cx).keys().len();
                (count > 0).then(|| count.to_string())
            }
            Screen::ClassicRecovery => Some("native".to_owned()),
            Screen::Type2Memory => self
                .type2
                .read(cx)
                .image()
                .map(|image| image.layout.page_count().to_string()),
            Screen::Type2Ndef => self.type2.read(cx).ndef().and_then(|ndef| {
                ndef.message()
                    .map(|message| message.records().len().to_string())
            }),
            Screen::Type2Protection => None,
            Screen::Activity => Some(self.session.read(cx).activity.len().to_string()),
            Screen::Diagnostics | Screen::Settings => None,
        }
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let selected = self.session.read(cx).selected_reader().cloned();
        let identity = match self.domain {
            Domain::Classic => self.session.read(cx).classic_card.clone(),
            Domain::Type2 => self.session.read(cx).type2_card.clone(),
        };
        let mut topbar = h_flex()
            .relative()
            .h(px(62.))
            .flex_shrink_0()
            .gap_3()
            .items_center()
            .px_5()
            .bg(cx.theme().popover)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("domain-classic")
                            .small()
                            .when(self.domain == Domain::Classic, |button| button.primary())
                            .when(self.domain != Domain::Classic, |button| button.ghost())
                            .label(translator.text(Key::DomainClassic))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_screen(Screen::ClassicBlocks, cx);
                            })),
                    )
                    .child(
                        Button::new("domain-type2")
                            .small()
                            .when(self.domain == Domain::Type2, |button| button.primary())
                            .when(self.domain != Domain::Type2, |button| button.ghost())
                            .label(translator.text(Key::DomainType2))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_screen(Screen::Type2Memory, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .max_w(px(340.))
                    .min_w_0()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(
                                identity
                                    .as_ref()
                                    .map(|identity| identity.card_type.clone())
                                    .unwrap_or_else(|| {
                                        translator.text(Key::NoCardSelected).to_owned()
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(identity.as_ref().map_or_else(
                                || "—".to_owned(),
                                |identity| format!("UID {} · {}", identity.uid, identity.detail),
                            )),
                    ),
            )
            .child(div().flex_1())
            .child(
                Button::new("reader-menu")
                    .small()
                    .outline()
                    .w(px(210.))
                    .label(
                        selected
                            .as_ref()
                            .map(|device| device.name.clone())
                            .unwrap_or_else(|| translator.text(Key::NoReaderSelected).to_owned()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reader_menu_open = !this.reader_menu_open;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("language-en")
                            .xsmall()
                            .when(translator.language() == Language::En, |button| {
                                button.primary()
                            })
                            .when(translator.language() != Language::En, |button| {
                                button.ghost()
                            })
                            .label(translator.text(Key::English))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_language(Language::En, cx);
                            })),
                    )
                    .child(
                        Button::new("language-zh")
                            .xsmall()
                            .when(translator.language() == Language::ZhCn, |button| {
                                button.primary()
                            })
                            .when(translator.language() != Language::ZhCn, |button| {
                                button.ghost()
                            })
                            .label(translator.text(Key::Chinese))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_language(Language::ZhCn, cx);
                            })),
                    ),
            )
            .child(
                Button::new("read-active")
                    .primary()
                    .label(match self.domain {
                        Domain::Classic => translator.text(Key::ReadCard),
                        Domain::Type2 => translator.text(Key::ReadTag),
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.read_active(window, cx))),
            )
            .child(
                Button::new("open-settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .accessibility_label(translator.text(Key::Settings))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_screen(Screen::Settings, cx);
                    })),
            );
        if self.reader_menu_open {
            topbar = topbar.child(deferred(self.render_reader_popover(cx)));
        }
        topbar.into_any_element()
    }

    fn render_reader_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let translator = session.translator;
        let selected = session.selected_device.clone();
        let mut list = v_flex().gap_0p5();
        if session.devices.is_empty() {
            list = list.child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::NoReadersFound)),
            );
        } else {
            for (index, device) in session.devices.iter().enumerate() {
                let active = selected.as_deref() == Some(device.conn_string.as_str());
                list = list.child(
                    h_flex()
                        .id(("reader-option", index))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(device.name.clone())
                        .gap_2()
                        .items_center()
                        .p_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .when(active, |item| item.bg(cx.theme().accent))
                        .hover(|item| item.bg(cx.theme().secondary))
                        .child(div().size(px(6.)).rounded_full().bg(cx.theme().success))
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(device.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(device.conn_string.clone()),
                                ),
                        )
                        .child(div().flex_1())
                        .when(active, |item| {
                            item.child(Icon::new(IconName::Check).xsmall())
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_reader(index, cx);
                        })),
                );
            }
        }
        let list = list.id("reader-list").max_h(px(360.)).overflow_y_scroll();
        v_flex()
            .absolute()
            .top(px(56.))
            .right(px(16.))
            .w(px(320.))
            .p_2()
            .rounded_lg()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .px_1()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::Readers)),
            )
            .child(list)
            .child(
                h_flex()
                    .gap_2()
                    .p_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("reader-refresh")
                            .xsmall()
                            .ghost()
                            .label(translator.text(Key::RefreshReaders))
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_devices(cx))),
                    )
                    .child(
                        Checkbox::new("watch-card")
                            .checked(self.watch_enabled)
                            .label(translator.text(Key::WatchCard))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_watch(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let content = match self.screen {
            Screen::ClassicBlocks => self
                .classic
                .update(cx, |workspace, cx| workspace.render_blocks(cx)),
            Screen::ClassicNdef => self
                .classic
                .update(cx, |workspace, cx| workspace.render_ndef(cx)),
            Screen::ClassicDiff => self
                .classic
                .update(cx, |workspace, cx| workspace.render_diff(cx)),
            Screen::ClassicKeys => self
                .classic
                .update(cx, |workspace, cx| workspace.render_keys(cx)),
            Screen::ClassicRecovery => self
                .classic
                .update(cx, |workspace, cx| workspace.render_recovery(cx)),
            Screen::Type2Memory => self
                .type2
                .update(cx, |workspace, cx| workspace.render_memory(cx)),
            Screen::Type2Ndef => self
                .type2
                .update(cx, |workspace, cx| workspace.render_ndef(cx)),
            Screen::Type2Protection => self
                .type2
                .update(cx, |workspace, cx| workspace.render_protection(cx)),
            Screen::Diagnostics => self.render_diagnostics(cx),
            Screen::Activity => self.render_activity(cx),
            Screen::Settings => self.render_settings(cx),
        };
        let scrolls = !matches!(
            self.screen,
            Screen::ClassicBlocks | Screen::ClassicKeys | Screen::Type2Memory
        );
        div()
            .id("page")
            .flex_1()
            .min_h_0()
            .p_5()
            .when(scrolls, |page| page.overflow_y_scroll())
            .child(content)
            .into_any_element()
    }

    fn render_statusbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let translator = session.translator;
        let status = &session.status;
        let color = match status.kind {
            StatusKind::Info => cx.theme().muted_foreground,
            StatusKind::Success => cx.theme().success,
            StatusKind::Warning => cx.theme().warning,
            StatusKind::Error => cx.theme().danger,
        };
        let summary = session
            .busy
            .map(|busy| format!("{}…", translator.text(busy.label)))
            .unwrap_or_else(|| translator.text(status.summary).to_owned());
        let detail = status.detail.clone().unwrap_or_default();
        let backup = session
            .last_backup
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| translator.text(Key::NoBackup).to_owned());
        let recovery_running = self.classic.read(cx).recovery_running();
        h_flex()
            .h(px(38.))
            .flex_shrink_0()
            .gap_2()
            .items_center()
            .px_4()
            .bg(cx.theme().status_bar)
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(div().size(px(6.)).rounded_full().bg(color))
            .child(
                div()
                    .id("status-summary")
                    .test_support()
                    .text_color(if session.busy.is_some() {
                        cx.theme().foreground
                    } else {
                        color
                    })
                    .child(summary),
            )
            .child(
                div()
                    .id("status-detail")
                    .truncate()
                    .max_w(px(420.))
                    .child(detail),
            )
            .child(div().flex_1())
            .when(recovery_running, |bar| {
                bar.child(
                    Button::new("status-cancel")
                        .xsmall()
                        .danger()
                        .label(translator.text(Key::Cancel))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.classic
                                .update(cx, |workspace, cx| workspace.cancel_recovery(cx));
                        })),
                )
            })
            .child(format!("{}: {backup}", translator.text(Key::Backup)))
            .child(div().w(px(1.)).h(px(14.)).bg(cx.theme().border))
            .child(format!("v{}", env!("CARGO_PKG_VERSION")))
            .into_any_element()
    }

    fn render_diagnostics(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let session = self.session.read(cx);
        let selected = session
            .selected_reader()
            .map(|device| format!("{} · {}", device.name, device.conn_string))
            .unwrap_or_else(|| translator.text(Key::NoReaderSelected).to_owned());
        let key_store = path_text(self.config.key_store_path.as_deref());
        let backups = path_text(self.config.backup_dir.as_deref());
        let tables = path_text(self.config.hardnested_tables.as_deref());
        let report = format!(
            "NFC Foundry {}\nOS: {} {}\nGUI: GPUI Kit\nReaders:\n{}\nKey store: {key_store}\nBackups: {backups}\nTables: {tables}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            session
                .devices
                .iter()
                .map(|device| format!("- {} ({})", device.name, device.conn_string))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let header = nfc_ui::page_header(
            cx,
            translator,
            Key::EyebrowSystemDiagnostics,
            Key::DiagnosticsTitle,
            translator.text(Key::DiagnosticsSubtitle),
            vec![
                Button::new("diagnostics-copy")
                    .small()
                    .label(translator.text(Key::CopyReport))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(report.clone()));
                        this.session.update(cx, |session, cx| {
                            session.set_status(StatusMessage::success(Key::CopyReportDone));
                            cx.notify();
                        });
                    }))
                    .into_any_element(),
                Button::new("diagnostics-settings")
                    .small()
                    .outline()
                    .label(translator.text(Key::Settings))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_screen(Screen::Settings, cx);
                    }))
                    .into_any_element(),
            ],
        );
        let card = |title: Key, rows: Vec<(String, String)>| -> AnyElement {
            let mut body = v_flex().gap_1();
            for (label, value) in rows {
                body = body.child(nfc_ui::kv_row(cx, label, value));
            }
            nfc_ui::card(cx, translator.text(title), "", body.into_any_element())
        };
        let grid = v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(v_flex().flex_1().gap_3().child(card(
                        Key::ApplicationSection,
                        vec![
                            (
                                translator.text(Key::Version).to_owned(),
                                env!("CARGO_PKG_VERSION").to_owned(),
                            ),
                            (
                                translator.text(Key::BuildLabel).to_owned(),
                                if cfg!(debug_assertions) {
                                    "debug".to_owned()
                                } else {
                                    "release".to_owned()
                                },
                            ),
                            (
                                translator.text(Key::PlatformLabel).to_owned(),
                                format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH),
                            ),
                            (
                                translator.text(Key::GuiLabel).to_owned(),
                                "GPUI Kit".to_owned(),
                            ),
                        ],
                    )))
                    .child(v_flex().flex_1().gap_3().child(card(
                        Key::ReadersSection,
                        vec![
                            (translator.text(Key::FieldReader).to_owned(), selected),
                            (
                                translator.text(Key::ReaderLease).to_owned(),
                                if session.is_busy() {
                                    "busy".to_owned()
                                } else {
                                    "idle".to_owned()
                                },
                            ),
                            (
                                translator.text(Key::PcscdLabel).to_owned(),
                                "ACR122U".to_owned(),
                            ),
                        ],
                    ))),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(v_flex().flex_1().gap_3().child(card(
                        Key::DataSection,
                        vec![
                            (translator.text(Key::KeyStoreLabel).to_owned(), key_store),
                            (translator.text(Key::BackupsLabel).to_owned(), backups),
                            (translator.text(Key::HardnestedTables).to_owned(), tables),
                            (
                                translator.text(Key::BackupMode).to_owned(),
                                "0600 · 0700".to_owned(),
                            ),
                        ],
                    )))
                    .child(v_flex().flex_1().gap_3().child(card(
                        Key::ChecksSection,
                        vec![
                            (
                                translator.text(Key::CardTransport).to_owned(),
                                translator.text(Key::ReadyLabel).to_owned(),
                            ),
                            (
                                translator.text(Key::NdefCodec).to_owned(),
                                translator.text(Key::ReadyLabel).to_owned(),
                            ),
                            (
                                translator.text(Key::RecoveryEngine).to_owned(),
                                translator.text(Key::ReadyLabel).to_owned(),
                            ),
                            (
                                translator.text(Key::HardwareTests).to_owned(),
                                translator.text(Key::ExplicitGateOnly).to_owned(),
                            ),
                        ],
                    ))),
            );
        v_flex()
            .size_full()
            .child(header)
            .child(grid)
            .into_any_element()
    }

    fn render_activity(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let session = self.session.read(cx);
        let header = nfc_ui::page_header(
            cx,
            translator,
            Key::EyebrowSystemActivity,
            Key::ActivityTitle,
            translator.text(Key::ActivitySubtitle),
            vec![
                Button::new("activity-clear")
                    .small()
                    .label(translator.text(Key::ClearActivity))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.clear_activity();
                            cx.notify();
                        });
                    }))
                    .into_any_element(),
            ],
        );
        let mut timeline = v_flex().gap_2();
        if session.activity.is_empty() {
            timeline = timeline.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(translator.text(Key::ActivityEmpty)),
            );
        } else {
            for entry in &session.activity {
                timeline = timeline.child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .p_2()
                        .rounded_md()
                        .bg(cx.theme().secondary)
                        .child(pill_for(cx, entry.kind, entry.time.clone()))
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(translator.text(entry.title)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(entry.detail.clone()),
                                ),
                        ),
                );
            }
        }
        let safety = v_flex()
            .gap_2()
            .child(nfc_ui::callout_with_id(
                cx,
                StatusKind::Info,
                "safety-backup",
                translator.text(Key::SafetyBackup),
            ))
            .child(nfc_ui::callout_with_id(
                cx,
                StatusKind::Warning,
                "safety-advanced",
                translator.text(Key::SafetyAdvanced),
            ))
            .child(nfc_ui::callout_with_id(
                cx,
                StatusKind::Error,
                "safety-block-zero",
                translator.text(Key::SafetyBlockZero),
            ));
        v_flex()
            .size_full()
            .child(header)
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .bg(cx.theme().popover)
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_lg()
                            .overflow_hidden()
                            .child(nfc_ui::card_head(
                                cx,
                                translator.text(Key::SessionActivity),
                                translator.text(Key::SessionActivitySub),
                            ))
                            .child(div().id("activity-scroll").p_3().child(timeline)),
                    )
                    .child(
                        v_flex()
                            .w(px(320.))
                            .flex_shrink_0()
                            .bg(cx.theme().popover)
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_lg()
                            .overflow_hidden()
                            .child(nfc_ui::card_head(
                                cx,
                                translator.text(Key::Safety),
                                translator.text(Key::SafetySub),
                            ))
                            .child(div().p_4().child(safety)),
                    ),
            )
            .into_any_element()
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let translator = self.session.read(cx).translator;
        let header = nfc_ui::page_header(
            cx,
            translator,
            Key::EyebrowSettings,
            Key::SettingsTitle,
            translator.text(Key::SettingsSubtitle),
            vec![
                Button::new("settings-reset")
                    .small()
                    .outline()
                    .label(translator.text(Key::ResetSettings))
                    .on_click(cx.listener(|this, _, window, cx| this.reset_settings(window, cx)))
                    .into_any_element(),
                Button::new("settings-save")
                    .small()
                    .primary()
                    .label(translator.text(Key::SaveSettings))
                    .on_click(cx.listener(|this, _, _, cx| this.save_settings(cx)))
                    .into_any_element(),
            ],
        );
        let mut formats = h_flex().gap_1();
        for (index, format) in DUMP_FORMATS.iter().enumerate() {
            let selected = index == self.settings_format;
            let key = dump_format_key(*format);
            formats = formats.child(
                Button::new(("settings-format", index))
                    .xsmall()
                    .when(selected, |button| button.primary())
                    .when(!selected, |button| button.outline())
                    .label(translator.text(key))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_format = index;
                        cx.notify();
                    })),
            );
        }
        let form = v_flex()
            .gap_3()
            .p_4()
            .child(nfc_ui::field(
                cx,
                translator.text(Key::KeyStorePath),
                None,
                Input::new(&self.key_store_input)
                    .id("settings-key-store")
                    .into_any_element(),
            ))
            .child(nfc_ui::field(
                cx,
                translator.text(Key::BackupDirectory),
                None,
                Input::new(&self.backup_input)
                    .id("settings-backup")
                    .into_any_element(),
            ))
            .child(nfc_ui::field(
                cx,
                translator.text(Key::HardnestedTablesLabel),
                None,
                Input::new(&self.tables_input)
                    .id("settings-tables")
                    .into_any_element(),
            ))
            .child(nfc_ui::field(
                cx,
                translator.text(Key::DefaultDumpFormat),
                None,
                formats.into_any_element(),
            ))
            .child(
                Checkbox::new("settings-common-keys")
                    .checked(self.settings_common_keys)
                    .label(translator.text(Key::UseCommonKeys))
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.settings_common_keys = *checked;
                        cx.notify();
                    })),
            )
            .child(nfc_ui::field(
                cx,
                translator.text(Key::LanguageSetting),
                None,
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("settings-language-en")
                            .xsmall()
                            .when(translator.language() == Language::En, |button| {
                                button.primary()
                            })
                            .when(translator.language() != Language::En, |button| {
                                button.outline()
                            })
                            .label(translator.text(Key::English))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_language(Language::En, cx)),
                            ),
                    )
                    .child(
                        Button::new("settings-language-zh")
                            .xsmall()
                            .when(translator.language() == Language::ZhCn, |button| {
                                button.primary()
                            })
                            .when(translator.language() != Language::ZhCn, |button| {
                                button.outline()
                            })
                            .label(translator.text(Key::Chinese))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_language(Language::ZhCn, cx)),
                            ),
                    )
                    .into_any_element(),
            ))
            .child(nfc_ui::callout(
                cx,
                StatusKind::Info,
                translator.text(Key::SettingsNote),
            ));
        v_flex()
            .size_full()
            .child(header)
            .child(
                v_flex()
                    .w(px(640.))
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_lg()
                    .overflow_hidden()
                    .child(nfc_ui::card_head(
                        cx,
                        translator.text(Key::ConfigurationSection),
                        translator.text(Key::SettingsSubtitle),
                    ))
                    .child(form),
            )
            .into_any_element()
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        // Client-decorated windows have no title bar from the window manager,
        // so the application draws and moves its own. Server-decorated windows
        // already have one.
        if matches!(window.window_decorations(), Decorations::Client { .. }) {
            root = root.child(
                TitleBar::new().child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::new(IconName::Cpu)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("NFC Foundry {}", env!("CARGO_PKG_VERSION"))),
                        ),
                ),
            );
        }
        root.child(
            h_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(self.render_sidebar(cx))
                .child(
                    v_flex()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .min_h_0()
                        .child(self.render_topbar(cx))
                        .child(self.render_page(cx))
                        .child(self.render_statusbar(cx)),
                ),
        )
    }
}

fn lock_service(service: &Arc<Mutex<Service>>) -> MutexGuard<'_, Service> {
    service.lock().unwrap_or_else(PoisonError::into_inner)
}

fn path_text(path: Option<&std::path::Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn nonempty_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

fn pill_for(cx: &App, kind: StatusKind, text: String) -> AnyElement {
    nfc_ui::pill(cx, kind, text)
}

/// Use the dark navigation rail from the workbench design regardless of the
/// light content theme.
pub fn install_shell_theme(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.sidebar = rgb(0x0d1522).into();
    theme.sidebar_foreground = rgb(0xcbd5e1).into();
    theme.sidebar_accent = rgb(0x1b2740).into();
    theme.sidebar_accent_foreground = rgb(0x8190a5).into();
    theme.sidebar_border = rgb(0x1f2b3a).into();
    theme.sidebar_primary = rgb(0x818cf8).into();
    theme.sidebar_primary_foreground = rgb(0x0d1522).into();
}

/// Merge stored preferences and the current locale into shell configuration.
pub fn load_config() -> (ShellConfig, Translator) {
    let path = preferences_path();
    let stored = path
        .as_deref()
        .map(UiPreferences::load)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default();
    let language = detect_language(stored.language(), |variable| std::env::var(variable).ok());
    let translator = Translator::new(language);
    (ShellConfig::from_preferences(stored, path), translator)
}

/// Locale detection used when no preference file exists.
pub fn initial_language() -> Language {
    default_language()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurations_derive_from_preferences() {
        let preferences = UiPreferences {
            key_store: Some(PathBuf::from("/tmp/keys.json")),
            backup_dir: Some(PathBuf::from("/tmp/backups")),
            default_dump_format: Some("mct".to_owned()),
            use_common_keys: Some(false),
            ..UiPreferences::default()
        };
        let config = ShellConfig::from_preferences(preferences, None);
        assert_eq!(config.key_store_path, Some(PathBuf::from("/tmp/keys.json")));
        assert_eq!(config.backup_dir, Some(PathBuf::from("/tmp/backups")));
        assert_eq!(config.default_dump_format, DumpFormat::Mct);
        assert!(!config.use_common_keys);
    }

    #[test]
    fn screens_map_to_domains() {
        assert_eq!(Screen::ClassicBlocks.domain(), Some(Domain::Classic));
        assert_eq!(Screen::Type2Protection.domain(), Some(Domain::Type2));
        assert_eq!(Screen::Diagnostics.domain(), None);
    }

    #[test]
    fn empty_paths_are_rejected() {
        assert_eq!(nonempty_path("  "), None);
        assert_eq!(nonempty_path(" /tmp/x "), Some(PathBuf::from("/tmp/x")));
    }
}
