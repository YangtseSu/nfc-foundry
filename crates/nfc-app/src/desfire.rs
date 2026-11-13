//! DESFire inventory, verification, authenticated read, and NDEF provisioning
//! workflows.

use nfc_core::{CancelToken, CardType, NfcError, Result};
use nfc_desfire::Desfire;
pub use nfc_desfire::{
    AccessRight, AccessRights, Application, AuthKeyType, CommunicationMode, FileContents, FileInfo,
    FileSettings, FileType, IntegrityCheck, Inventory, NDEF_APPLICATION_AID,
    NDEF_APPLICATION_ISO_FILE, NDEF_APPLICATION_KEY, NDEF_APPLICATION_KEY_SETTINGS,
    NDEF_CAPABILITY_CONTAINER, NDEF_CC_ACCESS_RIGHTS, NDEF_CC_FILE, NDEF_CC_ISO_FILE, NDEF_CC_SIZE,
    NDEF_DATA_ACCESS_RIGHTS, NDEF_DATA_FILE, NDEF_DATA_ISO_FILE, NDEF_DATA_SIZE, NDEF_DF_NAME,
    Version,
};
use nfc_type4::apdu;
use nfc_type4::{NdefState, Type4CapabilityContainer, Type4Ndef};

use crate::Service;
use crate::operation::{Operation, OperationKind};

/// One standard or backup file read after selecting an application.
///
/// The key is optional so a free-access file can be read without
/// authentication. The request deliberately does not implement `Debug`:
/// key material must not be logged.
pub struct DesfireReadRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub offset: u32,
    pub length: Option<u32>,
    pub start_record: u32,
    pub record_count: Option<u32>,
}

impl DesfireReadRequest {
    pub fn new(aid: [u8; 3], file_id: u8) -> Self {
        Self {
            aid,
            file_id,
            key: None,
            key_type: None,
            key_number: 0,
            offset: 0,
            length: None,
            start_record: 0,
            record_count: None,
        }
    }

    pub fn key(mut self, key: Vec<u8>) -> Self {
        self.key = Some(key);
        self
    }

    pub fn key_type(mut self, key_type: AuthKeyType) -> Self {
        self.key_type = Some(key_type);
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

    pub fn start_record(mut self, start_record: u32) -> Self {
        self.start_record = start_record;
        self
    }

    pub fn record_count(mut self, record_count: Option<u32>) -> Self {
        self.record_count = record_count;
        self
    }
}

/// One file read after the file type selected the command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesfireReadResult {
    Data(DesfireDataRead),
    Value(DesfireValueRead),
    Records(DesfireRecordRead),
}

/// Protection that was verified for one file read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesfireReadProtection {
    /// Communication mode actually used after the access-right decision.
    pub communication: CommunicationMode,
    /// Key family of the session that protected the response.
    pub key_type: Option<AuthKeyType>,
    /// Response check that passed before the data was returned.
    pub integrity: IntegrityCheck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireDataRead {
    pub settings: FileSettings,
    pub data: Vec<u8>,
    pub protection: DesfireReadProtection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireValueRead {
    pub settings: FileSettings,
    pub value: i32,
    pub protection: DesfireReadProtection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireRecordRead {
    pub settings: FileSettings,
    /// DESFire record offset: 0 is the most recent record.
    pub start_record: u32,
    /// Absolute index of the first returned record.
    pub first_record: u32,
    pub record_size: u32,
    pub records: Vec<Vec<u8>>,
    pub protection: DesfireReadProtection,
}

/// One Standard or Backup file write after selecting an application.
///
/// The key is optional so a free-access file can be written without
/// authentication. The request deliberately does not implement `Debug`:
/// key material must not be logged.
pub struct DesfireWriteRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub offset: u32,
    pub data: Vec<u8>,
    pub key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
}

impl DesfireWriteRequest {
    pub fn new(aid: [u8; 3], file_id: u8, data: Vec<u8>) -> Self {
        Self {
            aid,
            file_id,
            offset: 0,
            data,
            key: None,
            key_type: None,
            key_number: 0,
        }
    }

    pub fn key(mut self, key: Vec<u8>) -> Self {
        self.key = Some(key);
        self
    }

    pub fn key_type(mut self, key_type: AuthKeyType) -> Self {
        self.key_type = Some(key_type);
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
}

/// Verified result of one DESFire file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireWriteResult {
    pub settings: FileSettings,
    /// Communication mode used for the file data.
    pub communication: CommunicationMode,
    /// Key family of the session that protected the write, when any.
    pub key_type: Option<AuthKeyType>,
    /// Response check of the last write command.
    pub integrity: IntegrityCheck,
    pub offset: u32,
    pub written: usize,
    /// Complete file content exported before the first write.
    pub backup: Vec<u8>,
    /// Backup files commit their transaction; Standard files do not need it.
    pub committed: bool,
    /// Written range read back after the write.
    pub read_back: Vec<u8>,
}

/// Verify one application key without reading file data.
///
/// The key is session-scoped and deliberately not part of `Debug`.
pub struct DesfireVerifyKeyRequest {
    pub aid: [u8; 3],
    key: Vec<u8>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
}

impl DesfireVerifyKeyRequest {
    pub fn new(aid: [u8; 3], key: Vec<u8>) -> Self {
        Self {
            aid,
            key,
            key_type: None,
            key_number: 0,
        }
    }

    pub fn key_type(mut self, key_type: AuthKeyType) -> Self {
        self.key_type = Some(key_type);
        self
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }
}

/// Result of an authentication check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireKeyVerification {
    pub aid: [u8; 3],
    pub key_number: u8,
    pub key_type: AuthKeyType,
    pub secure_messaging: bool,
    pub key_version: Option<u8>,
}

