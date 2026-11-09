//! MIFARE Classic geometry, access conditions, value blocks, dumps, and diff.
//!
//! This crate is hardware-free. Reader operations live in `nfc-transport` and
//! workflows live in `nfc-app`.

pub mod access;
pub mod block;
pub mod diff;
pub mod layout;
pub mod value;

pub use block::{Block, BlockStatus, Dump, Readability};
pub use diff::{BlockDiff, DiffState, DumpDiff};
pub use layout::Layout;
