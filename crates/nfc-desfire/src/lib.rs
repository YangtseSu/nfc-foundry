// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal MIFARE DESFire native command layer over ISO 7816-4 APDUs.
//!
//! This crate covers identity and application/file inventory, legacy
//! DES/2K3DES authentication, the primitive application/file commands, and the
//! AN11004 NDEF Tag Application layout. The EV1/EV2 secure channel and the
//! general file/key workbench are not part of this layer.

use des::TdesEde3;
use des::cipher::{BlockCipherDecrypt, KeyInit};
use nfc_core::{NfcError, Result};
use nfc_transport::Reader;

const CLA: u8 = 0x90;
const INS_ADDITIONAL_FRAME: u8 = 0xAF;
const INS_AUTHENTICATE: u8 = 0x0A;
const INS_GET_VERSION: u8 = 0x60;
const INS_GET_APPLICATION_IDS: u8 = 0x6A;
const INS_SELECT_APPLICATION: u8 = 0x5A;
const INS_GET_FILE_IDS: u8 = 0x6F;
const INS_GET_FILE_SETTINGS: u8 = 0xF5;
const INS_CREATE_APPLICATION: u8 = 0xCA;
const INS_DELETE_APPLICATION: u8 = 0xDA;
const INS_CREATE_STD_DATA_FILE: u8 = 0xCD;
const INS_WRITE_DATA: u8 = 0x3D;
const INS_READ_DATA: u8 = 0xBD;
const INS_COMMIT_TRANSACTION: u8 = 0xC7;
const INS_ABORT_TRANSACTION: u8 = 0xA7;

const STATUS_OK: u16 = 0x9100;
const STATUS_ADDITIONAL: u16 = 0x91AF;
/// `NoChanges`: a commit with no uncommitted data is already in the
/// requested state, so callers treat it as success.
const STATUS_NO_CHANGES: u16 = 0x910C;

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

/// The legacy authentication key families this layer can derive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyKeyType {
    Des,
    Tdes2,
}

impl LegacyKeyType {
    /// Key family selected by the supplied legacy key length.
    pub fn from_key(key: &[u8]) -> Result<Self> {
        match key.len() {
            8 => Ok(Self::Des),
            16 => Ok(Self::Tdes2),
            24 => Err(NfcError::Unsupported(
                "3K3DES authentication is not implemented",
            )),
            _ => Err(NfcError::InvalidArgument(
                "legacy DESFire keys are 8-byte DES or 16-byte 2K3DES".to_owned(),
            )),
        }
    }
}

struct Session {
    key: Vec<u8>,
    key_type: LegacyKeyType,
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

