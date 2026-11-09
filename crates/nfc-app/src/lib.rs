//! Application services shared by the GUI and `nfctl`.
//!
//! Device ownership, exclusive operations, backups, and workflows live here.
//! The GUI and CLI never touch a driver directly.

pub mod device;
pub mod event;
pub mod operation;
pub mod service;

pub use device::DeviceManager;
pub use operation::{Operation, OperationKind};
pub use service::Service;
