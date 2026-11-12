//! Read-only DESFire inventory workflow.

use nfc_core::{CancelToken, Result};

use crate::Service;
use crate::operation::OperationKind;

impl Service {
    /// Read the DESFire version and application/file inventory without
    /// authentication.
    pub fn desfire_inventory(&mut self, cancel: CancelToken) -> Result<nfc_desfire::Inventory> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        nfc_desfire::inventory(operation.reader_mut())
    }
}