/// Read the authenticated card UID.
///
/// The key is session-scoped and deliberately not part of `Debug`.
pub struct DesfireUidRequest {
    pub aid: [u8; 3],
    key: Vec<u8>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
}

impl DesfireUidRequest {
    pub fn new(aid: [u8; 3], key: Vec<u8>) -> Self {
        Self {
            aid,
            key,
            key_type: None,
            key_number: 0,
        }
    }

    pub fn key_type(mut self, key_type: AuthKeyType) -> Self {
        self.key_type = Some(key_type);
        self
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }
}

/// Authenticated UID and the check that protected it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireUid {
    pub aid: [u8; 3],
    pub key_number: u8,
    pub key_type: AuthKeyType,
    pub uid: [u8; 7],
    pub integrity: IntegrityCheck,
}

/// Create the AN11004 NDEF Tag Application on an unformatted card.
///
/// The key is the PICC master key and deliberately stays out of `Debug`. The
/// operation requires Advanced mode and explicit confirmation because the
/// card has no byte-for-byte backup; deleting the application is the undo
/// path.
pub struct DesfireNdefFormatRequest {
    key: Vec<u8>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub advanced: bool,
    pub confirm: bool,
    pub cancel: CancelToken,
}

impl DesfireNdefFormatRequest {
    pub fn new(key: Vec<u8>) -> Self {
        Self {
            key,
            key_type: None,
            key_number: 0,
            advanced: false,
            confirm: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn key_type(mut self, key_type: AuthKeyType) -> Self {
        self.key_type = Some(key_type);
        self
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Delete the NDEF Tag Application created by [`Service::desfire_ndef_format`].
pub struct DesfireNdefDeleteRequest {
    key: Vec<u8>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub advanced: bool,
    pub confirm: bool,
    pub cancel: CancelToken,
}

impl DesfireNdefDeleteRequest {
    pub fn new(key: Vec<u8>) -> Self {
        Self {
            key,
            key_type: None,
            key_number: 0,
            advanced: false,
            confirm: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn key_type(mut self, key_type: AuthKeyType) -> Self {
        self.key_type = Some(key_type);
        self
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Verified NDEF Tag Application after formatting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireNdefFormatResult {
    pub cc: Type4CapabilityContainer,
    pub ndef: Type4Ndef,
}

/// Application inventory after deleting the NDEF Tag Application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireNdefDeleteResult {
    pub applications: Vec<[u8; 3]>,
}

impl Service {
    /// Read the DESFire version and application/file inventory without
    /// authentication.
    pub fn desfire_inventory(&mut self, cancel: CancelToken) -> Result<nfc_desfire::Inventory> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        nfc_desfire::inventory(operation.reader_mut())
    }

    /// Authenticate one application key and report the protocol result.
    ///
    /// This reads no file data and changes no stored card state. A wrong key or
    /// an invalid response check fails once; there is no retry.
    pub fn desfire_verify_key(
        &mut self,
        cancel: CancelToken,
        request: DesfireVerifyKeyRequest,
    ) -> Result<DesfireKeyVerification> {
        let key_type = validate_desfire_key(&request.key, request.key_type)?;
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        let verification = desfire.verify_key(request.key_number, &request.key, key_type)?;
        Ok(DesfireKeyVerification {
            aid: request.aid,
            key_number: request.key_number,
            key_type: verification.key_type,
            secure_messaging: verification.secure_messaging,
            key_version: verification.key_version,
        })
    }

    /// Read the UID after authentication and verify the response check.
    ///
    /// This reads no file data and changes no stored card state. A wrong key or
    /// an invalid response check fails once; there is no retry.
    pub fn desfire_card_uid(
        &mut self,
        cancel: CancelToken,
        request: DesfireUidRequest,
    ) -> Result<DesfireUid> {
        let key_type = validate_desfire_key(&request.key, request.key_type)?;
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        desfire.authenticate(request.key_number, &request.key, key_type)?;
        let card_uid = desfire.card_uid()?;
        Ok(DesfireUid {
            aid: request.aid,
            key_number: request.key_number,
            key_type,
            uid: card_uid.uid,
            integrity: card_uid.integrity,
        })
    }

    /// Read one file after GetFileSettings selects the command.
    ///
    /// Standard and backup files use ReadData with the byte offset and length.
    /// Value files use GetValue. Linear and cyclic record files use
    /// ReadRecords with the record number and record count. Authentication is
    /// optional; the explicit key family selects the secure-messaging channel.
    pub fn desfire_read(
        &mut self,
        cancel: CancelToken,
        request: DesfireReadRequest,
    ) -> Result<DesfireReadResult> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let mut desfire = nfc_desfire::Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            match request.key_type {
                Some(key_type) => {
                    desfire.authenticate(request.key_number, key, key_type)?;
                }
                None => desfire.authenticate_legacy(request.key_number, key)?,
            }
        }
        let raw = desfire.file_settings(request.file_id)?;
        let settings = FileSettings::parse(&raw)?;
        let communication =
            effective_read_communication(&settings, request.key_number, request.key.is_some());
        match &settings.contents {
            FileContents::Data { size } => {
                let size = *size;
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
                let read =
                    desfire.read_data(request.file_id, request.offset, length, communication)?;
                if read.data.len() as u32 != length {
                    return Err(NfcError::VerificationFailed(format!(
                        "read returned {} bytes, expected {length}",
                        read.data.len()
                    )));
                }
                let protection = DesfireReadProtection {
                    communication,
                    key_type: desfire.session_key_type(),
                    integrity: read.integrity,
                };
                Ok(DesfireReadResult::Data(DesfireDataRead {
                    settings,
                    data: read.data,
                    protection,
                }))
            }
            FileContents::Value {
                lower_limit,
                upper_limit,
                ..
            } => {
                let read = desfire.get_value(request.file_id, communication)?;
                if read.value < *lower_limit || read.value > *upper_limit {
                    return Err(NfcError::VerificationFailed(format!(
                        "GetValue returned {}, outside the declared limits {lower_limit}..{upper_limit}",
                        read.value
                    )));
                }
                let protection = DesfireReadProtection {
                    communication,
                    key_type: desfire.session_key_type(),
                    integrity: read.integrity,
                };
                Ok(DesfireReadResult::Value(DesfireValueRead {
                    settings,
                    value: read.value,
                    protection,
                }))
            }
            FileContents::Record {
                record_size,
                current_records,
                ..
            } => {
                let record_size = *record_size;
                let current_records = *current_records;
                if current_records == 0 {
                    if request.start_record != 0
                        || request.record_count.is_some_and(|count| count != 0)
                    {
                        return Err(NfcError::InvalidArgument(
                            "the record file is empty".to_owned(),
                        ));
                    }
                    return Ok(DesfireReadResult::Records(DesfireRecordRead {
                        settings,
                        start_record: 0,
                        first_record: 0,
                        record_size,
                        records: Vec::new(),
                        protection: DesfireReadProtection {
                            communication,
                            key_type: desfire.session_key_type(),
                            integrity: IntegrityCheck::None,
                        },
                    }));
                }
                if request.start_record >= current_records {
                    return Err(NfcError::InvalidArgument(format!(
                        "start record {} is beyond the {current_records} current records",
                        request.start_record
                    )));
                }
                let remaining = current_records - request.start_record;
                let count = request.record_count.unwrap_or(remaining);
                if count == 0 || count > remaining {
                    return Err(NfcError::InvalidArgument(format!(
                        "file {:02X}: requested {count} records but only {remaining} remain",
                        request.file_id
                    )));
                }
                let first_record = current_records - request.start_record - count;
                let read = desfire.read_records(
                    request.file_id,
                    request.start_record,
                    count,
                    record_size,
                    communication,
                )?;
                verify_record_bytes(read.data.len(), count, record_size)?;
                let records = read
                    .data
                    .chunks_exact(record_size as usize)
                    .map(<[u8]>::to_vec)
                    .collect();
                let protection = DesfireReadProtection {
                    communication,
                    key_type: desfire.session_key_type(),
                    integrity: read.integrity,
                };
                Ok(DesfireReadResult::Records(DesfireRecordRead {
                    settings,
                    start_record: request.start_record,
                    first_record,
                    record_size,
                    records,
                    protection,
                }))
            }
            FileContents::TransactionMac { .. } => Err(NfcError::Unsupported(
                "DESFire Transaction MAC files have no data read path",
            )),
        }
    }

    /// Write one Standard or Backup file and verify it.
    ///
    /// The current file content is exported before the first write, the write
    /// uses the file's communication mode, and the written range is read back.
    /// A file the same session cannot read back is refused: the product does
    /// not report an unverified write as complete.
    pub fn desfire_write(
        &mut self,
        cancel: CancelToken,
        request: DesfireWriteRequest,
    ) -> Result<DesfireWriteResult> {
        if request.data.is_empty() {
            return Err(NfcError::InvalidArgument(
                "DESFire write data must not be empty".to_owned(),
            ));
        }
        let length = u32::try_from(request.data.len()).map_err(|_| {
            NfcError::InvalidArgument("the DESFire write does not fit a 32-bit length".to_owned())
        })?;
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        operation.check_iso_dep_card()?;
        let cancel = operation.cancel().clone();
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            match request.key_type {
                Some(key_type) => desfire.authenticate(request.key_number, key, key_type)?,
                None => desfire.authenticate_legacy(request.key_number, key)?,
            }
        }
        let raw = desfire.file_settings(request.file_id)?;
        let settings = FileSettings::parse(&raw)?;
        let size = match settings.contents {
            FileContents::Data { size } => size,
            _ => {
                return Err(NfcError::InvalidArgument(format!(
                    "file {:02X} is {}; WriteData needs a Standard or Backup file",
                    request.file_id,
                    settings.file_type.name()
                )));
            }
        };
        if request.offset > size || length > size - request.offset {
            return Err(NfcError::InvalidArgument(format!(
                "write range {}..{} is outside the {size}-byte file {:02X}",
                request.offset,
                u64::from(request.offset) + u64::from(length),
                request.file_id
            )));
        }
        let authenticated = request.key.is_some();
        let communication = desfire_write_communication(
            &settings,
            request.key_number,
            authenticated,
            request.file_id,
        )?;
        if !desfire_read_granted(&settings, request.key_number, authenticated) {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} cannot be read back with this key; the write is refused because it cannot be verified",
                request.file_id
            )));
        }
        let read_communication =
            effective_read_communication(&settings, request.key_number, authenticated);
        let backup = desfire
            .read_data(request.file_id, 0, size, read_communication)?
            .data;
        if backup.len() != size as usize {
            return Err(NfcError::VerificationFailed(format!(
                "the pre-write backup of file {:02X} returned {} bytes, expected {size}",
                request.file_id,
                backup.len()
            )));
        }

