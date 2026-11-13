// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal MIFARE DESFire native command layer over ISO 7816-4 APDUs.
//!
//! This crate covers identity and application/file inventory, DES/2K3DES,
//! 3K3DES, AES EV1, and AES EV2 authentication with session-key derivation,
//! key verification, the authenticated card UID, EV1 and EV2 secure messaging
//! for file reads and writes, transactions, the primitive application/file
//! commands, key settings and key changes, and the AN11004 NDEF Tag
//! Application layout. The general workbench UI is not part of this layer.

use aes::Aes128;
use des::TdesEde3;
use des::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use nfc_core::{NfcError, Result};
use nfc_transport::Reader;

#[cfg(any(test, feature = "mock-card"))]
pub mod mock;

const CLA: u8 = 0x90;
const INS_ADDITIONAL_FRAME: u8 = 0xAF;
const INS_AUTHENTICATE: u8 = 0x0A;
const INS_GET_VERSION: u8 = 0x60;
const INS_GET_APPLICATION_IDS: u8 = 0x6A;
const INS_SELECT_APPLICATION: u8 = 0x5A;
const INS_GET_FILE_IDS: u8 = 0x6F;
const INS_GET_FILE_SETTINGS: u8 = 0xF5;
const INS_GET_KEY_SETTINGS: u8 = 0x45;
const INS_CHANGE_KEY_SETTINGS: u8 = 0x54;
const INS_CHANGE_KEY: u8 = 0xC4;
const INS_GET_KEY_VERSION: u8 = 0x64;
const INS_GET_CARD_UID: u8 = 0x51;
const INS_GET_FREE_MEMORY: u8 = 0x6E;
const INS_FORMAT_PICC: u8 = 0xFC;
const INS_CREATE_APPLICATION: u8 = 0xCA;
const INS_DELETE_APPLICATION: u8 = 0xDA;
const INS_CREATE_STD_DATA_FILE: u8 = 0xCD;
const INS_CREATE_BACKUP_DATA_FILE: u8 = 0xCB;
const INS_CREATE_VALUE_FILE: u8 = 0xCC;
const INS_CREATE_LINEAR_RECORD_FILE: u8 = 0xC1;
const INS_CREATE_CYCLIC_RECORD_FILE: u8 = 0xC0;
const INS_CREATE_TRANSACTION_MAC_FILE: u8 = 0xCE;
const INS_DELETE_FILE: u8 = 0xDF;
const INS_CHANGE_FILE_SETTINGS: u8 = 0x5F;
const INS_WRITE_DATA: u8 = 0x3D;
const INS_READ_DATA: u8 = 0xBD;
const INS_GET_VALUE: u8 = 0x6C;
const INS_CREDIT: u8 = 0x0C;
const INS_LIMITED_CREDIT: u8 = 0x1C;
const INS_DEBIT: u8 = 0xDC;
const INS_READ_RECORDS: u8 = 0xBB;
const INS_WRITE_RECORD: u8 = 0x3B;
const INS_UPDATE_RECORD: u8 = 0xDB;
const INS_CLEAR_RECORD_FILE: u8 = 0xEB;
const INS_COMMIT_TRANSACTION: u8 = 0xC7;
const INS_ABORT_TRANSACTION: u8 = 0xA7;

const STATUS_OK: u16 = 0x9100;
const STATUS_ADDITIONAL: u16 = 0x91AF;
/// `NoChanges`: a commit with no uncommitted data is already in the
/// requested state, so callers treat it as success.
const STATUS_NO_CHANGES: u16 = 0x910C;
/// `PermissionDenied`: the authenticated key may not run the command.
const STATUS_PERMISSION_DENIED: u16 = 0x919D;

/// DESFire application ID that carries the NFC Forum Type 4 NDEF Tag
/// Application, in native-command byte order (logical AID `D27600`).
pub const NDEF_APPLICATION_AID: [u8; 3] = [0x00, 0x76, 0xD2];
/// ISO 7816-4 DF name of the NDEF Tag Application.
pub const NDEF_DF_NAME: [u8; 7] = [0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01];
/// Application ISO file identifier used by the AN11004 layout.
pub const NDEF_APPLICATION_ISO_FILE: u16 = 0xE110;
/// Application key settings and key count for the NDEF application.
///
/// `0x0B` requires the application master key for create/delete and `0x21`
/// selects one DES key with ISO file ID support.
pub const NDEF_APPLICATION_KEY_SETTINGS: (u8, u8) = (0x0B, 0x21);
/// Factory application key of a newly created DESFire application.
pub const NDEF_APPLICATION_KEY: [u8; 8] = [0x00; 8];
/// DESFire file number of the Type 4 capability container.
pub const NDEF_CC_FILE: u8 = 0x01;
/// ISO file identifier of the Type 4 capability container.
pub const NDEF_CC_ISO_FILE: u16 = 0xE103;
/// Size of the capability container file.
pub const NDEF_CC_SIZE: u32 = 15;
/// Access rights of the capability container: read free, write key 0.
pub const NDEF_CC_ACCESS_RIGHTS: u16 = 0xE000;
/// DESFire file number of the NDEF data file.
pub const NDEF_DATA_FILE: u8 = 0x02;
/// ISO file identifier of the NDEF data file.
pub const NDEF_DATA_ISO_FILE: u16 = 0xE104;
/// Size of the NDEF data file, including the two NLEN bytes.
pub const NDEF_DATA_SIZE: u32 = 255;
/// Access rights of the NDEF data file: read and write free, change key 0.
pub const NDEF_DATA_ACCESS_RIGHTS: u16 = 0xEEE0;
/// Mapping-version 2.0 capability container for one writable NDEF file.
pub const NDEF_CAPABILITY_CONTAINER: [u8; 15] = [
    0x00, 0x0F, 0x20, 0x00, 0x3B, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x00, 0xFF, 0x00, 0x00,
];

/// DESFire authentication key families.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthKeyType {
    /// 8-byte DES or 2K3DES key through `Authenticate` (`0x0A`).
    Des,
    /// 16-byte 2K3DES key through `Authenticate` (`0x0A`).
    Tdes2,
    /// 24-byte 3K3DES key through `AuthenticateISO` (`0x1A`).
    Tdes3,
    /// 16-byte AES-128 key through `AuthenticateAES` (`0xAA`).
    Aes,
    /// 16-byte AES-128 key through `AuthenticateEV2First` (`0x71`).
    AesEv2,
}

impl AuthKeyType {
    /// Key family implied by a key length. A 16-byte key defaults to
    /// 2K3DES; AES needs an explicit selection.
    pub fn from_key(key: &[u8]) -> Result<Self> {
        match key.len() {
            8 => Ok(Self::Des),
            16 => Ok(Self::Tdes2),
            24 => Ok(Self::Tdes3),
            _ => Err(NfcError::InvalidArgument(
                "DESFire keys are 8-byte DES, 16-byte 2K3DES, 24-byte 3K3DES, or 16-byte AES"
                    .to_owned(),
            )),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Des => "DES",
            Self::Tdes2 => "2K3DES",
            Self::Tdes3 => "3K3DES",
            Self::Aes => "AES",
            Self::AesEv2 => "AES EV2",
        }
    }

    pub const fn key_length(self) -> usize {
        match self {
            Self::Des => 8,
            Self::Tdes2 | Self::Aes | Self::AesEv2 => 16,
            Self::Tdes3 => 24,
        }
    }

    const fn challenge_length(self) -> usize {
        match self {
            Self::Des | Self::Tdes2 => 8,
            Self::Tdes3 | Self::Aes | Self::AesEv2 => 16,
        }
    }

    const fn command(self) -> u8 {
        match self {
            Self::Des | Self::Tdes2 => INS_AUTHENTICATE,
            Self::Tdes3 => 0x1A,
            Self::Aes => 0xAA,
            Self::AesEv2 => 0x71,
        }
    }

    pub fn validate_key(self, key: &[u8]) -> Result<()> {
        if key.len() == self.key_length() {
            Ok(())
        } else {
            Err(NfcError::InvalidArgument(format!(
                "{} authentication needs a {}-byte key, got {}",
                self.name(),
                self.key_length(),
                key.len()
            )))
        }
    }

    /// Key family of an application that authenticates with this key type.
    pub const fn key_family(self) -> ApplicationKeyType {
        KeyFamily::of(self).application_key_type()
    }

    /// Validate a new key for a ChangeKey within this session's family.
    pub fn validate_change_key(self, key: &[u8]) -> Result<()> {
        KeyFamily::of(self).validate_key(key)
    }
}

/// Key family assigned to a new application by CreateApplication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationKeyType {
    /// DES / 2K3DES application keys (KeySettings2 bits 6-7 = 00).
    Tdes2,
    /// 3K3DES application keys (bits 6-7 = 01).
    Tdes3,
    /// AES-128 application keys (bits 6-7 = 10).
    Aes,
}

impl ApplicationKeyType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tdes2 => "2K3DES",
            Self::Tdes3 => "3K3DES",
            Self::Aes => "AES",
        }
    }

    pub const fn bits(self) -> u8 {
        match self {
            Self::Tdes2 => 0,
            Self::Tdes3 => 1,
            Self::Aes => 2,
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "des" | "2k3des" => Ok(Self::Tdes2),
            "3k3des" => Ok(Self::Tdes3),
            "aes" => Ok(Self::Aes),
            other => Err(NfcError::InvalidArgument(format!(
                "application key type {other:?} is not des, 3k3des, or aes"
            ))),
        }
    }

    pub const fn key_length(self) -> usize {
        match self {
            Self::Tdes2 => 16,
            Self::Tdes3 => 24,
            Self::Aes => 16,
        }
    }
}

/// Parameters for creating a DESFire application at the PICC level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationSpec {
    pub aid: [u8; 3],
    /// Key settings byte 1: application master key rights. `0x0F` is the
    /// factory default: configuration changeable, AMK changeable.
    pub key_settings: u8,
    /// Number of keys, 1..=14.
    pub keys: u8,
    pub key_type: ApplicationKeyType,
    /// ISO 7816-4 file ID and DF name, present together or not at all.
    pub iso_file_id: Option<u16>,
    pub df_name: Vec<u8>,
}

impl ApplicationSpec {
    pub fn new(aid: [u8; 3], keys: u8, key_type: ApplicationKeyType) -> Self {
        Self {
            aid,
            key_settings: 0x0F,
            keys,
            key_type,
            iso_file_id: None,
            df_name: Vec::new(),
        }
    }

    /// Encode KeySettings2 from the key count and family.
    pub fn key_settings2(&self) -> u8 {
        (self.key_type.bits() << 6) | (self.keys & 0x0F)
    }
}

struct Session {
    enc_key: Vec<u8>,
    mac_key: Vec<u8>,
    key_type: AuthKeyType,
    /// Key number used by the authentication that opened this session.
    key_no: u8,
    key_version: u8,
    /// EV1 CMAC and CBC chaining value. Empty for D40 and EV2 sessions.
    iv: Vec<u8>,
    /// EV2 transaction identifier from the authentication response.
    ti: [u8; 4],
    /// EV2 command counter. The card increments it between the command and
    /// the response.
    counter: u16,
}

impl Session {
    /// Only 3K3DES and AES sessions use secure messaging. DES/2K3DES keep the
    /// legacy D40 channel, whose responses carry no MAC.
    fn uses_secure_messaging(&self) -> bool {
        matches!(
            self.key_type,
            AuthKeyType::Tdes3 | AuthKeyType::Aes | AuthKeyType::AesEv2
        )
    }

    /// Protection applied to commands that are not file-data operations.
    ///
    /// EV1 MACs every response, so a plain command still advances the CMAC
    /// chain and verifies the response MAC. EV2 only protects commands whose
    /// communication mode calls for it, and its management commands are MACed.
    fn management_protection(&self) -> CommunicationMode {
        match self.key_type {
            AuthKeyType::AesEv2 => CommunicationMode::Maced,
            _ => CommunicationMode::Plain,
        }
    }

    fn block_size(&self) -> usize {
        match self.key_type {
            AuthKeyType::Tdes3 => 8,
            AuthKeyType::Aes | AuthKeyType::AesEv2 => 16,
            AuthKeyType::Des | AuthKeyType::Tdes2 => 8,
        }
    }

    fn command_mac(&self, ins: u8, data: &[u8]) -> Result<Vec<u8>> {
        match self.key_type {
            AuthKeyType::Tdes3 | AuthKeyType::Aes => {
                let mut input = Vec::with_capacity(1 + data.len());
                input.push(ins);
                input.extend_from_slice(data);
                ev1_cmac(self.key_type, &self.mac_key, &self.iv, &input)
            }
            AuthKeyType::AesEv2 => {
                let mut input = Vec::with_capacity(7 + data.len());
                input.push(ins);
                input.extend_from_slice(&self.counter.to_le_bytes());
                input.extend_from_slice(&self.ti);
                input.extend_from_slice(data);
                cmac_aes(&self.mac_key, &input)
            }
            AuthKeyType::Des | AuthKeyType::Tdes2 => Err(NfcError::Unsupported(
                "legacy DESFire sessions have no secure messaging",
            )),
        }
    }

    fn response_mac(&self, data: &[u8], status: u8) -> Result<Vec<u8>> {
        match self.key_type {
            AuthKeyType::Tdes3 | AuthKeyType::Aes => {
                let mut input = Vec::with_capacity(data.len() + 1);
                input.extend_from_slice(data);
                input.push(status);
                ev1_cmac(self.key_type, &self.mac_key, &self.iv, &input)
            }
            AuthKeyType::AesEv2 => {
                let mut input = Vec::with_capacity(7 + data.len());
                input.push(status);
                input.extend_from_slice(&self.counter.to_le_bytes());
                input.extend_from_slice(&self.ti);
                input.extend_from_slice(data);
                cmac_aes(&self.mac_key, &input)
            }
            AuthKeyType::Des | AuthKeyType::Tdes2 => Err(NfcError::Unsupported(
                "legacy DESFire sessions have no secure messaging",
            )),
        }
    }
}

/// DESFire version information.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub vendor_id: u8,
    pub product_type: u8,
    pub product_subtype: u8,
    pub major: u8,
    pub minor: u8,
    pub storage_size: u8,
    pub protocol: u8,
}

impl Version {
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 7 {
            return Err(NfcError::InvalidArgument(format!(
                "GET_VERSION returned {} bytes, expected at least 7",
                bytes.len()
            )));
        }
        Ok(Self {
            vendor_id: bytes[0],
            product_type: bytes[1],
            product_subtype: bytes[2],
            major: bytes[3],
            minor: bytes[4],
            storage_size: bytes[5],
            protocol: bytes[6],
        })
    }

    pub const fn name(&self) -> &'static str {
        match (self.vendor_id, self.product_type, self.major) {
            (0x04, 0x01, 0x33) => "MIFARE DESFire EV3",
            (0x04, 0x01, 0x12) => "MIFARE DESFire EV2",
            (0x04, 0x01, 0x01) => "MIFARE DESFire EV1",
            _ => "MIFARE DESFire",
        }
    }
}

/// One application and its file inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub aid: [u8; 3],
    pub files: Vec<FileInfo>,
}

/// One DESFire file and its raw settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub file_id: u8,
    pub settings: Vec<u8>,
}

impl FileInfo {
    /// Decode the raw GetFileSettings response.
    pub fn decoded(&self) -> Result<FileSettings> {
        FileSettings::parse(&self.settings)
    }
}

/// Read-only card inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub version: Version,
    pub applications: Vec<Application>,
}

/// DESFire file type reported by GetFileSettings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Standard,
    Backup,
    Value,
    LinearRecord,
    CyclicRecord,
    TransactionMac,
}

impl FileType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard data",
            Self::Backup => "backup data",
            Self::Value => "value",
            Self::LinearRecord => "linear record",
            Self::CyclicRecord => "cyclic record",
            Self::TransactionMac => "transaction MAC",
        }
    }

    fn parse(value: u8) -> Result<Self> {
        match value {
            0x00 => Ok(Self::Standard),
            0x01 => Ok(Self::Backup),
            0x02 => Ok(Self::Value),
            0x03 => Ok(Self::LinearRecord),
            0x04 => Ok(Self::CyclicRecord),
            0x05 => Ok(Self::TransactionMac),
            other => Err(NfcError::InvalidArgument(format!(
                "unknown DESFire file type {other:#04X}"
            ))),
        }
    }
}

/// File communication mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommunicationMode {
    Plain,
    Maced,
    Enciphered,
}

impl CommunicationMode {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Maced => "MACed",
            Self::Enciphered => "enciphered",
        }
    }

    /// Decode the two-bit communication field; 2 is reserved.
    pub fn from_raw(value: u8) -> Result<Self> {
        match value {
            0x00 => Ok(Self::Plain),
            0x01 => Ok(Self::Maced),
            0x03 => Ok(Self::Enciphered),
            0x02 => Err(NfcError::InvalidArgument(
                "DESFire file communication mode 2 is reserved".to_owned(),
            )),
            _ => unreachable!("communication mode is a two-bit field"),
        }
    }

    /// Raw two-bit value stored in file settings and create payloads.
    pub const fn raw(self) -> u8 {
        match self {
            Self::Plain => 0,
            Self::Maced => 1,
            Self::Enciphered => 3,
        }
    }
}

/// Cryptographic check a response passed before its data was returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrityCheck {
    /// No response MAC or checksum was present.
    None,
    /// 8-byte EV1 CMAC over the response.
    Ev1Cmac,
    /// EV1 enciphered response: CRC32 and zero padding.
    Ev1Crc32,
    /// 8-byte truncated EV2 CMAC over the response.
    Ev2Cmac,
}

impl IntegrityCheck {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "no response MAC",
            Self::Ev1Cmac => "EV1 CMAC verified",
            Self::Ev1Crc32 => "EV1 CRC32 verified",
            Self::Ev2Cmac => "EV2 CMAC verified",
        }
    }
}

/// One file-data read and the protection that was verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedData {
    pub data: Vec<u8>,
    pub integrity: IntegrityCheck,
}

/// One value read and the protection that was verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifiedValue {
    pub value: i32,
    pub integrity: IntegrityCheck,
}

/// Value-file operation selected by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueAdjustment {
    Credit,
    Debit,
    LimitedCredit,
}

impl ValueAdjustment {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Credit => "credit",
            Self::Debit => "debit",
            Self::LimitedCredit => "limited credit",
        }
    }

    const fn instruction(self) -> u8 {
        match self {
            Self::Credit => INS_CREDIT,
            Self::Debit => INS_DEBIT,
            Self::LimitedCredit => INS_LIMITED_CREDIT,
        }
    }
}

/// Transaction MAC counter and value returned by CommitTransaction or read
/// from a Transaction MAC file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionMac {
    /// Transaction MAC Counter used for this transaction.
    pub counter: u32,
    /// Truncated AES-CMAC over the Transaction MAC Input.
    pub value: [u8; 8],
}

/// Authenticated card UID response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardUid {
    pub uid: [u8; 7],
    pub integrity: IntegrityCheck,
}

/// Result of an explicit authentication check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyVerification {
    pub key_no: u8,
    pub key_type: AuthKeyType,
    /// The session uses CMAC secure messaging, not the legacy D40 channel.
    pub secure_messaging: bool,
    /// Version reported by GetKeyVersion, or `None` when the card does not
    /// expose it for this key.
    pub key_version: Option<u8>,
}

/// One access condition from a DESFire file access-rights word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessRight {
    Key(u8),
    Free,
    Denied,
}

impl AccessRight {
    const fn from_nibble(nibble: u8) -> Self {
        match nibble {
            0x00..=0x0D => Self::Key(nibble),
            0x0E => Self::Free,
            _ => Self::Denied,
        }
    }

    pub const fn nibble(self) -> u8 {
        match self {
            Self::Key(key) => key,
            Self::Free => 0x0E,
            Self::Denied => 0x0F,
        }
    }

    /// Parse `key0`..`key13`, `free`, or `denied`.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim().to_ascii_lowercase();
        match text.as_str() {
            "free" => Ok(Self::Free),
            "denied" | "deny" => Ok(Self::Denied),
            _ => match text.strip_prefix("key") {
                Some(number) => match number.trim().parse::<u8>() {
                    Ok(key) if key <= 13 => Ok(Self::Key(key)),
                    _ => Err(NfcError::InvalidArgument(format!(
                        "DESFire access right {text:?} is not key0..key13, free, or denied"
                    ))),
                },
                None => Err(NfcError::InvalidArgument(format!(
                    "DESFire access right {text:?} is not key0..key13, free, or denied"
                ))),
            },
        }
    }

    pub fn name(self) -> String {
        match self {
            Self::Key(key) => format!("key {key}"),
            Self::Free => "free".to_owned(),
            Self::Denied => "denied".to_owned(),
        }
    }
}

/// File access rights for read, write, read/write, and change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessRights {
    pub raw: u16,
    pub read: AccessRight,
    pub write: AccessRight,
    pub read_write: AccessRight,
    pub change: AccessRight,
}

impl AccessRights {
    fn parse(bytes: [u8; 2]) -> Self {
        Self {
            raw: u16::from_le_bytes(bytes),
            read: AccessRight::from_nibble(bytes[1] >> 4),
            write: AccessRight::from_nibble(bytes[1] & 0x0F),
            read_write: AccessRight::from_nibble(bytes[0] >> 4),
            change: AccessRight::from_nibble(bytes[0] & 0x0F),
        }
    }

    /// Build the raw little-endian access word from its four conditions.
    pub fn from_parts(
        read: AccessRight,
        write: AccessRight,
        read_write: AccessRight,
        change: AccessRight,
    ) -> Self {
        let low = (read_write.nibble() << 4) | change.nibble();
        let high = (read.nibble() << 4) | write.nibble();
        Self::parse([low, high])
    }

    /// Decode a raw access word for display and validation.
    pub fn from_raw(raw: u16) -> Self {
        Self::parse(raw.to_le_bytes())
    }

    pub fn summary(self) -> String {
        format!(
            "read {}, write {}, rw {}, change {}",
            self.read.name(),
            self.write.name(),
            self.read_write.name(),
            self.change.name()
        )
    }
}

/// Type-specific file contents from GetFileSettings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileContents {
    Data {
        size: u32,
    },
    Value {
        lower_limit: i32,
        upper_limit: i32,
        /// Value added by a LimitedCredit operation. The current value is
        /// only available through GetValue; this field is not it.
        limited_credit_value: i32,
        limited_credit: bool,
        free_get_value: bool,
    },
    Record {
        record_size: u32,
        max_records: u32,
        current_records: u32,
    },
    TransactionMac {
        key_option: u8,
        key_version: u8,
        counter_limit: Option<u32>,
    },
}

/// Decoded DESFire file settings. The raw response is preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSettings {
    pub file_type: FileType,
    pub communication: CommunicationMode,
    pub access: AccessRights,
    /// Access rights for keys 1..n when the additional-rights option is set.
    pub additional_access: Vec<AccessRights>,
    pub contents: FileContents,
    raw: Vec<u8>,
}

