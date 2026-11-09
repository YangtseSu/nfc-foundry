//! MIFARE Classic NDEF adapter: MAD1/MAD2, capability container, NDEF sectors.

pub mod layout;
pub mod mad;

pub use layout::ClassicNdefLayout;
pub use mad::Mad;
