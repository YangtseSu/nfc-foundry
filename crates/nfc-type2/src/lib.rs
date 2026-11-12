//! NFC Forum Type 2 / NTAG page model and NDEF TLV adapter.
//!
//! The crate owns page geometry, capability container validation, lock and
//! configuration interpretation, NDEF TLV mapping, project dumps, and
//! reader-backed read/write workflows. It reaches hardware only through
//! [`nfc_transport::Reader`].

pub mod auth;
pub mod cc;
pub mod config;
pub mod exchange;
pub mod layout;
pub mod lock;
pub mod ndef;
pub mod page;
pub mod read;
pub mod version;
pub mod write;

pub use auth::{Type2Auth, Type2AuthSession};
pub use cc::CapabilityContainer;
pub use config::{Configuration, PasswordUpdate};
pub use layout::{ConfigPages, PageRole, Type2Layout, Type2Product};
pub use lock::{LockBits, LockUpdate};
pub use ndef::{NdefState, Type2Ndef, encode_ndef_area, format_ndef_area, read_ndef};
pub use page::{Page, Type2Image};
pub use read::{Type2ReadResult, quad_starts, read_page, read_tag};
pub use version::Type2Version;
pub use write::{
    PageWriteResult, RestoreOptions, WritePolicy, WriteSummary, check_writable, format_ndef,
    lock_pages, restore_image, set_password, write_area, write_ndef, write_page_verified,
};
