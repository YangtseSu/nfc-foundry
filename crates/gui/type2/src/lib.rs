//! GPUI Kit Type 2 / NDEF screens.
//!
//! The workspace owns presentation state. All card and file work calls
//! `nfc-app` services that also back `nfctl`.

mod model;
mod views;
mod workspace;

pub use model::{NdefSource, PageFilter, Type2Settings, role_key};
pub use workspace::Type2Workspace;
