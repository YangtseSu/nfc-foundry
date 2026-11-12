//! DESFire inventory and authenticated read workflows.

use nfc_core::{CancelToken, NfcError, Result};

use crate::Service;
use crate::operation::OperationKind;

/// One standard or backup file read after selecting an application.
///
/// The key is optional so a free-access file can be read without
/// authentication. The request deliberately does not implement `Debug`:
/// key material must not be logged.
pub struct DesfireReadRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub key: Option<Vec<u8>>,
    pub key_number: u8,
    pub offset: u32,
    pub length: Option<u32>,
}

impl DesfireReadRequest {
    pub fn new(aid: [u8; 3], file_id: u8) -> Self {
        Self {
            aid,
            file_id,
            key: None,
            key_number: 0,
            offset: 0,
            length: None,
        }
    }

    pub fn key(mut self, key: Vec<u8>) -> Self {
        self.key = Some(key);
        self
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }

    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = offset;
        self
    }

    pub fn length(mut self, length: Option<u32>) -> Self {
        self.length = length;
        self
    }
}

/// File settings and data returned by one authenticated read.
pub struct DesfireRead {
    pub settings: Vec<u8>,
    pub data: Vec<u8>,
}

impl Service {
    /// Read the DESFire version and application/file inventory without
    /// authentication.
    pub fn desfire_inventory(&mut self, cancel: CancelToken) -> Result<nfc_desfire::Inventory> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        nfc_desfire::inventory(operation.reader_mut())
    }

    /// Read one standard or backup file. Authentication is optional; when a
    /// key is supplied it is used with the legacy DES/2K3DES authenticator.
    pub fn desfire_read(
        &mut self,
        cancel: CancelToken,
        request: DesfireReadRequest,
    ) -> Result<DesfireRead> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let mut desfire = nfc_desfire::Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            desfire.authenticate_legacy(request.key_number, key)?;
        }
        let settings = desfire.file_settings(request.file_id)?;
        let size = nfc_desfire::Desfire::data_file_size(&settings).ok_or(NfcError::Unsupported(
            "DESFire read currently supports standard and backup data files",
        ))?;
        if request.offset > size {
            return Err(NfcError::InvalidArgument(format!(
                "read offset {} is beyond file size {size}",
                request.offset
            )));
        }
        let remaining = size - request.offset;
        let length = request.length.unwrap_or(remaining);
        if length == 0 || length > remaining {
            return Err(NfcError::InvalidArgument(format!(
                "read length {length} is outside the remaining {remaining} bytes"
            )));
        }
        let data = desfire.read_data(request.file_id, request.offset, length)?;
        if data.len() as u32 != length {
            return Err(NfcError::VerificationFailed(format!(
                "read returned {} bytes, expected {length}",
                data.len()
            )));
        }
        Ok(DesfireRead { settings, data })
    }
}
