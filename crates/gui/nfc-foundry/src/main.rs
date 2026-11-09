//! NFC Foundry desktop shell. Feature views live in their own crates.

use std::sync::{Arc, Mutex};

use gpui_kit::*;
use nfc_app::Service;
use nfc_app::classic::{BackupStore, KeyStore};
use nfc_classic_gui::ClassicWorkspace;
use nfc_drivers::DriverRegistry;

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let service = Arc::new(Mutex::new(Service::with_transport(Arc::new(
                DriverRegistry,
            ))));
            let key_store_path = KeyStore::default_path();
            let keys = key_store_path
                .as_ref()
                .and_then(|path| KeyStore::load_or_default(path).ok())
                .unwrap_or_default();
            gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                let service = service.clone();
                let keys = keys.clone();
                let key_store_path = key_store_path.clone();
                cx.new(|cx| {
                    ClassicWorkspace::new(
                        window,
                        cx,
                        service,
                        keys,
                        key_store_path,
                        BackupStore::default(),
                    )
                })
            })
            .expect("failed to open the NFC Foundry window");
        });
}
