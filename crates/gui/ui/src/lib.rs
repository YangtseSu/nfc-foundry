//! Shared presentation layer for the NFC Foundry workbench.
//!
//! The crate owns the language catalogs, the session preference file, shared
//! screen primitives, and the state the shell and workspace crates exchange.
//! It performs no reader or card I/O.

mod components;
mod i18n;
mod preferences;
mod session;
mod util;

pub use components::{
    ascii, byte_grid, callout, callout_with_id, card, card_body, card_head, empty_state, field,
    hex_bytes, inspector, kv_row, metric, page_header, pill, section_title, text_value,
};
pub use i18n::{Key, Language, Translator, default_language, detect_language, locale_language};
pub use preferences::{UiPreferences, config_directory, preferences_path};

/// Shared typed-confirmation callback used by the workspace crates.
pub type ConfirmAction = std::rc::Rc<dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)>;
pub use session::{
    ActivityEntry, Busy, CardIdentity, ReaderSummary, RecoveryStage, RecoveryView, Session,
    StatusKind, StatusMessage, card_type_name, error_key, timestamp,
};
pub use util::{parse_hex_array, parse_hex_bytes, parse_page_list, write_private};
