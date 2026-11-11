//! MIFARE Classic workflows: dumps, writes, restores, keys, and backups.
//!
//! The GUI and CLI call these functions through [`crate::Service`]. Reader I/O
//! stays behind [`crate::Operation`]; this layer owns access checks, key
//! ordering, backups, and read-back verification.

pub mod backup;
pub mod keys;
pub(crate) mod keyscan;
pub mod ndef;
pub(crate) mod read;
pub(crate) mod restore;
pub(crate) mod write;

pub use backup::{BackupRecord, BackupStore};
pub use keys::{
    AuthenticatedKey, COMMON_KEYS, KeyEntry, KeyImportIssue, KeyImportReport, KeyKind, KeySource,
    KeyStore, KeyVerification, card_id, common_keys,
};
pub use keyscan::{
    ClassicKeyscan, ClassicKeyscanResult, KeyTypes, KeyscanProgress, KeyscanSink, SectorKeys,
};
pub use ndef::{
    ClassicNdefFormat, ClassicNdefFormatResult, ClassicNdefRead, ClassicNdefReadResult,
    ClassicNdefWrite, ClassicNdefWriteResult,
};
pub use read::{ClassicBlockReadResult, ClassicRead, ClassicReadResult};
pub use restore::{ClassicRestore, ClassicRestoreResult, RestoreBlockResult, RestoreBlockState};
pub use write::{ClassicBlockWrite, ClassicWriteResult};
