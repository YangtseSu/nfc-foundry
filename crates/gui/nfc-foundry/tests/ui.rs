//! Workbench shell UI integration tests.

use std::sync::{Arc, Mutex};

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use nfc_app::Service;
use nfc_app::classic::{BackupStore, KeyStore};
use nfc_classic::Layout;
use nfc_core::{BLOCK_SIZE, CardInfo, CardType, PAGE_SIZE};
use nfc_foundry::{AppShell, Screen, ShellConfig};
use nfc_transport::DeviceInfo;
use nfc_transport::mock::{MockHandle, MockTransport};
use nfc_ui::{Language, Translator, UiPreferences};

fn classic_card() -> CardInfo {
    CardInfo::new(
        vec![0x04, 0xA2, 0x24, 0x8A],
        [0x00, 0x04],
        0x08,
        CardType::MifareClassic1K,
    )
}

fn classic_image() -> Vec<(u8, [u8; BLOCK_SIZE])> {
    let uid = [0x04, 0xA2, 0x24, 0x8A];
    let mut block_zero = [0u8; BLOCK_SIZE];
    block_zero[0..4].copy_from_slice(&uid);
    block_zero[4] = nfc_classic::bcc_for_uid(&uid).expect("bcc");
    block_zero[5] = 0x08;
    block_zero[6..8].copy_from_slice(&[0x00, 0x04]);
    (0..Layout::Classic1K.block_count() as u8)
        .map(|block| {
            let data = if block == 0 {
                block_zero
            } else if block % 4 == 3 {
                let mut trailer = [0xFF; BLOCK_SIZE];
                trailer[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
                trailer[9] = 0x69;
                trailer
            } else {
                [block; BLOCK_SIZE]
            };
            (block, data)
        })
        .collect()
}

fn classic_transport() -> (Arc<MockTransport>, MockHandle) {
    let transport = Arc::new(MockTransport::new());
    let handle = transport.add(
        DeviceInfo::new("Mock Classic 1K", "mock:classic-1k"),
        Some(classic_card()),
    );
    for (block, data) in classic_image() {
        handle.set_block(block, data);
    }
    (transport, handle)
}

fn type2_transport() -> (Arc<MockTransport>, MockHandle) {
    let transport = Arc::new(MockTransport::new());
    let card = CardInfo::new(
        vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
        [0x00, 0x44],
        0x00,
        CardType::Type2,
    );
    let handle = transport.add(DeviceInfo::new("Mock NTAG215", "mock:ntag215"), Some(card));
    let mut pages: Vec<(u8, [u8; PAGE_SIZE])> =
        (0..135u8).map(|page| (page, [0u8; PAGE_SIZE])).collect();
    pages[0] = (0, [0x04, 0x5B, 0x1C, 0x2A]);
    pages[1] = (1, [0x3B, 0x4C, 0x80, 0x00]);
    pages[3] = (3, [0xE1, 0x10, 0x3E, 0x00]);
    pages[4] = (4, [0x03, 0x00, 0xFE, 0x00]);
    pages[130] = (130, [0x00, 0x00, 0x00, 0xBD]);
    pages[131] = (131, [0x00, 0x00, 0x00, 0xFF]);
    for (page, data) in pages {
        handle.set_page(page, data);
    }
    (transport, handle)
}

struct Harness {
    handle: gpui_kit::AnyWindowHandle,
    view: Entity<AppShell>,
    _directory: tempfile::TempDir,
}

fn open_shell(
    cx: &mut TestAppContext,
    transport: Arc<dyn nfc_transport::Transport>,
    preferences_path: Option<std::path::PathBuf>,
) -> Harness {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = ShellConfig {
        key_store_path: None,
        backup_dir: Some(directory.path().join("backups")),
        hardnested_tables: None,
        default_dump_format: nfc_classic::DumpFormat::Project,
        use_common_keys: true,
        preferences: UiPreferences::default(),
        preferences_path,
    };
    let service = Arc::new(Mutex::new(Service::with_transport(transport)));
    cx.update(gpui_kit::init);
    let (handle, view) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(1280.), px(800.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        AppShell::new(
                            window,
                            cx,
                            service,
                            KeyStore::new(),
                            config,
                            Translator::new(Language::En),
                        )
                    })
                },
            )
        })
        .expect("open test window");
    Harness {
        handle,
        view,
        _directory: directory,
    }
}

