use std::sync::Arc;

use nfc_core::{CancelToken, NfcError, Result};
use nfc_transport::{DeviceInfo, NoTransport, Reader, Transport};

use crate::operation::{Operation, OperationKind};

/// Reader selection and lifecycle state. A reader is opened only when an
/// operation needs it and is closed on release, device loss, or a selection
/// change.
pub struct DeviceManager {
    transport: Arc<dyn Transport>,
    devices: Vec<DeviceInfo>,
    selected: Option<String>,
    reader: Option<Box<dyn Reader>>,
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::with_transport(Arc::new(NoTransport))
    }
}

impl DeviceManager {
    pub fn with_transport(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            devices: Vec::new(),
            selected: None,
            reader: None,
        }
    }

    pub fn devices(&self) -> &[DeviceInfo] {
        &self.devices
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Refresh the device list from the transport. A selection that no longer
    /// exists is dropped and its reader is closed.
    pub fn refresh(&mut self) -> Result<()> {
        let devices = self.transport.enumerate()?;
        let selection_is_gone = self
            .selected
            .as_ref()
            .is_some_and(|selected| !devices.iter().any(|device| device.conn_string == *selected));
        if selection_is_gone {
            self.release()?;
            self.selected = None;
        }
        self.devices = devices;
        Ok(())
    }

    pub fn select(&mut self, conn_string: &str) -> Result<()> {
        if !self
            .devices
            .iter()
            .any(|device| device.conn_string == conn_string)
        {
            return Err(NfcError::InvalidArgument(format!(
                "reader {conn_string} is not in the device list"
            )));
        }
        if self.selected.as_deref() == Some(conn_string) {
            return Ok(());
        }
        self.release()?;
        self.selected = Some(conn_string.to_owned());
        Ok(())
    }

    pub fn release(&mut self) -> Result<()> {
        if let Some(mut reader) = self.reader.take() {
            reader.close()?;
        }
        Ok(())
    }

    /// Open the selected reader if needed and return the exclusive lease for
    /// one operation.
    pub fn begin_operation(
        &mut self,
        kind: OperationKind,
        cancel: CancelToken,
    ) -> Result<Operation<'_>> {
        cancel.check()?;
        let selected = self.selected.clone().ok_or(NfcError::NoReader)?;
        if self.reader.is_none() {
            let reader = self.transport.open(&selected)?;
            self.reader = Some(reader);
        }
        let reader = self.reader.as_mut().ok_or(NfcError::NoReader)?;
        let card = reader.card_info()?;
        Ok(Operation::new(kind, reader.as_mut(), card, cancel))
    }
}