impl FileSettings {
    /// Decode a GetFileSettings response and reject truncated or
    /// internally inconsistent data.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(NfcError::InvalidArgument(format!(
                "DESFire file settings are {} bytes, expected at least 4",
                bytes.len()
            )));
        }
        let file_type = FileType::parse(bytes[0])?;
        let option = bytes[1];
        let communication = CommunicationMode::from_raw(option & 0x03)?;
        let access = AccessRights::parse([bytes[2], bytes[3]]);
        if file_type != FileType::TransactionMac && option & 0x20 != 0 {
            return Err(NfcError::InvalidArgument(
                "the transaction MAC counter-limit bit is set on a non-MAC file".to_owned(),
            ));
        }
        let mut offset = 4;
        let contents = match file_type {
            FileType::Standard | FileType::Backup => {
                let raw = take(bytes, &mut offset, 3, "data file size")?;
                let size = u24(raw);
                if size == 0 {
                    return Err(NfcError::InvalidArgument(
                        "DESFire data file size is zero".to_owned(),
                    ));
                }
                FileContents::Data { size }
            }
            FileType::Value => {
                let raw = take(bytes, &mut offset, 13, "value file limits")?;
                let lower_limit =
                    i32::from_le_bytes(raw[0..4].try_into().expect("four-byte lower limit"));
                let upper_limit =
                    i32::from_le_bytes(raw[4..8].try_into().expect("four-byte upper limit"));
                let limited_credit_value =
                    i32::from_le_bytes(raw[8..12].try_into().expect("four-byte credit value"));
                let flags = raw[12];
                if flags & !0x03 != 0 {
                    return Err(NfcError::InvalidArgument(format!(
                        "DESFire value file flags {flags:#04X} use reserved bits"
                    )));
                }
                if lower_limit >= upper_limit {
                    return Err(NfcError::InvalidArgument(format!(
                        "DESFire value limits {lower_limit}..{upper_limit} are not increasing"
                    )));
                }
                FileContents::Value {
                    lower_limit,
                    upper_limit,
                    limited_credit_value,
                    limited_credit: flags & 0x01 != 0,
                    free_get_value: flags & 0x02 != 0,
                }
            }
            FileType::LinearRecord | FileType::CyclicRecord => {
                let raw = take(bytes, &mut offset, 9, "record file limits")?;
                let record_size = u24(&raw[0..3]);
                let max_records = u24(&raw[3..6]);
                let current_records = u24(&raw[6..9]);
                if record_size == 0 {
                    return Err(NfcError::InvalidArgument(
                        "DESFire record size is zero".to_owned(),
                    ));
                }
                if max_records == 0 {
                    return Err(NfcError::InvalidArgument(
                        "DESFire record file has no maximum record count".to_owned(),
                    ));
                }
                if current_records > max_records {
                    return Err(NfcError::InvalidArgument(format!(
                        "DESFire current record count {current_records} exceeds {max_records}"
                    )));
                }
                FileContents::Record {
                    record_size,
                    max_records,
                    current_records,
                }
            }
            FileType::TransactionMac => {
                let raw = take(bytes, &mut offset, 2, "transaction MAC key fields")?;
                let key_option = raw[0];
                let key_version = raw[1];
                let counter_limit = if option & 0x20 != 0 {
                    let length = if key_option & 0x02 != 0 { 4 } else { 2 };
                    let raw = take(bytes, &mut offset, length, "transaction MAC counter limit")?;
                    Some(
                        raw.iter()
                            .fold(0u32, |value, byte| (value << 8) | u32::from(*byte)),
                    )
                } else {
                    None
                };
                FileContents::TransactionMac {
                    key_option,
                    key_version,
                    counter_limit,
                }
            }
        };
        let additional_access = if option & 0x80 != 0 {
            let count = usize::from(take(bytes, &mut offset, 1, "additional access count")?[0]);
            if count == 0 || count > 13 {
                return Err(NfcError::InvalidArgument(format!(
                    "DESFire additional access-rights count {count} is outside 1..=13"
                )));
            }
            let raw = take(bytes, &mut offset, count * 2, "additional access rights")?;
            raw.as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| AccessRights::parse([chunk[0], chunk[1]]))
                .collect()
        } else {
            Vec::new()
        };
        if offset != bytes.len() {
            return Err(NfcError::InvalidArgument(format!(
                "DESFire file settings have {} trailing bytes",
                bytes.len() - offset
            )));
        }
        Ok(Self {
            file_type,
            communication,
            access,
            additional_access,
            contents,
            raw: bytes.to_vec(),
        })
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// One-line protocol summary used by `nfctl` and the GTK inventory.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "{} · {} · {}",
            self.file_type.name(),
            self.communication.name(),
            self.access.summary()
        );
        match &self.contents {
            FileContents::Data { size } => {
                text.push_str(&format!(" · {size} bytes"));
            }
            FileContents::Value {
                lower_limit,
                upper_limit,
                limited_credit_value,
                limited_credit,
                free_get_value,
            } => {
                text.push_str(&format!(
                    " · limits {lower_limit}..{upper_limit} · limited credit value {limited_credit_value} · limited credit {} · get value {}",
                    if *limited_credit { "enabled" } else { "disabled" },
                    if *free_get_value { "free" } else { "restricted" }
                ));
            }
            FileContents::Record {
                record_size,
                max_records,
                current_records,
            } => {
                text.push_str(&format!(
                    " · {record_size}-byte records · {current_records}/{max_records}"
                ));
            }
            FileContents::TransactionMac {
                key_option,
                key_version,
                counter_limit,
            } => {
                text.push_str(&format!(
                    " · key option {key_option:#04X} · key version {key_version}"
                ));
                match counter_limit {
                    Some(limit) => text.push_str(&format!(" · counter limit {limit}")),
                    None => text.push_str(" · counter limit none"),
                }
            }
        }
        for (index, rights) in self.additional_access.iter().enumerate() {
            text.push_str(&format!(" · key {}: {}", index + 1, rights.summary()));
        }
        text
    }
}

fn take<'a>(bytes: &'a [u8], offset: &mut usize, length: usize, field: &str) -> Result<&'a [u8]> {
    let end = offset.checked_add(length).ok_or_else(|| {
        NfcError::InvalidArgument("DESFire file settings length overflow".to_owned())
    })?;
    let value = bytes.get(*offset..end).ok_or_else(|| {
        NfcError::InvalidArgument(format!(
            "DESFire file settings are truncated at the {field} field"
        ))
    })?;
    *offset = end;
    Ok(value)
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

/// Parameters for creating a DESFire value file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueFileSpec {
    pub file_id: u8,
    pub iso_file_id: Option<u16>,
    pub communication: u8,
    pub access_rights: u16,
    pub lower_limit: i32,
    pub upper_limit: i32,
    pub value: i32,
    pub flags: u8,
}

/// Parameters for creating a linear or cyclic DESFire record file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordFileSpec {
    pub file_id: u8,
    pub iso_file_id: Option<u16>,
    pub communication: u8,
    pub access_rights: u16,
    pub record_size: u32,
    pub max_records: u32,
    pub cyclic: bool,
}

/// Parameters for creating a DESFire Transaction MAC file.
///
/// The Transaction MAC key is a separate AES-128 key stored by the card and
/// used only for transaction MAC generation; it is not an application key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TransactionMacFileSpec {
    pub file_id: u8,
    pub iso_file_id: Option<u16>,
    pub communication: u8,
    pub access_rights: u16,
    pub key: [u8; 16],
    pub key_version: u8,
}

impl std::fmt::Debug for TransactionMacFileSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransactionMacFileSpec")
            .field("file_id", &self.file_id)
            .field("iso_file_id", &self.iso_file_id)
            .field("communication", &self.communication)
            .field("access_rights", &self.access_rights)
            .field("key", &"<redacted>")
            .field("key_version", &self.key_version)
            .finish()
    }
}

/// Key settings reported by GetKeySettings.
///
/// The second response byte carries the key count, the key family, and the
/// ISO file ID flag. DESFire D40 cards return only the first byte; the
/// optional fields are then `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySettings {
    /// The KeySettings byte: change rights for the master key and the
    /// configuration, free directory listing, and free create/delete.
    pub key_settings: u8,
    pub max_keys: Option<u8>,
    pub key_type: Option<ApplicationKeyType>,
    pub iso_file_id: bool,
}

impl KeySettings {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let Some(&key_settings) = bytes.first() else {
            return Err(NfcError::VerificationFailed(
                "GetKeySettings returned no key settings".to_owned(),
            ));
        };
        if bytes.len() > 2 {
            return Err(NfcError::VerificationFailed(format!(
                "GetKeySettings returned {} bytes, expected 1 or 2",
                bytes.len()
            )));
        }
        let (max_keys, key_type, iso_file_id) = match bytes.get(1) {
            Some(&second) => (
                Some(second & 0x0F),
                match second >> 6 {
                    0 => Some(ApplicationKeyType::Tdes2),
                    1 => Some(ApplicationKeyType::Tdes3),
                    2 => Some(ApplicationKeyType::Aes),
                    _ => None,
                },
                second & 0x20 != 0,
            ),
            None => (None, None, false),
        };
        Ok(Self {
            key_settings,
            max_keys,
            key_type,
            iso_file_id,
        })
    }

    /// Whether ChangeKeySettings may change the configuration.
    pub const fn configuration_changeable(&self) -> bool {
        self.key_settings & 0x08 != 0
    }

    /// Whether the master key itself may be changed.
    pub const fn master_key_changeable(&self) -> bool {
        self.key_settings & 0x01 != 0
    }

    /// Human-readable facts for CLI and GUI output.
    pub fn summary(&self, application: bool) -> String {
        let mut text = format!("settings {:02X}", self.key_settings);
        if application {
            text.push_str(&format!(
                " · master key changeable: {}",
                yes_no(self.master_key_changeable())
            ));
            text.push_str(&format!(
                " · configuration changeable: {}",
                yes_no(self.configuration_changeable())
            ));
            text.push_str(&format!(
                " · free file list: {} · free create/delete: {}",
                yes_no(self.key_settings & 0x02 != 0),
                yes_no(self.key_settings & 0x04 != 0)
            ));
            let change = self.key_settings >> 4;
            let change = match change {
                0x0F => "keys frozen".to_owned(),
                0x0E => "same key authenticates the change".to_owned(),
                key => format!("key {key} authenticates key changes"),
            };
            text.push_str(&format!(" · change rights: {change}"));
        } else {
            text.push_str(&format!(
                " · master key changeable: {}",
                yes_no(self.master_key_changeable())
            ));
            text.push_str(&format!(
                " · configuration changeable: {}",
                yes_no(self.configuration_changeable())
            ));
            text.push_str(&format!(
                " · free application list: {} · free create/delete: {}",
                yes_no(self.key_settings & 0x02 != 0),
                yes_no(self.key_settings & 0x04 != 0)
            ));
        }
        if let Some(max_keys) = self.max_keys {
            text.push_str(&format!(" · {max_keys} keys"));
        }
        if let Some(key_type) = self.key_type {
            text.push_str(&format!(" · {}", key_type.name()));
        }
        if self.iso_file_id {
            text.push_str(" · ISO file ID");
        }
        text
    }
}

/// One ChangeKey request against the selected application.
///
/// `current_key` is the stored key of `key_number`, required only when that
/// key is not the key used to authenticate. It is used for the `NewKey XOR
/// OldKey` construction and is never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyChange {
    pub key_number: u8,
    pub new_key: Vec<u8>,
    pub new_key_version: u8,
    pub current_key: Option<Vec<u8>>,
}

