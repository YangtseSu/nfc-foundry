//! GPUI Kit UI integration tests for the Classic workspace.

use std::sync::{Arc, Mutex};

use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use nfc_app::Service;
use nfc_app::classic::{BackupStore, KeyStore};
use nfc_classic::Layout;
use nfc_classic_gui::ClassicWorkspace;
use nfc_core::{BLOCK_SIZE, CardInfo, CardType};
use nfc_transport::DeviceInfo;
use nfc_transport::mock::{MockHandle, MockTransport};

fn image() -> Vec<(u8, [u8; BLOCK_SIZE])> {
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

fn mock_service() -> (Arc<Mutex<Service>>, MockHandle) {
    let transport = MockTransport::new();
    let uid = [0x04, 0xA2, 0x24, 0x8A];
    let card = CardInfo::new(uid.to_vec(), [0x00, 0x04], 0x08, CardType::MifareClassic1K);
    let handle = transport.add(
        DeviceInfo::new("Mock Classic 1K", "mock:classic-1k"),
        Some(card),
    );
    for (block, data) in image() {
        handle.set_block(block, data);
    }
    let service = Service::with_transport(Arc::new(transport));
    (Arc::new(Mutex::new(service)), handle)
}

fn open(
    cx: &mut TestAppContext,
    service: Arc<Mutex<Service>>,
    keys: KeyStore,
    backup: BackupStore,
) -> (gpui_kit::AnyWindowHandle, Entity<ClassicWorkspace>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
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
                cx.new(|cx| ClassicWorkspace::new(window, cx, service, keys, None, backup))
            },
        )
    })
    .expect("open test window")
}

fn settle(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
    })
    .expect("render frame");
}

#[gpui_kit::test]
fn reads_a_card_and_selects_a_block(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-read", cx);
    })
    .expect("click read");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        assert!(window.find(("classic-block", 0usize)).visible());
        window.click(("classic-block", 4usize), cx);
    })
    .expect("click block");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, _| {
        let inspector = window
            .find("classic-inspector")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(inspector.contains("Block 4"), "{inspector}");
        assert!(inspector.contains("sector 1"), "{inspector}");
    })
    .expect("inspect");
    cx.update(|cx| {
        let workspace = view.read(cx);
        assert!(workspace.dump().expect("dump").complete());
        assert_eq!(workspace.selected_block(), Some(4));
        assert_eq!(
            workspace.status().map(|status| status.text.as_ref()),
            Some("Card read")
        );
    });
}

#[gpui_kit::test]
fn a_second_read_produces_a_diff(cx: &mut TestAppContext) {
    let (service, handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-read", cx);
    })
    .expect("first read");
    settle(cx, window_handle);

    handle.set_block(4, [0xA5; BLOCK_SIZE]);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-read", cx);
    })
    .expect("second read");
    settle(cx, window_handle);

    cx.update(|cx| {
        let workspace = view.read(cx);
        let diff = workspace.diff().expect("diff");
        assert_eq!(diff.changed_blocks, 1);
        assert_eq!(diff.blocks[4].state, nfc_classic::DiffState::Changed);
    });
    cx.update_window(window_handle, |_, window, cx| {
        window.click(2usize, cx);
    })
    .expect("open diff tab");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, _| {
        assert!(window.find("classic-diff-scroll").visible());
    })
    .expect("diff tab");
}

#[gpui_kit::test]
fn keys_tab_adds_and_removes_a_key(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click(3usize, cx);
    })
    .expect("open keys tab");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-key-hex", cx);
        window.input("A0A1A2A3A4A5", cx);
        window.click("classic-key-sector", cx);
        window.input("3", cx);
        window.click("classic-add-key", cx);
    })
    .expect("add key");
    settle(cx, window_handle);

    cx.update(|cx| {
        let workspace = view.read(cx);
        assert_eq!(workspace.keys().len(), 1);
        assert_eq!(workspace.keys().entries()[0].sector, Some(3));
        assert_eq!(workspace.keys().entries()[0].key.to_hex(), "A0A1A2A3A4A5");
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("Key added"))
        );
    });

    cx.update_window(window_handle, |_, window, cx| {
        window.click(("classic-remove-key", 0usize), cx);
    })
    .expect("remove key");
    settle(cx, window_handle);
    cx.update(|cx| assert!(view.read(cx).keys().is_empty()));
}

