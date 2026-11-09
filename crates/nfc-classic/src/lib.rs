//! MIFARE Classic geometry, access conditions, value blocks, dumps, and diff.
//!
//! This crate is hardware-free. Reader operations live in `nfc-transport` and
//! workflows live in `nfc-app`.

pub mod access;
pub mod bcc;
pub mod block;
pub mod diff;
pub mod exchange;
pub mod layout;
pub mod value;

pub use access::{AccessBits, AccessCondition, KeyMask, TrailerPermissions};
pub use bcc::{BccStatus, bcc_for_uid};
pub use block::{Block, BlockStatus, Dump, Readability};
pub use diff::{BlockDiff, BlockKind, ByteDiff, DiffState, DumpDiff, TrailerDiff};
pub use exchange::{DumpFormat, DumpLoad};
pub use layout::Layout;
pub use value::ValueBlock;
