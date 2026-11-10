//! GPUI Kit UI integration tests for the Type 2 / NDEF workspace.

use std::sync::{Arc, Mutex};

use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use nfc_app::Service;
use nfc_app::classic::BackupStore;
use nfc_core::{CardInfo, CardType, PAGE_SIZE};
use nfc_ndef::{NdefMessage, TextRecord};
use nfc_transport::DeviceInfo;
use nfc_transport::mock::{MockHandle, MockTransport};
use nfc_type2::NdefState;
use nfc_type2_gui::Type2Workspace;

const CONFIRMATION: &str = "I_OWN_THIS_CARD";

fn card() -> CardInfo {
    CardInfo::new(
        vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
        [0x00, 0x44],
        0x00,
        CardType::Type2,
    )
}

fn pages() -> Vec<(u8, [u8; PAGE_SIZE])> {
    let mut pages: Vec<(u8, [u8; PAGE_SIZE])> =
        (0..135u8).map(|page| (page, [0u8; PAGE_SIZE])).collect();
    pages[0] = (0, [0x04, 0x5B, 0x1C, 0x2A]);
    pages[1] = (1, [0x3B, 0x4C, 0x80, 0x00]);
    pages[3] = (3, [0xE1, 0x10, 0x3E, 0x00]);
    pages[4] = (4, [0x03, 0x00, 0xFE, 0x00]);
    pages[130] = (130, [0x00, 0x00, 0x00, 0xBD]);
    pages[131] = (131, [0x00, 0x00, 0x00, 0xFF]);
    pages
}

fn mock_service() -> (Arc<Mutex<Service>>, MockHandle) {
    let transport = MockTransport::new();
    let handle = transport.add(
        DeviceInfo::new("Mock NTAG215", "mock:ntag215"),
        Some(card()),
    );
    for (page, data) in pages() {
        handle.set_page(page, data);
    }
    let service = Service::with_transport(Arc::new(transport));
    (Arc::new(Mutex::new(service)), handle)
}

fn open(
    cx: &mut TestAppContext,
    service: Arc<Mutex<Service>>,
    backup: BackupStore,
) -> (gpui_kit::AnyWindowHandle, Entity<Type2Workspace>) {
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
            |window, cx| cx.new(|cx| Type2Workspace::new(window, cx, service, backup)),
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
fn reads_a_tag_and_selects_a_page(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(cx, service, BackupStore::new(directory.path()));
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);

    cx.update(|cx| {
        assert!(view.read(cx).image().is_some());
        assert_eq!(view.read(cx).selected_page(), Some(4));
    });

    cx.update_window(window_handle, |_, window, cx| {
        assert!(window.find(("type2-page", 8usize)).visible());
        window.click(("type2-page", 8usize), cx);
    })
    .expect("select page");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, _| {
        let inspector = window
            .find("type2-inspector")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(inspector.contains("Page 8"), "{inspector}");
        assert!(inspector.contains("user memory"), "{inspector}");
    })
    .expect("inspect");
    cx.update(|cx| {
        assert_eq!(view.read(cx).selected_page(), Some(8));
        assert_eq!(
            view.read(cx).status().map(|status| status.text.as_ref()),
            Some("Tag read")
        );
    });
}

#[gpui_kit::test]
fn ndef_write_and_format_round_trip(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(cx, service, BackupStore::new(directory.path()));
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click(1usize, cx);
    })
    .expect("open NDEF tab");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-ndef-text", cx);
        window.input("hello", cx);
        window.click("type2-ndef-write", cx);
    })
    .expect("write NDEF");
    settle(cx, window_handle);

    cx.update(|cx| {
        let expected = NdefMessage::single(
            TextRecord::new("hello", "en")
                .expect("text")
                .to_record()
                .expect("record"),
        )
        .expect("message");
        let workspace = view.read(cx);
        let ndef = workspace.ndef().expect("NDEF state");
        assert_eq!(ndef.state, NdefState::Message(expected));
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("NDEF written and verified"))
        );
    });

    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-ndef-format", cx);
    })
    .expect("open format confirmation");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.click("ok", cx);
    })
    .expect("confirm format");
    settle(cx, window_handle);

    cx.update(|cx| {
        let workspace = view.read(cx);
        assert_eq!(
            workspace.ndef().expect("NDEF state").state,
            NdefState::Empty
        );
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("NDEF formatted"))
        );
    });
}

#[gpui_kit::test]
fn advanced_page_write_waits_for_a_typed_confirmation(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(cx, service, BackupStore::new(directory.path()));
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click(("type2-page", 3usize), cx);
    })
    .expect("select CC page");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-write-page", cx);
    })
    .expect("open Advanced dialog");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.close_dialog(cx);
    })
    .expect("close dialog");
    settle(cx, window_handle);
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).image().expect("image").page(3),
            Some(&[0xE1, 0x10, 0x3E, 0x00])
        );
        assert!(view.read(cx).busy().is_none());
    });
}

#[gpui_kit::test]
fn lock_requires_the_typed_token(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, view) = open(cx, service, BackupStore::new(directory.path()));
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click(2usize, cx);
    })
    .expect("open protection tab");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-lock-pages", cx);
        window.input("4", cx);
        window.click("type2-lock-apply", cx);
    })
    .expect("open lock dialog");
    settle(cx, window_handle);

    cx.update_window(window_handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.click("type2-confirm-token", cx);
        window.press("secondary-a", cx);
        window.input(CONFIRMATION, cx);
        window.click("ok", cx);
    })
    .expect("confirm lock");
    settle(cx, window_handle);

    cx.update(|cx| {
        let workspace = view.read(cx);
        let image = workspace.image().expect("image");
        assert_eq!(image.page(2).expect("lock page")[2] & 0x10, 0x10);
        assert!(
            workspace
                .status()
                .is_some_and(|status| status.text.contains("Lock bits written"))
        );
    });
}

#[gpui_kit::test]
fn protection_tab_shows_configuration(cx: &mut TestAppContext) {
    let (service, _handle) = mock_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let (window_handle, _view) = open(cx, service, BackupStore::new(directory.path()));
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("type2-read", cx);
    })
    .expect("read");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, cx| {
        window.click(2usize, cx);
    })
    .expect("open protection tab");
    settle(cx, window_handle);
    cx.update_window(window_handle, |_, window, _| {
        assert!(window.find("type2-auth0").visible());
        assert!(window.find("type2-lock-pages").visible());
        assert!(window.find("type2-password-apply").visible());
    })
    .expect("protection controls");
}