fn settle(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
    })
    .expect("render frame");
}

fn click(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    cx.update_window(handle, |_, window, cx| {
        window.click(id, cx);
    })
    .expect("click");
    settle(cx, handle);
}

#[gpui_kit::test]
fn reads_a_classic_card_and_opens_every_screen(cx: &mut TestAppContext) {
    let (transport, handle) = classic_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);

    click(cx, harness.handle, "read-active");
    cx.update(|cx| {
        let workspace = harness.view.read(cx).classic().read(cx);
        assert!(workspace.dump().expect("dump").complete());
        let session = harness.view.read(cx).session().read(cx);
        assert_eq!(
            session.classic_card.as_ref().map(|card| card.uid.as_str()),
            Some("04 A2 24 8A")
        );
    });

    click(cx, harness.handle, ("classic-block", 4usize));
    cx.update(|cx| {
        assert_eq!(
            harness.view.read(cx).classic().read(cx).selected_block(),
            Some(4)
        );
    });
    cx.update_window(harness.handle, |_, window, _| {
        let inspector = window
            .find("classic-inspector")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(inspector.contains("Block 4"), "{inspector}");
        assert!(inspector.contains("sector 1"), "{inspector}");
    })
    .expect("inspect");

    handle.set_block(4, [0xA5; BLOCK_SIZE]);
    click(cx, harness.handle, "read-active");
    cx.update(|cx| {
        let workspace = harness.view.read(cx).classic().read(cx);
        let diff = workspace.diff().expect("diff");
        assert_eq!(diff.changed_blocks, 1);
    });

    for screen in [
        "nav-classic-ndef",
        "nav-diff",
        "nav-keys",
        "nav-recovery",
        "nav-diagnostics",
        "nav-activity",
        "nav-settings",
        "nav-blocks",
    ] {
        click(cx, harness.handle, screen);
    }
    cx.update(|cx| {
        assert_eq!(harness.view.read(cx).screen(), Screen::ClassicBlocks);
    });
    click(cx, harness.handle, "nav-diff");
    cx.update_window(harness.handle, |_, window, _| {
        assert!(window.find("classic-diff-scroll").visible());
    })
    .expect("diff scroll");
}

#[gpui_kit::test]
fn keys_screen_adds_a_key_and_settings_persist(cx: &mut TestAppContext) {
    let (transport, _handle) = classic_transport();
    let path = tempfile::tempdir()
        .expect("prefs dir")
        .path()
        .join("ui.json");
    let harness = open_shell(cx, transport, Some(path.clone()));
    settle(cx, harness.handle);

    click(cx, harness.handle, "nav-keys");
    click(cx, harness.handle, "classic-key-hex");
    cx.update_window(harness.handle, |_, window, cx| {
        window.input("A0A1A2A3A4A5", cx);
    })
    .expect("input key");
    click(cx, harness.handle, "classic-add-key");
    cx.update(|cx| {
        assert_eq!(harness.view.read(cx).classic().read(cx).keys().len(), 1);
    });

    click(cx, harness.handle, "nav-settings");
    click(cx, harness.handle, "settings-key-store");
    cx.update_window(harness.handle, |_, window, cx| {
        window.input("/tmp/nfc-foundry-keys.json", cx);
    })
    .expect("key store path");
    click(cx, harness.handle, "settings-save");
    let stored = UiPreferences::load(&path).expect("preferences");
    assert_eq!(
        stored.key_store.as_deref(),
        Some(std::path::Path::new("/tmp/nfc-foundry-keys.json"))
    );
}

#[gpui_kit::test]
fn language_switch_updates_the_session_and_file(cx: &mut TestAppContext) {
    let (transport, _handle) = classic_transport();
    let path = tempfile::tempdir()
        .expect("prefs dir")
        .path()
        .join("ui.json");
    let harness = open_shell(cx, transport, Some(path.clone()));
    settle(cx, harness.handle);

    click(cx, harness.handle, "language-zh");
    cx.update(|cx| {
        let session = harness.view.read(cx).session().read(cx);
        assert_eq!(session.translator.language(), Language::ZhCn);
        assert_eq!(session.text(nfc_ui::Key::ScreenBlocks), "数据块");
    });
    let stored = UiPreferences::load(&path).expect("preferences");
    assert_eq!(stored.language.as_deref(), Some("zh-CN"));

    click(cx, harness.handle, "language-en");
    cx.update(|cx| {
        assert_eq!(
            harness
                .view
                .read(cx)
                .session()
                .read(cx)
                .translator
                .language(),
            Language::En
        );
    });
}

