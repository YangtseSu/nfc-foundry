//! NFC Forum Type 2 / NTAG page model and NDEF TLV adapter.

pub mod cc;
pub mod config;
pub mod lock;
pub mod page;

pub use cc::CapabilityContainer;
pub use page::{Page, Type2Info};
