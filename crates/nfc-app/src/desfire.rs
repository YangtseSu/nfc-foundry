//! DESFire inventory, verification, authenticated read, and NDEF provisioning
//! workflows.

use nfc_core::{CancelToken, CardType, NfcError, Result};
use nfc_desfire::Desfire;
pub use nfc_desfire::{
    AccessRight, AccessRights, Application, ApplicationKeyType, ApplicationSpec, AuthKeyType,
    CommunicationMode, FileContents, FileInfo, FileSettings, FileType, IntegrityCheck, Inventory,
    NDEF_APPLICATION_AID, NDEF_APPLICATION_ISO_FILE, NDEF_APPLICATION_KEY,
    NDEF_APPLICATION_KEY_SETTINGS, NDEF_CAPABILITY_CONTAINER, NDEF_CC_ACCESS_RIGHTS, NDEF_CC_FILE,
    NDEF_CC_ISO_FILE, NDEF_CC_SIZE, NDEF_DATA_ACCESS_RIGHTS, NDEF_DATA_FILE, NDEF_DATA_ISO_FILE,
    NDEF_DATA_SIZE, NDEF_DF_NAME, RecordFileSpec, TransactionMacFileSpec, ValueFileSpec, Version,
};
use nfc_type4::apdu;
use nfc_type4::{NdefState, Type4CapabilityContainer, Type4Ndef};
use serde::Serialize;

use crate::Service;
use crate::fsutil;
use crate::operation::{Operation, OperationKind};

/// Largest plaintext chunk sent in one record command. Keeps header and
/// secure-messaging overhead inside the 255-byte APDU payload.
const MAX_RECORD_CHUNK: usize = 200;

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

/// A value-file change after selecting an application.
///
/// The key is optional so a free-access value file can be changed without
/// authentication. `tmac_key` is the separate AppTransactionMACKey used only
/// to verify the committed Transaction MAC; it stays out of `Debug`.
pub struct DesfireValueRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub operation: DesfireValueOperation,
    pub amount: u32,
    pub key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub tmac_key: Option<[u8; 16]>,
    pub cancel: CancelToken,
}

/// Credit, debit, or limited-credit a value file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesfireValueOperation {
    Credit,
    Debit,
    LimitedCredit,
}

impl DesfireValueOperation {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Credit => "credit",
            Self::Debit => "debit",
            Self::LimitedCredit => "limited credit",
        }
    }

    fn adjustment(self) -> nfc_desfire::ValueAdjustment {
        match self {
            Self::Credit => nfc_desfire::ValueAdjustment::Credit,
            Self::Debit => nfc_desfire::ValueAdjustment::Debit,
            Self::LimitedCredit => nfc_desfire::ValueAdjustment::LimitedCredit,
        }
    }
}

impl DesfireValueRequest {
    pub fn new(aid: [u8; 3], file_id: u8, operation: DesfireValueOperation, amount: u32) -> Self {
        Self {
            aid,
            file_id,
            operation,
            amount,
            key: None,
            key_type: None,
            key_number: 0,
            tmac_key: None,
            cancel: CancelToken::new(),
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

    pub fn tmac_key(mut self, key: [u8; 16]) -> Self {
        self.tmac_key = Some(key);
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// One committed value change with the old and new value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireValueResult {
    pub settings: FileSettings,
    pub operation: DesfireValueOperation,
    pub amount: u32,
    pub old_value: i32,
    pub new_value: i32,
    pub communication: CommunicationMode,
    pub key_type: Option<AuthKeyType>,
    pub integrity: IntegrityCheck,
    pub committed: bool,
    pub transaction_mac: Option<DesfireTransactionMacResult>,
}

/// A record-file mutation after selecting an application.
///
/// The key is optional so a free-access file can be changed without
/// authentication. `tmac_key` is used only to verify the committed Transaction
/// MAC and stays out of `Debug`.
pub struct DesfireRecordRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub operation: DesfireRecordOperation,
    pub data: Vec<u8>,
    pub key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub tmac_key: Option<[u8; 16]>,
    pub cancel: CancelToken,
}

/// The record mutation to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesfireRecordOperation {
    /// Write one complete record after the last one.
    Append,
    /// Replace a byte range of an existing record; 0 is the newest record.
    Update { record_number: u32, offset: u32 },
    /// Erase every record.
    Clear,
}

impl DesfireRecordOperation {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Update { .. } => "update",
            Self::Clear => "clear",
        }
    }
}

impl DesfireRecordRequest {
    pub fn new(aid: [u8; 3], file_id: u8, operation: DesfireRecordOperation) -> Self {
        Self {
            aid,
            file_id,
            operation,
            data: Vec::new(),
            key: None,
            key_type: None,
            key_number: 0,
            tmac_key: None,
            cancel: CancelToken::new(),
        }
    }

    pub fn data(mut self, data: Vec<u8>) -> Self {
        self.data = data;
        self
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

    pub fn tmac_key(mut self, key: [u8; 16]) -> Self {
        self.tmac_key = Some(key);
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// One committed record mutation with the verified before and after contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireRecordResult {
    pub settings_before: FileSettings,
    pub settings_after: FileSettings,
    pub operation: DesfireRecordOperation,
    /// Data written by an append or update.
    pub written: Vec<u8>,
    /// Records in chronological order before the mutation.
    pub before: Vec<Vec<u8>>,
    /// Records in chronological order after the commit and read-back.
    pub after: Vec<Vec<u8>>,
    pub record_size: u32,
    pub communication: CommunicationMode,
    pub key_type: Option<AuthKeyType>,
    pub integrity: IntegrityCheck,
    pub committed: bool,
    pub transaction_mac: Option<DesfireTransactionMacResult>,
}

/// Transaction MAC returned by CommitTransaction and cross-checked with the
/// Transaction MAC file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireTransactionMacResult {
    pub file_id: u8,
    pub counter: u32,
    pub value: [u8; 8],
    /// True only after the value was recomputed with the supplied
    /// AppTransactionMACKey and matched the card response and stored file.
    pub verified: bool,
    pub detail: String,
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

/// One file creation request for a selected application.
///
/// The variant selects the file type; `communication` is the raw DESFire
/// mode (0 plain, 1 MACed, 3 enciphered). Transaction MAC key material is
/// redacted from `Debug`.
pub enum DesfireFileSpec {
    Standard {
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        size: u32,
    },
    Backup {
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        size: u32,
    },
    Value {
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        lower_limit: i32,
        upper_limit: i32,
        value: i32,
        flags: u8,
    },
    LinearRecord {
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        record_size: u32,
        max_records: u32,
    },
    CyclicRecord {
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        record_size: u32,
        max_records: u32,
    },
    TransactionMac {
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        key: [u8; 16],
        key_version: u8,
    },
}

impl std::fmt::Debug for DesfireFileSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("DesfireFileSpec");
        debug
            .field("file_id", &self.file_id())
            .field("file_type", &self.file_type().name())
            .field("communication", &self.communication())
            .field(
                "access_rights",
                &format_args!("{:04X}", self.access_rights()),
            );
        if let Self::TransactionMac { key_version, .. } = self {
            debug.field("key", &"<redacted>");
            debug.field("key_version", key_version);
        }
        debug.finish()
    }
}

impl DesfireFileSpec {
    pub const fn file_id(&self) -> u8 {
        match self {
            Self::Standard { file_id, .. }
            | Self::Backup { file_id, .. }
            | Self::Value { file_id, .. }
            | Self::LinearRecord { file_id, .. }
            | Self::CyclicRecord { file_id, .. }
            | Self::TransactionMac { file_id, .. } => *file_id,
        }
    }