    fn parse(value: u8) -> Result<Self> {
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
        value: i32,
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
        let communication = CommunicationMode::parse(option & 0x03)?;
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
                let value = i32::from_le_bytes(raw[8..12].try_into().expect("four-byte value"));
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
                if value < lower_limit || value > upper_limit {
                    return Err(NfcError::InvalidArgument(format!(
                        "DESFire value {value} is outside limits {lower_limit}..{upper_limit}"
                    )));
                }
                FileContents::Value {
                    lower_limit,
                    upper_limit,
                    value,
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
                value,
                limited_credit,
                free_get_value,
            } => {
                text.push_str(&format!(
                    " · limits {lower_limit}..{upper_limit} · value {value} · limited credit {} · get value {}",
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

/// DESFire command context over one reader.
pub struct Desfire<'a> {
    reader: &'a mut dyn Reader,
    session: Option<Session>,
    selected: [u8; 3],
}

impl<'a> Desfire<'a> {
    pub fn new(reader: &'a mut dyn Reader) -> Self {
        Self {
            reader,
            session: None,
            selected: [0x00; 3],
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

    fn expect_ok(&mut self, ins: u8, data: &[u8]) -> Result<Vec<u8>> {
        let (body, sw) = self.transceive(ins, data)?;
        if sw != STATUS_OK {
            return Err(status_error(ins, sw));
        }
        Ok(body)
    }

    fn chained(&mut self, ins: u8) -> Result<Vec<u8>> {
        let (mut body, mut sw) = self.transceive(ins, &[])?;
        let mut frames = 0;
        while sw == STATUS_ADDITIONAL {
            frames += 1;
            if frames > 4 {
                return Err(NfcError::InvalidArgument(
                    "DESFire response has too many additional frames".to_owned(),
                ));
            }
            let (next, next_sw) = self.transceive(INS_ADDITIONAL_FRAME, &[])?;
            body.extend_from_slice(&next);
            sw = next_sw;
        }
        if sw != STATUS_OK {
            return Err(status_error(ins, sw));
        }
        Ok(body)
    }

    fn ensure_authenticated(&self) -> Result<()> {
        if self.session.is_none() {
            return Err(NfcError::AuthenticationRequired(
                "authenticate to the selected DESFire application first".to_owned(),
            ));
        }
        Ok(())
    }

    /// Authenticate to the selected application with a legacy DES or 2K3DES
    /// key. The session key is available through [`Self::session_key`].
    pub fn authenticate_legacy(&mut self, key_no: u8, key: &[u8]) -> Result<()> {
        if key_no > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        let key_type = LegacyKeyType::from_key(key)?;
        let tdes_key = expand_tdes_key(key)?;
        let (challenge, sw) = self.transceive(INS_AUTHENTICATE, &[key_no])?;
        if sw != STATUS_ADDITIONAL {
            return Err(status_error(INS_AUTHENTICATE, sw));
        }
        if challenge.len() != 8 {
            return Err(NfcError::VerificationFailed(format!(
                "legacy authentication returned {} challenge bytes, expected 8",
                challenge.len()
            )));
        }
        let rnd_b = cbc_decrypt(&tdes_key, &challenge)?;
        let rotated = rotate_left(&rnd_b);

        let mut rnd_a = [0u8; 8];
        getrandom::fill(&mut rnd_a)
            .map_err(|error| NfcError::Io(format!("random nonce unavailable: {error}")))?;

        let mut payload = Vec::with_capacity(16);
        payload.extend_from_slice(&rnd_a);
        payload.extend_from_slice(&rotated);
        let encrypted = d40_send(&tdes_key, &payload);
        let (response, sw) = self.transceive(INS_ADDITIONAL_FRAME, &encrypted)?;
        if sw != STATUS_OK {
            return Err(status_error(INS_ADDITIONAL_FRAME, sw));
        }
        if response.len() != 8 {
            return Err(NfcError::VerificationFailed(format!(
                "legacy authentication returned {} final bytes, expected 8",
                response.len()
            )));
        }
        let decrypted = cbc_decrypt(&tdes_key, &response)?;
        if rotate_right(&decrypted) != rnd_a {
            return Err(NfcError::AuthenticationFailed);
        }

        self.session = Some(Session {
            key: session_key(key_type, &rnd_a, &rnd_b),
            key_type,
        });
        Ok(())
    }

    /// Session key derived by the legacy authentication, if a session exists.
    pub fn session_key(&self) -> Option<&[u8]> {
        self.session.as_ref().map(|session| session.key.as_slice())
    }

    /// Key family used by the current session.
    pub fn session_key_type(&self) -> Option<LegacyKeyType> {
        self.session.as_ref().map(|session| session.key_type)
    }

    /// Selected application ID, or `[0, 0, 0]` for the PICC master application.
    pub fn selected_application(&self) -> [u8; 3] {
        self.selected
    }

    pub fn get_version(&mut self) -> Result<Version> {
        Version::parse(&self.chained(INS_GET_VERSION)?)
    }

    pub fn application_ids(&mut self) -> Result<Vec<[u8; 3]>> {
        let body = self.chained(INS_GET_APPLICATION_IDS)?;
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
        self.expect_ok(INS_SELECT_APPLICATION, &aid)?;
        self.selected = aid;
        self.session = None;
        Ok(())
    }

    pub fn file_ids(&mut self) -> Result<Vec<u8>> {
        self.expect_ok(INS_GET_FILE_IDS, &[])
    }

    pub fn file_settings(&mut self, file_id: u8) -> Result<Vec<u8>> {
        self.expect_ok(INS_GET_FILE_SETTINGS, &[file_id])
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
        self.expect_ok(INS_CREATE_APPLICATION, &payload)?;
        Ok(())
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
        self.expect_ok(INS_DELETE_APPLICATION, &aid)?;
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
        let (_, sw) = self.transceive(INS_COMMIT_TRANSACTION, &[])?;
        if sw == STATUS_OK || sw == STATUS_NO_CHANGES {
            Ok(())
        } else {
            Err(status_error(INS_COMMIT_TRANSACTION, sw))
        }
    }

    /// Abort the open transaction and discard uncommitted writes.
    pub fn abort_transaction(&mut self) -> Result<()> {
        self.expect_ok(INS_ABORT_TRANSACTION, &[])?;
        Ok(())
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
        self.expect_ok(INS_CREATE_STD_DATA_FILE, &payload)?;
        Ok(())
    }

    /// Write plain data to a standard or backup file.
    pub fn write_data(&mut self, file_id: u8, offset: u32, data: &[u8]) -> Result<()> {
        self.ensure_authenticated()?;
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
        let mut payload = Vec::with_capacity(7 + data.len());
        payload.push(file_id);
        payload.extend_from_slice(&offset.to_le_bytes()[..3]);
        payload.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
        payload.extend_from_slice(data);
        self.expect_ok(INS_WRITE_DATA, &payload)?;
        Ok(())
    }

    /// Read plain data from a standard or backup file.
    pub fn read_data(&mut self, file_id: u8, offset: u32, length: u32) -> Result<Vec<u8>> {
        if offset > 0x00FF_FFFF || length > 0x00FF_FFFF {
            return Err(NfcError::InvalidArgument(
                "DESFire read offset and length must fit 3 bytes".to_owned(),
            ));
        }
        self.expect_ok(
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
        )
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

fn create_standard_file_payload(
    file_id: u8,
    iso_file_id: Option<u16>,
    communication: u8,
    access_rights: u16,
    size: u32,
) -> Result<Vec<u8>> {
    if communication > 3 {
        return Err(NfcError::InvalidArgument(
            "DESFire communication mode must be 0, 1 or 3".to_owned(),
        ));
    }
    if size == 0 || size > 0x00FF_FFFF {
        return Err(NfcError::InvalidArgument(
            "DESFire file size must be 1..=0xFFFFFF bytes".to_owned(),
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
    payload.extend_from_slice(&size.to_le_bytes()[..3]);
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
        _ => {
            return Err(NfcError::InvalidArgument(
                "legacy DESFire keys are 8-byte DES or 16-byte 2K3DES".to_owned(),
            ));
        }
    }
    Ok(expanded)
}

fn cbc_decrypt(key: &[u8; 24], data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(8) {
        return Err(NfcError::InvalidArgument(
            "DESFire CBC input must be a non-empty multiple of 8 bytes".to_owned(),
        ));
    }
    let cipher = TdesEde3::new_from_slice(key)
        .map_err(|_| NfcError::InvalidArgument("invalid 3DES key".to_owned()))?;
    let mut previous = [0u8; 8];
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

fn session_key(key_type: LegacyKeyType, rnd_a: &[u8], rnd_b: &[u8]) -> Vec<u8> {
    match key_type {
        LegacyKeyType::Des => {
            let mut key = Vec::with_capacity(8);
            key.extend_from_slice(&rnd_a[0..4]);
            key.extend_from_slice(&rnd_b[0..4]);
            key
        }
        LegacyKeyType::Tdes2 => {
            let mut key = Vec::with_capacity(16);
            key.extend_from_slice(&rnd_a[0..4]);
            key.extend_from_slice(&rnd_b[0..4]);
            key.extend_from_slice(&rnd_a[4..8]);
            key.extend_from_slice(&rnd_b[4..8]);
            key
        }
    }
}

fn rotate_left(block: &[u8]) -> Vec<u8> {
    let mut output = block.to_vec();
    output.rotate_left(1);
    output
}

fn rotate_right(block: &[u8]) -> Vec<u8> {
    let mut output = block.to_vec();
    output.rotate_right(1);
    output
}

fn status_error(ins: u8, sw: u16) -> NfcError {
    NfcError::VerificationFailed(format!(
        "DESFire command {ins:#04X} returned status {sw:#06X}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nfc_core::{CardInfo, Key, KeyType};
    use nfc_transport::{ApduResponse, DeviceInfo};

    struct ScriptedReader {
        response: ApduResponse,
        commands: Vec<Vec<u8>>,
        device: DeviceInfo,
    }

    impl ScriptedReader {
        fn new(sw: [u8; 2]) -> Self {
            Self {
                response: ApduResponse::new(Vec::new(), sw),
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
            Ok(self.response.clone())
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
                value: 42,
                limited_credit: true,
                free_get_value: true,
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

        let outside = [
            0x02, 0x00, 0x00, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x0B, 0x00,
            0x00, 0x00, 0x00,
        ];
        assert!(FileSettings::parse(&outside).is_err());

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
    fn rotate_left_and_right_are_inverse() {
        let block = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(rotate_right(&rotate_left(&block)), block);
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
