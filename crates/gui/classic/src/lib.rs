//! GPUI Kit MIFARE Classic screens.
//!
//! The workspace owns presentation state. All card and file workflows call
//! `nfc-app` services that also back `nfctl`.

mod model;
mod views;
mod workspace;

pub use model::{
    ClassicSettings, DUMP_FORMATS, NdefSource, STRATEGIES, ScanTypes, dump_format_index,
    dump_format_key, strategy_description_key, strategy_key,
};
pub use workspace::ClassicWorkspace;
