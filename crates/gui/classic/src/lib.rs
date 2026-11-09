//! GPUI Kit MIFARE Classic workspace.
//!
//! The view owns presentation and retained UI state. All card and file
//! workflows call `nfc-app` services that also back `nfctl`.

mod model;
mod views;
mod workspace;

pub use model::{DeviceSummary, StatusKind, StatusMessage, WorkspaceTab};
pub use workspace::ClassicWorkspace;