        const MAX_WRITE_CHUNK: usize = 200;
        let backup_file = settings.file_type == FileType::Backup;
        let mut written = 0usize;
        let mut integrity = IntegrityCheck::None;
        let write_result = (|| -> Result<()> {
            while written < request.data.len() {
                cancel.check()?;
                let end = (written + MAX_WRITE_CHUNK).min(request.data.len());
                let offset = request.offset + written as u32;
                integrity = desfire.write_data(
                    request.file_id,
                    offset,
                    &request.data[written..end],
                    communication,
                )?;
                written = end;
            }
            Ok(())
        })();
        if let Err(error) = write_result {
            if backup_file {
                let _ = desfire.abort_transaction();
            }
            return Err(error);
        }

        let read_back = desfire
            .read_data(request.file_id, request.offset, length, read_communication)?
            .data;
        if read_back != request.data {
            if backup_file {
                let _ = desfire.abort_transaction();
            }
            let difference = read_back
                .iter()
                .zip(&request.data)
                .position(|(left, right)| left != right)
                .unwrap_or_else(|| read_back.len().min(request.data.len()));
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} offset {} read back differently at byte {difference}",
                request.file_id, request.offset
            )));
        }

        let mut committed = false;
        if backup_file {
            if let Err(error) = desfire.commit_transaction() {
                let _ = desfire.abort_transaction();
                return Err(error);
            }
            committed = true;
            let persisted = desfire
                .read_data(request.file_id, request.offset, length, read_communication)?
                .data;
            if persisted != request.data {
                return Err(NfcError::VerificationFailed(format!(
                    "file {:02X} offset {} differs after CommitTransaction",
                    request.file_id, request.offset
                )));
            }
        }
        let key_type = desfire.session_key_type();
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireWriteResult {
            settings,
            communication,
            key_type,
            integrity,
            offset: request.offset,
            written,
            backup,
            committed,
            read_back,
        })
    }

    /// Create the AN11004 NDEF Tag Application and its CC/NDEF files.
    ///
    /// This is a full-card-scope mutation: no byte-for-byte backup exists.
    /// The matching undo is [`Service::desfire_ndef_delete`].
    pub fn desfire_ndef_format(
        &mut self,
        request: DesfireNdefFormatRequest,
    ) -> Result<DesfireNdefFormatResult> {
        let cancel = request.cancel.clone();
        if !request.advanced {
            return Err(NfcError::InvalidArgument(
                "DESFire NDEF formatting requires Advanced mode".to_owned(),
            ));
        }
        if !request.confirm {
            return Err(NfcError::InvalidArgument(
                "DESFire NDEF formatting needs explicit confirmation".to_owned(),
            ));
        }
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        if let Some(key_type) = request.key_type {
            key_type.validate_key(&request.key)?;
        } else {
            nfc_desfire::AuthKeyType::from_key(&request.key)?;
        }
        let mut operation = self.begin_operation(OperationKind::Write, cancel.clone())?;
        format_ndef(
            &mut operation,
            &request.key,
            request.key_type,
            request.key_number,
            &cancel,
        )
    }

    /// Delete only the NDEF Tag Application created by
    /// [`Service::desfire_ndef_format`], leaving the other applications alone.
    pub fn desfire_ndef_delete(
        &mut self,
        request: DesfireNdefDeleteRequest,
    ) -> Result<DesfireNdefDeleteResult> {
        let cancel = request.cancel.clone();
        if !request.advanced {
            return Err(NfcError::InvalidArgument(
                "deleting the DESFire NDEF application requires Advanced mode".to_owned(),
            ));
        }
        if !request.confirm {
            return Err(NfcError::InvalidArgument(
                "deleting the DESFire NDEF application needs explicit confirmation".to_owned(),
            ));
        }
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        if let Some(key_type) = request.key_type {
            key_type.validate_key(&request.key)?;
        } else {
            nfc_desfire::AuthKeyType::from_key(&request.key)?;
        }
        let mut operation = self.begin_operation(OperationKind::Write, cancel.clone())?;
        delete_ndef(
            &mut operation,
            &request.key,
            request.key_type,
            request.key_number,
            &cancel,
        )
    }
}