#[gpui_kit::test]
fn writes_a_block_with_read_back_verification(cx: &mut TestAppContext) {
    let (service, handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click(("classic-block", 4usize), cx);
    })
    .expect("select block");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-block-data", cx);
        window.press("secondary-a", cx);
        window.input("000102030405060708090A0B0C0D0E0F", cx);
    })
    .expect("edit block");
    cx.update_window(window_handle, |_, window, cx| {
        assert_eq!(
            window.find("classic-block-data").value(),
            Some("000102030405060708090A0B0C0D0E0F")
        );
        window.click("classic-write-block", cx);
    })
    .expect("write block");
    settle(cx, window_handle);

    assert_eq!(
        handle.block(4),
        Some([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
            0x0E, 0x0F
        ])
    );
    cx.update(|cx| {
        let workspace = view.read(cx);
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("written and verified"))
        );
        assert_eq!(
            workspace.dump().expect("dump").blocks[4].data,
            [
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
                0x0E, 0x0F
            ]
        );
    });
}

#[gpui_kit::test]
fn restore_waits_for_confirmation(cx: &mut TestAppContext) {
    let (service, handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);
    handle.set_block(4, [0xA5; BLOCK_SIZE]);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-restore", cx);
    })
    .expect("open restore confirmation");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.close_dialog(cx);
    })
    .expect("dialog state");
    settle(cx, window_handle);
    assert_eq!(handle.block(4), Some([0xA5; BLOCK_SIZE]));
    cx.update(|cx| assert!(view.read(cx).busy().is_none()));
}

#[gpui_kit::test]
fn scans_keys_from_the_toolbar(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-scan-keys", cx);
    })
    .expect("scan keys");
    settle(cx, window_handle);
    cx.update(|cx| {
        let workspace = view.read(cx);
        assert!(workspace.keys().len() >= 16, "{}", workspace.keys().len());
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("Key scan"))
        );
    });
}

#[gpui_kit::test]
fn ndef_format_then_write_round_trips(cx: &mut TestAppContext) {
    let (service, handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click(1usize, cx);
    })
    .expect("open NDEF tab");
    settle(cx, window_handle);
    cx.update(|cx| {
        let workspace = view.read(cx);
        assert!(workspace.ndef().is_some(), "NDEF facts after read");
        assert!(
            workspace
                .ndef()
                .and_then(|info| info.layout.as_ref())
                .is_none()
        );
    });

    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-ndef-format", cx);
    })
    .expect("open format dialog");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.click("ok", cx);
    })
    .expect("confirm format");
    settle(cx, window_handle);

    cx.update(|cx| {
        let workspace = view.read(cx);
        let info = workspace.ndef().expect("NDEF facts");
        assert_eq!(info.ndef.state, nfc_app::ClassicNdefState::Empty);
        assert_eq!(
            info.layout.as_ref().expect("layout").sectors,
            (1..=15u8).collect::<Vec<_>>()
        );
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("NDEF formatted"))
        );
    });
    assert!(handle.block(3).expect("trailer")[..6] == [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-ndef-text", cx);
        window.input("hello classic", cx);
        window.click("classic-ndef-write", cx);
    })
    .expect("write NDEF");
    settle(cx, window_handle);

    cx.update(|cx| {
        let workspace = view.read(cx);
        let info = workspace.ndef().expect("NDEF facts");
        assert!(matches!(
            info.ndef.state,
            nfc_app::ClassicNdefState::Message(_)
        ));
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("NDEF written and verified"))
        );
    });
}

#[gpui_kit::test]
fn ndef_format_can_set_read_only(cx: &mut TestAppContext) {
    let (service, handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(
        cx,
        service,
        KeyStore::new(),
        BackupStore::new(directory.path()),
    );
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click(1usize, cx);
        window.click("classic-ndef-read-only", cx);
    })
    .expect("open NDEF and toggle read-only");
    settle(cx, window_handle);
    cx.update(|cx| assert!(view.read(cx).format_read_only()));

    cx.update_window(window_handle, |_, window, cx| {
        window.click("classic-ndef-format", cx);
    })
    .expect("open format dialog");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("ok", cx);
    })
    .expect("confirm format");
    settle(cx, window_handle);

    cx.update(|cx| {
        let workspace = view.read(cx);
        assert!(
            workspace
                .ndef()
                .and_then(|info| info.layout.as_ref())
                .is_some_and(|layout| layout.read_only)
        );
    });
    assert_eq!(
        &handle.block(7).expect("trailer")[6..9],
        &[0x07, 0x8F, 0x0F]
    );
}
