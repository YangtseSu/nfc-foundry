//! Application services shared by the GUI and `nfctl`.
//!
//! Device ownership, exclusive operations, backups, and workflows live here.
//! The GUI and CLI never touch a driver directly.

pub mod classic;
pub mod device;
pub mod event;
mod fsutil;
pub mod operation;
pub mod recovery;
pub mod service;
pub mod type2;

pub use classic::{
    BackupRecord, BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicKeyscanResult,
    ClassicRead, ClassicReadResult, ClassicRestore, ClassicRestoreResult, ClassicWriteResult,
    KeyEntry, KeyKind, KeySource, KeyStore, KeyTypes, SectorKeys,
};
pub use device::DeviceManager;
pub use operation::{Operation, OperationKind};
pub use service::Service;
pub use type2::{
    Type2Format, Type2Lock, Type2LockResult, Type2MutationResult, Type2NdefResult, Type2NdefWrite,
    Type2PageResult, Type2PageWrite, Type2Password, Type2ReadResult, Type2Restore,
};
