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

pub use classic::{
    BackupRecord, BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicKeyscanResult,
    ClassicRead, ClassicReadResult, ClassicRestore, ClassicRestoreResult, ClassicWriteResult,
    KeyEntry, KeyKind, KeySource, KeyStore, KeyTypes, SectorKeys,
};
pub use device::DeviceManager;
pub use operation::{Operation, OperationKind};
pub use service::Service;