fn format_ndef(
    operation: &mut Operation<'_>,
    key: &[u8],
    key_type: Option<AuthKeyType>,
    key_number: u8,
    cancel: &CancelToken,
) -> Result<DesfireNdefFormatResult> {
    ensure_iso_dep(operation)?;
    ensure_ndef_absent(operation, cancel)?;
    cancel.check()?;

    let mut created = false;
    let outcome = (|| -> Result<DesfireNdefFormatResult> {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application([0x00; 3])?;
        authenticate_key(&mut desfire, key_number, key, key_type)?;
        cancel.check()?;
        desfire.create_application(
            NDEF_APPLICATION_AID,
            NDEF_APPLICATION_KEY_SETTINGS.0,
            NDEF_APPLICATION_KEY_SETTINGS.1,
            Some(NDEF_APPLICATION_ISO_FILE),
            &NDEF_DF_NAME,
        )?;
        created = true;

        desfire.select_application(NDEF_APPLICATION_AID)?;
        desfire.authenticate_legacy(0, &NDEF_APPLICATION_KEY)?;
        cancel.check()?;
        desfire.create_standard_file(
            NDEF_CC_FILE,
            Some(NDEF_CC_ISO_FILE),
            0,
            NDEF_CC_ACCESS_RIGHTS,
            NDEF_CC_SIZE,
        )?;
        desfire.create_standard_file(
            NDEF_DATA_FILE,
            Some(NDEF_DATA_ISO_FILE),
            0,
            NDEF_DATA_ACCESS_RIGHTS,
            NDEF_DATA_SIZE,
        )?;
        desfire.write_data(
            NDEF_CC_FILE,
            0,
            &NDEF_CAPABILITY_CONTAINER,
            CommunicationMode::Plain,
        )?;
        desfire.write_data(NDEF_DATA_FILE, 0, &[0x00, 0x00], CommunicationMode::Plain)?;
        cancel.check()?;
        desfire.commit_transaction()?;
        drop(desfire);

        let card = operation.card().clone();
        let read = nfc_type4::read_tag(operation.reader_mut(), &card, cancel)?;
        if read.cc.raw() != NDEF_CAPABILITY_CONTAINER.as_slice() {
            return Err(NfcError::VerificationFailed(format!(
                "the NDEF capability container read back as {:02X?}",
                read.cc.raw()
            )));
        }
        if read.ndef.state != NdefState::Empty {
            return Err(NfcError::VerificationFailed(
                "the NDEF file is not empty after formatting".to_owned(),
            ));
        }
        operation.check_iso_dep_card()?;
        Ok(DesfireNdefFormatResult {
            cc: read.cc,
            ndef: read.ndef,
        })
    })();

    match outcome {
        Ok(result) => Ok(result),
        Err(error) if created => {
            match remove_ndef_application(operation, key, key_type, key_number) {
                Ok(()) => Err(error),
                Err(rollback) => Err(NfcError::VerificationFailed(format!(
                    "NDEF formatting failed: {error}; rollback failed: {rollback}"
                ))),
            }
        }
        Err(error) => Err(error),
    }
}

