//! Application services shared by the GUI and `nfctl`.
//!
//! Device ownership, exclusive operations, backups, and workflows live here.
//! The GUI and CLI never touch a driver directly.

pub mod classic;
pub mod desfire;
pub mod device;
pub mod diagnostics;
pub mod event;
mod fsutil;
pub mod health;
pub mod operation;
pub mod paths;
pub mod reader;
pub mod recovery;
pub mod service;
pub mod type2;
pub mod type4;

pub use classic::{
    BackupRecord, BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicKeyscanResult,
    ClassicNdefFormat, ClassicNdefFormatResult, ClassicNdefRead, ClassicNdefReadResult,
    ClassicNdefWrite, ClassicNdefWriteResult, ClassicRead, ClassicReadResult, ClassicRestore,
    ClassicRestoreResult, ClassicUidExecution, ClassicUidPlan, ClassicUidPreflight,
    ClassicUidResult, ClassicWorkbench, ClassicWriteExecution, ClassicWritePlan,
    ClassicWritePreflight, ClassicWriteResult, KeyEntry, KeyKind, KeySource, KeyStore, KeyTypes,
    KeyscanProgress, KeyscanSink, PlannedWriteBlock, RestoreBlockResult, RestoreBlockState,
    RestoreSelection, SectorKeys, UID_LEN, UidBackupRecord, UidWriteBackend, WorkbenchMergeReport,
    WorkbenchSnapshot, WorkbenchValidation, WriteKeyTypes, WriteToken, dump_fingerprint,
    format_uid, normalize_uid_text, parse_block_list, parse_uid,
};
pub use device::DeviceManager;
pub use diagnostics::{
    APP_NAME, AppInfo, CapabilityReport, DriverInfo, PcscInfo, PlatformInfo, ReaderDevice,
    ReaderListing, SelfCheck, TablesStatus,
};
pub use health::{CardHealth, CardIdentityHealth, KeyCoverage, classic_health};
pub use nfc_classic_ndef::{
    ClassicNdefInfo, ClassicNdefLayout, MappingVersion, NdefState as ClassicNdefState,
};
pub use operation::{Operation, OperationKind};
pub use reader::{
    DEFAULT_POLL_INTERVAL, DEFAULT_REMOVAL_MISSES, ReaderEvent, ReaderMonitorConfig,
    ReaderSnapshot, ReaderStatus, ReaderUpdate,
};
pub use service::{CardReadResult, Service};
pub use type2::{
    Type2Auth, Type2Format, Type2Lock, Type2LockResult, Type2MutationResult, Type2NdefResult,
    Type2NdefWrite, Type2PageResult, Type2PageWrite, Type2Password, Type2ReadRequest,
    Type2ReadResult, Type2Restore,
};
pub use type4::{Type4Clear, Type4MutationResult, Type4NdefWrite, Type4ReadResult};