impl std::fmt::Debug for KeyChange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeyChange")
            .field("key_number", &self.key_number)
            .field("new_key", &"<redacted>")
            .field("new_key_version", &self.new_key_version)
            .field(
                "current_key",
                &self.current_key.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl KeyChange {
    pub fn new(key_number: u8, new_key: Vec<u8>) -> Self {
        Self {
            key_number,
            new_key,
            new_key_version: 0,
            current_key: None,
        }
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Key family of a ChangeKey operation, derived from the authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyFamily {
    Tdes2,
    Tdes3,
    Aes,
}

impl KeyFamily {
    const fn of(key_type: AuthKeyType) -> Self {
        match key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => Self::Tdes2,
            AuthKeyType::Tdes3 => Self::Tdes3,
            AuthKeyType::Aes | AuthKeyType::AesEv2 => Self::Aes,
        }
    }

    const fn application_key_type(self) -> ApplicationKeyType {
        match self {
            Self::Tdes2 => ApplicationKeyType::Tdes2,
            Self::Tdes3 => ApplicationKeyType::Tdes3,
            Self::Aes => ApplicationKeyType::Aes,
        }
    }

    const fn embeds_version(self) -> bool {
        matches!(self, Self::Tdes2 | Self::Tdes3)
    }

    const fn block_size(self) -> usize {
        match self {
            Self::Tdes2 | Self::Tdes3 => 8,
            Self::Aes => 16,
        }
    }

    const fn key_length(self) -> usize {
        match self {
            Self::Tdes2 | Self::Aes => 16,
            Self::Tdes3 => 24,
        }
    }

    fn validate_key(self, key: &[u8]) -> Result<()> {
        let valid = match self {
            Self::Tdes2 => matches!(key.len(), 8 | 16),
            Self::Tdes3 => key.len() == 24,
            Self::Aes => key.len() == 16,
        };
        if valid {
            Ok(())
        } else {
            let expected = match self {
                Self::Tdes2 => "8-byte DES or 16-byte 2K3DES",
                Self::Tdes3 => "24-byte 3K3DES",
                Self::Aes => "16-byte AES",
            };
            Err(NfcError::InvalidArgument(format!(
                "a {} key change needs a {expected} key, got {} bytes",
                self.application_key_type().name(),
                key.len()
            )))
        }
    }

    /// Key area sent to the card: the key with its version, where AES carries
    /// the version as a separate trailing byte.
    fn key_area(self, key: &[u8], version: u8) -> Vec<u8> {
        let mut area = match (self, key.len()) {
            (Self::Tdes2, 8) => {
                let mut expanded = key.to_vec();
                expanded.extend_from_slice(key);
                expanded
            }
            _ => key.to_vec(),
        };
        if self.embeds_version() {
            set_des_key_version(&mut area, version);
        } else {
            area.push(version);
        }
        area
    }

    /// Key value covered by the second ChangeKey CRC: the new key value
    /// alone. AES keeps its version outside the key bytes, so the version is
    /// not part of this value.
    fn crc_key(self, key: &[u8], version: u8) -> Vec<u8> {
        if self.embeds_version() {
            self.key_area(key, version)
        } else {
            key.to_vec()
        }
    }
}

/// XOR the stored old key into the new key area.
///
/// A DES key is repeated to fill the 2K3DES key area, matching how the card
/// stores an 8-byte DES key internally.
fn xor_key(plain: &mut [u8], current: Option<&[u8]>, key_length: usize) {
    if let Some(current) = current {
        for index in 0..key_length {
            plain[index] ^= current[index % current.len()];
        }
    }
}

/// Embed a key version in the parity bits of DES/2K3DES/3K3DES key bytes.
///
/// The version is written least-significant bit first from the last key byte,
/// repeating every eight bytes, as DESFire stores it.
fn set_des_key_version(key: &mut [u8], version: u8) {
    let mut bit = 0;
    for index in (0..key.len()).rev() {
        key[index] = (key[index] & 0xFE) | ((version >> bit) & 1);
        bit = (bit + 1) % 8;
    }
}

/// ISO/IEC 14443-3 CRC_A, used by the D40 ChangeKey frame.
fn crc16_a(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0x6363;
    for &byte in data {
        let mut mixed = byte ^ crc as u8;
        mixed ^= mixed << 4;
        crc = (crc >> 8) ^ ((mixed as u16) << 8) ^ ((mixed as u16) << 3) ^ ((mixed as u16) >> 4);
    }
    [crc as u8, (crc >> 8) as u8]
}

/// DESFire command context over one reader.
pub struct Desfire<'a> {
    reader: &'a mut dyn Reader,
    session: Option<Session>,
    selected: [u8; 3],
    /// Transaction MAC Input accumulated since application selection.
    tmi: Vec<u8>,
    /// Set while reading the Transaction MAC file, which is excluded from TMI.
    suppress_tmi: bool,
}

impl<'a> Desfire<'a> {
    pub fn new(reader: &'a mut dyn Reader) -> Self {
        Self {
            reader,
            session: None,
            selected: [0x00; 3],
            tmi: Vec::new(),
            suppress_tmi: false,
        }
    }

    /// Transaction MAC Input accumulated since the application was selected.
    ///
    /// Only authenticated sessions update it; commands outside the DESFire
    /// Transaction MAC command list never touch it.
    pub fn transaction_mac_input(&self) -> &[u8] {
        &self.tmi
    }

    fn tmi_active(&self) -> bool {
        self.session.is_some() && !self.suppress_tmi
    }

    /// Append one command parameter block and pad it to a 16-byte boundary.
    fn tmi_header(&mut self, header: &[u8]) {
        if !self.tmi_active() {
            return;
        }
        self.tmi.extend_from_slice(header);
        while !self.tmi.len().is_multiple_of(16) {
            self.tmi.push(0);
        }
    }

    /// Append a parameter block padded to 16 bytes, then the plain data padded
    /// to 16 bytes.
    fn tmi_data(&mut self, header: &[u8], data: &[u8]) {
        if !self.tmi_active() {
            return;
        }
        self.tmi_header(header);
        self.tmi.extend_from_slice(data);
        while !self.tmi.len().is_multiple_of(16) {
            self.tmi.push(0);
        }
    }

    fn transceive(&mut self, ins: u8, data: &[u8]) -> Result<(Vec<u8>, u16)> {
        if data.len() > 255 {
            return Err(NfcError::InvalidArgument(
                "a DESFire command payload is at most 255 bytes".to_owned(),
            ));
        }
        let mut apdu = Vec::with_capacity(6 + data.len());
        apdu.extend_from_slice(&[CLA, ins, 0x00, 0x00]);
        if data.is_empty() {
            apdu.push(0x00);
        } else {
            apdu.push(data.len() as u8);
            apdu.extend_from_slice(data);
            apdu.push(0x00);
        }
        let response = self.reader.transceive_apdu(&apdu)?;
        Ok((response.data, u16::from_be_bytes(response.sw)))
    }

    /// Send a bare command outside the secure channel.
    fn expect_ok(&mut self, ins: u8, data: &[u8]) -> Result<Vec<u8>> {
        let (body, sw) = self.transceive(ins, data)?;
        if sw != STATUS_OK {
            return Err(status_error(ins, sw));
        }
        Ok(body)
    }

    /// Send a command that is not a file-data operation.
    ///
    /// Management commands are Plain under EV1 (the response is still MACed)
    /// and MACed under EV2. Legacy DES/2K3DES sessions send them unchanged.
    fn command(&mut self, ins: u8, data: &[u8]) -> Result<Vec<u8>> {
        let communication = self
            .session
            .as_ref()
            .map_or(CommunicationMode::Plain, Session::management_protection);
        let (body, sw, _) = self.exchange(ins, data, communication, None)?;
        if sw != STATUS_OK {
            return Err(status_error(ins, sw));
        }
        Ok(body)
    }

    fn command_verified(&mut self, ins: u8, data: &[u8]) -> Result<(Vec<u8>, IntegrityCheck)> {
        let communication = self
            .session
            .as_ref()
            .map_or(CommunicationMode::Plain, Session::management_protection);
        let (body, sw, integrity) = self.exchange(ins, data, communication, None)?;
        if sw != STATUS_OK {
            return Err(status_error(ins, sw));
        }
        Ok((body, integrity))
    }

    /// Send a file read command with the protection of the file's
    /// communication mode.
    ///
    /// `expected` is the plaintext length; an enciphered response cannot be
    /// unpadded without it.
    fn read_command(
        &mut self,
        ins: u8,
        data: &[u8],
        communication: CommunicationMode,
        expected: usize,
    ) -> Result<VerifiedData> {
        let (body, sw, integrity) = self.exchange(ins, data, communication, Some(expected))?;
        if sw != STATUS_OK {
            return Err(status_error(ins, sw));
        }
        if body.len() != expected {
            return Err(NfcError::VerificationFailed(format!(
                "DESFire command {ins:#04X} returned {} bytes, expected {expected}",
                body.len()
            )));
        }
        Ok(VerifiedData {
            data: body,
            integrity,
        })
    }

    fn exchange(
        &mut self,
        ins: u8,
        data: &[u8],
        communication: CommunicationMode,
        expected: Option<usize>,
    ) -> Result<(Vec<u8>, u16, IntegrityCheck)> {
        let wire = match self.encode_command(ins, data, communication) {
            Ok(wire) => wire,
            Err(error) => {
                self.session = None;
                return Err(error);
            }
        };
        self.exchange_wire(ins, wire, communication, expected)
    }

    /// Send an already encoded command and decode the response.
    ///
    /// Writes protect the command with one mode and the status-only response
    /// with another, so the two modes are passed separately.
    fn exchange_wire(
        &mut self,
        ins: u8,
        wire: Vec<u8>,
        response_communication: CommunicationMode,
        expected: Option<usize>,
    ) -> Result<(Vec<u8>, u16, IntegrityCheck)> {
        let (raw, sw) = match self.send_frames(ins, &wire) {
            Ok(response) => response,
            Err(error) => {
                self.session = None;
                return Err(error);
            }
        };
        if sw != STATUS_OK {
            // A refused command invalidates the secure-messaging session.
            self.session = None;
            return Ok((raw, sw, IntegrityCheck::None));
        }
        if let Some(session) = self.session.as_mut()
            && session.key_type == AuthKeyType::AesEv2
        {
            session.counter = session.counter.wrapping_add(1);
        }
        match self.decode_response(raw, response_communication, expected) {
            Ok((body, integrity)) => Ok((body, sw, integrity)),
            Err(error) => {
                self.session = None;
                Err(error)
            }
        }
    }

    fn send_frames(&mut self, ins: u8, data: &[u8]) -> Result<(Vec<u8>, u16)> {
        let (mut body, mut sw) = self.transceive(ins, data)?;
        let mut frames = 0;
        while sw == STATUS_ADDITIONAL {
            frames += 1;
            if frames > 256 {
                return Err(NfcError::InvalidArgument(
                    "DESFire response has too many additional frames".to_owned(),
                ));
            }
            let (next, next_sw) = self.transceive(INS_ADDITIONAL_FRAME, &[])?;
            body.extend_from_slice(&next);
            sw = next_sw;
        }
        Ok((body, sw))
    }

    fn encode_command(
        &mut self,
        ins: u8,
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<Vec<u8>> {
        let Some(session) = self.session.as_ref() else {
            return Ok(data.to_vec());
        };
        if !session.uses_secure_messaging() {
            return Ok(data.to_vec());
        }
        let mac = session.command_mac(ins, data)?;
        if session.key_type == AuthKeyType::AesEv2 {
            if matches!(
                communication,
                CommunicationMode::Maced | CommunicationMode::Enciphered
            ) {
                let mut wire = Vec::with_capacity(data.len() + 8);
                wire.extend_from_slice(data);
                wire.extend_from_slice(&ev2_truncate(&mac));
                return Ok(wire);
            }
            return Ok(data.to_vec());
        }
        // EV1 carries a command MAC only on write commands; read and
        // management commands advance the chaining value without appending it.
        if let Some(session) = self.session.as_mut() {
            session.iv = mac;
        }
        Ok(data.to_vec())
    }

    fn decode_response(
        &mut self,
        raw: Vec<u8>,
        communication: CommunicationMode,
        expected: Option<usize>,
    ) -> Result<(Vec<u8>, IntegrityCheck)> {
        let Some(session) = self.session.as_mut() else {
            return Ok((raw, IntegrityCheck::None));
        };
        if !session.uses_secure_messaging() {
            return Ok((raw, IntegrityCheck::None));
        }
        match session.key_type {
            AuthKeyType::Tdes3 | AuthKeyType::Aes => {
                decode_ev1_response(session, raw, communication, expected)
            }
            AuthKeyType::AesEv2 => decode_ev2_response(session, raw, communication, expected),
            AuthKeyType::Des | AuthKeyType::Tdes2 => Ok((raw, IntegrityCheck::None)),
        }
    }

    /// Encode a file write command with the file's communication mode.
    ///
    /// EV1 appends a command MAC for MACed writes and enciphers the data with
    /// the CRC for Enciphered writes. EV2 appends its truncated CMAC and
    /// enciphers the padded data before that MAC.
    fn encode_write(
        &mut self,
        ins: u8,
        header: &[u8],
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<Vec<u8>> {
        let Some(session) = self.session.as_ref() else {
            if communication != CommunicationMode::Plain {
                return Err(NfcError::AuthenticationRequired(
                    "authenticate to the selected DESFire application first".to_owned(),
                ));
            }
            let mut wire = Vec::with_capacity(header.len() + data.len());
            wire.extend_from_slice(header);
            wire.extend_from_slice(data);
            return Ok(wire);
        };
        if !session.uses_secure_messaging() {
            let mut wire = Vec::with_capacity(header.len() + data.len());
            wire.extend_from_slice(header);
            wire.extend_from_slice(data);
            return Ok(wire);
        }
        match session.key_type {
            AuthKeyType::Tdes3 | AuthKeyType::Aes => {
                self.encode_ev1_write(ins, header, data, communication)
            }
            AuthKeyType::AesEv2 => self.encode_ev2_write(ins, header, data, communication),
            AuthKeyType::Des | AuthKeyType::Tdes2 => unreachable!("legacy sessions return above"),
        }
    }

    fn encode_ev1_write(
        &mut self,
        ins: u8,
        header: &[u8],
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<Vec<u8>> {
        let session = self
            .session
            .as_mut()
            .expect("the caller checked the session");
        let mut wire = Vec::with_capacity(header.len() + data.len() + 8);
        match communication {
            CommunicationMode::Plain | CommunicationMode::Maced => {
                let mut input = Vec::with_capacity(1 + header.len() + data.len());
                input.push(ins);
                input.extend_from_slice(header);
                input.extend_from_slice(data);
                let mac = ev1_cmac(session.key_type, &session.mac_key, &session.iv, &input)?;
                wire.extend_from_slice(header);
                wire.extend_from_slice(data);
                if communication == CommunicationMode::Maced {
                    wire.extend_from_slice(&mac[..8]);
                }
                session.iv = mac;
            }
            CommunicationMode::Enciphered => {
                let mut crc_input = Vec::with_capacity(1 + header.len() + data.len());
                crc_input.push(ins);
                crc_input.extend_from_slice(header);
                crc_input.extend_from_slice(data);
                let crc = crc32_desfire(&crc_input);
                let mut plain = Vec::with_capacity(data.len() + 4 + session.block_size());
                plain.extend_from_slice(data);
                plain.extend_from_slice(&crc);
                while !plain.len().is_multiple_of(session.block_size()) {
                    plain.push(0);
                }
                let block = session.block_size();
                let cipher = match session.key_type {
                    AuthKeyType::Tdes3 => {
                        cbc_encrypt_iv(&expand_tdes_key(&session.enc_key)?, &session.iv, &plain)?
                    }
                    AuthKeyType::Aes => aes_cbc_encrypt(&session.enc_key, &session.iv, &plain)?,
                    _ => unreachable!("EV1 secure messaging needs 3K3DES or AES"),
                };
                session.iv = cipher[cipher.len() - block..].to_vec();
                wire.extend_from_slice(header);
                wire.extend_from_slice(&cipher);
            }
        }
        Ok(wire)
    }

    fn encode_ev2_write(
        &mut self,
        ins: u8,
        header: &[u8],
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<Vec<u8>> {
        let session = self
            .session
            .as_mut()
            .expect("the caller checked the session");
        let mut wire = Vec::with_capacity(header.len() + data.len() + 24);
        match communication {
            CommunicationMode::Plain => {
                wire.extend_from_slice(header);
                wire.extend_from_slice(data);
            }
            CommunicationMode::Maced => {
                wire.extend_from_slice(header);
                wire.extend_from_slice(data);
                let mac = session.command_mac(ins, &wire)?;
                wire.extend_from_slice(&ev2_truncate(&mac));
            }
            CommunicationMode::Enciphered => {
                let mut plain = Vec::with_capacity(data.len() + 16);
                plain.extend_from_slice(data);
                plain.push(0x80);
                while !plain.len().is_multiple_of(16) {
                    plain.push(0);
                }
                let iv = ev2_command_iv(&session.enc_key, &session.ti, session.counter)?;
                let cipher = aes_cbc_encrypt(&session.enc_key, &iv, &plain)?;
                wire.extend_from_slice(header);
                wire.extend_from_slice(&cipher);
                let mac = session.command_mac(ins, &wire)?;
                wire.extend_from_slice(&ev2_truncate(&mac));
            }
        }
        Ok(wire)
    }

    /// Send a file write command and verify the status-only response.
    fn write_command(
        &mut self,
        ins: u8,
        header: &[u8],
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<IntegrityCheck> {
        let wire = match self.encode_write(ins, header, data, communication) {
            Ok(wire) => wire,
            Err(error) => {
                self.session = None;
                return Err(error);
            }
        };
        let response = self.write_response_mode(communication);
        let (body, sw, integrity) = self.exchange_wire(ins, wire, response, Some(0))?;
        if sw != STATUS_OK {
            return Err(status_error(ins, sw));
        }
        if !body.is_empty() {
            return Err(NfcError::VerificationFailed(format!(
                "DESFire write {ins:#04X} returned {} unexpected data bytes",
                body.len()
            )));
        }
        Ok(integrity)
    }

    /// Protection of a status-only write response.
    ///
    /// EV1 MACs every response after authentication. EV2 MACs the response
    /// unless the file runs in Plain mode, where the counter still advances.
    fn write_response_mode(&self, communication: CommunicationMode) -> CommunicationMode {
        let Some(session) = self.session.as_ref() else {
            return CommunicationMode::Plain;
        };
        if !session.uses_secure_messaging() {
            return CommunicationMode::Plain;
        }
        match session.key_type {
            AuthKeyType::Tdes3 | AuthKeyType::Aes => CommunicationMode::Maced,
            AuthKeyType::AesEv2 => {
                if communication == CommunicationMode::Plain {
                    CommunicationMode::Plain
                } else {
                    CommunicationMode::Maced
                }
            }
            AuthKeyType::Des | AuthKeyType::Tdes2 => CommunicationMode::Plain,
        }
    }

    fn ensure_authenticated(&self) -> Result<()> {
        if self.session.is_none() {
            return Err(NfcError::AuthenticationRequired(
                "authenticate to the selected DESFire application first".to_owned(),
            ));
        }
        Ok(())
    }

    /// Authenticate with the key family inferred from the key length.
    ///
    /// A 16-byte key defaults to 2K3DES; AES must be selected explicitly with
    /// [`Self::authenticate`] and [`AuthKeyType::Aes`] or
    /// [`AuthKeyType::AesEv2`].
    pub fn authenticate_legacy(&mut self, key_no: u8, key: &[u8]) -> Result<()> {
        let key_type = AuthKeyType::from_key(key)?;
        self.authenticate(key_no, key, key_type)
    }

    /// Authenticate with an explicit key family.
    ///
    /// EV2 authentication uses key version 0 (current) and derives both the
    /// session ENC and MAC keys. Other families use the session key for both.
    pub fn authenticate(&mut self, key_no: u8, key: &[u8], key_type: AuthKeyType) -> Result<()> {
        if key_no > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        key_type.validate_key(key)?;
        // A new authentication replaces the previous session. Keep no local
        // state if this attempt fails.
        self.session = None;
        let command = key_type.command();
        let challenge_length = key_type.challenge_length();
        let request = if matches!(key_type, AuthKeyType::AesEv2) {
            vec![key_no, 0x00]
        } else {
            vec![key_no]
        };
        let (mut challenge, sw) = self.transceive(command, &request)?;
        if sw != STATUS_ADDITIONAL {
            return Err(status_error(command, sw));
        }
        strip_leading_status(&mut challenge, challenge_length);
        if challenge.len() != challenge_length {
            return Err(NfcError::VerificationFailed(format!(
                "{} authentication returned {} challenge bytes, expected {challenge_length}",
                key_type.name(),
                challenge.len()
            )));
        }
        let rnd_b = decrypt_auth_challenge(key, key_type, &challenge)?;
        if rnd_b.len() != challenge_length {
            return Err(NfcError::VerificationFailed(
                "the authentication challenge did not decrypt to one nonce".to_owned(),
            ));
        }
        let rotated_b = rotate_left(&rnd_b);
        let mut rnd_a = vec![0u8; challenge_length];
        getrandom::fill(&mut rnd_a)
            .map_err(|error| NfcError::Io(format!("random nonce unavailable: {error}")))?;
        let mut plain = Vec::with_capacity(challenge_length * 2);
        plain.extend_from_slice(&rnd_a);
        plain.extend_from_slice(&rotated_b);
        let sent = encrypt_auth_payload(key, key_type, &challenge, &plain)?;
        let (mut response, sw) = self.transceive(INS_ADDITIONAL_FRAME, &sent)?;
        if sw != STATUS_OK {
            return Err(status_error(INS_ADDITIONAL_FRAME, sw));
        }
        let expected = if matches!(key_type, AuthKeyType::AesEv2) {
            32
        } else {
            challenge_length
        };
        strip_leading_status(&mut response, expected);
        if response.len() != expected {
            return Err(NfcError::VerificationFailed(format!(
                "{} authentication returned {} final bytes, expected {expected}",
                key_type.name(),
                response.len()
            )));
        }
        let decrypted = decrypt_auth_response(key, key_type, &sent, &response)?;
        let rotated_a = rotate_left(&rnd_a);
        let (enc_key, mac_key, key_version, ti) = match key_type {
            AuthKeyType::AesEv2 => {
                if decrypted.len() != 32 {
                    return Err(NfcError::VerificationFailed(
                        "EV2 authentication did not return 32 plain bytes".to_owned(),
                    ));
                }
                if decrypted[4..20] != rotated_a {
                    return Err(NfcError::AuthenticationFailed);
                }
                let enc = ev2_session_key(key, &rnd_a, &rnd_b, true)?;
                let mac = ev2_session_key(key, &rnd_a, &rnd_b, false)?;
                let ti: [u8; 4] = decrypted[0..4].try_into().expect("four-byte TI");
                (enc, mac, 0, ti)
            }
            _ => {
                if decrypted.len() != challenge_length || decrypted != rotated_a {
                    return Err(NfcError::AuthenticationFailed);
                }
                let enc = derive_session_key(key_type, &rnd_a, &rnd_b);
                let mac = enc.clone();
                (enc, mac, 0, [0u8; 4])
            }
        };
        let iv = match key_type {
            AuthKeyType::Tdes3 => vec![0u8; 8],
            AuthKeyType::Aes => vec![0u8; 16],
            _ => Vec::new(),
        };
        self.session = Some(Session {
            enc_key,
            mac_key,
            key_type,
            key_no,
            key_version,
            iv,
            ti,
            counter: 0,
        });
        Ok(())
    }

    /// Authenticate and report the session and key version.
    ///
    /// This sends no file command. A wrong key fails once; `authenticate`
    /// never retries. `key_version` is `None` when the card refuses or does
    /// not implement GetKeyVersion for this key.
    pub fn verify_key(
        &mut self,
        key_no: u8,
        key: &[u8],
        key_type: AuthKeyType,
    ) -> Result<KeyVerification> {
        self.authenticate(key_no, key, key_type)?;
        let secure_messaging = self
            .session
            .as_ref()
            .is_some_and(Session::uses_secure_messaging);
        let key_version = self.key_version(key_no)?;
        Ok(KeyVerification {
            key_no,
            key_type,
            secure_messaging,
            key_version,
        })
    }

    /// Read the card's key version after authentication.
    ///
    /// `None` means the card answered `IllegalCommand`, `NoSuchKey`, or
    /// `PermissionDenied`; an invalid response MAC still fails closed and
    /// drops the session.
    pub fn key_version(&mut self, key_no: u8) -> Result<Option<u8>> {
        self.ensure_authenticated()?;
        if key_no > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let communication = self
            .session
            .as_ref()
            .map_or(CommunicationMode::Plain, Session::management_protection);
        let (body, sw, _) =
            self.exchange(INS_GET_KEY_VERSION, &[key_no], communication, Some(1))?;
        match sw {
            STATUS_OK => {}
            0x911C | 0x9140 | STATUS_PERMISSION_DENIED => return Ok(None),
            _ => return Err(status_error(INS_GET_KEY_VERSION, sw)),
        }
        if body.len() != 1 {
            return Err(NfcError::VerificationFailed(format!(
                "GetKeyVersion returned {} bytes, expected 1",
                body.len()
            )));
        }
        Ok(Some(body[0]))
    }

    /// Key number used by the current session.
    pub fn authenticated_key_number(&self) -> Option<u8> {
        self.session.as_ref().map(|session| session.key_no)
    }

    /// Read the KeySettings and key metadata of the selected application.
    ///
    /// GetKeySettings is readable without authentication when the card's
    /// key settings allow free directory access; with a session the command
    /// follows the session's management protection and any response MAC is
    /// verified.
    pub fn key_settings(&mut self) -> Result<KeySettings> {
        if self.session.is_some() {
            let communication = self
                .session
                .as_ref()
                .map_or(CommunicationMode::Plain, Session::management_protection);
            let (body, sw, _) = self.exchange(INS_GET_KEY_SETTINGS, &[], communication, None)?;
            if sw != STATUS_OK {
                return Err(status_error(INS_GET_KEY_SETTINGS, sw));
            }
            KeySettings::parse(&body)
        } else {
            KeySettings::parse(&self.expect_ok(INS_GET_KEY_SETTINGS, &[])?)
        }
    }

    /// Change the KeySettings byte of the selected application.
    ///
    /// The card must be authenticated, and its current KeySettings must have
    /// the configuration-changeable bit set; the card enforces both. This
    /// changes only the KeySettings byte; the key count, key family, and ISO
    /// file ID flags are fixed at application creation.
    pub fn change_key_settings(&mut self, key_settings: u8) -> Result<()> {
        self.ensure_authenticated()?;
        let secure = self
            .session
            .as_ref()
            .is_some_and(Session::uses_secure_messaging);
        if secure {
            // EV1 and EV2 send ChangeKeySettings in the enciphered mode: the
            // KeySettings byte carries its integrity CRC, or the EV2 padding
            // and CMAC, and the card answers with a response MAC. A plain
            // command is refused with LENGTH_ERROR on an EV3 card.
            self.write_command(
                INS_CHANGE_KEY_SETTINGS,
                &[],
                &[key_settings],
                CommunicationMode::Enciphered,
            )?;
            return Ok(());
        }
        let body = self.command(INS_CHANGE_KEY_SETTINGS, &[key_settings])?;
        if !body.is_empty() {
            return Err(NfcError::VerificationFailed(format!(
                "ChangeKeySettings returned {} unexpected data bytes",
                body.len()
            )));
        }
        Ok(())
    }

    /// Change one key of the selected application or the PICC master key.
    ///
    /// The wire encoding follows the authentication channel:
    ///
    /// - DES/2K3DES sessions use the D40 ChangeKey frame: the version is
    ///   embedded in the key parity bits, integrity is CRC16, and the command
    ///   data is CBC-encrypted with the session key and a zero IV.
    /// - 3K3DES and AES EV1 sessions use the EV1 frame: the version is
    ///   embedded (3K3DES) or a separate byte (AES), integrity is CRC32 over
    ///   the command header and data, and the data is CBC-encrypted with the
    ///   session IV.
    /// - AES EV2 sessions use the EV2 frame: the same key data and CRC32 for
    ///   a different key, ISO/IEC 9797-1 padding, the EV2 command IV, and a
    ///   truncated CMAC. A change of the authenticated key ends the session.
    ///
    /// Only a change within the session's key family is supported. A change
    /// of the authenticated key ends the session; the caller must
    /// re-authenticate with the new key.
    pub fn change_key(&mut self, change: KeyChange) -> Result<()> {
        self.ensure_authenticated()?;
        if change.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let session = self.session.as_ref().expect("the session was checked");
        let authenticated = session.key_no;
        let session_type = session.key_type;
        let family = KeyFamily::of(session_type);
        family.validate_key(&change.new_key)?;
        let same_key = change.key_number == authenticated;
        let current = match (same_key, change.current_key.as_deref()) {
            (true, _) => None,
            (false, Some(current)) => {
                family.validate_key(current)?;
                Some(current.to_vec())
            }
            (false, None) => {
                return Err(NfcError::InvalidArgument(format!(
                    "changing key {} while authenticated with key {authenticated} needs that key's current value",
                    change.key_number
                )));
            }
        };
        let mut key_number = change.key_number;
        if self.selected == [0x00; 3] {
            if change.key_number != 0 {
                return Err(NfcError::InvalidArgument(
                    "the PICC has one master key; the key number must be 0".to_owned(),
                ));
            }
            // The upper KeyNo bits select the crypto of a PICC master key.
            key_number |= family.application_key_type().bits() << 6;
        }

        let current = match current {
            Some(mut current) => {
                if family.embeds_version() {
                    match self.key_version(change.key_number)? {
                        Some(version) => set_des_key_version(&mut current, version),
                        None => {
                            return Err(NfcError::InvalidArgument(format!(
                                "the card does not report the key version of key {}; changing another key needs it to reconstruct the stored key",
                                change.key_number
                            )));
                        }
                    }
                }
                Some(current)
            }
            None => None,
        };
        let wire = match session_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                let mut plain = family.key_area(&change.new_key, change.new_key_version);
                xor_key(&mut plain, current.as_deref(), family.key_length());
                let crc = crc16_a(&plain);
                plain.extend_from_slice(&crc);
                if current.is_some() {
                    let plain_crc =
                        crc16_a(&family.key_area(&change.new_key, change.new_key_version));
                    plain.extend_from_slice(&plain_crc);
                }
                let block = family.block_size();
                while !plain.len().is_multiple_of(block) {
                    plain.push(0);
                }
                let session = self.session.as_ref().expect("the session was checked");
                // D40 "send" encrypts with the block decrypt direction.
                d40_send(&expand_tdes_key(&session.enc_key)?, &plain)
            }
            AuthKeyType::Tdes3 | AuthKeyType::Aes => {
                let mut plain = family.key_area(&change.new_key, change.new_key_version);
                xor_key(&mut plain, current.as_deref(), family.key_length());
                let mut crc_input = Vec::with_capacity(2 + plain.len());
                crc_input.push(INS_CHANGE_KEY);
                crc_input.push(key_number);
                crc_input.extend_from_slice(&plain);
                plain.extend_from_slice(&crc32_desfire(&crc_input));
                if current.is_some() {
                    plain.extend_from_slice(&crc32_desfire(
                        &family.crc_key(&change.new_key, change.new_key_version),
                    ));
                }
                let session = self.session.as_mut().expect("the session was checked");
                let block = session.block_size();
                while !plain.len().is_multiple_of(block) {
                    plain.push(0);
                }
                let cipher = match session_type {
                    AuthKeyType::Tdes3 => {
                        cbc_encrypt_iv(&expand_tdes_key(&session.enc_key)?, &session.iv, &plain)?
                    }
                    AuthKeyType::Aes => aes_cbc_encrypt(&session.enc_key, &session.iv, &plain)?,
                    _ => unreachable!("EV1 ChangeKey needs 3K3DES or AES"),
                };
                session.iv = cipher[cipher.len() - block..].to_vec();
                cipher
            }
            AuthKeyType::AesEv2 => {
                let mut plain = family.key_area(&change.new_key, change.new_key_version);
                xor_key(&mut plain, current.as_deref(), family.key_length());
                if current.is_some() {
                    plain.extend_from_slice(&crc32_desfire(
                        &family.crc_key(&change.new_key, change.new_key_version),
                    ));
                }
                plain.push(0x80);
                while !plain.len().is_multiple_of(16) {
                    plain.push(0);
                }
                let session = self.session.as_ref().expect("the session was checked");
                let iv = ev2_command_iv(&session.enc_key, &session.ti, session.counter)?;
                aes_cbc_encrypt(&session.enc_key, &iv, &plain)?
            }
        };
        let mut command = Vec::with_capacity(1 + wire.len() + 8);
        command.push(key_number);
        command.extend_from_slice(&wire);
        if session_type == AuthKeyType::AesEv2 {
            let session = self.session.as_ref().expect("the session was checked");
            let mac = session.command_mac(INS_CHANGE_KEY, &command)?;
            command.extend_from_slice(&ev2_truncate(&mac));
        }
        // Changing the authenticated key ends the session. The card answers
        // the status without a response MAC because there is no key left to
        // verify it with (confirmed on EV1 AES and by AN12343 for EV2 case 2),
        // so drop the session before decoding the status-only response.
        if same_key {
            self.session = None;
        }
        let response_mode = match session_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => CommunicationMode::Plain,
            AuthKeyType::Tdes3 | AuthKeyType::Aes | AuthKeyType::AesEv2 => {
                if same_key {
                    CommunicationMode::Plain
                } else {
                    CommunicationMode::Maced
                }
            }
        };
        let (body, sw, _) = self.exchange_wire(INS_CHANGE_KEY, command, response_mode, Some(0))?;
        if sw != STATUS_OK {
            return Err(status_error(INS_CHANGE_KEY, sw));
        }
        if !body.is_empty() {
            return Err(NfcError::VerificationFailed(format!(
                "ChangeKey for key {} returned {} unexpected data bytes",
                change.key_number,
                body.len()
            )));
        }
        Ok(())
    }

    /// Read the card UID after authentication.
    ///
    /// GetCardUID is always an encrypted response under EV1/EV2 secure
    /// messaging: the UID, CRC32 or padding, and any response MAC are verified
    /// before the UID is returned. A legacy DES/2K3DES session has no response
    /// encryption, so only authentication protects this value.
    pub fn card_uid(&mut self) -> Result<CardUid> {
        self.ensure_authenticated()?;
        let secure = self
            .session
            .as_ref()
            .is_some_and(Session::uses_secure_messaging);
        if !secure {
            let (body, integrity) = self.command_verified(INS_GET_CARD_UID, &[])?;
            return self.uid_result(body, integrity);
        }
        let read = self.read_command(INS_GET_CARD_UID, &[], CommunicationMode::Enciphered, 7)?;
        self.uid_result(read.data, read.integrity)
    }

    fn uid_result(&self, body: Vec<u8>, integrity: IntegrityCheck) -> Result<CardUid> {
        let uid: [u8; 7] = body.as_slice().try_into().map_err(|_| {
            NfcError::VerificationFailed(format!(
                "GetCardUID returned {} bytes, expected 7",
                body.len()
            ))
        })?;
        Ok(CardUid { uid, integrity })
    }

    /// Session ENC key, if a session exists.
    pub fn session_key(&self) -> Option<&[u8]> {
        self.session
            .as_ref()
            .map(|session| session.enc_key.as_slice())
    }

    /// Session MAC key. EV2 derives it separately; other families use the ENC
    /// key.
    pub fn session_mac_key(&self) -> Option<&[u8]> {
        self.session
            .as_ref()
            .map(|session| session.mac_key.as_slice())
    }

    /// Key family used by the current session.
    pub fn session_key_type(&self) -> Option<AuthKeyType> {
        self.session.as_ref().map(|session| session.key_type)
    }

    /// Key version used by the current session.
    pub fn session_key_version(&self) -> Option<u8> {
        self.session.as_ref().map(|session| session.key_version)
    }

    /// Selected application ID, or `[0, 0, 0]` for the PICC master application.
    pub fn selected_application(&self) -> [u8; 3] {
        self.selected
    }

    /// Format the PICC and erase every application.
    ///
    /// This is a destructive factory reset: deleted applications keep their
    /// EEPROM until this command runs, so it is the recovery path for a test
    /// card filled by repeated fixtures. The PICC key returns to the factory
    /// state. It is not used by ordinary workflows.
    pub fn format_picc(&mut self) -> Result<()> {
        self.ensure_authenticated()?;
        self.command(INS_FORMAT_PICC, &[])?;
        self.session = None;
        self.selected = [0x00; 3];
        self.tmi.clear();
        Ok(())
    }

    /// Free EEPROM bytes reported by the card.
    pub fn free_memory(&mut self) -> Result<u32> {
        let body = self.command(INS_GET_FREE_MEMORY, &[])?;
        if body.len() != 3 {
            return Err(NfcError::VerificationFailed(format!(
                "GetFreeMemory returned {} bytes, expected 3",
                body.len()
            )));
        }
        Ok(u24(&body))
    }

    pub fn get_version(&mut self) -> Result<Version> {
        Version::parse(&self.command(INS_GET_VERSION, &[])?)
    }

    pub fn application_ids(&mut self) -> Result<Vec<[u8; 3]>> {
        let body = self.command(INS_GET_APPLICATION_IDS, &[])?;
        if !body.len().is_multiple_of(3) {
            return Err(NfcError::InvalidArgument(format!(
                "DESFire application ID list length {} is not a multiple of 3",
                body.len()
            )));
        }
        let (chunks, _) = body.as_chunks::<3>();
        Ok(chunks
            .iter()
            .map(|chunk| [chunk[0], chunk[1], chunk[2]])
            .collect())
    }

    pub fn select_application(&mut self, aid: [u8; 3]) -> Result<()> {
        // The card drops authentication on select, so the command itself is
        // sent outside the old session. Application selection starts a new
        // transaction, and with it a new Transaction MAC Input.
        self.session = None;
        self.tmi.clear();
        self.expect_ok(INS_SELECT_APPLICATION, &aid)?;
        self.selected = aid;
        Ok(())
    }

    pub fn file_ids(&mut self) -> Result<Vec<u8>> {
        self.command(INS_GET_FILE_IDS, &[])
    }

    pub fn file_settings(&mut self, file_id: u8) -> Result<Vec<u8>> {
        self.command(INS_GET_FILE_SETTINGS, &[file_id])
    }

    /// Create an application while authenticated to the PICC master
    /// application. `key_settings2` carries the key count in its low nibble
    /// and the key type in bits 6-7.
    ///
    /// `iso_file_id` and `df_name` are optional together. When present, the
    /// application gets the ISO file ID and ISO 7816-4 DF name, and the ISO
    /// file ID bit is set in `key_settings2`.
    pub fn create_application(
        &mut self,
        aid: [u8; 3],
        key_settings: u8,
        key_settings2: u8,
        iso_file_id: Option<u16>,
        df_name: &[u8],
    ) -> Result<()> {
        self.ensure_authenticated()?;
        let payload =
            create_application_payload(aid, key_settings, key_settings2, iso_file_id, df_name)?;
        self.command(INS_CREATE_APPLICATION, &payload)?;
        Ok(())
    }

    /// Create an application from a typed key count and key family.
    pub fn create_application_from(&mut self, spec: &ApplicationSpec) -> Result<()> {
        self.create_application(
            spec.aid,
            spec.key_settings,
            spec.key_settings2(),
            spec.iso_file_id,
            &spec.df_name,
        )
    }

    /// Delete an application while authenticated to the PICC master
    /// application.
    pub fn delete_application(&mut self, aid: [u8; 3]) -> Result<()> {
        self.ensure_authenticated()?;
        if aid == [0x00; 3] {
            return Err(NfcError::InvalidArgument(
                "the PICC master application cannot be deleted".to_owned(),
            ));
        }
        self.command(INS_DELETE_APPLICATION, &aid)?;
        if self.selected == aid {
            self.selected = [0x00; 3];
            self.session = None;
        }
        Ok(())
    }

    /// Commit the open transaction so plain writes become persistent.
    ///
    /// `NoChanges` means the card has nothing pending to commit and the
    /// requested state is already stored; it is not an error.
    pub fn commit_transaction(&mut self) -> Result<()> {
        self.commit_transaction_inner(false).map(|_| ())
    }

    /// Commit and ask a Transaction MAC application for the counter and value.
    ///
    /// The option byte is only valid when the selected application has a
    /// Transaction MAC file; the card rejects it otherwise. A response with no
    /// data means the card committed without returning a MAC.
    pub fn commit_transaction_with_mac(&mut self) -> Result<Option<TransactionMac>> {
        self.commit_transaction_inner(true)
    }

    fn commit_transaction_inner(&mut self, return_mac: bool) -> Result<Option<TransactionMac>> {
        let communication = self
            .session
            .as_ref()
            .map_or(CommunicationMode::Plain, Session::management_protection);
        let data: &[u8] = if return_mac { &[0x01] } else { &[] };
        let (body, sw, _) = self.exchange(INS_COMMIT_TRANSACTION, data, communication, None)?;
        if sw != STATUS_OK && sw != STATUS_NO_CHANGES {
            return Err(status_error(INS_COMMIT_TRANSACTION, sw));
        }
        // The commit finalizes the transaction and starts a new one.
        self.tmi.clear();
        if !return_mac {
            return Ok(None);
        }
        match body.as_slice() {
            [] => Ok(None),
            bytes if bytes.len() == 12 => {
                let mut value = [0u8; 8];
                value.copy_from_slice(&bytes[4..]);
                Ok(Some(TransactionMac {
                    counter: u32::from_le_bytes(bytes[..4].try_into().expect("four-byte counter")),
                    value,
                }))
            }
            bytes => Err(NfcError::VerificationFailed(format!(
                "CommitTransaction returned {} bytes of Transaction MAC data, expected 0 or 12",
                bytes.len()
            ))),
        }
    }

    /// Abort the open transaction and discard uncommitted writes.
    pub fn abort_transaction(&mut self) -> Result<()> {
        self.command(INS_ABORT_TRANSACTION, &[])?;
        self.tmi.clear();
        Ok(())
    }

    /// Read the 12-byte counter and MAC stored in a Transaction MAC file.
    ///
    /// This read is excluded from the Transaction MAC Input by definition.
    pub fn read_transaction_mac_file(
        &mut self,
        file_id: u8,
        communication: CommunicationMode,
    ) -> Result<(TransactionMac, IntegrityCheck)> {
        let previous = self.suppress_tmi;
        self.suppress_tmi = true;
        let result = self.read_data(file_id, 0, 12, communication);
        self.suppress_tmi = previous;
        let read = result?;
        if read.data.len() != 12 {
            return Err(NfcError::VerificationFailed(format!(
                "Transaction MAC file {file_id:02X} returned {} bytes, expected 12",
                read.data.len()
            )));
        }
        let mut value = [0u8; 8];
        value.copy_from_slice(&read.data[4..]);
        Ok((
            TransactionMac {
                counter: u32::from_le_bytes(read.data[..4].try_into().expect("four-byte counter")),
                value,
            },
            read.integrity,
        ))
    }

    /// Create a standard data file in the selected application.
    ///
    /// `communication` is 0 for plain, 1 for MACed and 3 for enciphered.
    /// `access_rights` uses the raw DESFire 16-bit encoding. When
    /// `iso_file_id` is present, the file is registered under that ISO 7816-4
    /// file identifier for Type 4 access.
    pub fn create_standard_file(
        &mut self,
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        size: u32,
    ) -> Result<()> {
        self.ensure_authenticated()?;
        let payload =
            create_standard_file_payload(file_id, iso_file_id, communication, access_rights, size)?;
        self.command(INS_CREATE_STD_DATA_FILE, &payload)?;
        Ok(())
    }

    /// Create a backup data file in the selected application.
    ///
    /// Backup files keep writes pending until `CommitTransaction`.
    pub fn create_backup_file(
        &mut self,
        file_id: u8,
        iso_file_id: Option<u16>,
        communication: u8,
        access_rights: u16,
        size: u32,
    ) -> Result<()> {
        self.ensure_authenticated()?;
        let payload =
            create_standard_file_payload(file_id, iso_file_id, communication, access_rights, size)?;
        self.command(INS_CREATE_BACKUP_DATA_FILE, &payload)?;
        Ok(())
    }

    /// Create a value file in the selected application.
    ///
    /// Limits and the initial value use the signed 32-bit little-endian
    /// representation from the DESFire file settings. `flags` uses bit 0 for
    /// limited credit and bit 1 for free GetValue.
    pub fn create_value_file(&mut self, spec: ValueFileSpec) -> Result<()> {
        self.ensure_authenticated()?;
        let payload = create_value_file_payload(&spec)?;
        self.command(INS_CREATE_VALUE_FILE, &payload)?;
        Ok(())
    }

    /// Create a linear or cyclic record file in the selected application.
    pub fn create_record_file(&mut self, spec: RecordFileSpec) -> Result<()> {
        self.ensure_authenticated()?;
        let payload = create_record_file_payload(&spec)?;
        let instruction = if spec.cyclic {
            INS_CREATE_CYCLIC_RECORD_FILE
        } else {
            INS_CREATE_LINEAR_RECORD_FILE
        };
        self.command(instruction, &payload)?;
        Ok(())
    }

    /// Create a Transaction MAC file in the selected application.
    ///
    /// The card updates the separate AppTransactionMACKey with this command,
    /// so the key and version are sent in an enciphered secure-messaging
    /// command. A legacy DES/2K3DES session cannot protect them and is
    /// refused.
    pub fn create_transaction_mac_file(&mut self, spec: &TransactionMacFileSpec) -> Result<()> {
        self.ensure_authenticated()?;
        if !self
            .session
            .as_ref()
            .is_some_and(Session::uses_secure_messaging)
        {
            return Err(NfcError::Unsupported(
                "a Transaction MAC file needs an AES or 3K3DES secure-messaging session",
            ));
        }
        let (header, data) = transaction_mac_file_parts(spec)?;
        self.write_command(
            INS_CREATE_TRANSACTION_MAC_FILE,
            &header,
            &data,
            CommunicationMode::Enciphered,
        )?;
        Ok(())
    }

    /// Delete one file from the selected application.
    pub fn delete_file(&mut self, file_id: u8) -> Result<()> {
        self.ensure_authenticated()?;
        validate_file_id(file_id)?;
        self.command(INS_DELETE_FILE, &[file_id])?;
        Ok(())
    }

    /// Change a file's communication mode and primary access rights.
    ///
    /// `protection` is chosen from the file's change access right: a key
    /// right needs an authenticated enciphered command, while free access
    /// needs a plain command without authentication. Standard and Backup
    /// files accept a new communication mode. Value, record, and Transaction
    /// MAC files accept only access-right changes and must keep their current
    /// mode.
    pub fn change_file_settings(
        &mut self,
        file_id: u8,
        communication: u8,
        access_rights: u16,
        protection: CommunicationMode,
    ) -> Result<()> {
        let payload = change_file_settings_payload(file_id, communication, access_rights)?;
        let (header, data) = payload.split_at(1);
        match protection {
            CommunicationMode::Plain => {
                if self.session.is_some() {
                    return Err(NfcError::InvalidArgument(
                        "a free ChangeFileSettings right needs no authentication".to_owned(),
                    ));
                }
                self.write_command(
                    INS_CHANGE_FILE_SETTINGS,
                    header,
                    data,
                    CommunicationMode::Plain,
                )?;
            }
            CommunicationMode::Enciphered => {
                self.ensure_authenticated()?;
                if !self
                    .session
                    .as_ref()
                    .is_some_and(Session::uses_secure_messaging)
                {
                    return Err(NfcError::Unsupported(
                        "ChangeFileSettings with a key right needs an AES or 3K3DES session",
                    ));
                }
                self.write_command(
                    INS_CHANGE_FILE_SETTINGS,
                    header,
                    data,
                    CommunicationMode::Enciphered,
                )?;
            }
            CommunicationMode::Maced => {
                return Err(NfcError::InvalidArgument(
                    "ChangeFileSettings uses plain or enciphered communication".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Read the signed current value of a value file.
    ///
    /// `communication` is the file's communication mode from its settings.
    pub fn get_value(
        &mut self,
        file_id: u8,
        communication: CommunicationMode,
    ) -> Result<VerifiedValue> {
        let read = self.read_command(INS_GET_VALUE, &[file_id], communication, 4)?;
        let value = i32::from_le_bytes(read.data[..4].try_into().expect("four-byte value"));
        let mut header = Vec::with_capacity(6);
        header.push(INS_GET_VALUE);
        header.push(file_id);
        header.extend_from_slice(&read.data[..4]);
        self.tmi_header(&header);
        Ok(VerifiedValue {
            value,
            integrity: read.integrity,
        })
    }

    /// Read `count` records starting at `start_record`.
    ///
    /// `record_size` is used to validate the protected response length when
    /// the file is enciphered.
    pub fn read_records(
        &mut self,
        file_id: u8,
        start_record: u32,
        count: u32,
        record_size: u32,
        communication: CommunicationMode,
    ) -> Result<VerifiedData> {
        if start_record > 0x00FF_FFFF || count == 0 || count > 0x00FF_FFFF {
            return Err(NfcError::InvalidArgument(
                "DESFire ReadRecords needs a 24-bit record number and a positive record count"
                    .to_owned(),
            ));
        }
        if record_size == 0 {
            return Err(NfcError::InvalidArgument(
                "a DESFire record read needs the record size".to_owned(),
            ));
        }
        let expected =
            usize::try_from(u64::from(count) * u64::from(record_size)).map_err(|_| {
                NfcError::InvalidArgument("the requested record bytes do not fit memory".to_owned())
            })?;
        let read = self.read_command(
            INS_READ_RECORDS,
            &[
                file_id,
                start_record as u8,
                (start_record >> 8) as u8,
                (start_record >> 16) as u8,
                count as u8,
                (count >> 8) as u8,
                (count >> 16) as u8,
            ],
            communication,
            expected,
        )?;
        let mut header = Vec::with_capacity(8);
        header.push(INS_READ_RECORDS);
        header.push(file_id);
        header.extend_from_slice(&start_record.to_le_bytes()[..3]);
        header.extend_from_slice(&count.to_le_bytes()[..3]);
        self.tmi_data(&header, &read.data);
        Ok(read)
    }

    /// Write one record slot at a byte offset inside that record.
    ///
    /// `offset` is an offset within the current record, not a record number.
    /// One call writes the current slot and advances the record counter; use
    /// offset 0 and the full record data to append a complete record. The card
    /// rejects an offset or length beyond the record size. `communication` is
    /// the file's communication mode.
    pub fn write_record(
        &mut self,
        file_id: u8,
        offset: u32,
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<IntegrityCheck> {
        if offset > 0x00FF_FFFF {
            return Err(NfcError::InvalidArgument(
                "DESFire record offset must fit 3 bytes".to_owned(),
            ));
        }
        if data.is_empty() || data.len() > 248 {
            return Err(NfcError::InvalidArgument(
                "a DESFire record write is 1..=248 bytes".to_owned(),
            ));
        }
        let mut header = Vec::with_capacity(7);
        header.push(file_id);
        header.extend_from_slice(&offset.to_le_bytes()[..3]);
        header.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
        let integrity = self.write_command(INS_WRITE_RECORD, &header, data, communication)?;
        let mut tmi_header = Vec::with_capacity(8);
        tmi_header.push(INS_WRITE_RECORD);
        tmi_header.extend_from_slice(&header);
        self.tmi_data(&tmi_header, data);
        Ok(integrity)
    }

    /// Credit, debit, or limited-credit a value file.
    ///
    /// The amount is an unsigned magnitude sent little-endian; the card
    /// applies it as a signed increase or decrease. Value changes stay pending
    /// until CommitTransaction.
    pub fn adjust_value(
        &mut self,
        file_id: u8,
        adjustment: ValueAdjustment,
        amount: u32,
        communication: CommunicationMode,
    ) -> Result<IntegrityCheck> {
        let mut header = Vec::with_capacity(5);
        header.push(file_id);
        header.extend_from_slice(&amount.to_le_bytes());
        let instruction = adjustment.instruction();
        let integrity =
            self.write_command(instruction, &header[..1], &header[1..], communication)?;
        let mut tmi_header = Vec::with_capacity(6);
        tmi_header.push(instruction);
        tmi_header.extend_from_slice(&header);
        self.tmi_header(&tmi_header);
        Ok(integrity)
    }

    /// Update a byte range of an existing record.
    ///
    /// `record_number` follows the same numbering as ReadRecords: 0 is the
    /// most recent record, and the number is limited by the current record
    /// count. One UpdateRecord is allowed per file and transaction.
    pub fn update_record(
        &mut self,
        file_id: u8,
        record_number: u32,
        offset: u32,
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<IntegrityCheck> {
        if record_number > 0x00FF_FFFF || offset > 0x00FF_FFFF {
            return Err(NfcError::InvalidArgument(
                "DESFire record number and offset must fit 3 bytes".to_owned(),
            ));
        }
        if data.is_empty() {
            return Err(NfcError::InvalidArgument(
                "a DESFire record update needs at least one byte".to_owned(),
            ));
        }
        let mut header = Vec::with_capacity(10);
        header.push(file_id);
        header.extend_from_slice(&record_number.to_le_bytes()[..3]);
        header.extend_from_slice(&offset.to_le_bytes()[..3]);
        header.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
        let integrity = self.write_command(INS_UPDATE_RECORD, &header, data, communication)?;
        let mut tmi_header = Vec::with_capacity(11);
        tmi_header.push(INS_UPDATE_RECORD);
        tmi_header.extend_from_slice(&header);
        self.tmi_data(&tmi_header, data);
        Ok(integrity)
    }

    /// Erase every record of a record file.
    ///
    /// The clearance is pending until CommitTransaction. ClearRecordFile has no
    /// command data of its own: under EV2 secure messaging it is a MACed
    /// command, while under EV1 and legacy sessions the command is Plain and
    /// only the response is MACed. Sending the other mode is rejected with a
    /// length error on hardware.
    pub fn clear_record_file(&mut self, file_id: u8) -> Result<IntegrityCheck> {
        let communication = match self.session.as_ref().map(|session| session.key_type) {
            Some(AuthKeyType::AesEv2) => CommunicationMode::Maced,
            _ => CommunicationMode::Plain,
        };
        let integrity =
            self.write_command(INS_CLEAR_RECORD_FILE, &[file_id], &[], communication)?;
        let header = [INS_CLEAR_RECORD_FILE, file_id];
        self.tmi_header(&header);
        Ok(integrity)
    }

    /// Write data to a standard or backup file with the file's communication
    /// mode.
    pub fn write_data(
        &mut self,
        file_id: u8,
        offset: u32,
        data: &[u8],
        communication: CommunicationMode,
    ) -> Result<IntegrityCheck> {
        if data.is_empty() {
            return Err(NfcError::InvalidArgument(
                "DESFire write data must not be empty".to_owned(),
            ));
        }
        if offset > 0x00FF_FFFF || data.len() > 0x00FF_FFFF {
            return Err(NfcError::InvalidArgument(
                "DESFire write offset and length must fit 3 bytes".to_owned(),
            ));
        }
        if data.len() > 248 {
            return Err(NfcError::InvalidArgument(
                "a plain DESFire write is at most 248 bytes per APDU".to_owned(),
            ));
        }
        let mut header = Vec::with_capacity(7);
        header.push(file_id);
        header.extend_from_slice(&offset.to_le_bytes()[..3]);
        header.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
        let integrity = self.write_command(INS_WRITE_DATA, &header, data, communication)?;
        let mut tmi_header = Vec::with_capacity(8);
        tmi_header.push(INS_WRITE_DATA);
        tmi_header.extend_from_slice(&header);
        self.tmi_data(&tmi_header, data);
        Ok(integrity)
    }

    /// Read data from a standard or backup file.
    ///
    /// `communication` is the file's communication mode from its settings.
    /// An enciphered response is decrypted and its integrity is checked before
    /// any byte is returned.
    pub fn read_data(
        &mut self,
        file_id: u8,
        offset: u32,
        length: u32,
        communication: CommunicationMode,
    ) -> Result<VerifiedData> {
        if offset > 0x00FF_FFFF || length > 0x00FF_FFFF {
            return Err(NfcError::InvalidArgument(
                "DESFire read offset and length must fit 3 bytes".to_owned(),
            ));
        }
        if length == 0 {
            return Err(NfcError::InvalidArgument(
                "a DESFire read needs a positive length".to_owned(),
            ));
        }
        let expected = length as usize;
        let read = self.read_command(
            INS_READ_DATA,
            &[
                file_id,
                offset as u8,
                (offset >> 8) as u8,
                (offset >> 16) as u8,
                length as u8,
                (length >> 8) as u8,
                (length >> 16) as u8,
            ],
            communication,
            expected,
        )?;
        let mut header = Vec::with_capacity(8);
        header.push(INS_READ_DATA);
        header.push(file_id);
        header.extend_from_slice(&offset.to_le_bytes()[..3]);
        header.extend_from_slice(&length.to_le_bytes()[..3]);
        self.tmi_data(&header, &read.data);
        Ok(read)
    }

    /// Total data size for a standard or backup file from its settings.
    ///
    /// Returns `None` for value, record and Transaction MAC files, whose
    /// contents use different commands.
    pub fn data_file_size(settings: &[u8]) -> Option<u32> {
        if settings.len() < 7 || !matches!(settings[0], 0x00 | 0x01) {
            return None;
        }
        Some(
            u32::from(settings[4]) | (u32::from(settings[5]) << 8) | (u32::from(settings[6]) << 16),
        )
    }

    /// Inventory without authentication. Applications are selected one by one;
    /// free-directory-access cards expose their file list, otherwise the list is
    /// reported as empty.
    pub fn inventory(&mut self) -> Result<Inventory> {
        let version = self.get_version()?;
        let aids = self.application_ids()?;
        let mut applications = Vec::with_capacity(aids.len());
        for aid in aids {
            self.select_application(aid)?;
            let file_ids = self.file_ids().unwrap_or_default();
            let mut files = Vec::with_capacity(file_ids.len());
            for file_id in file_ids {
                let settings = self.file_settings(file_id).unwrap_or_default();
                files.push(FileInfo { file_id, settings });
            }
            applications.push(Application { aid, files });
        }
        Ok(Inventory {
            version,
            applications,
        })
    }
}

/// Read the card inventory without authentication.
pub fn inventory(reader: &mut dyn Reader) -> Result<Inventory> {
    Desfire::new(reader).inventory()
}

fn create_application_payload(
    aid: [u8; 3],
    key_settings: u8,
    key_settings2: u8,
    iso_file_id: Option<u16>,
    df_name: &[u8],
) -> Result<Vec<u8>> {
    if aid == [0x00; 3] {
        return Err(NfcError::InvalidArgument(
            "the PICC master application ID cannot be created".to_owned(),
        ));
    }
    if !(1..=14).contains(&(key_settings2 & 0x0F)) {
        return Err(NfcError::InvalidArgument(
            "a DESFire application has 1..=14 keys".to_owned(),
        ));
    }
    if df_name.len() > 16 {
        return Err(NfcError::InvalidArgument(
            "a DESFire DF name is at most 16 bytes".to_owned(),
        ));
    }
    if iso_file_id.is_some() != !df_name.is_empty() {
        return Err(NfcError::InvalidArgument(
            "a DESFire application needs both or neither of an ISO file ID and a DF name"
                .to_owned(),
        ));
    }
    if iso_file_id.is_none() && key_settings2 & 0x20 != 0 {
        return Err(NfcError::InvalidArgument(
            "the ISO file ID key-setting bit needs an ISO file ID and DF name".to_owned(),
        ));
    }
    let mut payload = Vec::with_capacity(7 + df_name.len());
    payload.extend_from_slice(&[aid[0], aid[1], aid[2], key_settings]);
    match iso_file_id {
        Some(iso_file_id) if iso_file_id != 0 => {
            payload.push(key_settings2 | 0x20);
            payload.extend_from_slice(&iso_file_id.to_le_bytes());
            payload.extend_from_slice(df_name);
        }
        Some(_) => {
            return Err(NfcError::InvalidArgument(
                "a DESFire application ISO file ID must not be zero".to_owned(),
            ));
        }
        None => payload.push(key_settings2),
    }
    Ok(payload)
}

fn validate_file_id(file_id: u8) -> Result<()> {
    if file_id > 0x1F {
        return Err(NfcError::InvalidArgument(format!(
            "DESFire file numbers are 0..=31, got {file_id}"
        )));
    }
    Ok(())
}

fn create_file_prefix(
    file_id: u8,
    iso_file_id: Option<u16>,
    communication: u8,
    access_rights: u16,
) -> Result<Vec<u8>> {
    validate_file_id(file_id)?;
    if !matches!(communication, 0 | 1 | 3) {
        return Err(NfcError::InvalidArgument(
            "DESFire communication mode must be 0, 1 or 3".to_owned(),
        ));
    }
    let mut payload = Vec::with_capacity(9);
    payload.push(file_id);
    if let Some(iso_file_id) = iso_file_id {
        if iso_file_id == 0 {
            return Err(NfcError::InvalidArgument(
                "a DESFire file ISO file ID must not be zero".to_owned(),
            ));
        }
        payload.extend_from_slice(&iso_file_id.to_le_bytes());
    }
    payload.push(communication);
    payload.extend_from_slice(&access_rights.to_le_bytes());
    Ok(payload)
}

fn create_standard_file_payload(
    file_id: u8,
    iso_file_id: Option<u16>,
    communication: u8,
    access_rights: u16,
    size: u32,
) -> Result<Vec<u8>> {
    if size == 0 || size > 0x00FF_FFFF {
        return Err(NfcError::InvalidArgument(
            "DESFire file size must be 1..=0xFFFFFF bytes".to_owned(),
        ));
    }
    let mut payload = create_file_prefix(file_id, iso_file_id, communication, access_rights)?;
    payload.extend_from_slice(&size.to_le_bytes()[..3]);
    Ok(payload)
}

fn create_value_file_payload(spec: &ValueFileSpec) -> Result<Vec<u8>> {
    if spec.lower_limit >= spec.upper_limit {
        return Err(NfcError::InvalidArgument(
            "DESFire value limits must be increasing".to_owned(),
        ));
    }
    if spec.value < spec.lower_limit || spec.value > spec.upper_limit {
        return Err(NfcError::InvalidArgument(
            "the DESFire value must be inside its limits".to_owned(),
        ));
    }
    if spec.flags & !0x03 != 0 {
        return Err(NfcError::InvalidArgument(
            "DESFire value flags use reserved bits".to_owned(),
        ));
    }
    let mut payload = create_file_prefix(
        spec.file_id,
        spec.iso_file_id,
        spec.communication,
        spec.access_rights,
    )?;
    payload.extend_from_slice(&spec.lower_limit.to_le_bytes());
    payload.extend_from_slice(&spec.upper_limit.to_le_bytes());
    payload.extend_from_slice(&spec.value.to_le_bytes());
    payload.push(spec.flags);
    Ok(payload)
}

fn create_record_file_payload(spec: &RecordFileSpec) -> Result<Vec<u8>> {
    if spec.record_size == 0 || spec.record_size > 0x00FF_FFFF {
        return Err(NfcError::InvalidArgument(
            "DESFire record size must be 1..=0xFFFFFF bytes".to_owned(),
        ));
    }
    if spec.max_records == 0 || spec.max_records > 0x00FF_FFFF {
        return Err(NfcError::InvalidArgument(
            "DESFire maximum record count must be 1..=0xFFFFFF".to_owned(),
        ));
    }
    let mut payload = create_file_prefix(
        spec.file_id,
        spec.iso_file_id,
        spec.communication,
        spec.access_rights,
    )?;
    payload.extend_from_slice(&spec.record_size.to_le_bytes()[..3]);
    payload.extend_from_slice(&spec.max_records.to_le_bytes()[..3]);
    Ok(payload)
}

fn transaction_mac_file_parts(spec: &TransactionMacFileSpec) -> Result<(Vec<u8>, Vec<u8>)> {
    let rights = AccessRights::from_raw(spec.access_rights);
    if rights.write != AccessRight::Denied {
        return Err(NfcError::InvalidArgument(
            "a Transaction MAC file refuses direct writes; set the write right to denied"
                .to_owned(),
        ));
    }
    if !matches!(
        rights.read_write,
        AccessRight::Free | AccessRight::Denied | AccessRight::Key(0..=4)
    ) {
        return Err(NfcError::InvalidArgument(
            "a Transaction MAC file read/write right is a key 0..=4, free, or denied".to_owned(),
        ));
    }
    let mut header = create_file_prefix(
        spec.file_id,
        spec.iso_file_id,
        spec.communication,
        spec.access_rights,
    )?;
    // TMACKeyOption 0x02: separate AES-128 Transaction MAC key, no counter
    // limit. Counter-limit creation is not offered because no reference
    // implementation writes that optional field consistently.
    header.push(0x02);
    let mut data = Vec::with_capacity(17);
    data.extend_from_slice(&spec.key);
    data.push(spec.key_version);
    Ok((header, data))
}

fn change_file_settings_payload(
    file_id: u8,
    communication: u8,
    access_rights: u16,
) -> Result<Vec<u8>> {
    validate_file_id(file_id)?;
    if !matches!(communication, 0 | 1 | 3) {
        return Err(NfcError::InvalidArgument(
            "DESFire communication mode must be 0, 1 or 3".to_owned(),
        ));
    }
    let mut payload = Vec::with_capacity(4);
    payload.push(file_id);
    payload.push(communication);
    payload.extend_from_slice(&access_rights.to_le_bytes());
    Ok(payload)
}

fn expand_tdes_key(key: &[u8]) -> Result<[u8; 24]> {
    let mut expanded = [0u8; 24];
    match key.len() {
        8 => {
            expanded[0..8].copy_from_slice(key);
            expanded[8..16].copy_from_slice(key);
            expanded[16..24].copy_from_slice(key);
        }
        16 => {
            expanded[0..16].copy_from_slice(key);
            expanded[16..24].copy_from_slice(&key[0..8]);
        }
        24 => expanded.copy_from_slice(key),
        _ => {
            return Err(NfcError::InvalidArgument(
                "DESFire legacy keys are 8-byte DES, 16-byte 2K3DES, or 24-byte 3K3DES".to_owned(),
            ));
        }
    }
    Ok(expanded)
}

/// D40 send mode: XOR the plaintext with the previous ciphertext block, then
/// decrypt it with the 3DES block cipher.
fn d40_send(key: &[u8; 24], data: &[u8]) -> Vec<u8> {
    let cipher = TdesEde3::new_from_slice(key).expect("expanded key length is 24");
    let mut previous = [0u8; 8];
    let mut output = Vec::with_capacity(data.len());
    for block in data.as_chunks::<8>().0 {
        let mut current = [0u8; 8];
        for index in 0..8 {
            current[index] = block[index] ^ previous[index];
        }
        cipher.decrypt_block((&mut current).into());
        output.extend_from_slice(&current);
        previous = current;
    }
    output
}

fn cbc_decrypt(key: &[u8; 24], data: &[u8]) -> Result<Vec<u8>> {
    cbc_decrypt_iv(key, &[0u8; 8], data)
}

fn cbc_decrypt_iv(key: &[u8; 24], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if iv.len() != 8 || data.is_empty() || !data.len().is_multiple_of(8) {
        return Err(NfcError::InvalidArgument(
            "DESFire CBC input must be a non-empty multiple of 8 bytes with an 8-byte IV"
                .to_owned(),
        ));
    }
    let cipher = TdesEde3::new_from_slice(key)
        .map_err(|_| NfcError::InvalidArgument("invalid 3DES key".to_owned()))?;
    let mut previous = [0u8; 8];
    previous.copy_from_slice(iv);
    let mut output = Vec::with_capacity(data.len());
    for block in data.as_chunks::<8>().0 {
        let mut current = [0u8; 8];
        current.copy_from_slice(block);
        cipher.decrypt_block((&mut current).into());
        for index in 0..8 {
            output.push(current[index] ^ previous[index]);
        }
        previous.copy_from_slice(block);
    }
    Ok(output)
}

fn cbc_encrypt_iv(key: &[u8; 24], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if iv.len() != 8 || data.is_empty() || !data.len().is_multiple_of(8) {
        return Err(NfcError::InvalidArgument(
            "DESFire CBC input must be a non-empty multiple of 8 bytes with an 8-byte IV"
                .to_owned(),
        ));
    }
    let cipher = TdesEde3::new_from_slice(key)
        .map_err(|_| NfcError::InvalidArgument("invalid 3DES key".to_owned()))?;
    let mut previous = [0u8; 8];
    previous.copy_from_slice(iv);
    let mut output = Vec::with_capacity(data.len());
    for block in data.as_chunks::<8>().0 {
        let mut current = [0u8; 8];
        for index in 0..8 {
            current[index] = block[index] ^ previous[index];
        }
        cipher.encrypt_block((&mut current).into());
        output.extend_from_slice(&current);
        previous = current;
    }
    Ok(output)
}

fn aes_cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if key.len() != 16 || iv.len() != 16 || data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(NfcError::InvalidArgument(
            "AES-CBC input must be a non-empty multiple of 16 bytes with a 16-byte key and IV"
                .to_owned(),
        ));
    }
    let cipher = Aes128::new_from_slice(key)
        .map_err(|_| NfcError::InvalidArgument("invalid AES-128 key".to_owned()))?;
    let mut previous = [0u8; 16];
    previous.copy_from_slice(iv);
    let mut output = Vec::with_capacity(data.len());
    for block in data.as_chunks::<16>().0 {
        let mut current = [0u8; 16];
        current.copy_from_slice(block);
        cipher.decrypt_block((&mut current).into());
        for index in 0..16 {
            output.push(current[index] ^ previous[index]);
        }
        previous.copy_from_slice(block);
    }
    Ok(output)
}

fn aes_cbc_encrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if key.len() != 16 || iv.len() != 16 || data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(NfcError::InvalidArgument(
            "AES-CBC input must be a non-empty multiple of 16 bytes with a 16-byte key and IV"
                .to_owned(),
        ));
    }
    let cipher = Aes128::new_from_slice(key)
        .map_err(|_| NfcError::InvalidArgument("invalid AES-128 key".to_owned()))?;
    let mut previous = [0u8; 16];
    previous.copy_from_slice(iv);
    let mut output = Vec::with_capacity(data.len());
    for block in data.as_chunks::<16>().0 {
        let mut current = [0u8; 16];
        for index in 0..16 {
            current[index] = block[index] ^ previous[index];
        }
        cipher.encrypt_block((&mut current).into());
        output.extend_from_slice(&current);
        previous = current;
    }
    Ok(output)
}

/// Block cipher used by the DESFire CMAC.
trait CmacCipher {
    const BLOCK_SIZE: usize;

    fn new(key: &[u8]) -> Result<Self>
    where
        Self: Sized;

    fn encrypt(&self, block: &mut [u8]);
}

impl CmacCipher for TdesEde3 {
    const BLOCK_SIZE: usize = 8;

    fn new(key: &[u8]) -> Result<Self> {
        TdesEde3::new_from_slice(key)
            .map_err(|_| NfcError::InvalidArgument("invalid 3DES key".to_owned()))
    }

    fn encrypt(&self, block: &mut [u8]) {
        let mut value = [0u8; 8];
        value.copy_from_slice(block);
        BlockCipherEncrypt::encrypt_block(self, (&mut value).into());
        block.copy_from_slice(&value);
    }
}

impl CmacCipher for Aes128 {
    const BLOCK_SIZE: usize = 16;

    fn new(key: &[u8]) -> Result<Self> {
        Aes128::new_from_slice(key)
            .map_err(|_| NfcError::InvalidArgument("invalid AES-128 key".to_owned()))
    }

    fn encrypt(&self, block: &mut [u8]) {
        let mut value = [0u8; 16];
        value.copy_from_slice(block);
        BlockCipherEncrypt::encrypt_block(self, (&mut value).into());
        block.copy_from_slice(&value);
    }
}

/// CMAC (NIST SP 800-38B) with an explicit starting chaining value.
///
/// DESFire EV1 chains the CMAC result from one command/response into the next,
/// so its IV is not the usual all-zero block.
fn cmac_chained<C: CmacCipher>(key: &[u8], iv: &[u8], data: &[u8], rb: u8) -> Result<Vec<u8>> {
    let block_size = C::BLOCK_SIZE;
    if iv.len() != block_size {
        return Err(NfcError::InvalidArgument(format!(
            "CMAC needs a {block_size}-byte chaining value"
        )));
    }
    let cipher = C::new(key)?;
    let mut zero = vec![0u8; block_size];
    cipher.encrypt(&mut zero);
    let k1 = cmac_subkey(&zero, rb);
    let k2 = cmac_subkey(&k1, rb);

    let mut blocks = data.to_vec();
    if data.is_empty() || !data.len().is_multiple_of(block_size) {
        blocks.push(0x80);
        while !blocks.len().is_multiple_of(block_size) {
            blocks.push(0x00);
        }
        let last = blocks.len() - block_size;
        for (index, byte) in k2.iter().enumerate() {
            blocks[last + index] ^= byte;
        }
    } else {
        let last = blocks.len() - block_size;
        for (index, byte) in k1.iter().enumerate() {
            blocks[last + index] ^= byte;
        }
    }

    let mut state = iv.to_vec();
    for chunk in blocks.chunks_exact(block_size) {
        for index in 0..block_size {
            state[index] ^= chunk[index];
        }
        cipher.encrypt(&mut state);
    }
    Ok(state)
}

fn cmac_subkey(block: &[u8], rb: u8) -> Vec<u8> {
    let mut output = vec![0u8; block.len()];
    let mut carry = 0u8;
    for index in (0..block.len()).rev() {
        output[index] = (block[index] << 1) | carry;
        carry = block[index] >> 7;
    }
    if block[0] & 0x80 != 0 {
        output[block.len() - 1] ^= rb;
    }
    output
}

fn cmac_aes(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if key.len() != 16 {
        return Err(NfcError::InvalidArgument(
            "AES-CMAC needs a 16-byte key".to_owned(),
        ));
    }
    cmac_chained::<Aes128>(key, &[0u8; 16], data, 0x87)
}

/// Compute the 8-byte Transaction MAC Value for one transaction.
///
/// `counter` is the incremented Transaction MAC Counter stored by the commit.
/// The session key is derived from the AppTransactionMACKey over
/// `5A 00 01 00 80 || counter || UID`, then the CMAC over `tmi` is truncated to
/// the even-numbered bytes like AES secure messaging (MF2DLHX0 10.3.2.3 and
/// 10.3.2.4).
pub fn transaction_mac(
    tmac_key: &[u8; 16],
    counter: u32,
    uid: &[u8; 7],
    tmi: &[u8],
) -> Result<[u8; 8]> {
    let mut derivation = Vec::with_capacity(16);
    derivation.extend_from_slice(&[0x5A, 0x00, 0x01, 0x00, 0x80]);
    derivation.extend_from_slice(&counter.to_le_bytes());
    derivation.extend_from_slice(uid);
    debug_assert_eq!(derivation.len(), 16);
    let session_key = cmac_aes(tmac_key, &derivation)?;
    let mac = cmac_aes(&session_key, tmi)?;
    Ok(ev2_truncate(&mac))
}

fn ev1_cmac(key_type: AuthKeyType, key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    match key_type {
        AuthKeyType::Tdes3 => cmac_chained::<TdesEde3>(key, iv, data, 0x1B),
        AuthKeyType::Aes => cmac_chained::<Aes128>(key, iv, data, 0x87),
        _ => Err(NfcError::InvalidArgument(
            "EV1 secure messaging needs a 3K3DES or AES key".to_owned(),
        )),
    }
}

fn split_mac(raw: &[u8]) -> Result<(&[u8], &[u8])> {
    if raw.len() < 8 {
        return Err(NfcError::VerificationFailed(format!(
            "secure-messaging response is {} bytes, shorter than the 8-byte MAC",
            raw.len()
        )));
    }
    Ok(raw.split_at(raw.len() - 8))
}

/// EV2 truncates the 16-byte AES-CMAC to every second byte, starting with the
/// second byte (AN12343 figure 5).
fn ev2_truncate(mac: &[u8]) -> [u8; 8] {
    let mut output = [0u8; 8];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = mac[index * 2 + 1];
    }
    output
}

fn mac_matches(received: &[u8], computed: &[u8]) -> bool {
    received.len() == 8
        && computed.len() >= 8
        && received
            .iter()
            .zip(computed)
            .fold(0u8, |difference, (left, right)| difference | (left ^ right))
            == 0
}

fn decode_ev1_response(
    session: &mut Session,
    raw: Vec<u8>,
    communication: CommunicationMode,
    expected: Option<usize>,
) -> Result<(Vec<u8>, IntegrityCheck)> {
    match communication {
        CommunicationMode::Plain | CommunicationMode::Maced => {
            let (body, received) = split_mac(&raw)?;
            let computed = session.response_mac(body, 0x00)?;
            if !mac_matches(received, &computed) {
                return Err(NfcError::VerificationFailed(
                    "the EV1 response MAC does not verify".to_owned(),
                ));
            }
            session.iv = computed;
            Ok((body.to_vec(), IntegrityCheck::Ev1Cmac))
        }
        CommunicationMode::Enciphered => {
            let expected = expected.ok_or_else(|| {
                NfcError::InvalidArgument(
                    "an enciphered EV1 response needs the expected plaintext length".to_owned(),
                )
            })?;
            let iv = session.iv.clone();
            let plain = match session.key_type {
                AuthKeyType::Tdes3 => {
                    cbc_decrypt_iv(&expand_tdes_key(&session.enc_key)?, &iv, &raw)?
                }
                _ => aes_cbc_decrypt(&session.enc_key, &iv, &raw)?,
            };
            let block_size = session.block_size();
            if raw.len() < block_size {
                return Err(NfcError::VerificationFailed(
                    "the EV1 enciphered response has no ciphertext block".to_owned(),
                ));
            }
            session.iv = raw[raw.len() - block_size..].to_vec();
            Ok((
                verify_ev1_plaintext(&plain, expected, block_size)?,
                IntegrityCheck::Ev1Crc32,
            ))
        }
    }
}

fn decode_ev2_response(
    session: &mut Session,
    raw: Vec<u8>,
    communication: CommunicationMode,
    expected: Option<usize>,
) -> Result<(Vec<u8>, IntegrityCheck)> {
    let counter = session.counter;
    match communication {
        CommunicationMode::Plain => Ok((raw, IntegrityCheck::None)),
        CommunicationMode::Maced => {
            let (body, received) = split_mac(&raw)?;
            let computed = session.response_mac(body, 0x00)?;
            if !mac_matches(received, &ev2_truncate(&computed)) {
                return Err(NfcError::VerificationFailed(
                    "the EV2 response MAC does not verify".to_owned(),
                ));
            }
            Ok((body.to_vec(), IntegrityCheck::Ev2Cmac))
        }
        CommunicationMode::Enciphered => {
            let expected = expected.ok_or_else(|| {
                NfcError::InvalidArgument(
                    "an enciphered EV2 response needs the expected plaintext length".to_owned(),
                )
            })?;
            let (cipher, received) = split_mac(&raw)?;
            let computed = session.response_mac(cipher, 0x00)?;
            if !mac_matches(received, &ev2_truncate(&computed)) {
                return Err(NfcError::VerificationFailed(
                    "the EV2 response MAC does not verify".to_owned(),
                ));
            }
            let iv = ev2_response_iv(&session.enc_key, &session.ti, counter)?;
            let plain = aes_cbc_decrypt(&session.enc_key, &iv, cipher)?;
            Ok((
                verify_ev2_plaintext(&plain, expected, 16)?,
                IntegrityCheck::Ev2Cmac,
            ))
        }
    }
}

fn ev2_response_iv(enc_key: &[u8], ti: &[u8; 4], counter: u16) -> Result<Vec<u8>> {
    if enc_key.len() != 16 {
        return Err(NfcError::InvalidArgument(
            "EV2 secure messaging needs a 16-byte AES ENC key".to_owned(),
        ));
    }
    let mut input = [0u8; 16];
    input[0] = 0x5A;
    input[1] = 0xA5;
    input[2..6].copy_from_slice(ti);
    input[6..8].copy_from_slice(&counter.to_le_bytes());
    let cipher = Aes128::new_from_slice(enc_key)
        .map_err(|_| NfcError::InvalidArgument("invalid AES-128 key".to_owned()))?;
    cipher.encrypt_block((&mut input).into());
    Ok(input.to_vec())
}

/// EV2 command IV for enciphered command data (AN12343: `A5 5A || TI || Ctr`).
fn ev2_command_iv(enc_key: &[u8], ti: &[u8; 4], counter: u16) -> Result<Vec<u8>> {
    if enc_key.len() != 16 {
        return Err(NfcError::InvalidArgument(
            "EV2 secure messaging needs a 16-byte AES ENC key".to_owned(),
        ));
    }
    let mut input = [0u8; 16];
    input[0] = 0xA5;
    input[1] = 0x5A;
    input[2..6].copy_from_slice(ti);
    input[6..8].copy_from_slice(&counter.to_le_bytes());
    let cipher = Aes128::new_from_slice(enc_key)
        .map_err(|_| NfcError::InvalidArgument("invalid AES-128 key".to_owned()))?;
    cipher.encrypt_block((&mut input).into());
    Ok(input.to_vec())
}

/// EV1 enciphered payload: data, a four-byte CRC32, then zero padding to the
/// block size. The CRC covers the data and the success status byte.
fn verify_ev1_plaintext(plain: &[u8], expected: usize, block_size: usize) -> Result<Vec<u8>> {
    let padded = (expected + 4).div_ceil(block_size) * block_size;
    if plain.len() != padded {
        return Err(NfcError::VerificationFailed(format!(
            "the EV1 enciphered response is {} bytes, expected {padded}",
            plain.len()
        )));
    }
    if plain[expected + 4..].iter().any(|byte| *byte != 0) {
        return Err(NfcError::VerificationFailed(
            "the EV1 enciphered response has non-zero padding".to_owned(),
        ));
    }
    let mut crc_input = Vec::with_capacity(expected + 1);
    crc_input.extend_from_slice(&plain[..expected]);
    crc_input.push(0x00);
    let crc = crc32_desfire(&crc_input);
    if plain[expected..expected + 4] != crc {
        return Err(NfcError::VerificationFailed(
            "the EV1 enciphered response CRC32 does not verify".to_owned(),
        ));
    }
    Ok(plain[..expected].to_vec())
}

/// EV2 enciphered payload: ISO/IEC 9797-1 padding method 2 (0x80 followed by
/// zeros), always adding a padding block when the data is already aligned.
fn verify_ev2_plaintext(plain: &[u8], expected: usize, block_size: usize) -> Result<Vec<u8>> {
    let padded = (expected / block_size + 1) * block_size;
    if plain.len() != padded || expected >= plain.len() {
        return Err(NfcError::VerificationFailed(format!(
            "the EV2 enciphered response is {} bytes, expected {padded}",
            plain.len()
        )));
    }
    if plain[expected] != 0x80 || plain[expected + 1..].iter().any(|byte| *byte != 0) {
        return Err(NfcError::VerificationFailed(
            "the EV2 enciphered response has invalid padding".to_owned(),
        ));
    }
    Ok(plain[..expected].to_vec())
}

/// DESFire CRC32: reflected polynomial 0xEDB88320, initial value 0xFFFFFFFF,
/// no final complement, little-endian output.
fn crc32_desfire(data: &[u8]) -> [u8; 4] {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc.to_le_bytes()
}

fn last_block(data: &[u8], length: usize) -> Result<&[u8]> {
    data.get(data.len().saturating_sub(length)..)
        .filter(|value| value.len() == length)
        .ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "DESFire authentication needs a {length}-byte ciphertext block"
            ))
        })
}

fn strip_leading_status(data: &mut Vec<u8>, expected: usize) {
    if data.len() == expected + 1 && data.first() == Some(&0x00) {
        data.remove(0);
    }
}

fn decrypt_auth_challenge(key: &[u8], key_type: AuthKeyType, challenge: &[u8]) -> Result<Vec<u8>> {
    match key_type {
        AuthKeyType::Des | AuthKeyType::Tdes2 => cbc_decrypt(&expand_tdes_key(key)?, challenge),
        AuthKeyType::Tdes3 => cbc_decrypt_iv(&expand_tdes_key(key)?, &[0u8; 8], challenge),
        AuthKeyType::Aes | AuthKeyType::AesEv2 => aes_cbc_decrypt(key, &[0u8; 16], challenge),
    }
}

fn encrypt_auth_payload(
    key: &[u8],
    key_type: AuthKeyType,
    challenge: &[u8],
    plain: &[u8],
) -> Result<Vec<u8>> {
    match key_type {
        AuthKeyType::Des | AuthKeyType::Tdes2 => Ok(d40_send(&expand_tdes_key(key)?, plain)),
        AuthKeyType::Tdes3 => {
            cbc_encrypt_iv(&expand_tdes_key(key)?, last_block(challenge, 8)?, plain)
        }
        AuthKeyType::Aes => aes_cbc_encrypt(key, last_block(challenge, 16)?, plain),
        AuthKeyType::AesEv2 => aes_cbc_encrypt(key, &[0u8; 16], plain),
    }
}

fn decrypt_auth_response(
    key: &[u8],
    key_type: AuthKeyType,
    sent: &[u8],
    response: &[u8],
) -> Result<Vec<u8>> {
    match key_type {
        AuthKeyType::Des | AuthKeyType::Tdes2 => cbc_decrypt(&expand_tdes_key(key)?, response),
        AuthKeyType::Tdes3 => {
            cbc_decrypt_iv(&expand_tdes_key(key)?, last_block(sent, 8)?, response)
        }
        AuthKeyType::Aes => aes_cbc_decrypt(key, last_block(sent, 16)?, response),
        AuthKeyType::AesEv2 => aes_cbc_decrypt(key, &[0u8; 16], response),
    }
}

fn derive_session_key(key_type: AuthKeyType, rnd_a: &[u8], rnd_b: &[u8]) -> Vec<u8> {
    match key_type {
        AuthKeyType::Des => {
            let mut key = Vec::with_capacity(8);
            key.extend_from_slice(&rnd_a[0..4]);
            key.extend_from_slice(&rnd_b[0..4]);
            key
        }
        AuthKeyType::Tdes2 => {
            let mut key = Vec::with_capacity(16);
            key.extend_from_slice(&rnd_a[0..4]);
            key.extend_from_slice(&rnd_b[0..4]);
            key.extend_from_slice(&rnd_a[4..8]);
            key.extend_from_slice(&rnd_b[4..8]);
            key
        }
        AuthKeyType::Tdes3 => {
            let mut key = Vec::with_capacity(24);
            key.extend_from_slice(&rnd_a[0..4]);
            key.extend_from_slice(&rnd_b[0..4]);
            key.extend_from_slice(&rnd_a[6..10]);
            key.extend_from_slice(&rnd_b[6..10]);
            key.extend_from_slice(&rnd_a[12..16]);
            key.extend_from_slice(&rnd_b[12..16]);
            key
        }
        AuthKeyType::Aes => {
            let mut key = Vec::with_capacity(16);
            key.extend_from_slice(&rnd_a[0..4]);
            key.extend_from_slice(&rnd_b[0..4]);
            key.extend_from_slice(&rnd_a[12..16]);
            key.extend_from_slice(&rnd_b[12..16]);
            key
        }
        AuthKeyType::AesEv2 => unreachable!("EV2 session keys use CMAC derivation"),
    }
}

fn ev2_session_key(key: &[u8], rnd_a: &[u8], rnd_b: &[u8], enc: bool) -> Result<Vec<u8>> {
    if key.len() != 16 || rnd_a.len() != 16 || rnd_b.len() != 16 {
        return Err(NfcError::InvalidArgument(
            "EV2 session-key derivation needs 16-byte inputs".to_owned(),
        ));
    }
    let mut data = [0u8; 32];
    data[0] = if enc { 0xA5 } else { 0x5A };
    data[1] = if enc { 0x5A } else { 0xA5 };
    data[3] = 0x01;
    data[5] = 0x80;
    data[6..14].copy_from_slice(&rnd_a[0..8]);
    for index in 0..6 {
        data[8 + index] ^= rnd_b[index];
    }
    data[14..24].copy_from_slice(&rnd_b[6..16]);
    data[24..32].copy_from_slice(&rnd_a[8..16]);
    cmac_aes(key, &data)
}

fn rotate_left(block: &[u8]) -> Vec<u8> {
    let mut output = block.to_vec();
    output.rotate_left(1);
    output
}

fn status_error(ins: u8, sw: u16) -> NfcError {
    let label = match sw {
        0x910C => "no changes",
        0x910E => "out of EEPROM memory",
        0x911C => "illegal command",
        0x911E => "integrity error",
        0x9140 => "no such key",
        0x917E => "length error",
        0x919D => "permission denied",
        0x919E => "parameter error",
        0x91A0 => "application not found",
        0x91A1 => "application integrity error",
        0x91AE => "authentication error",
        0x91BE => "boundary error",
        0x91C1 => "card integrity error",
        0x91CA => "command aborted",
        0x91CD => "card disabled",
        0x91CE => "count error",
        0x91DE => "file not found",
        0x91DF => "file integrity error",
        0x91EE => "duplicate file or application",
        0x91F0 => "file already exists",
        _ => "error",
    };
    NfcError::VerificationFailed(format!(
        "DESFire command {ins:#04X} returned {label} (status {sw:#06X})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use nfc_core::{CardInfo, Key, KeyType};
    use nfc_transport::{ApduResponse, DeviceInfo, Transport};

    struct ScriptedReader {
        responses: VecDeque<ApduResponse>,
        commands: Vec<Vec<u8>>,
        device: DeviceInfo,
    }

    impl ScriptedReader {
        fn new(sw: [u8; 2]) -> Self {
            Self::with_responses(vec![ApduResponse::new(Vec::new(), sw)])
        }

        fn with_responses(responses: Vec<ApduResponse>) -> Self {
            Self {
                responses: VecDeque::from(responses),
                commands: Vec::new(),
                device: DeviceInfo::new("scripted", "scripted:0"),
            }
        }
    }

    impl Reader for ScriptedReader {
        fn device(&self) -> &DeviceInfo {
            &self.device
        }

        fn open(&mut self, _conn_string: &str) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn card_info(&mut self) -> Result<CardInfo> {
            Err(NfcError::Unsupported("scripted reader has no card"))
        }

        fn authenticate(&mut self, _block: u8, _key_type: KeyType, _key: Key) -> Result<()> {
            Err(NfcError::Unsupported(
                "scripted reader has no authentication",
            ))
        }

        fn read_block(&mut self, _block: u8) -> Result<[u8; 16]> {
            Err(NfcError::Unsupported("scripted reader has no blocks"))
        }

        fn write_block(&mut self, _block: u8, _data: [u8; 16]) -> Result<()> {
            Err(NfcError::Unsupported("scripted reader has no blocks"))
        }

        fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
            Err(NfcError::Unsupported("scripted reader has no raw mode"))
        }

        fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
            Err(NfcError::Unsupported("scripted reader has no raw mode"))
        }

        fn set_raw_mode(&mut self, _enabled: bool) -> Result<()> {
            Ok(())
        }

        fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
            self.commands.push(apdu.to_vec());
            self.responses
                .pop_front()
                .ok_or_else(|| NfcError::Io("scripted reader ran out of responses".to_owned()))
        }
    }

    fn valid(bytes: &[u8]) -> FileSettings {
        FileSettings::parse(bytes).expect("valid file settings")
    }

    #[test]
    fn parses_standard_and_backup_file_settings() {
        let standard = valid(&[0x00, 0x00, 0x00, 0xE0, 0x0F, 0x00, 0x00]);
        assert_eq!(standard.file_type, FileType::Standard);
        assert_eq!(standard.communication, CommunicationMode::Plain);
        assert_eq!(standard.access.raw, 0xE000);
        assert_eq!(standard.access.read, AccessRight::Free);
        assert_eq!(standard.access.write, AccessRight::Key(0));
        assert_eq!(standard.access.read_write, AccessRight::Key(0));
        assert_eq!(standard.access.change, AccessRight::Key(0));
        assert_eq!(standard.contents, FileContents::Data { size: 15 });
        assert_eq!(standard.raw(), &[0x00, 0x00, 0x00, 0xE0, 0x0F, 0x00, 0x00]);

        let backup = valid(&[0x01, 0x00, 0x00, 0xE0, 0x20, 0x00, 0x00]);
        assert_eq!(backup.file_type, FileType::Backup);
        assert_eq!(backup.contents, FileContents::Data { size: 32 });
    }

    #[test]
    fn parses_value_file_settings() {
        let settings = valid(&[
            0x02, 0x01, 0x00, 0xE0, 0x9C, 0xFF, 0xFF, 0xFF, 0x64, 0x00, 0x00, 0x00, 0x2A, 0x00,
            0x00, 0x00, 0x03,
        ]);
        assert_eq!(settings.file_type, FileType::Value);
        assert_eq!(settings.communication, CommunicationMode::Maced);
        assert_eq!(
            settings.contents,
            FileContents::Value {
                lower_limit: -100,
                upper_limit: 100,
                limited_credit_value: 42,
                limited_credit: true,
                free_get_value: true,
            }
        );
        // The third field is the LimitedCredit amount, not the current value;
        // it may be outside the value limits (here 0 with limits 5..10).
        let zero_credit = valid(&[
            0x02, 0x00, 0x00, 0xE0, 0x05, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x01,
        ]);
        assert_eq!(
            zero_credit.contents,
            FileContents::Value {
                lower_limit: 5,
                upper_limit: 10,
                limited_credit_value: 0,
                limited_credit: true,
                free_get_value: false,
            }
        );
    }

    #[test]
    fn parses_linear_and_cyclic_record_file_settings() {
        let linear = valid(&[
            0x03, 0x00, 0x00, 0xE0, 0x10, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x03, 0x00, 0x00,
        ]);
        assert_eq!(linear.file_type, FileType::LinearRecord);
        assert_eq!(
            linear.contents,
            FileContents::Record {
                record_size: 16,
                max_records: 10,
                current_records: 3,
            }
        );

        let cyclic = valid(&[
            0x04, 0x03, 0x00, 0xE0, 0x20, 0x00, 0x00, 0x04, 0x00, 0x00, 0x04, 0x00, 0x00,
        ]);
        assert_eq!(cyclic.file_type, FileType::CyclicRecord);
        assert_eq!(cyclic.communication, CommunicationMode::Enciphered);
        assert_eq!(
            cyclic.contents,
            FileContents::Record {
                record_size: 32,
                max_records: 4,
                current_records: 4,
            }
        );
    }

    #[test]
    fn parses_transaction_mac_file_settings() {
        let with_counter = valid(&[0x05, 0x20, 0x00, 0xE0, 0x02, 0x05, 0x00, 0x00, 0x01, 0x00]);
        assert_eq!(with_counter.file_type, FileType::TransactionMac);
        assert_eq!(
            with_counter.contents,
            FileContents::TransactionMac {
                key_option: 0x02,
                key_version: 5,
                counter_limit: Some(256),
            }
        );

        let without_counter = valid(&[0x05, 0x00, 0x00, 0xE0, 0x02, 0x05]);
        assert_eq!(
            without_counter.contents,
            FileContents::TransactionMac {
                key_option: 0x02,
                key_version: 5,
                counter_limit: None,
            }
        );
    }

    #[test]
    fn parses_additional_access_rights() {
        let settings = valid(&[
            0x00, 0x80, 0x00, 0xE0, 0x0F, 0x00, 0x00, 0x02, 0x00, 0xE0, 0x0E, 0x0E,
        ]);
        assert_eq!(settings.additional_access.len(), 2);
        assert_eq!(settings.additional_access[0].raw, 0xE000);
        assert_eq!(settings.additional_access[1].raw, 0x0E0E);
        assert!(settings.summary().contains("key 1: read free"));
    }

    #[test]
    fn rejects_truncated_standard_settings() {
        assert!(FileSettings::parse(&[0x00, 0x00, 0x00, 0xE0, 0x0F]).is_err());
    }

    #[test]
    fn rejects_zero_size_backup_file() {
        assert!(FileSettings::parse(&[0x01, 0x00, 0x00, 0xE0, 0x00, 0x00, 0x00]).is_err());
    }

    #[test]
    fn rejects_inconsistent_value_file() {
        let equal_limits = [
            0x02, 0x00, 0x00, 0xE0, 0x64, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x64, 0x00,
            0x00, 0x00, 0x00,
        ];
        assert!(FileSettings::parse(&equal_limits).is_err());

        let credit_outside_limits = [
            0x02, 0x00, 0x00, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x0B, 0x00,
            0x00, 0x00, 0x00,
        ];
        // The third field is the LimitedCredit amount and is not bounded by
        // the value limits.
        assert!(FileSettings::parse(&credit_outside_limits).is_ok());

        let reserved_flags = [
            0x02, 0x00, 0x00, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x05, 0x00,
            0x00, 0x00, 0x04,
        ];
        assert!(FileSettings::parse(&reserved_flags).is_err());
    }

    #[test]
    fn rejects_inconsistent_record_file() {
        let current_over_max = [
            0x03, 0x00, 0x00, 0xE0, 0x10, 0x00, 0x00, 0x02, 0x00, 0x00, 0x03, 0x00, 0x00,
        ];
        assert!(FileSettings::parse(&current_over_max).is_err());

        let zero_record_size = [
            0x04, 0x00, 0x00, 0xE0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(FileSettings::parse(&zero_record_size).is_err());
    }

    #[test]
    fn rejects_truncated_transaction_mac_counter() {
        assert!(FileSettings::parse(&[0x05, 0x20, 0x00, 0xE0, 0x02, 0x05, 0x00, 0x00]).is_err());
    }

    #[test]
    fn rejects_unknown_file_type() {
        assert!(FileSettings::parse(&[0x06, 0x00, 0x00, 0xE0]).is_err());
    }

    #[test]
    fn rejects_trailing_bytes_without_the_additional_rights_flag() {
        assert!(FileSettings::parse(&[0x00, 0x00, 0x00, 0xE0, 0x0F, 0x00, 0x00, 0x00]).is_err());
    }

    #[test]
    fn rejects_inconsistent_additional_rights() {
        let zero_count = [0x00, 0x80, 0x00, 0xE0, 0x0F, 0x00, 0x00, 0x00];
        assert!(FileSettings::parse(&zero_count).is_err());

        let short = [0x00, 0x80, 0x00, 0xE0, 0x0F, 0x00, 0x00, 0x02, 0x00, 0xE0];
        assert!(FileSettings::parse(&short).is_err());
    }

    #[test]
    fn rejects_counter_limit_bit_on_a_non_mac_file() {
        assert!(FileSettings::parse(&[0x00, 0x20, 0x00, 0xE0, 0x0F, 0x00, 0x00]).is_err());
    }

    #[test]
    fn ev2_session_keys_match_an12343_vectors() {
        let rnd_a = [
            0xB0, 0x4D, 0x07, 0x87, 0xC9, 0x3E, 0xE0, 0xCC, 0x8C, 0xAC, 0xC8, 0xE8, 0x6F, 0x16,
            0xC6, 0xFE,
        ];
        let rnd_b = [
            0xFA, 0x65, 0x9A, 0xD0, 0xDC, 0xA7, 0x38, 0xDD, 0x65, 0xDC, 0x7D, 0xC3, 0x86, 0x12,
            0xAD, 0x81,
        ];
        assert_eq!(
            ev2_session_key(&[0x00; 16], &rnd_a, &rnd_b, true).expect("ENC"),
            [
                0x63, 0xDC, 0x07, 0x28, 0x62, 0x89, 0xA7, 0xA6, 0xC0, 0x33, 0x4C, 0xA3, 0x1C, 0x31,
                0x4A, 0x04
            ]
        );
        assert_eq!(
            ev2_session_key(&[0x00; 16], &rnd_a, &rnd_b, false).expect("MAC"),
            [
                0x77, 0x4F, 0x26, 0x74, 0x3E, 0xCE, 0x6A, 0xF5, 0x03, 0x3B, 0x6A, 0xE8, 0x52, 0x29,
                0x46, 0xF6
            ]
        );

        let key = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
            0xEE, 0xFF,
        ];
        let rnd_a = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB,
            0xCD, 0xEF,
        ];
        let rnd_b = [
            0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, 0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54,
            0x32, 0x10,
        ];
        assert_eq!(
            ev2_session_key(&key, &rnd_a, &rnd_b, true).expect("ENC2"),
            [
                0x19, 0x54, 0x28, 0x11, 0x9C, 0x85, 0x28, 0xD5, 0x95, 0x80, 0xA7, 0x3C, 0x7C, 0x96,
                0xC6, 0xA3
            ]
        );
        assert_eq!(
            ev2_session_key(&key, &rnd_a, &rnd_b, false).expect("MAC2"),
            [
                0x0A, 0xF0, 0x0F, 0x6E, 0x39, 0xF9, 0x04, 0xC4, 0xA6, 0x38, 0x8C, 0x81, 0x2D, 0x53,
                0xDC, 0x94
            ]
        );
    }

    #[test]
    fn aes_and_3des_cbc_round_trip() {
        let aes_key = [0x11; 16];
        let iv = [0x22; 16];
        let plain = [0x33; 32];
        let cipher = aes_cbc_encrypt(&aes_key, &iv, &plain).expect("AES encrypt");
        assert_ne!(cipher, plain);
        assert_eq!(
            aes_cbc_decrypt(&aes_key, &iv, &cipher).expect("AES decrypt"),
            plain
        );

        let tdes_key = expand_tdes_key(&[0x44; 24]).expect("3K3DES key");
        let iv = [0x55; 8];
        let plain = [0x66; 16];
        let cipher = cbc_encrypt_iv(&tdes_key, &iv, &plain).expect("3DES encrypt");
        assert_ne!(cipher, plain);
        assert_eq!(
            cbc_decrypt_iv(&tdes_key, &iv, &cipher).expect("3DES decrypt"),
            plain
        );
    }

    #[test]
    fn key_types_map_from_lengths_and_validate_keys() {
        assert_eq!(
            AuthKeyType::from_key(&[0u8; 8]).expect("DES"),
            AuthKeyType::Des
        );
        assert_eq!(
            AuthKeyType::from_key(&[0u8; 16]).expect("2K3DES"),
            AuthKeyType::Tdes2
        );
        assert_eq!(
            AuthKeyType::from_key(&[0u8; 24]).expect("3K3DES"),
            AuthKeyType::Tdes3
        );
        assert!(AuthKeyType::from_key(&[0u8; 7]).is_err());
        assert!(AuthKeyType::Aes.validate_key(&[0u8; 16]).is_ok());
        assert!(AuthKeyType::Aes.validate_key(&[0u8; 8]).is_err());
        assert!(AuthKeyType::Tdes3.validate_key(&[0u8; 16]).is_err());
    }

    #[test]
    fn commit_accepts_no_changes_as_success() {
        let mut reader = ScriptedReader::new([0x91, 0x0C]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.commit_transaction().expect("NoChanges is success");
        assert_eq!(reader.commands, [vec![0x90, 0xC7, 0x00, 0x00, 0x00]]);
    }

    #[test]
    fn commit_rejects_other_status_words() {
        let mut reader = ScriptedReader::new([0x91, 0x01]);
        let mut desfire = Desfire::new(&mut reader);
        assert!(desfire.commit_transaction().is_err());
    }

    #[test]
    fn expand_tdes_key_repeats_des_and_uses_third_key_for_2k3des() {
        assert_eq!(expand_tdes_key(&[1u8; 8]).expect("des key"), [1u8; 24]);
        let mut key = [0u8; 16];
        key[..8].fill(1);
        key[8..].fill(2);
        let expanded = expand_tdes_key(&key).expect("2k3des key");
        assert_eq!(&expanded[..8], &[1u8; 8]);
        assert_eq!(&expanded[8..16], &[2u8; 8]);
        assert_eq!(&expanded[16..], &[1u8; 8]);
    }

    #[test]
    fn standard_file_size_is_little_endian() {
        let settings = [0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00];
        assert_eq!(Desfire::data_file_size(&settings), Some(32));
        assert_eq!(Desfire::data_file_size(&[0x02, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(Desfire::data_file_size(&[0x00; 6]), None);
    }

    #[test]
    fn rotate_left_wraps_the_nonce() {
        let block = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(
            rotate_left(&block),
            [0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x01]
        );
    }

    #[test]
    fn ndef_application_payload_has_iso_id_and_df_name() {
        let payload = create_application_payload(
            NDEF_APPLICATION_AID,
            NDEF_APPLICATION_KEY_SETTINGS.0,
            NDEF_APPLICATION_KEY_SETTINGS.1,
            Some(NDEF_APPLICATION_ISO_FILE),
            &NDEF_DF_NAME,
        )
        .expect("application payload");
        assert_eq!(
            payload,
            [
                0x00, 0x76, 0xD2, 0x0B, 0x21, 0x10, 0xE1, 0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01
            ]
        );
    }

    #[test]
    fn ndef_file_payloads_carry_iso_ids_and_free_write_access() {
        let cc = create_standard_file_payload(
            NDEF_CC_FILE,
            Some(NDEF_CC_ISO_FILE),
            0,
            NDEF_CC_ACCESS_RIGHTS,
            NDEF_CC_SIZE,
        )
        .expect("CC file payload");
        assert_eq!(cc, [0x01, 0x03, 0xE1, 0x00, 0x00, 0xE0, 0x0F, 0x00, 0x00]);

        let ndef = create_standard_file_payload(
            NDEF_DATA_FILE,
            Some(NDEF_DATA_ISO_FILE),
            0,
            NDEF_DATA_ACCESS_RIGHTS,
            NDEF_DATA_SIZE,
        )
        .expect("NDEF file payload");
        assert_eq!(ndef, [0x02, 0x04, 0xE1, 0x00, 0xE0, 0xEE, 0xFF, 0x00, 0x00]);
    }

    #[test]
    fn value_and_record_create_payloads_match_the_settings_layout() {
        let value = create_value_file_payload(&ValueFileSpec {
            file_id: 1,
            iso_file_id: None,
            communication: 0,
            access_rights: 0x0000,
            lower_limit: -50,
            upper_limit: 50,
            value: 7,
            flags: 0x03,
        })
        .expect("value payload");
        assert_eq!(
            value,
            [
                0x01, 0x00, 0x00, 0x00, 0xCE, 0xFF, 0xFF, 0xFF, 0x32, 0x00, 0x00, 0x00, 0x07, 0x00,
                0x00, 0x00, 0x03
            ]
        );

        let record = create_record_file_payload(&RecordFileSpec {
            file_id: 2,
            iso_file_id: None,
            communication: 1,
            access_rights: 0x0000,
            record_size: 8,
            max_records: 4,
            cyclic: false,
        })
        .expect("record payload");
        assert_eq!(
            record,
            [0x02, 0x01, 0x00, 0x00, 0x08, 0x00, 0x00, 0x04, 0x00, 0x00]
        );

        let mut spec = ValueFileSpec {
            file_id: 1,
            iso_file_id: None,
            communication: 0,
            access_rights: 0,
            lower_limit: 50,
            upper_limit: 50,
            value: 50,
            flags: 0,
        };
        assert!(create_value_file_payload(&spec).is_err());
        spec.lower_limit = 0;
        spec.value = 51;
        assert!(create_value_file_payload(&spec).is_err());
        spec.value = 0;
        spec.flags = 0x04;
        assert!(create_value_file_payload(&spec).is_err());

        let mut record = RecordFileSpec {
            file_id: 2,
            iso_file_id: None,
            communication: 0,
            access_rights: 0,
            record_size: 0,
            max_records: 4,
            cyclic: false,
        };
        assert!(create_record_file_payload(&record).is_err());
        record.record_size = 8;
        record.max_records = 0;
        assert!(create_record_file_payload(&record).is_err());
    }

    #[test]
    fn access_rights_encode_and_parse_the_access_word() {
        let rights = AccessRights::from_parts(
            AccessRight::Free,
            AccessRight::Key(2),
            AccessRight::Denied,
            AccessRight::Key(1),
        );
        assert_eq!(rights.raw, 0xE2F1);
        assert_eq!(
            rights.summary(),
            "read free, write key 2, rw denied, change key 1"
        );
        assert_eq!(
            AccessRight::parse("key0").expect("key 0"),
            AccessRight::Key(0)
        );
        assert_eq!(
            AccessRight::parse(" KEY13 ").expect("key 13"),
            AccessRight::Key(13)
        );
        assert_eq!(AccessRight::parse("free").expect("free"), AccessRight::Free);
        assert_eq!(
            AccessRight::parse("deny").expect("denied"),
            AccessRight::Denied
        );
        assert!(AccessRight::parse("key14").is_err());
        assert!(AccessRight::parse("open").is_err());
        let decoded = AccessRights::from_raw(0xE2F1);
        assert_eq!(decoded, rights);
    }

    #[test]
    fn management_payloads_carry_the_expected_fields() {
        let (header, data) = transaction_mac_file_parts(&TransactionMacFileSpec {
            file_id: 7,
            iso_file_id: None,
            communication: 1,
            access_rights: 0xEF21,
            key: [0xA5; 16],
            key_version: 0x05,
        })
        .expect("Transaction MAC payload");
        assert_eq!(header, [0x07, 0x01, 0x21, 0xEF, 0x02]);
        assert_eq!(data.len(), 17);
        assert_eq!(&data[..16], &[0xA5; 16]);
        assert_eq!(data[16], 0x05);
        assert!(
            transaction_mac_file_parts(&TransactionMacFileSpec {
                file_id: 7,
                iso_file_id: None,
                communication: 1,
                access_rights: 0x0000,
                key: [0xA5; 16],
                key_version: 0x05,
            })
            .is_err(),
            "a Transaction MAC file refuses direct writes"
        );

        assert_eq!(
            change_file_settings_payload(7, 1, 0xE2F1).expect("settings payload"),
            [0x07, 0x01, 0xF1, 0xE2]
        );
        assert!(change_file_settings_payload(32, 0, 0).is_err());
        assert!(change_file_settings_payload(1, 2, 0).is_err());
        assert!(create_standard_file_payload(32, None, 0, 0, 1).is_err());
        assert!(create_standard_file_payload(1, None, 2, 0, 1).is_err());

        let application = ApplicationSpec::new([0xF0, 0x0D, 0x22], 4, ApplicationKeyType::Aes);
        assert_eq!(application.key_settings2(), 0x84);
        assert_eq!(application.key_settings, 0x0F);
    }

    #[test]
    fn management_commands_create_change_and_delete_files() {
        let transport = crate::mock::MockDesfireTransport::with_tag(crate::mock::MockDesfire::new(
            [0u8; 16],
            AuthKeyType::Aes,
        ));
        let mut reader = transport.open("mock:desfire").expect("open mock card");
        let mut desfire = Desfire::new(reader.as_mut());
        desfire.select_application([0x00; 3]).expect("select PICC");
        desfire
            .authenticate(0, &[0u8; 16], AuthKeyType::Aes)
            .expect("authenticate PICC");
        let application = ApplicationSpec::new([0xF0, 0x0D, 0x22], 1, ApplicationKeyType::Aes);
        desfire
            .create_application_from(&application)
            .expect("create application");
        assert!(
            desfire
                .application_ids()
                .expect("application ids")
                .contains(&application.aid)
        );

        desfire
            .select_application(application.aid)
            .expect("select application");
        desfire
            .authenticate(0, &[0u8; 16], AuthKeyType::Aes)
            .expect("authenticate application");
        desfire
            .create_standard_file(1, None, 0, 0x0000, 16)
            .expect("create standard file");
        desfire
            .create_value_file(ValueFileSpec {
                file_id: 2,
                iso_file_id: None,
                communication: 0,
                access_rights: 0,
                lower_limit: 0,
                upper_limit: 100,
                value: 5,
                flags: 0,
            })
            .expect("create value file");
        desfire
            .create_record_file(RecordFileSpec {
                file_id: 3,
                iso_file_id: None,
                communication: 0,
                access_rights: 0,
                record_size: 4,
                max_records: 2,
                cyclic: false,
            })
            .expect("create record file");
        desfire
            .create_transaction_mac_file(&TransactionMacFileSpec {
                file_id: 4,
                iso_file_id: None,
                communication: 0,
                access_rights: 0xEFF0,
                key: [0x11; 16],
                key_version: 1,
            })
            .expect("create Transaction MAC file");
        assert_eq!(desfire.file_ids().expect("file ids"), [1, 2, 3, 4]);

        desfire
            .change_file_settings(1, 3, 0x0000, CommunicationMode::Enciphered)
            .expect("change settings");
        let settings =
            FileSettings::parse(&desfire.file_settings(1).expect("settings")).expect("decode");
        assert_eq!(settings.communication, CommunicationMode::Enciphered);
        let tmac =
            FileSettings::parse(&desfire.file_settings(4).expect("settings")).expect("decode");
        assert_eq!(tmac.file_type, FileType::TransactionMac);

        desfire.delete_file(3).expect("delete record file");
        assert_eq!(desfire.file_ids().expect("file ids"), [1, 2, 4]);

        desfire.select_application([0x00; 3]).expect("select PICC");
        desfire
            .authenticate(0, &[0u8; 16], AuthKeyType::Aes)
            .expect("authenticate PICC");
        desfire
            .delete_application(application.aid)
            .expect("delete application");
        assert!(
            !desfire
                .application_ids()
                .expect("application ids")
                .contains(&application.aid)
        );
    }

    #[test]
    fn get_value_and_read_records_use_the_native_command_shape() {
        let mut reader = ScriptedReader::new([0x91, 0x00]);
        reader.responses = VecDeque::from([ApduResponse::new(
            vec![0xCE, 0xFF, 0xFF, 0xFF],
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        assert_eq!(
            desfire
                .get_value(5, CommunicationMode::Plain)
                .expect("value")
                .value,
            -50
        );
        assert_eq!(
            reader.commands,
            [vec![0x90, 0x6C, 0x00, 0x00, 0x01, 0x05, 0x00]]
        );

        let mut reader = ScriptedReader::new([0x91, 0x00]);
        reader.responses = VecDeque::from([ApduResponse::new(vec![0xAA; 16], [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        assert_eq!(
            desfire
                .read_records(2, 1, 2, 8, CommunicationMode::Plain)
                .expect("records")
                .data,
            vec![0xAA; 16]
        );
        assert!(
            desfire
                .read_records(2, 0, 0, 8, CommunicationMode::Plain)
                .is_err()
        );
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![
                0x90, 0xBB, 0x00, 0x00, 0x07, 0x02, 0x01, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00
            ]]
        );
    }

    #[test]
    fn read_data_consumes_additional_frames() {
        let mut reader = ScriptedReader::with_responses(vec![
            ApduResponse::new(vec![0x01, 0x02], [0x91, 0xAF]),
            ApduResponse::new(vec![0x03, 0x04], [0x91, 0x00]),
        ]);
        let mut desfire = Desfire::new(&mut reader);
        assert_eq!(
            desfire
                .read_data(2, 0, 4, CommunicationMode::Plain)
                .expect("chained data")
                .data,
            vec![0x01, 0x02, 0x03, 0x04]
        );
        drop(desfire);
        assert_eq!(
            reader.commands,
            [
                vec![
                    0x90, 0xBD, 0x00, 0x00, 0x07, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00
                ],
                vec![0x90, 0xAF, 0x00, 0x00, 0x00]
            ]
        );
    }

    #[test]
    fn ndef_capability_container_matches_mapping_version_2() {
        assert_eq!(
            NDEF_CAPABILITY_CONTAINER,
            [
                0x00, 0x0F, 0x20, 0x00, 0x3B, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x00, 0xFF, 0x00,
                0x00
            ]
        );
    }

    #[test]
    fn application_and_file_iso_names_are_validated() {
        assert!(
            create_application_payload(
                NDEF_APPLICATION_AID,
                0x0B,
                0x21,
                Some(NDEF_APPLICATION_ISO_FILE),
                &[],
            )
            .is_err()
        );
        assert!(
            create_application_payload(NDEF_APPLICATION_AID, 0x0B, 0x21, None, &NDEF_DF_NAME,)
                .is_err()
        );
        assert!(
            create_application_payload(NDEF_APPLICATION_AID, 0x0B, 0x21, Some(0), &NDEF_DF_NAME)
                .is_err()
        );
        assert!(create_application_payload(NDEF_APPLICATION_AID, 0x0B, 0x21, None, &[]).is_err());
        assert!(
            create_application_payload(
                NDEF_APPLICATION_AID,
                0x0B,
                0x21,
                Some(NDEF_APPLICATION_ISO_FILE),
                &[0u8; 17],
            )
            .is_err()
        );
        assert!(create_standard_file_payload(1, Some(0), 0, 0, 15).is_err());
        assert!(create_standard_file_payload(1, None, 4, 0, 15).is_err());
        assert!(create_standard_file_payload(1, None, 0, 0, 0).is_err());
    }

    fn hex(text: &str) -> Vec<u8> {
        let digits: String = text
            .chars()
            .filter(|character| character.is_ascii_hexdigit())
            .collect();
        digits
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ascii hex"), 16)
                    .expect("valid hex")
            })
            .collect()
    }

    fn ev1_session(key_type: AuthKeyType, key: &[u8], iv: Vec<u8>) -> Session {
        Session {
            enc_key: key.to_vec(),
            mac_key: key.to_vec(),
            key_type,
            key_no: 0,
            key_version: 0,
            iv,
            ti: [0u8; 4],
            counter: 0,
        }
    }

    fn ev2_session(mac_key: &[u8], enc_key: &[u8], ti: [u8; 4], counter: u16) -> Session {
        Session {
            enc_key: enc_key.to_vec(),
            mac_key: mac_key.to_vec(),
            key_type: AuthKeyType::AesEv2,
            key_no: 0,
            key_version: 0,
            iv: Vec::new(),
            ti,
            counter,
        }
    }

    #[test]
    fn cmac_matches_published_vectors() {
        let data = hex("6BC1BEE22E409F96E93D7E117393172AAE2D8A571E03AC9C9EB76FAC45AF8E51");
        let key = hex("0123456789ABCDEF23456789ABCDEF01456789ABCDEF0123");
        for (length, expected) in [
            (0, "7DB0D37DF936C550"),
            (16, "30239CF1F52E6609"),
            (20, "6C9F3EE4923F6BE2"),
            (32, "99429BD0BF7904E5"),
        ] {
            assert_eq!(
                ev1_cmac(AuthKeyType::Tdes3, &key, &[0u8; 8], &data[..length]).expect("cmac"),
                hex(expected)
            );
        }

        let aes_key = hex("2B7E151628AED2A6ABF7158809CF4F3C");
        let aes_data = hex("6BC1BEE22E409F96E93D7E117393172A");
        assert_eq!(
            cmac_aes(&aes_key, &aes_data).expect("aes cmac"),
            hex("070A16B46B4D4144F79BDD9DD04A287C")
        );
    }

    #[test]
    fn ev2_macs_match_an12343() {
        let mut session = ev2_session(
            &hex("9366FA195EB566F5BD2BAD4020B83002"),
            &[0u8; 16],
            [0xE2, 0xD3, 0xAF, 0x69],
            0,
        );
        let mut write = hex("00000000190000");
        write.extend_from_slice(&[0x22; 25]);
        assert_eq!(
            ev2_truncate(&session.command_mac(0x8D, &write).expect("write mac")),
            [0x68, 0xF2, 0xC2, 0x8C, 0x57, 0x5A, 0x16, 0x28]
        );

        session.counter = 1;
        assert_eq!(
            ev2_truncate(&session.response_mac(&[], 0x00).expect("write response mac")),
            [0x08, 0x20, 0xF6, 0x88, 0x98, 0xC2, 0xA7, 0xF1]
        );
        assert_eq!(
            ev2_truncate(
                &session
                    .command_mac(0xAD, &hex("00000000300000"))
                    .expect("read mac")
            ),
            [0x0D, 0x9B, 0xE1, 0x91, 0xD5, 0x96, 0x08, 0x34]
        );

        session.counter = 2;
        let mut response = vec![0x22; 25];
        response.extend_from_slice(&[0x00; 23]);
        assert_eq!(
            ev2_truncate(
                &session
                    .response_mac(&response, 0x00)
                    .expect("read response mac")
            ),
            [0xA4, 0x9A, 0x44, 0x22, 0x2D, 0x92, 0x66, 0x66]
        );
    }

    #[test]
    fn ev2_response_iv_matches_an12343() {
        assert_eq!(
            ev2_response_iv(
                &hex("FFBCFE1F41840A09C9A88D0A4B10DF05"),
                &[0xCD, 0x73, 0xD8, 0xE5],
                2
            )
            .expect("iv"),
            hex("13659A1BC863D4109FC3FB92C93D952F")
        );
    }

    #[test]
    fn crc32_matches_the_desfire_vector() {
        assert_eq!(
            crc32_desfire(&hex("04440F3276318000")),
            [0x99, 0xCE, 0x1A, 0xD4]
        );
    }

    #[test]
    fn ev1_maced_read_verifies_the_response_mac() {
        let key = hex("000102030405060708090A0B0C0D0E0F1011121314151617");
        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            hex("01020304F12683C01A4BD67F"),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Tdes3, &key, vec![0u8; 8]));
        let read = desfire
            .read_data(2, 0, 4, CommunicationMode::Maced)
            .expect("maced read");
        assert_eq!(read.data, vec![0x01, 0x02, 0x03, 0x04]);
        assert_eq!(read.integrity, IntegrityCheck::Ev1Cmac);
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![
                0x90, 0xBD, 0x00, 0x00, 0x07, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00
            ]]
        );
    }

    #[test]
    fn ev1_tampered_response_mac_is_rejected() {
        let key = hex("000102030405060708090A0B0C0D0E0F1011121314151617");
        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            hex("01020304F12683C01A4BD67E"),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Tdes3, &key, vec![0u8; 8]));
        let error = desfire
            .read_data(2, 0, 4, CommunicationMode::Maced)
            .expect_err("the MAC is wrong");
        assert!(matches!(error, NfcError::VerificationFailed(_)));
        assert!(
            desfire.session_key().is_none(),
            "the session must be dropped"
        );
    }

    #[test]
    fn ev1_enciphered_read_decrypts_and_checks_the_crc() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let ciphertext = hex(
            "6DC82D119AA47B43E08DBB8CC3555C324710A1FEC13452F85E242DF7BEF1A592             357E4865B053AFF6B82A002764BE912A",
        );
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(ciphertext, [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let expected: Vec<u8> = (1..=40).collect();
        let read = desfire
            .read_data(2, 0, 40, CommunicationMode::Enciphered)
            .expect("enciphered read");
        assert_eq!(read.data, expected);
        assert_eq!(read.integrity, IntegrityCheck::Ev1Crc32);
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![
                0x90, 0xBD, 0x00, 0x00, 0x07, 0x02, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00
            ]]
        );
    }

    #[test]
    fn ev1_tampered_ciphertext_is_rejected() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let mut ciphertext = hex(
            "6DC82D119AA47B43E08DBB8CC3555C324710A1FEC13452F85E242DF7BEF1A592             357E4865B053AFF6B82A002764BE912A",
        );
        ciphertext[0] ^= 0x01;
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(ciphertext, [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let error = desfire
            .read_data(2, 0, 40, CommunicationMode::Enciphered)
            .expect_err("the CRC is wrong");
        assert!(matches!(error, NfcError::VerificationFailed(_)));
        assert!(
            desfire.session_key().is_none(),
            "the session must be dropped"
        );
    }

    #[test]
    fn ev2_maced_read_reassembles_frames_and_verifies_the_mac() {
        let mac_key = hex("9366FA195EB566F5BD2BAD4020B83002");
        let mut body = vec![0x22u8; 25];
        body.extend_from_slice(&[0x00; 23]);
        let mut raw = body.clone();
        raw.extend_from_slice(&hex("A49A44222D926666"));
        let mut reader = ScriptedReader::with_responses(vec![
            ApduResponse::new(raw[..20].to_vec(), [0x91, 0xAF]),
            ApduResponse::new(raw[20..].to_vec(), [0x91, 0x00]),
        ]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev2_session(
            &mac_key,
            &[0u8; 16],
            [0xE2, 0xD3, 0xAF, 0x69],
            1,
        ));
        let read = desfire
            .read_data(0, 0, 48, CommunicationMode::Maced)
            .expect("maced read");
        assert_eq!(read.data, body);
        assert_eq!(read.integrity, IntegrityCheck::Ev2Cmac);
        drop(desfire);
        let mut expected = vec![0x90, 0xBD, 0x00, 0x00, 0x0F];
        expected.extend_from_slice(&hex("000000003000005A17B6D0C9EB96A5"));
        expected.push(0x00);
        assert_eq!(reader.commands[0], expected);
        assert_eq!(reader.commands[1], vec![0x90, 0xAF, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn ev2_enciphered_read_verifies_the_mac_then_decrypts() {
        let mac_key = hex("37E7234B11BEBEFDE41A8F290090EF80");
        let enc_key = hex("FFBCFE1F41840A09C9A88D0A4B10DF05");
        let mut raw = hex(
            "8848D0F9B9FD4495770C89925B2A85C7274D350FA9029C484D43804886662DC             42D7F40A6D7A415E4A71EFF79EB8E5721AC3BF1CAAFE8EB2CAA2DC162E67A97A3",
        );
        raw.extend_from_slice(&hex("8ED7888F22B3A587"));
        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(raw, [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev2_session(&mac_key, &enc_key, [0xCD, 0x73, 0xD8, 0xE5], 1));
        let read = desfire
            .read_data(0, 0, 48, CommunicationMode::Enciphered)
            .expect("enciphered read");
        assert_eq!(read.integrity, IntegrityCheck::Ev2Cmac);
        assert_eq!(read.data, {
            let mut expected = vec![0x22; 25];
            expected.extend_from_slice(&[0x00; 23]);
            expected
        });
        drop(desfire);
        let mut expected = vec![0x90, 0xBD, 0x00, 0x00, 0x0F];
        expected.extend_from_slice(&hex("00000000300000A793CB9E6B283100"));
        expected.push(0x00);
        assert_eq!(reader.commands, [expected]);
    }

    #[test]
    fn ev2_tampered_enciphered_response_is_rejected() {
        let mac_key = hex("37E7234B11BEBEFDE41A8F290090EF80");
        let enc_key = hex("FFBCFE1F41840A09C9A88D0A4B10DF05");
        let ciphertext = hex(
            "8848D0F9B9FD4495770C89925B2A85C7274D350FA9029C484D43804886662DC             42D7F40A6D7A415E4A71EFF79EB8E5721AC3BF1CAAFE8EB2CAA2DC162E67A97A3",
        );

        for mutate in 0..2 {
            let mut raw = ciphertext.clone();
            if mutate == 0 {
                raw[0] ^= 0x01;
            }
            raw.extend_from_slice(&hex("8ED7888F22B3A587"));
            if mutate == 1 {
                let last = raw.len() - 1;
                raw[last] ^= 0x01;
            }
            let mut reader =
                ScriptedReader::with_responses(vec![ApduResponse::new(raw, [0x91, 0x00])]);
            let mut desfire = Desfire::new(&mut reader);
            desfire.session = Some(ev2_session(&mac_key, &enc_key, [0xCD, 0x73, 0xD8, 0xE5], 1));
            let error = desfire
                .read_data(0, 0, 48, CommunicationMode::Enciphered)
                .expect_err("the response is tampered");
            assert!(matches!(error, NfcError::VerificationFailed(_)));
            assert!(
                desfire.session_key().is_none(),
                "the session must be dropped"
            );
        }
    }

    #[test]
    fn ev2_plain_read_has_no_mac_and_advances_the_counter() {
        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            vec![0x2A, 0x00, 0x00, 0x00],
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev2_session(&[0u8; 16], &[0u8; 16], [1, 2, 3, 4], 0));
        let value = desfire
            .get_value(3, CommunicationMode::Plain)
            .expect("plain value");
        assert_eq!(value.value, 42);
        assert_eq!(value.integrity, IntegrityCheck::None);
        assert_eq!(
            desfire.session.as_ref().expect("session").counter,
            1,
            "plain commands still advance the EV2 counter"
        );
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![0x90, 0x6C, 0x00, 0x00, 0x01, 0x03, 0x00]]
        );
    }

    #[test]
    fn key_version_is_read_after_authentication_or_reported_as_absent() {
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(vec![0x07], [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Des, &[0u8; 8], Vec::new()));
        assert_eq!(desfire.key_version(0).expect("key version"), Some(7));
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![0x90, 0x64, 0x00, 0x00, 0x01, 0x00, 0x00]]
        );

        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(Vec::new(), [0x91, 0x9D])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Des, &[0u8; 8], Vec::new()));
        assert_eq!(
            desfire
                .key_version(0)
                .expect("permission denied is not a failure"),
            None
        );

        let mut reader = ScriptedReader::new([0x91, 0x00]);
        let mut desfire = Desfire::new(&mut reader);
        assert!(matches!(
            desfire.key_version(0),
            Err(NfcError::AuthenticationRequired(_))
        ));
    }

    #[test]
    fn legacy_card_uid_has_no_response_mac() {
        let uid = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(uid.to_vec(), [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Des, &[0u8; 8], Vec::new()));
        let result = desfire.card_uid().expect("legacy UID");
        assert_eq!(result.uid, uid);
        assert_eq!(result.integrity, IntegrityCheck::None);
        drop(desfire);
        assert_eq!(reader.commands, [vec![0x90, 0x51, 0x00, 0x00, 0x00]]);
    }

    #[test]
    fn ev1_card_uid_decrypts_and_checks_the_crc() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let uid = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];
        let mut server = ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]);
        let command_mac = server
            .command_mac(INS_GET_CARD_UID, &[])
            .expect("command MAC");
        server.iv = command_mac;
        let mut crc_input = uid.to_vec();
        crc_input.push(0x00);
        let mut plain = uid.to_vec();
        plain.extend_from_slice(&crc32_desfire(&crc_input));
        plain.resize(16, 0);
        let ciphertext = aes_cbc_encrypt(&key, &server.iv, &plain).expect("UID encryption");

        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(ciphertext, [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let result = desfire.card_uid().expect("EV1 UID");
        assert_eq!(result.uid, uid);
        assert_eq!(result.integrity, IntegrityCheck::Ev1Crc32);
    }

    #[test]
    fn ev1_card_uid_rejects_a_tampered_ciphertext() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let uid = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];
        let mut server = ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]);
        let command_mac = server
            .command_mac(INS_GET_CARD_UID, &[])
            .expect("command MAC");
        server.iv = command_mac;
        let mut crc_input = uid.to_vec();
        crc_input.push(0x00);
        let mut plain = uid.to_vec();
        plain.extend_from_slice(&crc32_desfire(&crc_input));
        plain.resize(16, 0);
        let mut ciphertext = aes_cbc_encrypt(&key, &server.iv, &plain).expect("UID encryption");
        ciphertext[0] ^= 0x01;

        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(ciphertext, [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let error = desfire.card_uid().expect_err("the ciphertext is tampered");
        assert!(matches!(error, NfcError::VerificationFailed(_)));
        assert!(
            desfire.session_key().is_none(),
            "the session must be dropped"
        );
    }

    #[test]
    fn ev2_card_uid_verifies_the_counter_bound_mac() {
        let mac_key = hex("9366FA195EB566F5BD2BAD4020B83002");
        let enc_key = hex("FFBCFE1F41840A09C9A88D0A4B10DF05");
        let uid = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];
        let mut plain = uid.to_vec();
        plain.push(0x80);
        plain.resize(16, 0);
        let iv = ev2_response_iv(&enc_key, &[0xE2, 0xD3, 0xAF, 0x69], 1).expect("response IV");
        let ciphertext = aes_cbc_encrypt(&enc_key, &iv, &plain).expect("UID encryption");
        let server = ev2_session(&mac_key, &enc_key, [0xE2, 0xD3, 0xAF, 0x69], 1);
        let mut raw = ciphertext.clone();
        raw.extend_from_slice(&ev2_truncate(
            &server
                .response_mac(&ciphertext, 0x00)
                .expect("response MAC"),
        ));

        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(raw, [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev2_session(&mac_key, &enc_key, [0xE2, 0xD3, 0xAF, 0x69], 0));
        let result = desfire.card_uid().expect("EV2 UID");
        assert_eq!(result.uid, uid);
        assert_eq!(result.integrity, IntegrityCheck::Ev2Cmac);
    }

    #[test]
    fn ev1_maced_write_appends_the_command_mac_and_verifies_the_response() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let header = hex("02 10 00 00 04 00 00");
        let data = hex("AABBCCDD");
        let mut input = vec![INS_WRITE_DATA];
        input.extend_from_slice(&header);
        input.extend_from_slice(&data);
        let command_mac =
            ev1_cmac(AuthKeyType::Aes, &key, &[0u8; 16], &input).expect("command MAC");
        let response_mac =
            ev1_cmac(AuthKeyType::Aes, &key, &command_mac, &[0x00]).expect("response MAC");
        let mut expected = header.clone();
        expected.extend_from_slice(&data);
        expected.extend_from_slice(&command_mac[..8]);

        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            response_mac[..8].to_vec(),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let integrity = desfire
            .write_data(2, 0x10, &data, CommunicationMode::Maced)
            .expect("maced write");
        assert_eq!(integrity, IntegrityCheck::Ev1Cmac);
        drop(desfire);
        assert_eq!(reader.commands.len(), 1);
        assert_eq!(
            reader.commands[0][5..reader.commands[0].len() - 1],
            expected
        );
    }

    #[test]
    fn ev1_enciphered_write_encrypts_data_with_the_crc() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let header = hex("02 20 00 00 04 00 00");
        let data = hex("11223344");
        let mut crc_input = vec![INS_WRITE_DATA];
        crc_input.extend_from_slice(&header);
        crc_input.extend_from_slice(&data);
        let crc = crc32_desfire(&crc_input);
        let mut plain = data.clone();
        plain.extend_from_slice(&crc);
        plain.resize(16, 0);
        let cipher = aes_cbc_encrypt(&key, &[0u8; 16], &plain).expect("write ciphertext");
        let post_iv = cipher[cipher.len() - 16..].to_vec();
        let response_mac =
            ev1_cmac(AuthKeyType::Aes, &key, &post_iv, &[0x00]).expect("response MAC");
        let mut expected = header.clone();
        expected.extend_from_slice(&cipher);

        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            response_mac[..8].to_vec(),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let integrity = desfire
            .write_data(2, 0x20, &data, CommunicationMode::Enciphered)
            .expect("enciphered write");
        assert_eq!(integrity, IntegrityCheck::Ev1Cmac);
        drop(desfire);
        assert_eq!(
            reader.commands[0][5..reader.commands[0].len() - 1],
            expected
        );
    }

    #[test]
    fn ev2_maced_write_appends_the_counter_bound_mac() {
        let mac_key = hex("9366FA195EB566F5BD2BAD4020B83002");
        let enc_key = hex("FFBCFE1F41840A09C9A88D0A4B10DF05");
        let ti = [0xE2, 0xD3, 0xAF, 0x69];
        let header = hex("02 30 00 00 04 00 00");
        let data = hex("A1B2C3D4");
        let mut payload = header.clone();
        payload.extend_from_slice(&data);
        let server = ev2_session(&mac_key, &enc_key, ti, 0);
        let command_mac = server
            .command_mac(INS_WRITE_DATA, &payload)
            .expect("command MAC");
        let response_mac = ev2_session(&mac_key, &enc_key, ti, 1)
            .response_mac(&[], 0x00)
            .expect("response MAC");
        let mut expected = payload.clone();
        expected.extend_from_slice(&ev2_truncate(&command_mac));

        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            ev2_truncate(&response_mac).to_vec(),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev2_session(&mac_key, &enc_key, ti, 0));
        let integrity = desfire
            .write_data(2, 0x30, &data, CommunicationMode::Maced)
            .expect("maced write");
        assert_eq!(integrity, IntegrityCheck::Ev2Cmac);
        drop(desfire);
        assert_eq!(
            reader.commands[0][5..reader.commands[0].len() - 1],
            expected
        );
    }

    #[test]
    fn ev2_enciphered_write_encrypts_the_padded_data_before_the_mac() {
        let mac_key = hex("9366FA195EB566F5BD2BAD4020B83002");
        let enc_key = hex("FFBCFE1F41840A09C9A88D0A4B10DF05");
        let ti = [0xE2, 0xD3, 0xAF, 0x69];
        let header = hex("02 40 00 00 04 00 00");
        let data = hex("DEADBEEF");
        let mut plain = data.clone();
        plain.push(0x80);
        plain.resize(16, 0);
        let iv = ev2_command_iv(&enc_key, &ti, 0).expect("command IV");
        let cipher = aes_cbc_encrypt(&enc_key, &iv, &plain).expect("write ciphertext");
        let mut payload = header.clone();
        payload.extend_from_slice(&cipher);
        let server = ev2_session(&mac_key, &enc_key, ti, 0);
        let command_mac = server
            .command_mac(INS_WRITE_DATA, &payload)
            .expect("command MAC");
        let response_mac = ev2_session(&mac_key, &enc_key, ti, 1)
            .response_mac(&[], 0x00)
            .expect("response MAC");
        let mut expected = payload.clone();
        expected.extend_from_slice(&ev2_truncate(&command_mac));

        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            ev2_truncate(&response_mac).to_vec(),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev2_session(&mac_key, &enc_key, ti, 0));
        let integrity = desfire
            .write_data(2, 0x40, &data, CommunicationMode::Enciphered)
            .expect("enciphered write");
        assert_eq!(integrity, IntegrityCheck::Ev2Cmac);
        drop(desfire);
        assert_eq!(
            reader.commands[0][5..reader.commands[0].len() - 1],
            expected
        );
    }

    #[test]
    fn write_rejects_a_tampered_response_mac_and_drops_the_session() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(vec![0u8; 8], [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let error = desfire
            .write_data(2, 0, &[0xAA], CommunicationMode::Maced)
            .expect_err("the response MAC is wrong");
        assert!(matches!(error, NfcError::VerificationFailed(_)));
        assert!(
            desfire.session_key().is_none(),
            "the session must be dropped"
        );
    }

    #[test]
    fn write_rejects_a_refused_status_and_drops_the_session() {
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(Vec::new(), [0x91, 0x9D])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &key, vec![0u8; 16]));
        let error = desfire
            .write_data(2, 0, &[0xAA], CommunicationMode::Maced)
            .expect_err("the card refused the write");
        assert!(matches!(error, NfcError::VerificationFailed(_)));
        assert!(
            desfire.session_key().is_none(),
            "the session must be dropped"
        );
    }

    #[test]
    fn value_adjustments_send_the_instruction_and_little_endian_amount() {
        for (adjustment, instruction) in [
            (ValueAdjustment::Credit, 0x0C),
            (ValueAdjustment::Debit, 0xDC),
            (ValueAdjustment::LimitedCredit, 0x1C),
        ] {
            let mut reader = ScriptedReader::new([0x91, 0x00]);
            let mut desfire = Desfire::new(&mut reader);
            desfire
                .adjust_value(1, adjustment, 0x0102_0304, CommunicationMode::Plain)
                .expect("value adjustment");
            drop(desfire);
            assert_eq!(
                reader.commands,
                [vec![
                    0x90,
                    instruction,
                    0x00,
                    0x00,
                    0x05,
                    0x01,
                    0x04,
                    0x03,
                    0x02,
                    0x01,
                    0x00
                ]]
            );
        }
    }

    #[test]
    fn update_record_sends_record_number_offset_and_length() {
        let mut reader = ScriptedReader::new([0x91, 0x00]);
        let mut desfire = Desfire::new(&mut reader);
        desfire
            .update_record(
                2,
                0x0000_0302,
                0x0000_0104,
                &[0xAA, 0xBB],
                CommunicationMode::Plain,
            )
            .expect("record update");
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![
                0x90, 0xDB, 0x00, 0x00, 0x0C, 0x02, 0x02, 0x03, 0x00, 0x04, 0x01, 0x00, 0x02, 0x00,
                0x00, 0xAA, 0xBB, 0x00
            ]]
        );
    }

    #[test]
    fn clear_record_file_sends_the_file_number() {
        let mut reader = ScriptedReader::new([0x91, 0x00]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.clear_record_file(0x0A).expect("record clear");
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![0x90, 0xEB, 0x00, 0x00, 0x01, 0x0A, 0x00]]
        );
    }

    #[test]
    fn clear_record_file_is_maced_under_ev2() {
        let mac_key = hex("9366FA195EB566F5BD2BAD4020B83002");
        let enc_key = hex("FFBCFE1F41840A09C9A88D0A4B10DF05");
        let ti = [0xE2, 0xD3, 0xAF, 0x69];
        let header = vec![0x0A];
        let command_mac = ev2_session(&mac_key, &enc_key, ti, 0)
            .command_mac(INS_CLEAR_RECORD_FILE, &header)
            .expect("command MAC");
        let response_mac = ev2_session(&mac_key, &enc_key, ti, 1)
            .response_mac(&[], 0x00)
            .expect("response MAC");
        let mut expected = header.clone();
        expected.extend_from_slice(&ev2_truncate(&command_mac));

        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            ev2_truncate(&response_mac).to_vec(),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev2_session(&mac_key, &enc_key, ti, 0));
        let integrity = desfire.clear_record_file(0x0A).expect("EV2 clear");
        assert_eq!(integrity, IntegrityCheck::Ev2Cmac);
        drop(desfire);
        assert_eq!(
            reader.commands[0][5..reader.commands[0].len() - 1],
            expected
        );
    }

    #[test]
    fn transaction_mac_input_pads_each_command_to_sixteen_bytes() {
        let mut reader = ScriptedReader::with_responses(vec![
            ApduResponse::new(7i32.to_le_bytes().to_vec(), [0x91, 0x00]),
            ApduResponse::new(Vec::new(), [0x91, 0x00]),
        ]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Des, &[0u8; 8], Vec::new()));
        desfire
            .get_value(1, CommunicationMode::Plain)
            .expect("read");
        desfire
            .adjust_value(1, ValueAdjustment::Credit, 3, CommunicationMode::Plain)
            .expect("credit");
        let expected = hex("6C 01 07 00 00 00 00 00 00 00 00 00 00 00 00 00 \
             0C 01 03 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(desfire.transaction_mac_input(), expected.as_slice());
    }

    #[test]
    fn transaction_mac_matches_an_independent_vector() {
        let key = hex("00112233445566778899AABBCCDDEEFF");
        let key: [u8; 16] = key.try_into().expect("16-byte key");
        let uid = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];
        let tmi = hex("6C 01 07 00 00 00 00 00 00 00 00 00 00 00 00 00 \
             0C 01 03 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(
            transaction_mac(&key, 5, &uid, &tmi).expect("TMV"),
            [0x08, 0x6E, 0xD1, 0x76, 0xD1, 0x1B, 0x4A, 0x91]
        );
    }

    #[test]
    fn commit_with_mac_returns_the_counter_and_value() {
        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            hex("05 00 00 00 08 6E D1 76 D1 1B 4A 91"),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        let mac = desfire
            .commit_transaction_with_mac()
            .expect("commit")
            .expect("returned MAC");
        assert_eq!(mac.counter, 5);
        assert_eq!(mac.value, [0x08, 0x6E, 0xD1, 0x76, 0xD1, 0x1B, 0x4A, 0x91]);
        drop(desfire);
        assert_eq!(
            reader.commands,
            [vec![0x90, 0xC7, 0x00, 0x00, 0x01, 0x01, 0x00]]
        );
    }

    #[test]
    fn transaction_mac_file_read_is_excluded_from_the_input() {
        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            hex("01 00 00 00 08 6E D1 76 D1 1B 4A 91"),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.session = Some(ev1_session(AuthKeyType::Des, &[0u8; 8], Vec::new()));
        let (mac, _) = desfire
            .read_transaction_mac_file(0x0F, CommunicationMode::Plain)
            .expect("TMAC file read");
        assert_eq!(mac.counter, 1);
        assert!(desfire.transaction_mac_input().is_empty());
    }

    #[test]
    fn change_key_ev2_case_1_matches_an12343() {
        // AN12343 9.2.1: authenticated with key 0, change key 1.
        let mac_key = hex("605526E474B2304354D05A800F56B7A9");
        let enc_key = hex("A664EA370E90B249FE2870772291A820");
        let mut reader = ScriptedReader::with_responses(vec![ApduResponse::new(
            hex("BB94CB85EBC43B9D"),
            [0x91, 0x00],
        )]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.selected = [0xF0, 0x0D, 0x01];
        desfire.session = Some(ev2_session(&mac_key, &enc_key, [0xBC, 0x35, 0x4C, 0xD5], 0));
        desfire
            .change_key(KeyChange {
                key_number: 1,
                new_key: hex("01234567890123456789012345678901"),
                new_key_version: 0,
                current_key: Some(vec![0u8; 16]),
            })
            .expect("ChangeKey case 1");
        assert!(
            desfire.session.is_some(),
            "changing another key keeps the session"
        );
        drop(desfire);
        assert_eq!(
            reader.commands,
            [hex("90 C4 00 00 29 \
                 01 80D40DB52D5D8CA136249A0A14154DBA1BE0D67C408AB24CF0F3D3B4FE333C6A \
                 D27EF8006B374ABB 00")]
        );
    }

    #[test]
    fn change_key_ev2_case_2_matches_an12343_and_ends_the_session() {
        // AN12343 9.2.2: authenticated with key 0, change key 0.
        let mac_key = hex("45D50C1570000D2F173DF949288E3CAD");
        let enc_key = hex("E156C8522F7C8DC82B0C99BA847DE723");
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(Vec::new(), [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.selected = [0xF0, 0x0D, 0x01];
        desfire.session = Some(ev2_session(&mac_key, &enc_key, [0x94, 0x29, 0x7F, 0x4D], 0));
        desfire
            .change_key(KeyChange {
                key_number: 0,
                new_key: hex("01234567890123456789012345678901"),
                new_key_version: 0,
                current_key: None,
            })
            .expect("ChangeKey case 2");
        assert!(
            desfire.session.is_none(),
            "changing the authenticated key ends the session"
        );
        drop(desfire);
        assert_eq!(
            reader.commands,
            [hex("90 C4 00 00 29 \
                 00 BF5400DC97A1FBD65BE870716D6F11F8161BB4CA472856DB94AB94B2EC1A13E6 \
                 27CE07CF56C11091 00")]
        );
    }

    #[test]
    fn change_key_requires_the_old_key_for_another_key_number() {
        let mut reader = ScriptedReader::new([0x91, 0x00]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.selected = [0xF0, 0x0D, 0x01];
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &[0xA5; 16], vec![0u8; 16]));
        let error = desfire
            .change_key(KeyChange::new(1, vec![0x11; 16]))
            .expect_err("key 1 needs its current value");
        assert!(error.to_string().contains("current value"), "{error}");
        assert!(reader.commands.is_empty(), "no command before the check");
    }

    #[test]
    fn des_key_version_is_embedded_in_parity_bits() {
        let mut key = [0xFFu8; 24];
        set_des_key_version(&mut key, 0x05);
        // Bits are written LSB first from the last byte, repeating every 8.
        assert_eq!(key[23] & 1, 1);
        assert_eq!(key[22] & 1, 0);
        assert_eq!(key[21] & 1, 1);
        assert_eq!(key[20] & 1, 0);
        assert_eq!(key[19] & 1, 0);
        assert_eq!(key[16] & 1, 0);
        assert_eq!(key[15] & 1, 1);
        assert_eq!(key[23] & 0xFE, 0xFE);
    }

    #[test]
    fn change_key_ev1_aes_uses_header_crc_iv_and_a_maced_response() {
        let session_key = [0x11u8; 16];
        let iv = [0x22u8; 16];
        let new_key = [0x33u8; 16];
        let current = [0x44u8; 16];
        let expected_plain = {
            let mut plain = new_key.to_vec();
            plain.push(0x05);
            xor_key(&mut plain, Some(&current), 16);
            let mut crc_input = vec![INS_CHANGE_KEY, 0x01];
            crc_input.extend_from_slice(&plain);
            let crc = crc32_desfire(&crc_input);
            plain.extend_from_slice(&crc);
            let crc = crc32_desfire(&new_key);
            plain.extend_from_slice(&crc);
            while !plain.len().is_multiple_of(16) {
                plain.push(0);
            }
            plain
        };
        let expected_cipher =
            aes_cbc_encrypt(&session_key, &iv, &expected_plain).expect("CBC encryption");
        let response_mac = {
            let next_iv = &expected_cipher[expected_cipher.len() - 16..];
            ev1_cmac(AuthKeyType::Aes, &session_key, next_iv, &[0x00]).expect("response MAC")[..8]
                .to_vec()
        };
        let mut reader =
            ScriptedReader::with_responses(vec![ApduResponse::new(response_mac, [0x91, 0x00])]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.selected = [0xF0, 0x0D, 0x01];
        desfire.session = Some(ev1_session(AuthKeyType::Aes, &session_key, iv.to_vec()));
        let result = desfire.change_key(KeyChange {
            key_number: 1,
            new_key: new_key.to_vec(),
            new_key_version: 0x05,
            current_key: Some(current.to_vec()),
        });
        let dump = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>()
        };
        drop(desfire);
        eprintln!("expected cipher: {}", dump(&expected_cipher));
        eprintln!(
            "commands: {:?}",
            reader.commands.iter().map(|c| dump(c)).collect::<Vec<_>>()
        );
        result.expect("EV1 AES ChangeKey");
        assert_eq!(reader.commands.len(), 1);
        let data = &reader.commands[0][5..reader.commands[0].len() - 1];
        assert_eq!(data[0], 0x01, "KeyNo header");
        assert_eq!(&data[1..], expected_cipher.as_slice());
    }

    #[test]
    fn change_key_d40_encodes_crc16_and_zero_iv() {
        let session_key = [0x11u8; 8];
        let new_key = [0x33u8; 8];
        let current = [0x44u8; 8];
        let expected_plain = {
            let mut plain = KeyFamily::Tdes2.key_area(&new_key, 0x02);
            xor_key(&mut plain, Some(&current), 16);
            let crc = crc16_a(&plain);
            plain.extend_from_slice(&crc);
            let crc = crc16_a(&KeyFamily::Tdes2.key_area(&new_key, 0x02));
            plain.extend_from_slice(&crc);
            while !plain.len().is_multiple_of(8) {
                plain.push(0);
            }
            plain
        };
        let expected_cipher = d40_send(
            &expand_tdes_key(&session_key).expect("key"),
            &expected_plain,
        );
        let mut reader = ScriptedReader::with_responses(vec![
            ApduResponse::new(vec![0x00], [0x91, 0x00]),
            ApduResponse::new(Vec::new(), [0x91, 0x00]),
        ]);
        let mut desfire = Desfire::new(&mut reader);
        desfire.selected = [0xF0, 0x0D, 0x01];
        desfire.session = Some(ev1_session(AuthKeyType::Des, &session_key, Vec::new()));
        desfire
            .change_key(KeyChange {
                key_number: 1,
                new_key: new_key.to_vec(),
                new_key_version: 0x02,
                current_key: Some(current.to_vec()),
            })
            .expect("D40 ChangeKey");
        drop(desfire);
        assert_eq!(reader.commands.len(), 2, "GetKeyVersion, then ChangeKey");
        let data = &reader.commands[1][5..reader.commands[1].len() - 1];
        assert_eq!(data[0], 0x01);
        assert_eq!(&data[1..], expected_cipher.as_slice());
    }

    #[test]
    fn empty_apdu_uses_case_two_short() {
        // The APDU builder is private, but the empty-data path is the one that
        // GET_VERSION needs; this guards the 5-byte shape at the protocol level.
        let mut apdu = Vec::new();
        apdu.extend_from_slice(&[CLA, INS_GET_VERSION, 0x00, 0x00]);
        apdu.push(0x00);
        assert_eq!(apdu.len(), 5);
    }
}