fn delete_ndef(
    operation: &mut Operation<'_>,
    key: &[u8],
    key_type: Option<AuthKeyType>,
    key_number: u8,
    cancel: &CancelToken,
) -> Result<DesfireNdefDeleteResult> {
    ensure_iso_dep(operation)?;
    cancel.check()?;
    let inventory = {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.inventory()?
    };
    if !inventory
        .applications
        .iter()
        .any(|application| application.aid == NDEF_APPLICATION_AID)
    {
        return Err(NfcError::InvalidArgument(
            "the NDEF application D27600 is not present".to_owned(),
        ));
    }
    validate_ndef_layout(operation, cancel)?;
    cancel.check()?;
    {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application([0x00; 3])?;
        authenticate_key(&mut desfire, key_number, key, key_type)?;
        desfire.delete_application(NDEF_APPLICATION_AID)?;
    }
    let applications = {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.application_ids()?
    };
    if applications.contains(&NDEF_APPLICATION_AID) {
        return Err(NfcError::VerificationFailed(
            "the NDEF application is still present after deletion".to_owned(),
        ));
    }
    operation.check_iso_dep_card()?;
    Ok(DesfireNdefDeleteResult { applications })
}

/// Effective protection for a read operation.
///
/// When the operation is granted by a free access right, the PICC runs it in
/// plain whatever the file communication mode says. A free GetValue option
/// does the same for GetValue. A key-based right held by the authenticated key
/// keeps the file mode.
fn effective_read_communication(
    settings: &FileSettings,
    key_number: u8,
    authenticated: bool,
) -> CommunicationMode {
    if let FileContents::Value {
        free_get_value: true,
        ..
    } = settings.contents
    {
        return CommunicationMode::Plain;
    }
    let key_matches = |right: AccessRight| {
        authenticated && matches!(right, AccessRight::Key(key) if key == key_number)
    };
    if key_matches(settings.access.read) || key_matches(settings.access.read_write) {
        return settings.communication;
    }
    if settings.access.read == AccessRight::Free || settings.access.read_write == AccessRight::Free
    {
        return CommunicationMode::Plain;
    }
    settings.communication
}

fn desfire_key_grant(right: AccessRight, key_number: u8, authenticated: bool) -> bool {
    authenticated && matches!(right, AccessRight::Key(key) if key == key_number)
}