#[gpui_kit::test]
fn type2_reads_a_tag_and_selects_a_page(cx: &mut TestAppContext) {
    let (transport, _handle) = type2_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);

    click(cx, harness.handle, "read-active");
    cx.update(|cx| {
        let workspace = harness.view.read(cx).type2().read(cx);
        let image = workspace.image().expect("tag");
        assert_eq!(image.known_pages(), 135);
        assert_eq!(workspace.selected_page(), Some(4));
        assert_eq!(
            harness
                .view
                .read(cx)
                .session()
                .read(cx)
                .type2_card
                .as_ref()
                .map(|card| card.card_type.as_str()),
            Some("NTAG215")
        );
    });
    cx.update_window(harness.handle, |_, window, _| {
        assert!(
            window.try_find("nav-blocks").is_none(),
            "Classic tasks must be hidden for a Type 2 tag"
        );
        assert!(
            window.try_find("nav-memory").is_some(),
            "Type 2 tasks must be shown for a Type 2 tag"
        );
    })
    .expect("family navigation");
    click(cx, harness.handle, ("type2-page", 6usize));
    cx.update(|cx| {
        assert_eq!(
            harness.view.read(cx).type2().read(cx).selected_page(),
            Some(6)
        );
    });
    cx.update_window(harness.handle, |_, window, _| {
        let inspector = window
            .find("type2-inspector")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(inspector.contains("Page 6"), "{inspector}");
        assert!(inspector.contains("User memory"), "{inspector}");
    })
    .expect("inspect");
    for screen in ["nav-type2-ndef", "nav-protection", "nav-memory"] {
        click(cx, harness.handle, screen);
    }
}

#[gpui_kit::test]
fn advanced_writes_require_the_typed_dialog(cx: &mut TestAppContext) {
    let (transport, _handle) = type2_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);
    click(cx, harness.handle, "read-active");

    click(cx, harness.handle, ("type2-page", 3usize));
    click(cx, harness.handle, "type2-page-data");
    cx.update_window(harness.handle, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("E1103E01", cx);
    })
    .expect("edit page");
    click(cx, harness.handle, "type2-write-page");
    cx.update_window(harness.handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.press("escape", cx);
        assert!(!window.has_active_dialog(cx));
    })
    .expect("dialog escape");
    settle(cx, harness.handle);
    cx.update(|cx| {
        assert!(harness.view.read(cx).type2().read(cx).image().is_some());
    });
}

#[gpui_kit::test]
fn trailer_writes_require_confirmation_and_block_zero_is_locked(cx: &mut TestAppContext) {
    let (transport, handle) = classic_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);
    click(cx, harness.handle, "read-active");

    click(cx, harness.handle, ("classic-block", 3usize));
    click(cx, harness.handle, "classic-write-block");
    cx.update_window(harness.handle, |_, window, cx| {
        assert!(
            !window.has_active_dialog(cx),
            "unchanged trailer must not open a write dialog"
        );
    })
    .expect("unchanged trailer");
    click(cx, harness.handle, "classic-block-data");
    cx.update_window(harness.handle, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF", cx);
    })
    .expect("edit trailer");
    click(cx, harness.handle, "classic-write-block");
    cx.update_window(harness.handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.close_dialog(cx);
    })
    .expect("trailer dialog");
    settle(cx, harness.handle);
    assert_eq!(handle.block(3), Some(classic_image()[3].1));

    click(cx, harness.handle, ("classic-block", 0usize));
    click(cx, harness.handle, "classic-write-block");
    cx.update_window(harness.handle, |_, window, cx| {
        assert!(!window.has_active_dialog(cx));
    })
    .expect("no dialog");
    assert_eq!(handle.block(0), Some(classic_image()[0].1));
}

