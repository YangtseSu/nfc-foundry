//! NFC Foundry desktop entry point.

use std::sync::{Arc, Mutex};

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use nfc_app::Service;
use nfc_app::classic::KeyStore;
use nfc_drivers::DriverRegistry;
use nfc_foundry::{AppShell, install_shell_theme, load_config};

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            install_shell_theme(cx);
            let (config, translator) = load_config();
            let keys = config
                .key_store_path
                .as_ref()
                .and_then(|path| KeyStore::load_or_default(path).ok())
                .unwrap_or_default();
            let service = Arc::new(Mutex::new(Service::with_transport(Arc::new(
                DriverRegistry,
            ))));
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(1280.), px(800.)),
                    ))),
                    window_min_size: Some(size(px(960.), px(640.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(format!("NFC Foundry {}", env!("CARGO_PKG_VERSION")).into()),
                        ..TitleBar::title_bar_options()
                    }),
                    ..TitleBar::window_options()
                },
                cx,
                move |window, cx| {
                    cx.new(|cx| AppShell::new(window, cx, service, keys, config, translator))
                },
            )
            .expect("failed to open the NFC Foundry window");
        });
}