    pub const fn file_type(&self) -> FileType {
        match self {
            Self::Standard { .. } => FileType::Standard,
            Self::Backup { .. } => FileType::Backup,
            Self::Value { .. } => FileType::Value,
            Self::LinearRecord { .. } => FileType::LinearRecord,
            Self::CyclicRecord { .. } => FileType::CyclicRecord,
            Self::TransactionMac { .. } => FileType::TransactionMac,
        }
    }

    pub const fn communication(&self) -> u8 {
        match self {
            Self::Standard { communication, .. }
            | Self::Backup { communication, .. }
            | Self::Value { communication, .. }
            | Self::LinearRecord { communication, .. }
            | Self::CyclicRecord { communication, .. }
            | Self::TransactionMac { communication, .. } => *communication,
        }
    }

    pub const fn access_rights(&self) -> u16 {
        match self {
            Self::Standard { access_rights, .. }
            | Self::Backup { access_rights, .. }
            | Self::Value { access_rights, .. }
            | Self::LinearRecord { access_rights, .. }
            | Self::CyclicRecord { access_rights, .. }
            | Self::TransactionMac { access_rights, .. } => *access_rights,
        }
    }

    /// Bytes of file content the card must have free before creation.
    ///
    /// This is a lower bound: the card's own per-file overhead is not
    /// modelled, so a value at or below the free count may still be refused
    /// by the card.
    pub fn required_bytes(&self) -> u64 {
        match self {
            Self::Standard { size, .. } | Self::Backup { size, .. } => u64::from(*size),
            Self::Value { .. } => 13,
            Self::LinearRecord {
                record_size,
                max_records,
                ..
            }
            | Self::CyclicRecord {
                record_size,
                max_records,
                ..
            } => u64::from(*record_size) * u64::from(*max_records),
            Self::TransactionMac { .. } => 18,
        }
    }

    fn transaction_mac_spec(&self) -> Option<TransactionMacFileSpec> {
        match self {
            Self::TransactionMac {
                file_id,
                iso_file_id,
                communication,
                access_rights,
                key,
                key_version,
            } => Some(TransactionMacFileSpec {
                file_id: *file_id,
                iso_file_id: *iso_file_id,
                communication: *communication,
                access_rights: *access_rights,
                key: *key,
                key_version: *key_version,
            }),
            _ => None,
        }
    }
}

/// Read the free EEPROM bytes of the PICC or one application.
///
/// The key is session-scoped and deliberately not part of `Debug`.
#[derive(Default)]
pub struct DesfireFreeMemoryRequest {
    pub aid: Option<[u8; 3]>,
    key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
}

