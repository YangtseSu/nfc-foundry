//! NFC Foundry GTK4 desktop application.

use adw::prelude::*;

mod app;
mod classic;
mod i18n;
mod ndef;
mod settings;
mod type2;

fn main() -> glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("io.github.nfc_foundry.NfcFoundry")
        .build();
    application.connect_activate(app::build_ui);
    application.run()
}
