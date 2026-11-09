//! NFC Foundry desktop shell. Feature views are added in later phases.

use gpui_kit::*;
use nfc_app::Service;

#[derive(Default)]
struct FoundryApp {
    service: Service,
}

impl Render for FoundryApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let device_count = self.service.devices().len();
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_4()
            .child("NFC Foundry")
            .child("MIFARE Classic · NDEF")
            .child(format!("{device_count} readers"))
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| FoundryApp::default())
            })
            .expect("failed to open the NFC Foundry window");
        });
}