impl DesfireFreeMemoryRequest {
    pub fn new(aid: Option<[u8; 3]>) -> Self {
        Self {
            aid,
            ..Self::default()
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
}

/// Create an application at the PICC level.
///
/// The key is the PICC master key and deliberately stays out of `Debug`.
pub struct DesfireAppCreateRequest {
    pub aid: [u8; 3],
    key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub keys: u8,
    pub app_key_type: ApplicationKeyType,
    pub cancel: CancelToken,
}

impl DesfireAppCreateRequest {
    pub fn new(aid: [u8; 3]) -> Self {
        Self {
            aid,
            key: None,
            key_type: None,
            key_number: 0,
            keys: 1,
            app_key_type: ApplicationKeyType::Tdes2,
            cancel: CancelToken::new(),
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

    pub fn keys(mut self, keys: u8) -> Self {
        self.keys = keys;
        self
    }

    pub fn app_key_type(mut self, key_type: ApplicationKeyType) -> Self {
        self.app_key_type = key_type;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of creating an application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireAppCreateResult {
    pub aid: [u8; 3],
    pub keys: u8,
    pub app_key_type: ApplicationKeyType,
    pub free_memory_before: u32,
    pub free_memory_after: u32,
    pub applications: Vec<[u8; 3]>,
}

/// Delete an application after exporting its manifest.
///
/// The key is the PICC master key and deliberately stays out of `Debug`.
pub struct DesfireAppDeleteRequest {
    pub aid: [u8; 3],
    key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub advanced: bool,
    pub confirm: bool,
    /// Explicit manifest destination; the default backups directory is used
    /// when absent.
    pub manifest_path: Option<std::path::PathBuf>,
    pub cancel: CancelToken,
}

impl DesfireAppDeleteRequest {
    pub fn new(aid: [u8; 3]) -> Self {
        Self {
            aid,
            key: None,
            key_type: None,
            key_number: 0,
            advanced: false,
            confirm: false,
            manifest_path: None,
            cancel: CancelToken::new(),
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

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn manifest_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.manifest_path = Some(path.into());
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of deleting an application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireAppDeleteResult {
    pub aid: [u8; 3],
    pub manifest_path: std::path::PathBuf,
    pub manifest: String,
    pub free_memory_before: Option<u32>,
    pub free_memory_after: Option<u32>,
    pub applications: Vec<[u8; 3]>,
}

/// Create one file in an application.
///
/// The key authenticates the application and deliberately stays out of
/// `Debug`.
pub struct DesfireFileCreateRequest {
    pub aid: [u8; 3],
    pub spec: DesfireFileSpec,
    key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub cancel: CancelToken,
}

impl DesfireFileCreateRequest {
    pub fn new(aid: [u8; 3], spec: DesfireFileSpec) -> Self {
        Self {
            aid,
            spec,
            key: None,
            key_type: None,
            key_number: 0,
            cancel: CancelToken::new(),
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

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of creating a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireFileCreateResult {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub settings: FileSettings,
    pub free_memory_before: u32,
    pub free_memory_after: u32,
    pub required: u64,
}

/// Delete one file after exporting its manifest.
///
/// The key authenticates the application and deliberately stays out of
/// `Debug`.
pub struct DesfireFileDeleteRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub advanced: bool,
    pub confirm: bool,
    pub manifest_path: Option<std::path::PathBuf>,
    pub cancel: CancelToken,
}

impl DesfireFileDeleteRequest {
    pub fn new(aid: [u8; 3], file_id: u8) -> Self {
        Self {
            aid,
            file_id,
            key: None,
            key_type: None,
            key_number: 0,
            advanced: false,
            confirm: false,
            manifest_path: None,
            cancel: CancelToken::new(),
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

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn manifest_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.manifest_path = Some(path.into());
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of deleting a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireFileDeleteResult {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub manifest_path: std::path::PathBuf,
    pub manifest: String,
    pub free_memory_before: Option<u32>,
    pub free_memory_after: Option<u32>,
    pub files: Vec<u8>,
}

/// Change a file's communication mode and access rights.
///
/// The key authenticates the application and deliberately stays out of
/// `Debug`.
pub struct DesfireFileSettingsRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    /// New mode; `None` keeps the current mode.
    pub communication: Option<CommunicationMode>,
    /// New primary access word; `None` keeps the current rights.
    pub access_rights: Option<u16>,
    key: Option<Vec<u8>>,
    pub key_type: Option<AuthKeyType>,
    pub key_number: u8,
    pub cancel: CancelToken,
}

impl DesfireFileSettingsRequest {
    pub fn new(aid: [u8; 3], file_id: u8) -> Self {
        Self {
            aid,
            file_id,
            communication: None,
            access_rights: None,
            key: None,
            key_type: None,
            key_number: 0,
            cancel: CancelToken::new(),
        }
    }

    pub fn communication(mut self, communication: CommunicationMode) -> Self {
        self.communication = Some(communication);
        self
    }

    pub fn access_rights(mut self, access_rights: u16) -> Self {
        self.access_rights = Some(access_rights);
        self
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

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of changing file settings and reading them back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireFileSettingsResult {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub before: FileSettings,
    pub after: FileSettings,
    /// False when the requested values already matched the card.
    pub changed: bool,
}

#[derive(Serialize)]
struct DesfireManifest {
    schema: &'static str,
    generated_unix: u64,
    card: ManifestCard,
    free_memory: Option<u32>,
    application: ManifestApplication,
    note: &'static str,
}

#[derive(Serialize)]
struct ManifestCard {
    name: String,
    vendor_id: String,
    product_type: String,
    product_subtype: String,
    major: u8,
    minor: u8,
    storage_size: String,
    protocol: String,
}

#[derive(Serialize)]
struct ManifestApplication {
    aid: String,
    files_available: bool,
    files_error: Option<String>,
    files: Vec<ManifestFile>,
}

#[derive(Serialize)]
struct ManifestFile {
    file_id: String,
    file_type: Option<&'static str>,
    summary: Option<String>,
    raw_settings: String,
    error: Option<String>,
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

        // A Backup file keeps writes pending until CommitTransaction, so its
        // read-back happens after the commit; a Standard file is readable
        // immediately.
        let committed = if backup_file {
            if let Err(error) = desfire.commit_transaction() {
                let _ = desfire.abort_transaction();
                return Err(error);
            }
            true
        } else {
            false
        };
        let read_back = desfire
            .read_data(request.file_id, request.offset, length, read_communication)?
            .data;
        if read_back != request.data {
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

    /// Credit, debit, or limited-credit one value file, commit, and verify the
    /// new value.
    ///
    /// The current value is read first, the amount is checked against the
    /// file limits, the card change stays pending until CommitTransaction, and
    /// the committed value must match the exact expected value. A Transaction
    /// MAC returned by the commit is cross-checked with the card's Transaction
    /// MAC file and, when the AppTransactionMACKey is supplied, recomputed.
    pub fn desfire_value_change(
        &mut self,
        request: DesfireValueRequest,
    ) -> Result<DesfireValueResult> {
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        if request.operation != DesfireValueOperation::LimitedCredit && request.amount == 0 {
            return Err(NfcError::InvalidArgument(
                "a credit or debit amount must be positive".to_owned(),
            ));
        }
        if request.amount > i32::MAX as u32 {
            return Err(NfcError::InvalidArgument(
                "a DESFire value amount is at most 0x7FFFFFFF".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Write, request.cancel)?;
        operation.check_iso_dep_card()?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        let settings = FileSettings::parse(&desfire.file_settings(request.file_id)?)?;
        let FileContents::Value {
            lower_limit,
            upper_limit,
            limited_credit,
            free_get_value,
            ..
        } = &settings.contents
        else {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} is {}; value operations need a Value file",
                request.file_id,
                settings.file_type.name()
            )));
        };
        let (lower_limit, upper_limit, limited_credit, free_get_value) =
            (*lower_limit, *upper_limit, *limited_credit, *free_get_value);
        if request.operation == DesfireValueOperation::LimitedCredit && !limited_credit {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} does not enable LimitedCredit",
                request.file_id
            )));
        }
        let authenticated = request.key.is_some();
        let read_communication =
            value_read_communication(&settings, request.key_number, authenticated, free_get_value);
        if !value_read_granted(&settings, request.key_number, authenticated) {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} cannot be read with this key; the value change is refused because it cannot be verified",
                request.file_id
            )));
        }
        let tmac_file = desfire_transaction_mac_file(&mut desfire)?;
        let uid = match request.tmac_key {
            Some(_) => Some(desfire.card_uid()?.uid),
            None => None,
        };
        let old_value = desfire
            .get_value(request.file_id, read_communication)?
            .value;
        if old_value < lower_limit || old_value > upper_limit {
            return Err(NfcError::VerificationFailed(format!(
                "GetValue returned {old_value}, outside the declared limits {lower_limit}..{upper_limit}"
            )));
        }
        let expected = match request.operation {
            DesfireValueOperation::Credit | DesfireValueOperation::LimitedCredit => {
                i64::from(old_value) + i64::from(request.amount)
            }
            DesfireValueOperation::Debit => i64::from(old_value) - i64::from(request.amount),
        };
        if expected < i64::from(lower_limit) || expected > i64::from(upper_limit) {
            return Err(NfcError::InvalidArgument(format!(
                "the {} of {} would take the value from {old_value} to {expected}, outside the limits {lower_limit}..{upper_limit}",
                request.operation.name(),
                request.amount
            )));
        }
        let expected = i32::try_from(expected).map_err(|_| {
            NfcError::InvalidArgument("the value result does not fit 32 bits".to_owned())
        })?;
        let communication = value_operation_communication(
            &settings,
            request.operation,
            request.key_number,
            authenticated,
        )?;
        let integrity = desfire.adjust_value(
            request.file_id,
            request.operation.adjustment(),
            request.amount,
            communication,
        )?;
        let tmi = desfire.transaction_mac_input().to_vec();
        let returned = if tmac_file.is_some() {
            desfire.commit_transaction_with_mac()?
        } else {
            desfire.commit_transaction()?;
            None
        };
        let transaction_mac = finish_transaction_mac(
            &mut desfire,
            tmac_file,
            request.tmac_key.as_ref(),
            uid,
            returned,
            &tmi,
            request.key_number,
            authenticated,
        )?;
        let new_value = desfire
            .get_value(request.file_id, read_communication)?
            .value;
        if new_value != expected {
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} value read back as {new_value}, expected {expected}",
                request.file_id
            )));
        }
        let settings_after = FileSettings::parse(&desfire.file_settings(request.file_id)?)?;
        let key_type = desfire.session_key_type();
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireValueResult {
            settings: settings_after,
            operation: request.operation,
            amount: request.amount,
            old_value,
            new_value,
            communication,
            key_type,
            integrity,
            committed: true,
            transaction_mac,
        })
    }

    /// Append, update, or clear a record file, commit, and verify every record.
    ///
    /// The complete record list is read before the mutation, the operation is
    /// checked against the record bounds, and the committed list must equal
    /// the expected chronological list exactly. A Transaction MAC returned by
    /// the commit is cross-checked and, with the AppTransactionMACKey, verified.
    pub fn desfire_record_change(
        &mut self,
        request: DesfireRecordRequest,
    ) -> Result<DesfireRecordResult> {
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Write, request.cancel.clone())?;
        operation.check_iso_dep_card()?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        let settings_before = FileSettings::parse(&desfire.file_settings(request.file_id)?)?;
        let FileContents::Record {
            record_size,
            max_records,
            current_records,
        } = &settings_before.contents
        else {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} is {}; record operations need a Linear or Cyclic Record file",
                request.file_id,
                settings_before.file_type.name()
            )));
        };
        let (record_size, max_records, current_records) =
            (*record_size, *max_records, *current_records);
        let authenticated = request.key.is_some();
        let read_communication =
            effective_read_communication(&settings_before, request.key_number, authenticated);
        if !desfire_read_granted(&settings_before, request.key_number, authenticated) {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} cannot be read with this key; the record change is refused because it cannot be verified",
                request.file_id
            )));
        }
        match request.operation {
            DesfireRecordOperation::Append => {
                if request.data.len() as u32 != record_size {
                    return Err(NfcError::InvalidArgument(format!(
                        "an append writes one complete {record_size}-byte record, got {} bytes",
                        request.data.len()
                    )));
                }
                if settings_before.file_type == FileType::LinearRecord
                    && current_records >= max_records
                {
                    return Err(NfcError::InvalidArgument(format!(
                        "linear record file {:02X} is full at {current_records} records",
                        request.file_id
                    )));
                }
            }
            DesfireRecordOperation::Update {
                record_number,
                offset,
            } => {
                if record_number >= current_records {
                    return Err(NfcError::InvalidArgument(format!(
                        "record {record_number} is beyond the {current_records} existing records"
                    )));
                }
                let end = u64::from(offset) + request.data.len() as u64;
                if request.data.is_empty() || offset >= record_size || end > u64::from(record_size)
                {
                    return Err(NfcError::InvalidArgument(format!(
                        "update range {offset}..{end} is outside the {record_size}-byte record"
                    )));
                }
                if request.data.len() > MAX_RECORD_CHUNK {
                    return Err(NfcError::InvalidArgument(format!(
                        "one UpdateRecord writes at most {MAX_RECORD_CHUNK} bytes"
                    )));
                }
            }
            DesfireRecordOperation::Clear => {
                if current_records == 0 {
                    return Err(NfcError::InvalidArgument(format!(
                        "record file {:02X} is already empty",
                        request.file_id
                    )));
                }
            }
        }
        let before = read_all_records(
            &mut desfire,
            request.file_id,
            record_size,
            current_records,
            read_communication,
        )?;
        let communication = record_operation_communication(
            &settings_before,
            request.operation,
            request.key_number,
            authenticated,
        )?;
        // ClearRecordFile is MACed under EV2 and Plain under EV1, regardless of
        // the targeted file's communication mode.
        let effective_communication = if request.operation == DesfireRecordOperation::Clear {
            if desfire.session_key_type() == Some(AuthKeyType::AesEv2) {
                CommunicationMode::Maced
            } else {
                CommunicationMode::Plain
            }
        } else {
            communication
        };
        let tmac_file = desfire_transaction_mac_file(&mut desfire)?;
        let uid = match request.tmac_key {
            Some(_) => Some(desfire.card_uid()?.uid),
            None => None,
        };
        let integrity = match request.operation {
            DesfireRecordOperation::Append => {
                let mut integrity = IntegrityCheck::None;
                for (index, chunk) in request.data.chunks(MAX_RECORD_CHUNK).enumerate() {
                    integrity = desfire.write_record(
                        request.file_id,
                        (index * MAX_RECORD_CHUNK) as u32,
                        chunk,
                        communication,
                    )?;
                }
                integrity
            }
            DesfireRecordOperation::Update {
                record_number,
                offset,
            } => desfire.update_record(
                request.file_id,
                record_number,
                offset,
                &request.data,
                communication,
            )?,
            DesfireRecordOperation::Clear => desfire.clear_record_file(request.file_id)?,
        };
        let tmi = desfire.transaction_mac_input().to_vec();
        let returned = if tmac_file.is_some() {
            desfire.commit_transaction_with_mac()?
        } else {
            desfire.commit_transaction()?;
            None
        };
        let transaction_mac = finish_transaction_mac(
            &mut desfire,
            tmac_file,
            request.tmac_key.as_ref(),
            uid,
            returned,
            &tmi,
            request.key_number,
            authenticated,
        )?;
        let settings_after = FileSettings::parse(&desfire.file_settings(request.file_id)?)?;
        let FileContents::Record {
            record_size: after_size,
            current_records: after_count,
            ..
        } = &settings_after.contents
        else {
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} stopped being a record file",
                request.file_id
            )));
        };
        let (after_size, after_count) = (*after_size, *after_count);
        if after_size != record_size {
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} record size changed from {record_size} to {after_size}",
                request.file_id
            )));
        }
        let after = read_all_records(
            &mut desfire,
            request.file_id,
            record_size,
            after_count,
            read_communication,
        )?;
        let expected = expected_records(
            &before,
            &request,
            record_size,
            max_records,
            settings_before.file_type,
        )?;
        if after != expected {
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} records read back differently after the {}",
                request.file_id,
                request.operation.name()
            )));
        }
        if after_count as usize != after.len() {
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} reports {after_count} current records but read back {}",
                request.file_id,
                after.len()
            )));
        }
        let key_type = desfire.session_key_type();
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireRecordResult {
            settings_before,
            settings_after,
            operation: request.operation,
            written: request.data,
            before,
            after,
            record_size,
            communication: effective_communication,
            key_type,
            integrity,
            committed: true,
            transaction_mac,
        })
    }

    /// Read the free EEPROM bytes of the PICC or one application.
    ///
    /// Authentication is optional; a key is needed when the card's key
    /// settings protect the PICC commands.
    pub fn desfire_free_memory(
        &mut self,
        cancel: CancelToken,
        request: DesfireFreeMemoryRequest,
    ) -> Result<u32> {
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        ensure_iso_dep(&operation)?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid.unwrap_or([0x00; 3]))?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        desfire.free_memory()
    }

    /// Create an application after checking the free memory.
    ///
    /// The new application is read back through the application ID list. The
    /// card owns the application key settings; this product creates no
    /// template application and writes no default file set.
    pub fn desfire_create_application(
        &mut self,
        request: DesfireAppCreateRequest,
    ) -> Result<DesfireAppCreateResult> {
        validate_application_aid(request.aid)?;
        if !(1..=14).contains(&request.keys) {
            return Err(NfcError::InvalidArgument(
                "a DESFire application has 1..=14 keys".to_owned(),
            ));
        }
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Write, request.cancel)?;
        ensure_iso_dep(&operation)?;
        operation.check_iso_dep_card()?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application([0x00; 3])?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        let free_memory_before = desfire.free_memory()?;
        if free_memory_before == 0 {
            return Err(NfcError::InvalidArgument(
                "the card reports no free EEPROM bytes; deleted applications keep their space until FormatPICC"
                    .to_owned(),
            ));
        }
        let applications = desfire.application_ids()?;
        if applications.contains(&request.aid) {
            return Err(NfcError::InvalidArgument(format!(
                "application {} already exists",
                hex_compact(&request.aid)
            )));
        }
        let spec = ApplicationSpec::new(request.aid, request.keys, request.app_key_type);
        desfire.create_application_from(&spec)?;
        let applications = desfire.application_ids()?;
        if !applications.contains(&request.aid) {
            return Err(NfcError::VerificationFailed(format!(
                "application {} is missing after creation",
                hex_compact(&request.aid)
            )));
        }
        let free_memory_after = desfire.free_memory()?;
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireAppCreateResult {
            aid: request.aid,
            keys: request.keys,
            app_key_type: request.app_key_type,
            free_memory_before,
            free_memory_after,
            applications,
        })
    }

    /// Delete an application after exporting its manifest.
    ///
    /// The manifest is written before the first destructive command. A
    /// deletion is reported successful only when the application ID list no
    /// longer contains the application. This is not a byte-for-byte restore.
    pub fn desfire_delete_application(
        &mut self,
        request: DesfireAppDeleteRequest,
    ) -> Result<DesfireAppDeleteResult> {
        if !request.advanced {
            return Err(NfcError::InvalidArgument(
                "deleting a DESFire application requires Advanced mode".to_owned(),
            ));
        }
        if !request.confirm {
            return Err(NfcError::InvalidArgument(
                "deleting a DESFire application needs explicit confirmation".to_owned(),
            ));
        }
        validate_application_aid(request.aid)?;
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel.clone())?;
        ensure_iso_dep(&operation)?;
        operation.check_iso_dep_card()?;
        let free_memory_before = {
            let mut desfire = Desfire::new(operation.reader_mut());
            desfire.select_application([0x00; 3])?;
            if let Some(key) = request.key.as_deref() {
                authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
            }
            desfire.free_memory().ok()
        };
        cancel.check()?;
        let inventory = {
            let mut desfire = Desfire::new(operation.reader_mut());
            desfire.inventory()?
        };
        let Some(application) = inventory
            .applications
            .iter()
            .find(|application| application.aid == request.aid)
        else {
            return Err(NfcError::InvalidArgument(format!(
                "application {} is not present",
                hex_compact(&request.aid)
            )));
        };
        let manifest = desfire_manifest(
            &inventory.version,
            manifest_application(application),
            free_memory_before,
        )?;
        let manifest_path = write_manifest(
            request.manifest_path.as_deref(),
            &format!("desfire-manifest-{}", hex_compact(&request.aid)),
            &manifest,
        )?;
        cancel.check()?;

        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application([0x00; 3])?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        desfire.delete_application(request.aid)?;
        let applications = desfire.application_ids()?;
        if applications.contains(&request.aid) {
            return Err(NfcError::VerificationFailed(format!(
                "application {} is still present after deletion",
                hex_compact(&request.aid)
            )));
        }
        let free_memory_after = desfire.free_memory().ok();
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireAppDeleteResult {
            aid: request.aid,
            manifest_path,
            manifest,
            free_memory_before,
            free_memory_after,
            applications,
        })
    }

    /// Create one file after the capacity preflight and read its settings back.
    ///
    /// The free-space check is a lower bound on the file content; the card's
    /// per-file overhead can still refuse a creation at the limit.
    pub fn desfire_create_file(
        &mut self,
        request: DesfireFileCreateRequest,
    ) -> Result<DesfireFileCreateResult> {
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let file_id = request.spec.file_id();
        let required = request.spec.required_bytes();
        if required == 0 {
            return Err(NfcError::InvalidArgument(
                "a new DESFire file needs at least one byte of content".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Write, request.cancel)?;
        ensure_iso_dep(&operation)?;
        operation.check_iso_dep_card()?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        let files = desfire.file_ids()?;
        if files.contains(&file_id) {
            return Err(NfcError::InvalidArgument(format!(
                "file {file_id:02X} already exists in application {}",
                hex_compact(&request.aid)
            )));
        }
        let free_memory_before = desfire.free_memory()?;
        if required > u64::from(free_memory_before) {
            return Err(NfcError::InvalidArgument(format!(
                "file {file_id:02X} needs {required} content bytes but the card reports {free_memory_before} free"
            )));
        }
        match &request.spec {
            DesfireFileSpec::Standard {
                file_id,
                iso_file_id,
                communication,
                access_rights,
                size,
            } => desfire.create_standard_file(
                *file_id,
                *iso_file_id,
                *communication,
                *access_rights,
                *size,
            )?,
            DesfireFileSpec::Backup {
                file_id,
                iso_file_id,
                communication,
                access_rights,
                size,
            } => desfire.create_backup_file(
                *file_id,
                *iso_file_id,
                *communication,
                *access_rights,
                *size,
            )?,
            DesfireFileSpec::Value {
                file_id,
                iso_file_id,
                communication,
                access_rights,
                lower_limit,
                upper_limit,
                value,
                flags,
            } => desfire.create_value_file(ValueFileSpec {
                file_id: *file_id,
                iso_file_id: *iso_file_id,
                communication: *communication,
                access_rights: *access_rights,
                lower_limit: *lower_limit,
                upper_limit: *upper_limit,
                value: *value,
                flags: *flags,
            })?,
            DesfireFileSpec::LinearRecord {
                file_id,
                iso_file_id,
                communication,
                access_rights,
                record_size,
                max_records,
            }
            | DesfireFileSpec::CyclicRecord {
                file_id,
                iso_file_id,
                communication,
                access_rights,
                record_size,
                max_records,
            } => desfire.create_record_file(RecordFileSpec {
                file_id: *file_id,
                iso_file_id: *iso_file_id,
                communication: *communication,
                access_rights: *access_rights,
                record_size: *record_size,
                max_records: *max_records,
                cyclic: matches!(request.spec, DesfireFileSpec::CyclicRecord { .. }),
            })?,
            DesfireFileSpec::TransactionMac { .. } => desfire.create_transaction_mac_file(
                &request
                    .spec
                    .transaction_mac_spec()
                    .expect("the Transaction MAC variant has a spec"),
            )?,
        }
        let settings = FileSettings::parse(&desfire.file_settings(file_id)?)?;
        verify_created_settings(&request.spec, &settings)?;
        if let DesfireFileSpec::Value { value, .. } = request.spec {
            let authenticated = request.key.is_some();
            let communication =
                effective_read_communication(&settings, request.key_number, authenticated);
            let read = desfire.get_value(file_id, communication)?;
            if read.value != value {
                return Err(NfcError::VerificationFailed(format!(
                    "file {file_id:02X} GetValue returned {}, expected the initial value {value}",
                    read.value
                )));
            }
        }
        let free_memory_after = desfire.free_memory()?;
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireFileCreateResult {
            aid: request.aid,
            file_id,
            settings,
            free_memory_before,
            free_memory_after,
            required,
        })
    }

    /// Delete one file after exporting its manifest.
    ///
    /// The file must be absent from the file list afterwards; a refused or
    /// partially applied command is an error.
    pub fn desfire_delete_file(
        &mut self,
        request: DesfireFileDeleteRequest,
    ) -> Result<DesfireFileDeleteResult> {
        if !request.advanced {
            return Err(NfcError::InvalidArgument(
                "deleting a DESFire file requires Advanced mode".to_owned(),
            ));
        }
        if !request.confirm {
            return Err(NfcError::InvalidArgument(
                "deleting a DESFire file needs explicit confirmation".to_owned(),
            ));
        }
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel.clone())?;
        ensure_iso_dep(&operation)?;
        operation.check_iso_dep_card()?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        let files = desfire.file_ids()?;
        if !files.contains(&request.file_id) {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} is not present in application {}",
                request.file_id,
                hex_compact(&request.aid)
            )));
        }
        let free_memory_before = desfire.free_memory().ok();
        let raw = desfire.file_settings(request.file_id)?;
        let file = manifest_file(request.file_id, &raw);
        let version = desfire.get_version()?;
        let manifest = desfire_manifest(
            &version,
            ManifestApplication {
                aid: hex_compact(&request.aid),
                files_available: true,
                files_error: None,
                files: vec![file],
            },
            free_memory_before,
        )?;
        let manifest_path = write_manifest(
            request.manifest_path.as_deref(),
            &format!(
                "desfire-manifest-{}-{:02X}",
                hex_compact(&request.aid),
                request.file_id
            ),
            &manifest,
        )?;
        cancel.check()?;
        desfire.delete_file(request.file_id)?;
        let files = desfire.file_ids()?;
        if files.contains(&request.file_id) {
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} is still present after deletion",
                request.file_id
            )));
        }
        let free_memory_after = desfire.free_memory().ok();
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireFileDeleteResult {
            aid: request.aid,
            file_id: request.file_id,
            manifest_path,
            manifest,
            free_memory_before,
            free_memory_after,
            files,
        })
    }

    /// Change a file's communication mode and access rights, then read them
    /// back.
    ///
    /// Value, record, and Transaction MAC files accept only access-right
    /// changes; their mode is the one set at creation.
    pub fn desfire_change_file_settings(
        &mut self,
        request: DesfireFileSettingsRequest,
    ) -> Result<DesfireFileSettingsResult> {
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        if request.communication.is_none() && request.access_rights.is_none() {
            return Err(NfcError::InvalidArgument(
                "give a new communication mode, new access rights, or both".to_owned(),
            ));
        }
        let mut operation = self.begin_operation(OperationKind::Write, request.cancel)?;
        ensure_iso_dep(&operation)?;
        operation.check_iso_dep_card()?;
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            authenticate_key(&mut desfire, request.key_number, key, request.key_type)?;
        }
        let before = FileSettings::parse(&desfire.file_settings(request.file_id)?)?;
        let communication = request.communication.unwrap_or(before.communication);
        if communication != before.communication
            && !matches!(before.file_type, FileType::Standard | FileType::Backup)
        {
            return Err(NfcError::InvalidArgument(format!(
                "file {:02X} is {}; only Standard and Backup files can change communication mode",
                request.file_id,
                before.file_type.name()
            )));
        }
        let access_rights = request.access_rights.unwrap_or(before.access.raw);
        let changed = communication != before.communication || access_rights != before.access.raw;
        if changed {
            let protection = match before.access.change {
                AccessRight::Denied => {
                    return Err(NfcError::InvalidArgument(format!(
                        "file {:02X} denies ChangeFileSettings",
                        request.file_id
                    )));
                }
                AccessRight::Free => {
                    if request.key.is_some() {
                        return Err(NfcError::InvalidArgument(format!(
                            "file {:02X} allows free ChangeFileSettings; authenticate with no key",
                            request.file_id
                        )));
                    }
                    CommunicationMode::Plain
                }
                AccessRight::Key(key) => {
                    if request.key.is_none() {
                        return Err(NfcError::AuthenticationRequired(format!(
                            "file {:02X} needs key {key} for ChangeFileSettings",
                            request.file_id
                        )));
                    }
                    if request.key_number != key {
                        return Err(NfcError::InvalidArgument(format!(
                            "file {:02X} needs key {key} for ChangeFileSettings, not key {}",
                            request.file_id, request.key_number
                        )));
                    }
                    CommunicationMode::Enciphered
                }
            };
            desfire.change_file_settings(
                request.file_id,
                communication.raw(),
                access_rights,
                protection,
            )?;
        }
        let after = FileSettings::parse(&desfire.file_settings(request.file_id)?)?;
        if after.communication != communication
            || after.access.raw != access_rights
            || after.file_type != before.file_type
        {
            return Err(NfcError::VerificationFailed(format!(
                "file {:02X} settings did not read back as requested",
                request.file_id
            )));
        }
        drop(desfire);
        operation.check_iso_dep_card()?;
        Ok(DesfireFileSettingsResult {
            aid: request.aid,
            file_id: request.file_id,
            before,
            after,
            changed,
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

fn validate_application_aid(aid: [u8; 3]) -> Result<()> {
    if aid == [0x00; 3] {
        return Err(NfcError::InvalidArgument(
            "the PICC master application cannot be created or deleted".to_owned(),
        ));
    }
    Ok(())
}

/// Uppercase compact hexadecimal, used for application IDs and manifest names.
fn hex_compact(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

fn verify_created_settings(spec: &DesfireFileSpec, settings: &FileSettings) -> Result<()> {
    let communication = CommunicationMode::from_raw(spec.communication())?;
    if settings.file_type != spec.file_type() {
        return Err(NfcError::VerificationFailed(format!(
            "file {:02X} read back as {}, expected {}",
            spec.file_id(),
            settings.file_type.name(),
            spec.file_type().name()
        )));
    }
    if settings.communication != communication {
        return Err(NfcError::VerificationFailed(format!(
            "file {:02X} read back as {}, expected {}",
            spec.file_id(),
            settings.communication.name(),
            communication.name()
        )));
    }
    if settings.access.raw != spec.access_rights() {
        return Err(NfcError::VerificationFailed(format!(
            "file {:02X} access rights read back as {:04X}, expected {:04X}",
            spec.file_id(),
            settings.access.raw,
            spec.access_rights()
        )));
    }
    let matches = match (spec, &settings.contents) {
        (
            DesfireFileSpec::Standard { size, .. } | DesfireFileSpec::Backup { size, .. },
            FileContents::Data { size: actual },
        ) => size == actual,
        (
            DesfireFileSpec::Value {
                lower_limit,
                upper_limit,
                flags,
                ..
            },
            FileContents::Value {
                lower_limit: actual_lower,
                upper_limit: actual_upper,
                limited_credit,
                free_get_value,
                ..
            },
        ) => {
            lower_limit == actual_lower
                && upper_limit == actual_upper
                && (*flags & 0x01 != 0) == *limited_credit
                && (*flags & 0x02 != 0) == *free_get_value
        }
        (
            DesfireFileSpec::LinearRecord {
                record_size,
                max_records,
                ..
            }
            | DesfireFileSpec::CyclicRecord {
                record_size,
                max_records,
                ..
            },
            FileContents::Record {
                record_size: actual_size,
                max_records: actual_max,
                current_records,
            },
        ) => record_size == actual_size && max_records == actual_max && *current_records == 0,
        (
            DesfireFileSpec::TransactionMac { key_version, .. },
            FileContents::TransactionMac {
                key_option,
                key_version: actual_version,
                counter_limit: None,
            },
        ) => key_option & 0x02 != 0 && key_version == actual_version,
        _ => false,
    };
    if !matches {
        return Err(NfcError::VerificationFailed(format!(
            "file {:02X} contents read back differently from the create request",
            spec.file_id()
        )));
    }
    Ok(())
}

fn manifest_file(file_id: u8, raw: &[u8]) -> ManifestFile {
    match FileSettings::parse(raw) {
        Ok(settings) => ManifestFile {
            file_id: format!("{file_id:02X}"),
            file_type: Some(settings.file_type.name()),
            summary: Some(settings.summary()),
            raw_settings: hex_bytes(raw),
            error: None,
        },
        Err(error) => ManifestFile {
            file_id: format!("{file_id:02X}"),
            file_type: None,
            summary: None,
            raw_settings: hex_bytes(raw),
            error: Some(error.to_string()),
        },
    }
}

fn manifest_application(application: &Application) -> ManifestApplication {
    ManifestApplication {
        aid: hex_compact(&application.aid),
        files_available: !application.files.is_empty(),
        files_error: if application.files.is_empty() {
            Some(
                "the file list is empty: the application has no files or directory access is denied"
                    .to_owned(),
            )
        } else {
            None
        },
        files: application
            .files
            .iter()
            .map(|file| manifest_file(file.file_id, &file.settings))
            .collect(),
    }
}

fn desfire_manifest(
    version: &Version,
    application: ManifestApplication,
    free_memory: Option<u32>,
) -> Result<String> {
    let manifest = DesfireManifest {
        schema: "nfc-foundry/desfire-manifest/1",
        generated_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs()),
        card: ManifestCard {
            name: version.name().to_owned(),
            vendor_id: format!("{:02X}", version.vendor_id),
            product_type: format!("{:02X}", version.product_type),
            product_subtype: format!("{:02X}", version.product_subtype),
            major: version.major,
            minor: version.minor,
            storage_size: format!("{:02X}", version.storage_size),
            protocol: format!("{:02X}", version.protocol),
        },
        free_memory,
        application,
        note: "Logical application/file manifest. It records settings and raw settings bytes, not file contents, and cannot restore a deleted application or file.",
    };
    serde_json::to_string_pretty(&manifest)
        .map_err(|error| NfcError::Io(format!("could not encode the DESFire manifest: {error}")))
}

fn write_manifest(
    path: Option<&std::path::Path>,
    label: &str,
    manifest: &str,
) -> Result<std::path::PathBuf> {
    match path {
        Some(path) => {
            fsutil::write_atomic(path, manifest.as_bytes())?;
            Ok(path.to_owned())
        }
        None => {
            let store = crate::classic::BackupStore::default();
            Ok(store.save_bytes(label, manifest.as_bytes())?.path)
        }
    }
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

/// GetValue is granted by Read, Write, or Read/Write, or by the free GetValue
/// option that also forces Plain communication.
fn value_read_granted(settings: &FileSettings, key_number: u8, authenticated: bool) -> bool {
    if matches!(
        settings.contents,
        FileContents::Value {
            free_get_value: true,
            ..
        }
    ) {
        return true;
    }
    desfire_key_grant(settings.access.read, key_number, authenticated)
        || desfire_key_grant(settings.access.write, key_number, authenticated)
        || desfire_key_grant(settings.access.read_write, key_number, authenticated)
        || matches!(settings.access.read, AccessRight::Free)
        || matches!(settings.access.write, AccessRight::Free)
        || matches!(settings.access.read_write, AccessRight::Free)
}

fn value_read_communication(
    settings: &FileSettings,
    key_number: u8,
    authenticated: bool,
    free_get_value: bool,
) -> CommunicationMode {
    if free_get_value {
        return CommunicationMode::Plain;
    }
    if desfire_key_grant(settings.access.read, key_number, authenticated)
        || desfire_key_grant(settings.access.write, key_number, authenticated)
        || desfire_key_grant(settings.access.read_write, key_number, authenticated)
    {
        return settings.communication;
    }
    CommunicationMode::Plain
}

/// Communication mode for a value change.
///
/// Credit is granted by Read/Write only; Debit by Read, Write, or Read/Write;
/// LimitedCredit by Write or Read/Write. A matching key right applies the
/// file's mode, while a free right runs the command in Plain mode.
fn value_operation_communication(
    settings: &FileSettings,
    operation: DesfireValueOperation,
    key_number: u8,
    authenticated: bool,
) -> Result<CommunicationMode> {
    let rights: &[AccessRight] = match operation {
        DesfireValueOperation::Credit => &[settings.access.read_write],
        DesfireValueOperation::Debit => &[
            settings.access.read,
            settings.access.write,
            settings.access.read_write,
        ],
        DesfireValueOperation::LimitedCredit => {
            &[settings.access.write, settings.access.read_write]
        }
    };
    if authenticated
        && rights
            .iter()
            .any(|right| matches!(right, AccessRight::Key(key) if *key == key_number))
    {
        return Ok(settings.communication);
    }
    if rights.contains(&AccessRight::Free) {
        return Ok(CommunicationMode::Plain);
    }
    if !authenticated
        && rights
            .iter()
            .any(|right| matches!(right, AccessRight::Key(_)))
    {
        return Err(NfcError::AuthenticationRequired(
            "this value operation needs an application key".to_owned(),
        ));
    }
    Err(NfcError::InvalidArgument(format!(
        "the file access rights deny {}",
        operation.name()
    )))
}

/// Communication mode for a record mutation.
///
/// WriteRecord and UpdateRecord are granted by Write or Read/Write;
/// ClearRecordFile is granted by Read/Write.
fn record_operation_communication(
    settings: &FileSettings,
    operation: DesfireRecordOperation,
    key_number: u8,
    authenticated: bool,
) -> Result<CommunicationMode> {
    let rights: &[AccessRight] = match operation {
        DesfireRecordOperation::Append | DesfireRecordOperation::Update { .. } => {
            &[settings.access.write, settings.access.read_write]
        }
        DesfireRecordOperation::Clear => &[settings.access.read_write],
    };
    if authenticated
        && rights
            .iter()
            .any(|right| matches!(right, AccessRight::Key(key) if *key == key_number))
    {
        return Ok(settings.communication);
    }
    if rights.contains(&AccessRight::Free) {
        return Ok(CommunicationMode::Plain);
    }
    if !authenticated
        && rights
            .iter()
            .any(|right| matches!(right, AccessRight::Key(_)))
    {
        return Err(NfcError::AuthenticationRequired(
            "this record operation needs an application key".to_owned(),
        ));
    }
    Err(NfcError::InvalidArgument(format!(
        "the file access rights deny record {}",
        operation.name()
    )))
}

fn desfire_transaction_mac_file(desfire: &mut Desfire) -> Result<Option<(u8, FileSettings)>> {
    let mut found = None;
    for file_id in desfire.file_ids()? {
        let settings = FileSettings::parse(&desfire.file_settings(file_id)?)?;
        if settings.file_type == FileType::TransactionMac {
            if found.is_some() {
                return Err(NfcError::VerificationFailed(
                    "the application has more than one Transaction MAC file".to_owned(),
                ));
            }
            found = Some((file_id, settings));
        }
    }
    Ok(found)
}

fn read_all_records(
    desfire: &mut Desfire,
    file_id: u8,
    record_size: u32,
    current_records: u32,
    communication: CommunicationMode,
) -> Result<Vec<Vec<u8>>> {
    if current_records == 0 {
        return Ok(Vec::new());
    }
    let read = desfire.read_records(file_id, 0, current_records, record_size, communication)?;
    let expected = usize::try_from(u64::from(record_size) * u64::from(current_records))
        .map_err(|_| NfcError::InvalidArgument("the record list does not fit memory".to_owned()))?;
    if read.data.len() != expected {
        return Err(NfcError::VerificationFailed(format!(
            "file {file_id:02X} returned {} record bytes, expected {expected}",
            read.data.len()
        )));
    }
    Ok(read
        .data
        .chunks_exact(record_size as usize)
        .map(<[u8]>::to_vec)
        .collect())
}

fn expected_records(
    before: &[Vec<u8>],
    request: &DesfireRecordRequest,
    record_size: u32,
    max_records: u32,
    file_type: FileType,
) -> Result<Vec<Vec<u8>>> {
    match request.operation {
        DesfireRecordOperation::Append => {
            let mut expected = before.to_vec();
            if file_type == FileType::CyclicRecord && expected.len() as u32 >= max_records {
                expected.remove(0);
            }
            expected.push(request.data.clone());
            Ok(expected)
        }
        DesfireRecordOperation::Update {
            record_number,
            offset,
        } => {
            let index = before
                .len()
                .checked_sub(1 + record_number as usize)
                .ok_or_else(|| {
                    NfcError::InvalidArgument(format!(
                        "record {record_number} is beyond the {} records read before the update",
                        before.len()
                    ))
                })?;
            if request.data.len() as u64 + u64::from(offset) > u64::from(record_size) {
                return Err(NfcError::InvalidArgument(format!(
                    "update range {}..{} is outside the {record_size}-byte record",
                    offset,
                    u64::from(offset) + request.data.len() as u64
                )));
            }
            let mut expected = before.to_vec();
            let start = offset as usize;
            let end = start + request.data.len();
            expected[index][start..end].copy_from_slice(&request.data);
            Ok(expected)
        }
        DesfireRecordOperation::Clear => Ok(Vec::new()),
    }
}

/// Cross-check a CommitTransaction MAC with the stored Transaction MAC file
/// and, when the Application Transaction MAC key is supplied, recompute it.
#[allow(clippy::too_many_arguments)]
fn finish_transaction_mac(
    desfire: &mut Desfire,
    tmac_file: Option<(u8, FileSettings)>,
    tmac_key: Option<&[u8; 16]>,
    uid: Option<[u8; 7]>,
    returned: Option<nfc_desfire::TransactionMac>,
    tmi: &[u8],
    key_number: u8,
    authenticated: bool,
) -> Result<Option<DesfireTransactionMacResult>> {
    let Some((file_id, settings)) = tmac_file else {
        return Ok(None);
    };
    let returned = returned.ok_or_else(|| {
        NfcError::VerificationFailed(format!(
            "the card committed without returning the Transaction MAC for file {file_id:02X}"
        ))
    })?;
    let readable = desfire_read_granted(&settings, key_number, authenticated);
    let stored = if readable {
        let communication = effective_read_communication(&settings, key_number, authenticated);
        Some(desfire.read_transaction_mac_file(file_id, communication)?.0)
    } else {
        None
    };
    if let Some(stored) = stored
        && (stored.counter != returned.counter || stored.value != returned.value)
    {
        return Err(NfcError::VerificationFailed(format!(
            "Transaction MAC file {file_id:02X} does not match the CommitTransaction response"
        )));
    }
    let Some(tmac_key) = tmac_key else {
        return Ok(Some(DesfireTransactionMacResult {
            file_id,
            counter: returned.counter,
            value: returned.value,
            verified: false,
            detail: if readable {
                "card response matches the stored file; no AppTransactionMACKey supplied".to_owned()
            } else {
                "no AppTransactionMACKey supplied and the Transaction MAC file is not readable"
                    .to_owned()
            },
        }));
    };
    let uid = uid.ok_or_else(|| {
        NfcError::InvalidArgument(
            "Transaction MAC verification needs the card UID from an authenticated session"
                .to_owned(),
        )
    })?;
    let computed = nfc_desfire::transaction_mac(tmac_key, returned.counter, &uid, tmi)?;
    if computed != returned.value {
        return Err(NfcError::VerificationFailed(format!(
            "the Transaction MAC for counter {} does not verify with the supplied AppTransactionMACKey",
            returned.counter
        )));
    }
    Ok(Some(DesfireTransactionMacResult {
        file_id,
        counter: returned.counter,
        value: returned.value,
        verified: true,
        detail: if readable {
            "verified with the AppTransactionMACKey; matches the stored Transaction MAC file"
                .to_owned()
        } else {
            "verified with the AppTransactionMACKey; the Transaction MAC file is not readable"
                .to_owned()
        },
    }))
}

/// One-line result used by `nfctl` and the GTK write view.
pub fn write_text(result: &DesfireWriteResult) -> String {
    let protection = match result.key_type {
        Some(key_type) => format!("{} · {}", key_type.name(), result.integrity.label()),
        None => result.integrity.label().to_owned(),
    };
    let end = u64::from(result.offset) + result.written as u64;
    format!(
        "settings: {}\nprotection: {}\nwritten: {}..{} ({} bytes)\nbackup: {} bytes\ncommit: {}\nread-back: {} bytes verified\n",
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

/// Human-readable value change result.
pub fn value_text(result: &DesfireValueResult) -> String {
    let protection = match result.key_type {
        Some(key_type) => format!("{} · {}", key_type.name(), result.integrity.label()),
        None => result.integrity.label().to_owned(),
    };
    format!(
        "settings: {}\nprotection: {}\nvalue: {} → {} ({} {})\ncommit: committed\nnew value: {} verified\n{}",
        result.settings.summary(),
        protection,
        result.old_value,
        result.new_value,
        result.operation.name(),
        result.amount,
        result.new_value,
        transaction_mac_text(result.transaction_mac.as_ref())
    )
}

/// Human-readable record mutation result.
pub fn record_text(result: &DesfireRecordResult) -> String {
    let protection = match result.key_type {
        Some(key_type) => format!("{} · {}", key_type.name(), result.integrity.label()),
        None => result.integrity.label().to_owned(),
    };
    let mut text = format!(
        "settings: {}\nprotection: {}\nbefore: {} records × {} bytes\n",
        result.settings_before.summary(),
        protection,
        result.before.len(),
        result.record_size
    );
    for (index, record) in result.before.iter().enumerate() {
        text.push_str(&format!(
            "record {index}: {} | {}\n",
            hex_bytes(record),
            ascii(record)
        ));
    }
    text.push_str(&format!("operation: {}", result.operation.name()));
    match result.operation {
        DesfireRecordOperation::Append => {
            text.push_str(&format!(" · wrote {} bytes\n", result.written.len()));
        }
        DesfireRecordOperation::Update { .. } => {
            text.push_str(&format!(" · wrote {} bytes\n", result.written.len()));
        }
        DesfireRecordOperation::Clear => text.push('\n'),
    }
    text.push_str(&format!(
        "after: {} records × {} bytes\n",
        result.after.len(),
        result.record_size
    ));
    for (index, record) in result.after.iter().enumerate() {
        text.push_str(&format!(
            "record {index}: {} | {}\n",
            hex_bytes(record),
            ascii(record)
        ));
    }
    text.push_str("commit: committed\n");
    text.push_str("read-back: every record verified\n");
    text.push_str(&transaction_mac_text(result.transaction_mac.as_ref()));
    text
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

fn transaction_mac_text(result: Option<&DesfireTransactionMacResult>) -> String {
    match result {
        None => "transaction MAC: not present\n".to_owned(),
        Some(result) => {
            let state = if result.verified {
                "verified"
            } else {
                "UNVERIFIED"
            };
            format!(
                "transaction MAC: file {:02X} · counter {} · {} · {state}: {}\n",
                result.file_id,
                result.counter,
                hex_bytes(&result.value),
                result.detail
            )
        }
    }
}

/// Human-readable result of reading free memory.
pub fn free_memory_text(aid: Option<[u8; 3]>, free_memory: u32) -> String {
    let scope = match aid {
        Some(aid) => format!("application {}", hex_bytes(&aid)),
        None => "PICC".to_owned(),
    };
    format!("{scope} · free {free_memory} bytes\n")
}

/// Human-readable application creation result.
pub fn app_create_text(result: &DesfireAppCreateResult) -> String {
    let applications = if result.applications.is_empty() {
        "none".to_owned()
    } else {
        result
            .applications
            .iter()
            .map(|aid| hex_bytes(aid))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "application {} created\nkeys: {} · {}\nfree: {} → {} bytes\napplications: {applications}\n",
        hex_bytes(&result.aid),
        result.keys,
        result.app_key_type.name(),
        result.free_memory_before,
        result.free_memory_after
    )
}

/// Human-readable application deletion result.
pub fn app_delete_text(result: &DesfireAppDeleteResult) -> String {
    let applications = if result.applications.is_empty() {
        "none".to_owned()
    } else {
        result
            .applications
            .iter()
            .map(|aid| hex_bytes(aid))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let free = match (result.free_memory_before, result.free_memory_after) {
        (Some(before), Some(after)) => format!("free: {before} → {after} bytes\n"),
        (Some(before), None) => format!("free: {before} bytes → unavailable after deletion\n"),
        _ => String::new(),
    };
    format!(
        "application {} deleted\nmanifest: {}\n{free}applications: {applications}\nnot a byte-for-byte restore\n",
        hex_bytes(&result.aid),
        result.manifest_path.display()
    )
}

/// Human-readable file creation result.
pub fn file_create_text(result: &DesfireFileCreateResult) -> String {
    format!(
        "aid {} file {:02X} created\ntype: {}\ncontent bytes: {} · free: {} → {} bytes\n",
        hex_bytes(&result.aid),
        result.file_id,
        result.settings.summary(),
        result.required,
        result.free_memory_before,
        result.free_memory_after
    )
}

/// Human-readable file deletion result.
pub fn file_delete_text(result: &DesfireFileDeleteResult) -> String {
    let files = if result.files.is_empty() {
        "none".to_owned()
    } else {
        result
            .files
            .iter()
            .map(|file| format!("{file:02X}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let free = match (result.free_memory_before, result.free_memory_after) {
        (Some(before), Some(after)) => format!("free: {before} → {after} bytes\n"),
        (Some(before), None) => format!("free: {before} bytes → unavailable after deletion\n"),
        _ => String::new(),
    };
    format!(
        "aid {} file {:02X} deleted\nmanifest: {}\n{free}files: {files}\n",
        hex_bytes(&result.aid),
        result.file_id,
        result.manifest_path.display()
    )
}

/// Human-readable file-settings change result.
pub fn file_settings_text(result: &DesfireFileSettingsResult) -> String {
    format!(
        "aid {} file {:02X}\nbefore: {}\nafter: {}\nchange: {}\n",
        hex_bytes(&result.aid),
        result.file_id,
        result.before.summary(),
        result.after.summary(),
        if result.changed {
            "applied"
        } else {
            "not needed"
        }
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