/// Effective communication mode for a write.
///
/// The write right or the read/write right grants the operation. A matching
/// key right applies the file's mode; free access runs in Plain mode whatever
/// the file settings say.
fn desfire_write_communication(
    settings: &FileSettings,
    key_number: u8,
    authenticated: bool,
    file_id: u8,
) -> Result<CommunicationMode> {
    if desfire_key_grant(settings.access.write, key_number, authenticated)
        || desfire_key_grant(settings.access.read_write, key_number, authenticated)
    {
        return Ok(settings.communication);
    }
    if settings.access.write == AccessRight::Free || settings.access.read_write == AccessRight::Free
    {
        return Ok(CommunicationMode::Plain);
    }
    if !authenticated
        && [settings.access.write, settings.access.read_write]
            .into_iter()
            .any(|right| matches!(right, AccessRight::Key(_)))
    {
        return Err(NfcError::AuthenticationRequired(format!(
            "file {file_id:02X} needs a key for writes"
        )));
    }
    Err(NfcError::InvalidArgument(format!(
        "file {file_id:02X} denies write access to this key"
    )))
}

fn desfire_read_granted(settings: &FileSettings, key_number: u8, authenticated: bool) -> bool {
    desfire_key_grant(settings.access.read, key_number, authenticated)
        || desfire_key_grant(settings.access.read_write, key_number, authenticated)
        || settings.access.read == AccessRight::Free
        || settings.access.read_write == AccessRight::Free
}

/// One-line result used by `nfctl` and the GTK write view.
pub fn write_text(result: &DesfireWriteResult) -> String {
    let protection = match result.key_type {
        Some(key_type) => format!("{} · {}", key_type.name(), result.integrity.label()),
        None => result.integrity.label().to_owned(),
    };
    let end = u64::from(result.offset) + result.written as u64;
    format!(
        "settings: {}\nprotection: {}\nwritten: {}..{} ({} bytes)\nbackup: {} bytes\ncommit: {}\nread-back: {} bytes verified",
        result.settings.summary(),
        protection,
        result.offset,
        end,
        result.written,
        result.backup.len(),
        if result.committed {
            "committed"
        } else {
            "not needed"
        },
        result.read_back.len()
    )
}

fn verify_record_bytes(actual: usize, count: u32, record_size: u32) -> Result<()> {
    let expected = u64::from(count) * u64::from(record_size);
    if actual as u64 == expected {
        Ok(())
    } else {
        Err(NfcError::VerificationFailed(format!(
            "ReadRecords returned {actual} bytes, expected {expected}"
        )))
    }
}

fn validate_desfire_key(key: &[u8], key_type: Option<AuthKeyType>) -> Result<AuthKeyType> {
    match key_type {
        Some(key_type) => {
            key_type.validate_key(key)?;
            Ok(key_type)
        }
        None => AuthKeyType::from_key(key),
    }
}

fn authenticate_key(
    desfire: &mut Desfire<'_>,
    key_number: u8,
    key: &[u8],
    key_type: Option<AuthKeyType>,
) -> Result<()> {
    match key_type {
        Some(key_type) => desfire.authenticate(key_number, key, key_type),
        None => desfire.authenticate_legacy(key_number, key),
    }
}

fn ensure_iso_dep(operation: &Operation<'_>) -> Result<()> {
    if operation.card().card_type != CardType::Iso14443 {
        return Err(NfcError::Unsupported(
            "DESFire operations need an ISO/IEC 14443-4 card",
        ));
    }
    Ok(())
}

