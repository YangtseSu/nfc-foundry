//! MIFARE Classic workflows: dumps, writes, restores, keys, and backups.
//!
//! The GUI and CLI call these functions through [`crate::Service`]. Reader I/O
//! stays behind [`crate::Operation`]; this layer owns access checks, key
//! ordering, backups, and read-back verification.

pub mod backup;
pub mod keys;
pub(crate) mod keyscan;
pub mod ndef;
pub(crate) mod plan;
pub(crate) mod read;
pub(crate) mod restore;
pub mod uid;
pub(crate) mod workbench;
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
pub use plan::{
    ClassicWriteExecution, ClassicWritePlan, ClassicWritePreflight, PlannedWriteBlock,
    RestoreSelection, WRITE_PLAN_TTL, WriteKeyTypes, WriteToken, parse_block_list, validate_source,
};
pub use read::{ClassicBlockReadResult, ClassicRead, ClassicReadResult};
pub use restore::{ClassicRestore, ClassicRestoreResult, RestoreBlockResult, RestoreBlockState};
pub use uid::{
    ClassicUidExecution, ClassicUidPlan, ClassicUidPreflight, ClassicUidResult, UID_LEN,
    UidBackupRecord, UidWriteBackend, format_uid, normalize_uid_text, parse_uid,
};
pub use workbench::{
    ClassicWorkbench, WorkbenchMergeReport, WorkbenchSnapshot, WorkbenchValidation,
    dump_fingerprint,
};
pub use write::{ClassicBlockWrite, ClassicWriteResult};