#[gpui_kit::test]
fn settings_reset_restores_defaults(cx: &mut TestAppContext) {
    let (transport, _handle) = classic_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);
    click(cx, harness.handle, "nav-settings");
    click(cx, harness.handle, "settings-common-keys");
    click(cx, harness.handle, "settings-save");
    cx.update(|cx| {
        assert!(!harness.view.read(cx).config().use_common_keys);
    });
    click(cx, harness.handle, "settings-reset");
    cx.update(|cx| {
        assert!(harness.view.read(cx).config().use_common_keys);
        assert_eq!(
            harness.view.read(cx).config().default_dump_format,
            nfc_classic::DumpFormat::Project
        );
    });
}

#[test]
fn backup_store_defaults_are_available_for_the_shell() {
    let store = BackupStore::default();
    assert!(!store.directory().as_os_str().is_empty());
}

#[gpui_kit::test]
fn empty_reader_keeps_both_task_groups_available(cx: &mut TestAppContext) {
    let transport = Arc::new(MockTransport::new());
    transport.add(DeviceInfo::new("Mock Empty", "mock:empty"), None);
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);

    cx.update_window(harness.handle, |_, window, _| {
        for id in ["nav-blocks", "nav-memory", "nav-keys", "nav-protection"] {
            assert!(
                window.try_find(id).is_some(),
                "both task groups must remain reachable without a card: {id}"
            );
        }
    })
    .expect("navigation");
    cx.update(|cx| {
        let session = harness.view.read(cx).session().read(cx);
        assert!(session.present_card.is_none());
    });
}

#[gpui_kit::test]
fn reader_menu_paints_and_clicks_above_page_content(cx: &mut TestAppContext) {
    let (transport, _handle) = classic_transport();
    transport.add(DeviceInfo::new("Mock Second", "mock:second"), None);
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);

    click(cx, harness.handle, "reader-menu");
    let popover_top = cx
        .update_window(harness.handle, |_, window, _| {
            let option = window.find(("reader-option", 1usize));
            assert!(option.visible());
            option.bounds().origin.y
        })
        .expect("popover bounds");
    assert!(popover_top > px(0.));

    click(cx, harness.handle, ("reader-option", 1usize));
    cx.update(|cx| {
        let session = harness.view.read(cx).session().read(cx);
        assert_eq!(session.selected_device.as_deref(), Some("mock:second"));
    });
}

#[gpui_kit::test]
fn recovery_scope_note_wraps_inside_the_run_card(cx: &mut TestAppContext) {
    let (transport, _handle) = classic_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);

    click(cx, harness.handle, "nav-recovery");
    cx.update_window(harness.handle, |_, window, _| {
        let snapshot = window.find("recovery-scope-note");
        assert!(snapshot.visible());
        assert!(
            snapshot.bounds().size.width <= px(330.),
            "note width {} exceeds the run card",
            snapshot.bounds().size.width
        );
        assert!(
            snapshot.bounds().size.height > px(40.),
            "note did not wrap: {:?}",
            snapshot.bounds()
        );
    })
    .expect("inspect recovery note");
}

#[gpui_kit::test]
fn activity_safety_notes_wrap_inside_the_inspector(cx: &mut TestAppContext) {
    let (transport, _handle) = classic_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);

    click(cx, harness.handle, "nav-activity");
    cx.update_window(harness.handle, |_, window, _| {
        for id in ["safety-backup", "safety-advanced", "safety-block-zero"] {
            let snapshot = window.find(id);
            assert!(snapshot.visible());
            assert!(
                snapshot.bounds().size.width <= px(290.),
                "{id} width {} exceeds the safety panel",
                snapshot.bounds().size.width
            );
            assert!(
                snapshot.bounds().size.height > px(40.),
                "{id} did not wrap: {:?}",
                snapshot.bounds()
            );
        }
    })
    .expect("inspect safety notes");
}

#[gpui_kit::test]
fn shell_keeps_the_topbar_and_statusbar_anchored(cx: &mut TestAppContext) {
    let (transport, _handle) = classic_transport();
    let harness = open_shell(cx, transport, None);
    settle(cx, harness.handle);

    cx.update_window(harness.handle, |_, window, _| {
        let topbar = window.find("reader-menu");
        assert!(
            topbar.bounds().origin.y < px(100.),
            "reader menu drifted from the top: {:?}",
            topbar.bounds()
        );
        let status = window.find("status-summary");
        assert!(
            status.bounds().origin.y > px(740.),
            "status bar drifted from the bottom: {:?}",
            status.bounds()
        );
    })
    .expect("inspect shell anchors");
}