fn ensure_ndef_absent(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<()> {
    cancel.check()?;
    let inventory = {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.inventory()?
    };
    if inventory
        .applications
        .iter()
        .any(|application| application.aid == NDEF_APPLICATION_AID)
    {
        return Err(NfcError::InvalidArgument(
            "the NDEF application D27600 already exists; delete it before formatting".to_owned(),
        ));
    }
    cancel.check()?;
    if ndef_application_selected(operation)? {
        return Err(NfcError::InvalidArgument(
            "a valid NDEF Tag Application already exists on this card".to_owned(),
        ));
    }
    Ok(())
}

fn ndef_application_selected(operation: &mut Operation<'_>) -> Result<bool> {
    let response = apdu::transceive(
        operation.reader_mut(),
        &apdu::select_tag_application(),
        "selecting the NDEF tag application",
    )?;
    match response.sw {
        [0x90, 0x00] => Ok(true),
        [0x6A, 0x82] => Ok(false),
        _ => {
            let _ = apdu::check_ok(&response, "selecting the NDEF tag application")?;
            Ok(true)
        }
    }
}

fn validate_ndef_layout(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<()> {
    cancel.check()?;
    let mut desfire = Desfire::new(operation.reader_mut());
    desfire.select_application(NDEF_APPLICATION_AID)?;
    if desfire.file_ids()? != [NDEF_CC_FILE, NDEF_DATA_FILE] {
        return Err(NfcError::Unsupported(
            "the NDEF application contains files that this product did not create",
        ));
    }
    let cc_settings = desfire.file_settings(NDEF_CC_FILE)?;
    let ndef_settings = desfire.file_settings(NDEF_DATA_FILE)?;
    if !file_layout_matches(&cc_settings, [0x00, 0xE0], NDEF_CC_SIZE)
        || !file_layout_matches(&ndef_settings, [0xE0, 0xEE], NDEF_DATA_SIZE)
    {
        return Err(NfcError::Unsupported(
            "the NDEF application files do not match the AN11004 layout",
        ));
    }
    let cc = desfire.read_data(
        NDEF_CC_FILE,
        0,
        NDEF_CC_SIZE,
        nfc_desfire::CommunicationMode::Plain,
    )?;
    if cc.data.as_slice() != NDEF_CAPABILITY_CONTAINER.as_slice() {
        return Err(NfcError::Unsupported(
            "the NDEF capability container was not created by this product",
        ));
    }
    cancel.check()?;
    Ok(())
}

fn file_layout_matches(settings: &[u8], access: [u8; 2], size: u32) -> bool {
    settings.len() >= 7
        && settings[1] == 0
        && settings[2] == access[0]
        && settings[3] == access[1]
        && Desfire::data_file_size(settings) == Some(size)
}

fn remove_ndef_application(
    operation: &mut Operation<'_>,
    key: &[u8],
    key_type: Option<AuthKeyType>,
    key_number: u8,
) -> Result<()> {
    let mut desfire = Desfire::new(operation.reader_mut());
    let _ = desfire.abort_transaction();
    desfire.select_application([0x00; 3])?;
    authenticate_key(&mut desfire, key_number, key, key_type)?;
    desfire.delete_application(NDEF_APPLICATION_AID)?;
    if desfire.application_ids()?.contains(&NDEF_APPLICATION_AID) {
        return Err(NfcError::VerificationFailed(
            "the NDEF application is still present after rollback".to_owned(),
        ));
    }
    Ok(())
}

/// Human-readable inventory shared by `nfctl` and the GTK DESFire workspace.
///
/// A file whose settings cannot be decoded is reported as `undecoded` with the
/// parse error; its raw settings are always printed.
pub fn inventory_text(inventory: &Inventory) -> String {
    let mut text = format!(
        "{} · vendor {:02X} type {:02X} subtype {:02X} version {}.{} storage {:02X} protocol {:02X}\n",
        inventory.version.name(),
        inventory.version.vendor_id,
        inventory.version.product_type,
        inventory.version.product_subtype,
        inventory.version.major,
        inventory.version.minor,
        inventory.version.storage_size,
        inventory.version.protocol
    );
    if inventory.applications.is_empty() {
        text.push_str("applications: none\n");
    }
    for application in &inventory.applications {
        text.push_str(&format!("application {}\n", hex_bytes(&application.aid)));
        if application.files.is_empty() {
            text.push_str("  files: none or directory access denied\n");
        }
        for file in &application.files {
            match file.decoded() {
                Ok(settings) => text.push_str(&format!(
                    "  file {:02X}: {}\n",
                    file.file_id,
                    settings.summary()
                )),
                Err(error) => text.push_str(&format!(
                    "  file {:02X}: undecoded · {error}\n",
                    file.file_id
                )),
            }
            text.push_str(&format!("    raw: {}\n", hex_bytes(&file.settings)));
        }
    }
    text
}

/// Human-readable authentication result shared by `nfctl` and the GTK
/// DESFire workspace.
pub fn key_verification_text(result: &DesfireKeyVerification) -> String {
    let session = if result.secure_messaging {
        "session: established · CMAC secure messaging"
    } else {
        "session: established · legacy D40 channel"
    };
    let version = match result.key_version {
        Some(version) => format!("key version: {version}"),
        None => "key version: unavailable".to_owned(),
    };
    format!(
        "aid {} · key {} · {}\n{session}\n{version}\n",
        hex_bytes(&result.aid),
        result.key_number,
        result.key_type.name()
    )
}

/// Human-readable authenticated UID result.
pub fn card_uid_text(result: &DesfireUid) -> String {
    format!(
        "aid {} · key {} · {}\nuid: {}\nresponse: {}\n",
        hex_bytes(&result.aid),
        result.key_number,
        result.key_type.name(),
        hex_bytes(&result.uid),
        result.integrity.label()
    )
}

/// Effective read protection and the response check that passed.
pub fn read_protection_text(protection: &DesfireReadProtection) -> String {
    format!(
        "{} · {} · {}",
        protection.communication.name(),
        protection.key_type.map_or("no session", AuthKeyType::name),
        protection.integrity.label()
    )
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_requires_advanced_and_confirmation_before_reader_selection() {
        let mut service = Service::default();
        let error = service
            .desfire_ndef_format(DesfireNdefFormatRequest::new(vec![0x00; 8]))
            .expect_err("Advanced mode is required");
        assert!(error.to_string().contains("Advanced mode"), "{error}");

        let error = service
            .desfire_ndef_format(DesfireNdefFormatRequest::new(vec![0x00; 8]).advanced(true))
            .expect_err("confirmation is required");
        assert!(error.to_string().contains("confirmation"), "{error}");
    }

    #[test]
    fn delete_requires_advanced_and_confirmation_before_reader_selection() {
        let mut service = Service::default();
        let error = service
            .desfire_ndef_delete(DesfireNdefDeleteRequest::new(vec![0x00; 8]))
            .expect_err("Advanced mode is required");
        assert!(error.to_string().contains("Advanced mode"), "{error}");

        let error = service
            .desfire_ndef_delete(DesfireNdefDeleteRequest::new(vec![0x00; 8]).advanced(true))
            .expect_err("confirmation is required");
        assert!(error.to_string().contains("confirmation"), "{error}");
    }

    #[test]
    fn inventory_text_decodes_known_files_and_marks_unknown_types() {
        let inventory = Inventory {
            version: Version {
                vendor_id: 0x04,
                product_type: 0x01,
                product_subtype: 0x01,
                major: 0x33,
                minor: 0x00,
                storage_size: 0x1A,
                protocol: 0x05,
            },
            applications: vec![Application {
                aid: [0x00, 0x76, 0xD2],
                files: vec![
                    FileInfo {
                        file_id: 0x01,
                        settings: vec![0x00, 0x00, 0x00, 0xE0, 0x0F, 0x00, 0x00],
                    },
                    FileInfo {
                        file_id: 0x02,
                        settings: vec![0x06, 0x00, 0x00, 0xE0],
                    },
                ],
            }],
        };
        let text = inventory_text(&inventory);
        assert!(text.contains("MIFARE DESFire EV3"));
        assert!(text.contains("application 00 76 D2"));
        assert!(text.contains("file 01: standard data · plain"));
        assert!(text.contains("raw: 00 00 00 E0 0F 00 00"));
        assert!(text.contains("file 02: undecoded"));
        assert!(text.contains("raw: 06 00 00 E0"));
        assert!(!text.contains("file 02: standard data"));
    }

    #[test]
    fn free_access_and_free_get_value_read_in_plain() {
        let free_read = FileSettings::parse(&[0x00, 0x03, 0xE0, 0xE0, 0x10, 0x00, 0x00])
            .expect("standard settings");
        assert_eq!(free_read.communication, CommunicationMode::Enciphered);
        assert_eq!(
            effective_read_communication(&free_read, 0, false),
            CommunicationMode::Plain
        );
        assert_eq!(
            effective_read_communication(&free_read, 0, true),
            CommunicationMode::Plain
        );

        let key_read = FileSettings::parse(&[0x00, 0x03, 0x00, 0x00, 0x10, 0x00, 0x00])
            .expect("standard settings");
        assert_eq!(
            effective_read_communication(&key_read, 0, true),
            CommunicationMode::Enciphered
        );
        assert_eq!(
            effective_read_communication(&key_read, 1, true),
            CommunicationMode::Enciphered
        );

        let free_value = FileSettings::parse(&[
            0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x07, 0x00,
            0x00, 0x00, 0x02,
        ])
        .expect("value settings");
        assert_eq!(free_value.communication, CommunicationMode::Maced);
        assert_eq!(
            effective_read_communication(&free_value, 0, true),
            CommunicationMode::Plain
        );
    }

    #[test]
    fn verification_text_names_the_protocol_facts() {
        let verified = DesfireKeyVerification {
            aid: [0xF0, 0x0D, 0x01],
            key_number: 2,
            key_type: AuthKeyType::AesEv2,
            secure_messaging: true,
            key_version: Some(4),
        };
        let text = key_verification_text(&verified);
        assert!(text.contains("aid F0 0D 01 · key 2 · AES EV2"), "{text}");
        assert!(text.contains("CMAC secure messaging"), "{text}");
        assert!(text.contains("key version: 4"), "{text}");

        let unknown = DesfireKeyVerification {
            key_version: None,
            ..verified
        };
        assert!(
            key_verification_text(&unknown).contains("key version: unavailable"),
            "a refused GetKeyVersion is reported, not guessed"
        );

        let uid = DesfireUid {
            aid: [0xF0, 0x0D, 0x01],
            key_number: 0,
            key_type: AuthKeyType::Aes,
            uid: [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26],
            integrity: IntegrityCheck::Ev1Crc32,
        };
        let text = card_uid_text(&uid);
        assert!(text.contains("uid: 04 21 22 23 24 25 26"), "{text}");
        assert!(text.contains("response: EV1 CRC32 verified"), "{text}");

        assert_eq!(
            read_protection_text(&DesfireReadProtection {
                communication: CommunicationMode::Enciphered,
                key_type: Some(AuthKeyType::AesEv2),
                integrity: IntegrityCheck::Ev2Cmac,
            }),
            "enciphered · AES EV2 · EV2 CMAC verified"
        );
        assert_eq!(
            read_protection_text(&DesfireReadProtection {
                communication: CommunicationMode::Plain,
                key_type: None,
                integrity: IntegrityCheck::None,
            }),
            "plain · no session · no response MAC"
        );
    }

    #[test]
    fn record_reads_reject_short_or_long_responses() {
        assert!(verify_record_bytes(16, 2, 8).is_ok());
        assert!(verify_record_bytes(8, 2, 8).is_err());
        assert!(verify_record_bytes(24, 2, 8).is_err());
    }

    #[test]
    fn invalid_legacy_key_length_fails_before_any_reader_command() {
        let mut service = Service::default();
        let error = service
            .desfire_ndef_format(
                DesfireNdefFormatRequest::new(vec![0x00; 7])
                    .advanced(true)
                    .confirm(true),
            )
            .expect_err("a 7-byte legacy key is invalid");
        assert!(error.to_string().contains("8-byte DES"), "{error}");
    }
}
