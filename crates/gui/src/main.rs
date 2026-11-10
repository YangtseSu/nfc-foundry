//! NFC Foundry GTK4 desktop application.

use adw::prelude::*;

mod app;
mod classic;

fn main() -> glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("io.github.nfc_foundry.NfcFoundry")
        .build();
    application.connect_activate(app::build_ui);
    application.run()
}
