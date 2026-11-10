//! GPUI Kit Type 2 / NDEF workspace.
//!
//! The view owns presentation and retained UI state. All card and file work
//! calls `nfc-app` services that also back `nfctl`.

mod model;
mod views;
mod workspace;

pub use model::{DeviceSummary, StatusKind, StatusMessage, WorkspaceTab};
pub use workspace::Type2Workspace;
