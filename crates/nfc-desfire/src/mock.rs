// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scripted DESFire card for service and workflow tests.
//!
//! The card answers native DESFire APDUs for legacy DES/2K3DES, 3K3DES, and
//! AES EV1 authentication, GetKeyVersion, GetCardUID, GetFileSettings,
//! ReadData, WriteData, CommitTransaction, AbortTransaction, and FormatPICC.
//! It also answers the ISO 7816-4 SELECT, READ BINARY, and UPDATE BINARY
//! commands that the Type 4 reader sends to the NDEF Tag Application. EV2 and
//! secure-messaging file writes are not simulated; they return
//! `IllegalCommand`.
//!
//! This module is compiled for tests and for callers that enable the
//! `mock-card` feature. It is not part of the production command path.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use des::TdesEde3;
use des::cipher::{BlockCipherEncrypt, KeyInit};
use nfc_core::{
    BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result,
    TYPE2_VERSION_SIZE,
};
use nfc_transport::{ApduCapabilities, ApduResponse, DeviceInfo, Reader, Transport};

use crate::{
    AuthKeyType, INS_ABORT_TRANSACTION, INS_ADDITIONAL_FRAME, INS_CHANGE_FILE_SETTINGS,
    INS_CHANGE_KEY, INS_CHANGE_KEY_SETTINGS, INS_CLEAR_RECORD_FILE, INS_COMMIT_TRANSACTION,
    INS_CREATE_APPLICATION, INS_CREATE_BACKUP_DATA_FILE, INS_CREATE_CYCLIC_RECORD_FILE,
    INS_CREATE_LINEAR_RECORD_FILE, INS_CREATE_STD_DATA_FILE, INS_CREATE_TRANSACTION_MAC_FILE,
    INS_CREATE_VALUE_FILE, INS_CREDIT, INS_DEBIT, INS_DELETE_APPLICATION, INS_DELETE_FILE,
    INS_FORMAT_PICC, INS_GET_APPLICATION_IDS, INS_GET_CARD_UID, INS_GET_FILE_IDS,
    INS_GET_FILE_SETTINGS, INS_GET_FREE_MEMORY, INS_GET_KEY_SETTINGS, INS_GET_KEY_VERSION,
    INS_GET_VALUE, INS_GET_VERSION, INS_LIMITED_CREDIT, INS_READ_DATA, INS_READ_RECORDS,
    INS_SELECT_APPLICATION, INS_UPDATE_RECORD, INS_WRITE_DATA, INS_WRITE_RECORD,
    NDEF_APPLICATION_AID, NDEF_DF_NAME, aes_cbc_decrypt, aes_cbc_encrypt, cbc_decrypt_iv,
    cbc_encrypt_iv, crc16_a, crc32_desfire, derive_session_key, ev1_cmac, expand_tdes_key,
    last_block, rotate_left,
};

const STATUS_OK: [u8; 2] = [0x91, 0x00];
const STATUS_ADDITIONAL: [u8; 2] = [0x91, 0xAF];
const STATUS_NO_MEMORY: [u8; 2] = [0x91, 0x0E];
const STATUS_APPLICATION_NOT_FOUND: [u8; 2] = [0x91, 0xA0];
const STATUS_AUTHENTICATION_ERROR: [u8; 2] = [0x91, 0xAE];
const STATUS_ILLEGAL_COMMAND: [u8; 2] = [0x91, 0x1C];
const STATUS_PARAMETER_ERROR: [u8; 2] = [0x91, 0x9E];
const STATUS_NO_SUCH_KEY: [u8; 2] = [0x91, 0x40];
const STATUS_PERMISSION_DENIED: [u8; 2] = [0x91, 0x9D];
const STATUS_BOUNDARY_ERROR: [u8; 2] = [0x91, 0xBE];
const STATUS_LENGTH_ERROR: [u8; 2] = [0x91, 0x7E];
const STATUS_INTEGRITY_ERROR: [u8; 2] = [0x91, 0x1E];
const STATUS_FILE_NOT_FOUND: [u8; 2] = [0x91, 0xDE];
const STATUS_FILE_EXISTS: [u8; 2] = [0x91, 0xF0];
const STATUS_DUPLICATE: [u8; 2] = [0x91, 0xEE];

const PICC: [u8; 3] = [0x00; 3];
const DEFAULT_APP: [u8; 3] = [0xF0, 0x0D, 0x01];

const DEFAULT_UID: [u8; 7] = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];

enum AuthState {
    Idle,
    Challenge {
        key_no: u8,
        key: Vec<u8>,
        rnd_b: Vec<u8>,
        challenge: Vec<u8>,
    },
    Session {
        key_no: u8,
        session_key: Vec<u8>,
        iv: Vec<u8>,
    },
}

struct MockKey {
    value: Vec<u8>,
    version: u8,
}

struct MockFile {
    settings: Vec<u8>,
    data: Vec<u8>,
    /// ISO 7816-4 file ID, used by the Type 4 SELECT FILE path.
    iso_file_id: Option<u16>,
    /// Current value for a Value file, returned by GetValue.
    value: i32,
    /// Value committed by the next CommitTransaction.
    pending_value: Option<i32>,
    /// Records committed by the next CommitTransaction.
    pending_data: Option<Vec<u8>>,
    /// Settings committed by the next CommitTransaction.
    pending_settings: Option<Vec<u8>>,
    /// A WriteRecord created the current record and later writes target it.
    record_open: bool,
}

#[derive(Default)]
struct MockApp {
    files: BTreeMap<u8, MockFile>,
}

/// A card-side DESFire simulator.
pub struct MockDesfire {
    key_type: AuthKeyType,
    keys: BTreeMap<u8, MockKey>,
    key_settings: u8,
    max_keys: u8,
    uid: [u8; 7],
    corrupt_mac: bool,
    corrupt_uid: bool,
    fail_commit: bool,
    corrupt_readback: bool,
    wrote: bool,
    state: AuthState,
    applications: BTreeMap<[u8; 3], MockApp>,
    selected: [u8; 3],
    /// ISO 7816-4 file selected by SELECT FILE, used by READ/UPDATE BINARY.
    iso_file: Option<u16>,
    free_memory: u32,
    /// EEPROM size restored by FormatPICC.
    total_memory: u32,
    commit_mac: Option<(u32, [u8; 8])>,
    commands: Vec<Vec<u8>>,
}

impl Default for MockDesfire {
    fn default() -> Self {
        Self::new(vec![0u8; 8], AuthKeyType::Des)
    }
}

impl MockDesfire {
    pub fn new(key: impl Into<Vec<u8>>, key_type: AuthKeyType) -> Self {
        let key = key.into();
        key_type
            .validate_key(&key)
            .expect("the mock key matches its family");
        let mut keys = BTreeMap::new();
        keys.insert(
            0,
            MockKey {
                value: key,
                version: 0,
            },
        );
        Self {
            key_type,
            keys,
            key_settings: 0x0F,
            max_keys: 1,
            uid: DEFAULT_UID,
            corrupt_mac: false,
            corrupt_uid: false,
            fail_commit: false,
            corrupt_readback: false,
            wrote: false,
            state: AuthState::Idle,
            applications: BTreeMap::new(),
            selected: PICC,
            iso_file: None,
            free_memory: 8192,
            total_memory: 8192,
            commit_mac: None,
            commands: Vec::new(),
        }
    }

    /// Add a Standard or Backup file to the default reserved application.
    pub fn with_file(mut self, file_id: u8, settings: Vec<u8>, data: Vec<u8>) -> Self {
        self.applications
            .entry(DEFAULT_APP)
            .or_default()
            .files
            .insert(
                file_id,
                MockFile {
                    settings,
                    data,
                    iso_file_id: None,
                    value: 0,
                    pending_value: None,
                    pending_data: None,
                    pending_settings: None,
                    record_open: false,
                },
            );
        self
    }

    /// Add a Value file with a committed current value.
    pub fn with_value(mut self, file_id: u8, settings: Vec<u8>, value: i32) -> Self {
        self.applications
            .entry(DEFAULT_APP)
            .or_default()
            .files
            .insert(
                file_id,
                MockFile {
                    settings,
                    data: Vec::new(),
                    iso_file_id: None,
                    value,
                    pending_value: None,
                    pending_data: None,
                    pending_settings: None,
                    record_open: false,
                },
            );
        self
    }

    /// Add a file to a named application, creating it when absent.
    pub fn with_file_in(
        mut self,
        aid: [u8; 3],
        file_id: u8,
        settings: Vec<u8>,
        data: Vec<u8>,
    ) -> Self {
        self.applications.entry(aid).or_default().files.insert(
            file_id,
            MockFile {
                settings,
                data,
                iso_file_id: None,
                value: 0,
                pending_value: None,
                pending_data: None,
                pending_settings: None,
                record_open: false,
            },
        );
        self
    }

    /// Seed an application with no files.
    pub fn with_application(mut self, aid: [u8; 3]) -> Self {
        self.applications.entry(aid).or_default();
        self
    }

    /// Free EEPROM bytes reported by GetFreeMemory.
    pub fn with_free_memory(mut self, free_memory: u32) -> Self {
        self.free_memory = free_memory;
        self
    }

    /// Return this Transaction MAC from CommitTransaction when a Transaction
    /// MAC file is present.
    pub fn with_commit_mac(mut self, counter: u32, value: [u8; 8]) -> Self {
        self.commit_mac = Some((counter, value));
        self
    }

    /// Refuse CommitTransaction, for the failed-commit path.
    pub fn with_commit_failure(mut self) -> Self {
        self.fail_commit = true;
        self
    }

    /// Corrupt the first read after a write, for the failed read-back path.
    pub fn with_corrupt_readback(mut self) -> Self {
        self.corrupt_readback = true;
        self
    }

    /// Current file content in the default reserved application.
    pub fn file_data(&self, file_id: u8) -> Option<&[u8]> {
        self.file_data_in(DEFAULT_APP, file_id)
    }

    /// Current file content in a named application.
    pub fn file_data_in(&self, aid: [u8; 3], file_id: u8) -> Option<&[u8]> {
        self.applications
            .get(&aid)
            .and_then(|app| app.files.get(&file_id))
            .map(|file| file.data.as_slice())
    }

    /// Raw GetFileSettings response stored for a file.
    pub fn file_settings_in(&self, aid: [u8; 3], file_id: u8) -> Option<&[u8]> {
        self.applications
            .get(&aid)
            .and_then(|app| app.files.get(&file_id))
            .map(|file| file.settings.as_slice())
    }

    /// Overwrite a file's content, for corrupt-data and read-back tests.
    pub fn set_file_data(&mut self, aid: [u8; 3], file_id: u8, data: &[u8]) {
        if let Some(file) = self
            .applications
            .get_mut(&aid)
            .and_then(|app| app.files.get_mut(&file_id))
        {
            file.data = data.to_vec();
        }
    }

    /// Application IDs currently stored on the card.
    pub fn application_ids(&self) -> Vec<[u8; 3]> {
        self.applications.keys().copied().collect()
    }

    /// File IDs in a named application.
    pub fn file_ids_in(&self, aid: [u8; 3]) -> Vec<u8> {
        self.applications
            .get(&aid)
            .map(|app| app.files.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Free EEPROM bytes.
    pub fn free_memory(&self) -> u32 {
        self.free_memory
    }

    pub fn with_key_number(mut self, key_no: u8) -> Self {
        if key_no != 0
            && let Some(key) = self.keys.remove(&0)
        {
            self.keys.insert(key_no, key);
        }
        self
    }

    pub fn with_key_version(mut self, key_version: u8) -> Self {
        for key in self.keys.values_mut() {
            key.version = key_version;
        }
        self
    }

    /// Store one key value and version at a key number.
    pub fn with_key(mut self, key_no: u8, key: impl Into<Vec<u8>>, version: u8) -> Self {
        self.keys.insert(
            key_no,
            MockKey {
                value: key.into(),
                version,
            },
        );
        self
    }

    /// KeySettings byte returned by GetKeySettings.
    pub fn with_key_settings(mut self, key_settings: u8) -> Self {
        self.key_settings = key_settings;
        self
    }

    /// Key count reported by GetKeySettings.
    pub fn with_max_keys(mut self, max_keys: u8) -> Self {
        self.max_keys = max_keys;
        self
    }

    /// Stored key value, for asserting a successful ChangeKey.
    pub fn key_value(&self, key_no: u8) -> Option<&[u8]> {
        self.keys.get(&key_no).map(|key| key.value.as_slice())
    }

    /// Stored key version, for asserting a successful ChangeKey.
    pub fn key_version_of(&self, key_no: u8) -> Option<u8> {
        self.keys.get(&key_no).map(|key| key.version)
    }

    /// Current KeySettings byte.
    pub fn key_settings(&self) -> u8 {
        self.key_settings
    }

    /// Current key count reported by GetKeySettings.
    pub fn max_keys(&self) -> u8 {
        self.max_keys
    }

    pub fn with_uid(mut self, uid: [u8; 7]) -> Self {
        self.uid = uid;
        self
    }

    /// Flip the last response-MAC byte, for fail-closed tests.
    pub fn with_corrupt_mac(mut self) -> Self {
        self.corrupt_mac = true;
        self
    }

    /// Flip the last GetCardUID ciphertext byte, for fail-closed tests.
    pub fn with_corrupt_uid(mut self) -> Self {
        self.corrupt_uid = true;
        self
    }

    pub fn commands(&self) -> &[Vec<u8>] {
        &self.commands
    }

    pub fn key_type(&self) -> AuthKeyType {
        self.key_type
    }

    /// Answer one complete native DESFire command APDU.
    pub fn handle(&mut self, apdu: &[u8]) -> ApduResponse {
        self.commands.push(apdu.to_vec());
        let Some(ins) = apdu.get(1).copied() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if apdu.len() >= 4
            && apdu[0] == 0x00
            && let Some(response) = self.handle_iso(apdu)
        {
            return response;
        }
        let body = command_body(apdu);
        match ins {
            INS_SELECT_APPLICATION => self.select_application(body),
            INS_GET_VERSION => self.reply_version(ins),
            INS_GET_APPLICATION_IDS => self.reply_application_ids(ins),
            INS_GET_FILE_IDS => self.reply_file_ids(ins),
            INS_GET_FREE_MEMORY => self.reply_free_memory(ins),
            INS_FORMAT_PICC => self.format_picc(),
            INS_CREATE_APPLICATION => self.create_application(body),
            INS_DELETE_APPLICATION => self.delete_application(body),
            INS_CREATE_STD_DATA_FILE
            | INS_CREATE_BACKUP_DATA_FILE
            | INS_CREATE_VALUE_FILE
            | INS_CREATE_LINEAR_RECORD_FILE
            | INS_CREATE_CYCLIC_RECORD_FILE => self.create_file(ins, body),
            INS_CREATE_TRANSACTION_MAC_FILE => self.create_transaction_mac_file(ins, body),
            INS_DELETE_FILE => self.delete_file(body),
            INS_CHANGE_FILE_SETTINGS => self.change_file_settings(ins, body),
            INS_GET_FILE_SETTINGS => self.reply_file_settings(ins, body),
            INS_GET_VALUE => self.reply_get_value(ins, body),
            INS_CREDIT | INS_DEBIT | INS_LIMITED_CREDIT => self.apply_value_operation(ins, body),
            INS_READ_DATA => self.reply_read_data(ins, body),
            INS_READ_RECORDS => self.reply_read_records(ins, body),
            INS_WRITE_DATA => self.apply_write_data(ins, body),
            INS_WRITE_RECORD => self.apply_write_record(ins, body),
            INS_UPDATE_RECORD => self.apply_update_record(ins, body),
            INS_CLEAR_RECORD_FILE => self.apply_clear_record_file(ins, body),
            INS_COMMIT_TRANSACTION => self.commit_transaction(ins, body),
            INS_ABORT_TRANSACTION => self.abort_transaction(ins),
            INS_GET_KEY_VERSION => self.reply_key_version(ins, body),
            INS_GET_KEY_SETTINGS => self.reply_key_settings(ins),
            INS_CHANGE_KEY_SETTINGS => self.change_key_settings(ins, body),
            INS_CHANGE_KEY => self.change_key(ins, body),
            INS_GET_CARD_UID => self.reply_card_uid(ins),
            INS_ADDITIONAL_FRAME => self.finish_authentication(body),
            command if command == self.key_type.command() => self.begin_authentication(body),
            _ => ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND),
        }
    }

    /// Answer the ISO 7816-4 commands the Type 4 adapter sends for the NDEF
    /// Tag Application, or `None` for a native DESFire command.
    fn handle_iso(&mut self, apdu: &[u8]) -> Option<ApduResponse> {
        match apdu[1] {
            0xA4 => Some(self.iso_select(apdu)),
            0xB0 => Some(self.iso_read_binary(apdu)),
            0xD6 => Some(self.iso_update_binary(apdu)),
            _ => None,
        }
    }

    fn iso_select(&mut self, apdu: &[u8]) -> ApduResponse {
        match (apdu[2], apdu[3]) {
            (0x04, 0x00) => {
                let Some(&lc) = apdu.get(4) else {
                    return ApduResponse::new(Vec::new(), STATUS_PARAMETER_ERROR);
                };
                let length = usize::from(lc);
                if apdu.len() < 5 + length {
                    return ApduResponse::new(Vec::new(), STATUS_LENGTH_ERROR);
                }
                if &apdu[5..5 + length] != NDEF_DF_NAME.as_slice()
                    || !self.applications.contains_key(&NDEF_APPLICATION_AID)
                {
                    return ApduResponse::new(Vec::new(), [0x6A, 0x82]);
                }
                self.selected = NDEF_APPLICATION_AID;
                self.state = AuthState::Idle;
                self.iso_file = None;
                ApduResponse::new(Vec::new(), [0x90, 0x00])
            }
            (0x00, 0x0C) => {
                if apdu.len() < 7 || apdu[4] != 0x02 {
                    return ApduResponse::new(Vec::new(), STATUS_LENGTH_ERROR);
                }
                let file_id = u16::from_be_bytes([apdu[5], apdu[6]]);
                let found = self.selected_files().is_some_and(|files| {
                    files.values().any(|file| file.iso_file_id == Some(file_id))
                });
                if !found {
                    return ApduResponse::new(Vec::new(), [0x6A, 0x82]);
                }
                self.iso_file = Some(file_id);
                ApduResponse::new(Vec::new(), [0x90, 0x00])
            }
            _ => ApduResponse::new(Vec::new(), STATUS_PARAMETER_ERROR),
        }
    }

    fn iso_read_binary(&mut self, apdu: &[u8]) -> ApduResponse {
        let Some(offset) =
            (apdu.len() >= 5).then(|| usize::from(u16::from_be_bytes([apdu[2], apdu[3]])))
        else {
            return ApduResponse::new(Vec::new(), [0x67, 0x00]);
        };
        let length = match apdu[4] {
            0x00 => 256,
            value => usize::from(value),
        };
        let Some(file_id) = self.iso_file_id() else {
            return ApduResponse::new(Vec::new(), [0x6A, 0x82]);
        };
        let Some(file) = self.selected_files().and_then(|files| files.get(&file_id)) else {
            return ApduResponse::new(Vec::new(), [0x6A, 0x82]);
        };
        let end = offset.saturating_add(length);
        if end > file.data.len() {
            return ApduResponse::new(Vec::new(), [0x6B, 0x00]);
        }
        ApduResponse::new(file.data[offset..end].to_vec(), [0x90, 0x00])
    }

    fn iso_update_binary(&mut self, apdu: &[u8]) -> ApduResponse {
        if apdu.len() < 5 {
            return ApduResponse::new(Vec::new(), [0x67, 0x00]);
        }
        let offset = usize::from(u16::from_be_bytes([apdu[2], apdu[3]]));
        let length = usize::from(apdu[4]);
        if apdu.len() != 5 + length {
            return ApduResponse::new(Vec::new(), [0x67, 0x00]);
        }
        let Some(file_id) = self.iso_file_id() else {
            return ApduResponse::new(Vec::new(), [0x6A, 0x82]);
        };
        let selected = self.selected;
        let Some(file) = self
            .applications
            .get_mut(&selected)
            .and_then(|app| app.files.get_mut(&file_id))
        else {
            return ApduResponse::new(Vec::new(), [0x6A, 0x82]);
        };
        let end = offset.saturating_add(length);
        if end > file.data.len() {
            return ApduResponse::new(Vec::new(), [0x6B, 0x00]);
        }
        file.data[offset..end].copy_from_slice(&apdu[5..5 + length]);
        ApduResponse::new(Vec::new(), [0x90, 0x00])
    }

    fn iso_file_id(&self) -> Option<u8> {
        let iso_file = self.iso_file?;
        self.selected_files()?
            .iter()
            .find(|(_, file)| file.iso_file_id == Some(iso_file))
            .map(|(file_id, _)| *file_id)
    }

    fn format_picc(&mut self) -> ApduResponse {
        if !self.authenticated() {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let response = self.secure_reply(INS_FORMAT_PICC, &[], Vec::new());
        self.applications.clear();
        self.selected = PICC;
        self.iso_file = None;
        self.state = AuthState::Idle;
        self.free_memory = self.total_memory;
        self.keys.clear();
        self.keys.insert(
            0,
            MockKey {
                value: vec![0; self.key_type.key_length()],
                version: 0,
            },
        );
        self.key_settings = 0x0F;
        self.max_keys = 1;
        response
    }

    fn select_application(&mut self, body: &[u8]) -> ApduResponse {
        let Ok(aid) = <[u8; 3]>::try_from(body) else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        self.state = AuthState::Idle;
        self.selected = aid;
        // Existing fixtures select the reserved AID before it has any file;
        // treat selection as creating an empty in-memory application so the
        // scripted card stays usable without an explicit create step.
        if aid != PICC {
            self.applications.entry(aid).or_default();
        }
        ApduResponse::new(Vec::new(), STATUS_OK)
    }

    fn reply_version(&mut self, ins: u8) -> ApduResponse {
        // MIFARE DESFire EV3 8K, protocol 0x05.
        self.secure_reply(ins, &[], vec![0x04, 0x01, 0x01, 0x33, 0x00, 0x1A, 0x05])
    }

    fn reply_application_ids(&mut self, ins: u8) -> ApduResponse {
        let body = self
            .applications
            .keys()
            .flat_map(|aid| aid.iter().copied())
            .collect();
        self.secure_reply(ins, &[], body)
    }

    fn reply_file_ids(&mut self, ins: u8) -> ApduResponse {
        let body = self
            .applications
            .get(&self.selected)
            .map(|app| app.files.keys().copied().collect())
            .unwrap_or_default();
        self.secure_reply(ins, &[], body)
    }

    fn reply_free_memory(&mut self, ins: u8) -> ApduResponse {
        let value = self.free_memory;
        let body = vec![value as u8, (value >> 8) as u8, (value >> 16) as u8];
        self.secure_reply(ins, &[], body)
    }

    fn authenticated(&self) -> bool {
        matches!(self.state, AuthState::Session { .. })
    }

    fn create_application(&mut self, body: &[u8]) -> ApduResponse {
        if body.len() < 5 || self.selected != PICC {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        if !self.authenticated() {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let aid = [body[0], body[1], body[2]];
        if aid == PICC || self.applications.contains_key(&aid) {
            return ApduResponse::new(Vec::new(), STATUS_DUPLICATE);
        }
        if self.free_memory < 10 {
            return ApduResponse::new(Vec::new(), STATUS_NO_MEMORY);
        }
        self.free_memory -= 10;
        self.applications.insert(aid, MockApp::default());
        self.secure_reply(INS_CREATE_APPLICATION, body, Vec::new())
    }

    fn delete_application(&mut self, body: &[u8]) -> ApduResponse {
        let Ok(aid) = <[u8; 3]>::try_from(body) else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if self.selected != PICC || !self.authenticated() {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        if aid == PICC || self.applications.remove(&aid).is_none() {
            return ApduResponse::new(Vec::new(), STATUS_APPLICATION_NOT_FOUND);
        }
        self.secure_reply(INS_DELETE_APPLICATION, body, Vec::new())
    }

    fn create_file(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        match self.store_created_file(ins, body) {
            Ok(()) => self.secure_reply(ins, body, Vec::new()),
            Err(status) => ApduResponse::new(Vec::new(), status),
        }
    }

    /// Create a Transaction MAC file from an EV1 enciphered command.
    ///
    /// The file header is plaintext; the 16-byte Transaction MAC key and its
    /// version are CBC-enciphered with a CRC32, as the card requires for this
    /// command. The response MAC chains from the last ciphertext block.
    fn create_transaction_mac_file(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if !self.authenticated() || self.selected == PICC {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let Some(plain) = self.decrypt_ev1_command(ins, body, 5, 17) else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        match self.store_created_file(ins, &plain) {
            Ok(()) => self.secure_reply_enciphered(Vec::new()),
            Err(status) => ApduResponse::new(Vec::new(), status),
        }
    }

    /// Decode an EV1 enciphered command: plaintext header, CBC-enciphered
    /// `data || CRC32(INS || header || data)` with zero padding.
    fn decrypt_ev1_command(
        &mut self,
        ins: u8,
        body: &[u8],
        header_len: usize,
        data_len: usize,
    ) -> Option<Vec<u8>> {
        let AuthState::Session {
            session_key, iv, ..
        } = &mut self.state
        else {
            return None;
        };
        if !matches!(self.key_type, AuthKeyType::Tdes3 | AuthKeyType::Aes)
            || body.len() <= header_len
        {
            return None;
        }
        let (header, cipher) = body.split_at(header_len);
        let block_size = if self.key_type == AuthKeyType::Tdes3 {
            8
        } else {
            16
        };
        let padded = (data_len + 4).div_ceil(block_size) * block_size;
        if cipher.len() != padded {
            return None;
        }
        let plain = match self.key_type {
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(session_key).ok()?;
                cbc_decrypt_iv(&key, iv, cipher).ok()?
            }
            AuthKeyType::Aes => aes_cbc_decrypt(session_key, iv, cipher).ok()?,
            _ => return None,
        };
        let mut crc_input = Vec::with_capacity(1 + header.len() + data_len);
        crc_input.push(ins);
        crc_input.extend_from_slice(header);
        crc_input.extend_from_slice(&plain[..data_len]);
        let crc = crate::crc32_desfire(&crc_input);
        if plain[data_len..data_len + 4] != crc
            || plain[data_len + 4..].iter().any(|byte| *byte != 0)
        {
            return None;
        }
        *iv = cipher[cipher.len() - block_size..].to_vec();
        let mut decoded = header.to_vec();
        decoded.extend_from_slice(&plain[..data_len]);
        Some(decoded)
    }

    fn store_created_file(&mut self, ins: u8, body: &[u8]) -> std::result::Result<(), [u8; 2]> {
        if !self.authenticated() || self.selected == PICC {
            return Err(STATUS_PERMISSION_DENIED);
        }
        let Some((&file_id, rest)) = body.split_first() else {
            return Err(STATUS_ILLEGAL_COMMAND);
        };
        if file_id > 0x1F || rest.len() < 3 {
            return Err(STATUS_ILLEGAL_COMMAND);
        }
        let (settings, required, value, iso_file_id) = match ins {
            INS_CREATE_STD_DATA_FILE | INS_CREATE_BACKUP_DATA_FILE => {
                let (iso_file_id, rest) = split_iso_prefix(rest, 6, 8)?;
                let size = u24(&rest[3..6]);
                let mut settings = vec![if ins == INS_CREATE_BACKUP_DATA_FILE {
                    0x01
                } else {
                    0x00
                }];
                settings.extend_from_slice(rest);
                (settings, size, 0, iso_file_id)
            }
            INS_CREATE_VALUE_FILE => {
                let (iso_file_id, rest) = split_iso_prefix(rest, 16, 18)?;
                let initial = i32::from_le_bytes(rest[11..15].try_into().expect("value bytes"));
                // GetFileSettings reports the LimitedCredit amount, not the
                // current value; the card leaves it at zero after creation.
                let mut settings = vec![0x02];
                settings.extend_from_slice(&rest[..11]);
                settings.extend_from_slice(&[0, 0, 0, 0]);
                settings.push(rest[15]);
                (settings, 13, initial, iso_file_id)
            }
            INS_CREATE_LINEAR_RECORD_FILE | INS_CREATE_CYCLIC_RECORD_FILE => {
                let (iso_file_id, rest) = split_iso_prefix(rest, 9, 11)?;
                let record_size = u24(&rest[3..6]);
                let max_records = u24(&rest[6..9]);
                let mut settings = vec![if ins == INS_CREATE_CYCLIC_RECORD_FILE {
                    0x04
                } else {
                    0x03
                }];
                settings.extend_from_slice(rest);
                settings.extend_from_slice(&[0, 0, 0]);
                (
                    settings,
                    record_size.saturating_mul(max_records),
                    0,
                    iso_file_id,
                )
            }
            INS_CREATE_TRANSACTION_MAC_FILE => {
                if rest.len() != 21 || rest[3] != 0x02 {
                    return Err(STATUS_ILLEGAL_COMMAND);
                }
                let mut settings = vec![0x05];
                settings.extend_from_slice(&rest[..3]);
                settings.push(rest[3]);
                settings.push(rest[20]);
                (settings, 18, 0, None)
            }
            _ => unreachable!("the dispatch selects a create command"),
        };
        let selected = self.selected;
        let Some(app) = self.applications.get_mut(&selected) else {
            return Err(STATUS_APPLICATION_NOT_FOUND);
        };
        if app.files.contains_key(&file_id) {
            return Err(STATUS_FILE_EXISTS);
        }
        if required > self.free_memory {
            return Err(STATUS_NO_MEMORY);
        }
        self.free_memory -= required;
        let data = if matches!(ins, INS_CREATE_STD_DATA_FILE | INS_CREATE_BACKUP_DATA_FILE) {
            vec![0u8; required as usize]
        } else {
            Vec::new()
        };
        app.files.insert(
            file_id,
            MockFile {
                settings,
                data,
                iso_file_id,
                value,
                pending_value: None,
                pending_data: None,
                pending_settings: None,
                record_open: false,
            },
        );
        Ok(())
    }

    fn secure_reply_enciphered(&mut self, body: Vec<u8>) -> ApduResponse {
        let AuthState::Session {
            session_key, iv, ..
        } = &mut self.state
        else {
            return ApduResponse::new(body, STATUS_OK);
        };
        if !matches!(self.key_type, AuthKeyType::Tdes3 | AuthKeyType::Aes) {
            return ApduResponse::new(body, STATUS_OK);
        }
        let mut input = body.clone();
        input.push(0x00);
        let mac = ev1_cmac(self.key_type, session_key, iv, &input).expect("response MAC");
        *iv = mac.clone();
        let mut raw = body;
        raw.extend_from_slice(&mac[..8]);
        ApduResponse::new(raw, STATUS_OK)
    }

    fn delete_file(&mut self, body: &[u8]) -> ApduResponse {
        if !self.authenticated() {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let Some(file_id) = body.first().copied() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let Some(app) = self.applications.get_mut(&self.selected) else {
            return ApduResponse::new(Vec::new(), STATUS_APPLICATION_NOT_FOUND);
        };
        if app.files.remove(&file_id).is_none() {
            return ApduResponse::new(Vec::new(), STATUS_FILE_NOT_FOUND);
        }
        self.secure_reply(INS_DELETE_FILE, body, Vec::new())
    }

    fn change_file_settings(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if self.authenticated() {
            let Some(plain) = self.decrypt_ev1_command(ins, body, 1, 3) else {
                return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
            };
            return match self.store_changed_settings(&plain) {
                Ok(()) => self.secure_reply_enciphered(Vec::new()),
                Err(status) => ApduResponse::new(Vec::new(), status),
            };
        }
        match self.store_changed_settings(body) {
            Ok(()) => ApduResponse::new(Vec::new(), STATUS_OK),
            Err(status) => ApduResponse::new(Vec::new(), status),
        }
    }

    fn store_changed_settings(&mut self, body: &[u8]) -> std::result::Result<(), [u8; 2]> {
        if body.len() != 4 {
            return Err(STATUS_ILLEGAL_COMMAND);
        }
        let file_id = body[0];
        let communication = body[1];
        if !matches!(communication, 0 | 1 | 3) {
            return Err(STATUS_PARAMETER_ERROR);
        }
        let selected = self.selected;
        let Some(app) = self.applications.get_mut(&selected) else {
            return Err(STATUS_APPLICATION_NOT_FOUND);
        };
        let Some(file) = app.files.get_mut(&file_id) else {
            return Err(STATUS_FILE_NOT_FOUND);
        };
        if !matches!(file.settings.first(), Some(0x00 | 0x01)) && file.settings[1] != communication
        {
            return Err(STATUS_PARAMETER_ERROR);
        }
        file.settings[1] = communication;
        file.settings[2] = body[2];
        file.settings[3] = body[3];
        Ok(())
    }

    fn begin_authentication(&mut self, body: &[u8]) -> ApduResponse {
        if matches!(self.key_type, AuthKeyType::AesEv2) {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let Some((&key_no, rest)) = body.split_first() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if !rest.is_empty() {
            return ApduResponse::new(Vec::new(), STATUS_AUTHENTICATION_ERROR);
        }
        let Some(key) = self.keys.get(&key_no) else {
            return ApduResponse::new(Vec::new(), STATUS_NO_SUCH_KEY);
        };
        let key_value = key.value.clone();
        let length = match self.key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => 8,
            AuthKeyType::Tdes3 | AuthKeyType::Aes => 16,
            AuthKeyType::AesEv2 => 16,
        };
        let rnd_b = (0..length)
            .map(|index| 0xB0 ^ index as u8)
            .collect::<Vec<_>>();
        let challenge = match self.key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                let key = expand_tdes_key(&key_value).expect("valid legacy key");
                cbc_encrypt_iv(&key, &[0u8; 8], &rnd_b).expect("challenge encryption")
            }
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(&key_value).expect("valid 3K3DES key");
                cbc_encrypt_iv(&key, &[0u8; 8], &rnd_b).expect("challenge encryption")
            }
            AuthKeyType::Aes => {
                aes_cbc_encrypt(&key_value, &[0u8; 16], &rnd_b).expect("challenge encryption")
            }
            AuthKeyType::AesEv2 => unreachable!("EV2 is refused above"),
        };
        self.state = AuthState::Challenge {
            key_no,
            key: key_value,
            rnd_b: rnd_b.clone(),
            challenge: challenge.clone(),
        };
        ApduResponse::new(challenge, STATUS_ADDITIONAL)
    }

    fn finish_authentication(&mut self, body: &[u8]) -> ApduResponse {
        let AuthState::Challenge {
            key_no,
            key,
            rnd_b,
            challenge,
        } = std::mem::replace(&mut self.state, AuthState::Idle)
        else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let length = rnd_b.len();
        if body.len() != length * 2 {
            return ApduResponse::new(Vec::new(), STATUS_AUTHENTICATION_ERROR);
        }
        let plain = match self.key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                let key = expand_tdes_key(&key).expect("valid legacy key");
                d40_receive(&key, body).expect("legacy receive")
            }
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(&key).expect("valid 3K3DES key");
                cbc_decrypt_iv(
                    &key,
                    last_block(&challenge, 8).expect("challenge block"),
                    body,
                )
                .expect("3K3DES receive")
            }
            AuthKeyType::Aes => aes_cbc_decrypt(&key, &challenge, body).expect("AES receive"),
            AuthKeyType::AesEv2 => unreachable!("EV2 is refused before this step"),
        };
        if plain[length..] != rotate_left(&rnd_b) {
            return ApduResponse::new(Vec::new(), STATUS_AUTHENTICATION_ERROR);
        }
        let rnd_a = &plain[..length];
        let rotated = rotate_left(rnd_a);
        let response = match self.key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                let key = expand_tdes_key(&key).expect("valid legacy key");
                cbc_encrypt_iv(&key, &[0u8; 8], &rotated).expect("response encryption")
            }
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(&key).expect("valid 3K3DES key");
                cbc_encrypt_iv(&key, last_block(body, 8).expect("request block"), &rotated)
                    .expect("response encryption")
            }
            AuthKeyType::Aes => {
                aes_cbc_encrypt(&key, last_block(body, 16).expect("request block"), &rotated)
                    .expect("response encryption")
            }
            AuthKeyType::AesEv2 => unreachable!("EV2 is refused before this step"),
        };
        let session_key = derive_session_key(self.key_type, rnd_a, &rnd_b);
        let iv = match self.key_type {
            AuthKeyType::Tdes3 => vec![0u8; 8],
            AuthKeyType::Aes => vec![0u8; 16],
            _ => Vec::new(),
        };
        self.state = AuthState::Session {
            key_no,
            session_key,
            iv,
        };
        ApduResponse::new(response, STATUS_OK)
    }

    fn reply_key_version(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if !matches!(self.state, AuthState::Session { .. }) {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let Some(&key_no) = body.first() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let Some(version) = self.keys.get(&key_no).map(|key| key.version) else {
            return ApduResponse::new(Vec::new(), STATUS_NO_SUCH_KEY);
        };
        self.secure_reply(ins, body, vec![version])
    }

    fn reply_key_settings(&mut self, ins: u8) -> ApduResponse {
        let family = match self.key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => 0,
            AuthKeyType::Tdes3 => 1,
            AuthKeyType::Aes | AuthKeyType::AesEv2 => 2,
        };
        let second = (family << 6) | (self.max_keys & 0x0F);
        self.secure_reply(ins, &[], vec![self.key_settings, second])
    }

    fn change_key_settings(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if !matches!(self.state, AuthState::Session { .. }) {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        if self.key_settings & 0x08 == 0 {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        match self.key_type {
            AuthKeyType::Tdes3 | AuthKeyType::Aes => {
                let Some(plain) = self.decrypt_ev1_command(ins, body, 0, 1) else {
                    return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                };
                self.key_settings = plain[0];
                self.secure_reply_enciphered(Vec::new())
            }
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                if body.len() != 1 {
                    return ApduResponse::new(Vec::new(), STATUS_LENGTH_ERROR);
                }
                self.key_settings = body[0];
                ApduResponse::new(Vec::new(), STATUS_OK)
            }
            AuthKeyType::AesEv2 => ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND),
        }
    }

    fn change_key(&mut self, _ins: u8, body: &[u8]) -> ApduResponse {
        let Some((&command_key_no, cipher)) = body.split_first() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let target = command_key_no & 0x0F;
        let AuthState::Session {
            key_no: auth_key_no,
            session_key,
            iv,
        } = &mut self.state
        else {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        };
        let same_key = target == *auth_key_no;
        let key_type = self.key_type;
        match key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                if cipher.len() != 24 {
                    return ApduResponse::new(Vec::new(), STATUS_LENGTH_ERROR);
                }
                let key = expand_tdes_key(session_key).expect("valid session key");
                let Ok(plain) = d40_receive(&key, cipher) else {
                    return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
                };
                let key_area = &plain[..16];
                if crc16_a(key_area) != plain[16..18] {
                    return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                }
                let (new_area, tail) = if same_key {
                    (key_area.to_vec(), &plain[18..])
                } else {
                    let Some(old) = self.keys.get(&target) else {
                        return ApduResponse::new(Vec::new(), STATUS_NO_SUCH_KEY);
                    };
                    let mut new_area = key_area.to_vec();
                    for (index, byte) in new_area.iter_mut().enumerate() {
                        *byte ^= old.value[index % old.value.len()];
                    }
                    let Some(crc) = plain.get(18..20) else {
                        return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                    };
                    if crc16_a(&new_area) != crc {
                        return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                    }
                    (new_area, &plain[20..])
                };
                if tail.iter().any(|byte| *byte != 0) {
                    return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                }
                let version = des_key_version(&new_area);
                self.keys.insert(
                    target,
                    MockKey {
                        value: new_area,
                        version,
                    },
                );
                let ended = same_key;
                if ended {
                    self.state = AuthState::Idle;
                }
                ApduResponse::new(Vec::new(), STATUS_OK)
            }
            AuthKeyType::Tdes3 | AuthKeyType::Aes => {
                let block = if key_type == AuthKeyType::Tdes3 {
                    8
                } else {
                    16
                };
                if cipher.len() < block * 2 || !cipher.len().is_multiple_of(block) {
                    return ApduResponse::new(Vec::new(), STATUS_LENGTH_ERROR);
                }
                let Ok(plain) = (if key_type == AuthKeyType::Tdes3 {
                    cbc_decrypt_iv(
                        &expand_tdes_key(session_key).expect("session key"),
                        iv,
                        cipher,
                    )
                } else {
                    aes_cbc_decrypt(session_key, iv, cipher)
                }) else {
                    return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
                };
                *iv = cipher[cipher.len() - block..].to_vec();
                let key_area_len = if key_type == AuthKeyType::Tdes3 {
                    24
                } else {
                    17
                };
                if plain.len() < key_area_len + 4 {
                    return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                }
                let mut crc_input = vec![INS_CHANGE_KEY, command_key_no];
                crc_input.extend_from_slice(&plain[..key_area_len]);
                let crc = crc32_desfire(&crc_input);
                if plain[key_area_len..key_area_len + 4] != crc {
                    return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                }
                let mut pos = key_area_len + 4;
                let (new_area, tail) = if same_key {
                    (plain[..key_area_len].to_vec(), &plain[pos..])
                } else {
                    let Some(old) = self.keys.get(&target) else {
                        return ApduResponse::new(Vec::new(), STATUS_NO_SUCH_KEY);
                    };
                    let mut new_area = plain[..key_area_len].to_vec();
                    // AES carries the version in the byte after the key and
                    // does not XOR it.
                    let xor_len = if key_type == AuthKeyType::Tdes3 {
                        24
                    } else {
                        16
                    };
                    for (index, byte) in new_area.iter_mut().enumerate().take(xor_len) {
                        *byte ^= old.value[index % old.value.len()];
                    }
                    let Some(crc) = plain.get(pos..pos + 4) else {
                        return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                    };
                    let crc_key = if key_type == AuthKeyType::Tdes3 {
                        new_area.as_slice()
                    } else {
                        &new_area[..16]
                    };
                    if crc32_desfire(crc_key) != crc {
                        return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                    }
                    pos += 4;
                    (new_area, &plain[pos..])
                };
                if tail.iter().any(|byte| *byte != 0) {
                    return ApduResponse::new(Vec::new(), STATUS_INTEGRITY_ERROR);
                }
                let (value, version) = if key_type == AuthKeyType::Tdes3 {
                    let version = des_key_version(&new_area);
                    (new_area, version)
                } else {
                    let version = new_area[16];
                    (new_area[..16].to_vec(), version)
                };
                self.keys.insert(target, MockKey { value, version });
                if same_key {
                    // ChangeKey of the authenticated key ends the session and
                    // the card returns the status without a response MAC.
                    self.state = AuthState::Idle;
                    return ApduResponse::new(Vec::new(), STATUS_OK);
                }
                let mac = ev1_cmac(key_type, session_key, iv, &[0x00]).expect("response MAC");
                *iv = mac.clone();
                ApduResponse::new(mac[..8].to_vec(), STATUS_OK)
            }
            AuthKeyType::AesEv2 => ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND),
        }
    }

    fn reply_card_uid(&mut self, ins: u8) -> ApduResponse {
        if !matches!(self.state, AuthState::Session { .. }) {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let uid = self.uid.to_vec();
        let AuthState::Session {
            session_key, iv, ..
        } = &mut self.state
        else {
            return ApduResponse::new(uid, STATUS_OK);
        };
        if !matches!(self.key_type, AuthKeyType::Tdes3 | AuthKeyType::Aes) {
            return ApduResponse::new(uid, STATUS_OK);
        }
        let command_mac = ev1_cmac(self.key_type, session_key, iv, &[ins]).expect("command MAC");
        *iv = command_mac;
        let block_size = if self.key_type == AuthKeyType::Tdes3 {
            8
        } else {
            16
        };
        let mut crc_input = uid.clone();
        crc_input.push(0x00);
        let crc = crate::crc32_desfire(&crc_input);
        let mut plain = uid;
        plain.extend_from_slice(&crc);
        while !plain.len().is_multiple_of(block_size) {
            plain.push(0);
        }
        let cipher = match self.key_type {
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(session_key).expect("valid 3K3DES key");
                cbc_encrypt_iv(&key, iv, &plain).expect("UID encryption")
            }
            AuthKeyType::Aes => aes_cbc_encrypt(session_key, iv, &plain).expect("UID encryption"),
            _ => unreachable!("legacy sessions are returned above"),
        };
        *iv = cipher[cipher.len() - block_size..].to_vec();
        let mut raw = cipher;
        if self.corrupt_uid {
            let last = raw.len() - 1;
            raw[last] ^= 0x01;
        }
        ApduResponse::new(raw, STATUS_OK)
    }

    fn reply_file_settings(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        let Some(file_id) = body.first().copied() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let Some(file) = self.selected_files().and_then(|files| files.get(&file_id)) else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let settings = file.settings.clone();
        self.secure_reply(ins, body, settings)
    }

    fn reply_get_value(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        let Some(file_id) = body.first().copied() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let Some(file) = self.selected_files().and_then(|files| files.get(&file_id)) else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if file.settings.first() != Some(&0x02) {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let value = file.value.to_le_bytes().to_vec();
        self.secure_reply(ins, body, value)
    }

    fn selected_files(&self) -> Option<&BTreeMap<u8, MockFile>> {
        self.applications.get(&self.selected).map(|app| &app.files)
    }

    fn apply_value_operation(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if body.len() != 5 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let file_id = body[0];
        let amount = i64::from(u32::from_le_bytes(
            body[1..5].try_into().expect("four-byte amount"),
        ));
        let selected = self.selected;
        let Some(file) = self
            .applications
            .get_mut(&selected)
            .and_then(|app| app.files.get_mut(&file_id))
        else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if file.settings.first() != Some(&0x02) || file.settings.len() < 17 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let lower = i32::from_le_bytes(file.settings[4..8].try_into().expect("lower limit"));
        let upper = i32::from_le_bytes(file.settings[8..12].try_into().expect("upper limit"));
        let flags = file.settings[16];
        if ins == INS_LIMITED_CREDIT && flags & 0x01 == 0 {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let current = file.pending_value.unwrap_or(file.value);
        let result = match ins {
            INS_CREDIT | INS_LIMITED_CREDIT => i64::from(current) + amount,
            INS_DEBIT => i64::from(current) - amount,
            _ => return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND),
        };
        if result < i64::from(lower) || result > i64::from(upper) {
            return ApduResponse::new(Vec::new(), STATUS_BOUNDARY_ERROR);
        }
        file.pending_value = Some(result as i32);
        self.secure_reply(ins, body, Vec::new())
    }

    fn reply_read_records(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if body.len() != 7 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let Some(file) = self.selected_files().and_then(|files| files.get(&body[0])) else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if !matches!(file.settings.first(), Some(0x03 | 0x04)) || file.settings.len() < 13 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let record_size = u24(&file.settings[4..7]) as usize;
        let current = u24(&file.settings[10..13]);
        let record_number = u24(&body[1..4]);
        let count = u24(&body[4..7]);
        if count == 0 || record_number >= current || count > current - record_number {
            return ApduResponse::new(Vec::new(), STATUS_BOUNDARY_ERROR);
        }
        let first = (current - record_number - count) as usize;
        let data = file.data[first * record_size..(first + count as usize) * record_size].to_vec();
        self.secure_reply(ins, body, data)
    }

    fn apply_write_record(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if body.len() < 7 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let file_id = body[0];
        let offset = u24(&body[1..4]) as usize;
        let length = u24(&body[4..7]) as usize;
        if body.len() != 7 + length {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let selected = self.selected;
        let Some(file) = self
            .applications
            .get_mut(&selected)
            .and_then(|app| app.files.get_mut(&file_id))
        else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if !matches!(file.settings.first(), Some(0x03 | 0x04)) || file.settings.len() < 13 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let record_size = u24(&file.settings[4..7]) as usize;
        let max_records = u24(&file.settings[7..10]);
        let mut settings = file
            .pending_settings
            .take()
            .unwrap_or_else(|| file.settings.clone());
        let mut current = u24(&settings[10..13]);
        let mut data = file
            .pending_data
            .take()
            .unwrap_or_else(|| file.data.clone());
        if offset + length > record_size {
            return ApduResponse::new(Vec::new(), STATUS_BOUNDARY_ERROR);
        }
        if !file.record_open {
            if current == max_records {
                if settings[0] == 0x03 {
                    return ApduResponse::new(Vec::new(), STATUS_BOUNDARY_ERROR);
                }
                data.drain(..record_size);
                current -= 1;
            }
            data.resize(data.len() + record_size, 0);
            current += 1;
        }
        let base = (current as usize - 1) * record_size + offset;
        data[base..base + length].copy_from_slice(&body[7..]);
        settings[10..13].copy_from_slice(&current.to_le_bytes()[..3]);
        file.pending_data = Some(data);
        file.pending_settings = Some(settings);
        file.record_open = true;
        self.secure_reply(ins, body, Vec::new())
    }

    fn apply_update_record(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if body.len() < 10 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let file_id = body[0];
        let record_number = u24(&body[1..4]);
        let offset = u24(&body[4..7]) as usize;
        let length = u24(&body[7..10]) as usize;
        if body.len() != 10 + length {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let selected = self.selected;
        let Some(file) = self
            .applications
            .get_mut(&selected)
            .and_then(|app| app.files.get_mut(&file_id))
        else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if !matches!(file.settings.first(), Some(0x03 | 0x04)) || file.settings.len() < 13 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let record_size = u24(&file.settings[4..7]) as usize;
        let current = u24(&file.pending_settings.as_deref().unwrap_or(&file.settings)[10..13]);
        if record_number >= current || offset + length > record_size {
            return ApduResponse::new(Vec::new(), STATUS_BOUNDARY_ERROR);
        }
        let mut data = file
            .pending_data
            .take()
            .unwrap_or_else(|| file.data.clone());
        let index = current - 1 - record_number;
        let base = index as usize * record_size + offset;
        data[base..base + length].copy_from_slice(&body[10..]);
        file.pending_data = Some(data);
        self.secure_reply(ins, body, Vec::new())
    }

    fn apply_clear_record_file(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        let Some(file_id) = body.first().copied() else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let selected = self.selected;
        let Some(file) = self
            .applications
            .get_mut(&selected)
            .and_then(|app| app.files.get_mut(&file_id))
        else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        if !matches!(file.settings.first(), Some(0x03 | 0x04)) || file.settings.len() < 13 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let mut settings = file
            .pending_settings
            .take()
            .unwrap_or_else(|| file.settings.clone());
        settings[10..13].copy_from_slice(&[0, 0, 0]);
        file.pending_data = Some(Vec::new());
        file.pending_settings = Some(settings);
        file.record_open = false;
        self.secure_reply(ins, body, Vec::new())
    }

    fn commit_transaction(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if self.fail_commit {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        if body.len() > 1 {
            return ApduResponse::new(Vec::new(), STATUS_PARAMETER_ERROR);
        }
        let option = body.first().copied().unwrap_or(0);
        if option & !0x01 != 0 {
            return ApduResponse::new(Vec::new(), STATUS_PARAMETER_ERROR);
        }
        let selected = self.selected;
        let tmac_id = self.applications.get(&selected).and_then(|app| {
            app.files
                .iter()
                .find(|(_, file)| file.settings.first() == Some(&0x05))
                .map(|(file_id, _)| *file_id)
        });
        if option & 0x01 != 0 && tmac_id.is_none() {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        if let Some(app) = self.applications.get_mut(&selected) {
            for file in app.files.values_mut() {
                if let Some(value) = file.pending_value.take() {
                    file.value = value;
                }
                if let Some(data) = file.pending_data.take() {
                    file.data = data;
                }
                if let Some(settings) = file.pending_settings.take() {
                    file.settings = settings;
                }
                file.record_open = false;
            }
        }
        let mut response = Vec::new();
        if option & 0x01 != 0 {
            let (counter, value) = self.commit_mac.unwrap_or((1, [0u8; 8]));
            response.extend_from_slice(&counter.to_le_bytes());
            response.extend_from_slice(&value);
            if let Some(app) = self.applications.get_mut(&selected)
                && let Some(file) = app.files.get_mut(&tmac_id.expect("TMAC file checked"))
            {
                file.data = response.clone();
            }
        }
        self.secure_reply(ins, body, response)
    }

    fn abort_transaction(&mut self, ins: u8) -> ApduResponse {
        let selected = self.selected;
        if let Some(app) = self.applications.get_mut(&selected) {
            for file in app.files.values_mut() {
                file.pending_value = None;
                file.pending_data = None;
                file.pending_settings = None;
                file.record_open = false;
            }
        }
        self.secure_reply(ins, &[], Vec::new())
    }

    fn reply_read_data(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if body.len() != 7 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let Some(file) = self.selected_files().and_then(|files| files.get(&body[0])) else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let offset = u24(&body[1..4]);
        let length = u24(&body[4..7]);
        let start = offset as usize;
        let end = start.saturating_add(length as usize);
        if end > file.data.len() {
            return ApduResponse::new(Vec::new(), STATUS_BOUNDARY_ERROR);
        }
        let mut data = file.data[start..end].to_vec();
        if self.corrupt_readback && self.wrote {
            if let Some(first) = data.first_mut() {
                *first ^= 0x01;
            }
            self.corrupt_readback = false;
        }
        self.secure_reply(ins, body, data)
    }

    fn apply_write_data(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if body.len() < 7 {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let length = u24(&body[4..7]) as usize;
        if body.len() != 7 + length {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        if matches!(
            self.key_type,
            AuthKeyType::Tdes3 | AuthKeyType::Aes | AuthKeyType::AesEv2
        ) && matches!(self.state, AuthState::Session { .. })
        {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        let file_id = body[0];
        let offset = u24(&body[1..4]) as usize;
        let selected = self.selected;
        let Some(file) = self
            .applications
            .get_mut(&selected)
            .and_then(|app| app.files.get_mut(&file_id))
        else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let end = offset.saturating_add(length);
        if end > file.data.len() {
            return ApduResponse::new(Vec::new(), STATUS_BOUNDARY_ERROR);
        }
        file.data[offset..end].copy_from_slice(&body[7..]);
        self.wrote = true;
        self.secure_reply(ins, body, Vec::new())
    }

    fn secure_reply(&mut self, ins: u8, request: &[u8], body: Vec<u8>) -> ApduResponse {
        let AuthState::Session {
            session_key, iv, ..
        } = &mut self.state
        else {
            return ApduResponse::new(body, STATUS_OK);
        };
        if !matches!(self.key_type, AuthKeyType::Tdes3 | AuthKeyType::Aes) {
            return ApduResponse::new(body, STATUS_OK);
        }
        let mut command_input = Vec::with_capacity(1 + request.len());
        command_input.push(ins);
        command_input.extend_from_slice(request);
        let command_mac =
            ev1_cmac(self.key_type, session_key, iv, &command_input).expect("command MAC");
        *iv = command_mac.clone();
        let mut input = body.clone();
        input.push(0x00);
        let mac = ev1_cmac(self.key_type, session_key, iv, &input).expect("response MAC");
        *iv = mac.clone();
        let mut raw = body;
        raw.extend_from_slice(&mac[..8]);
        if self.corrupt_mac {
            let last = raw.len() - 1;
            raw[last] ^= 0x01;
        }
        ApduResponse::new(raw, STATUS_OK)
    }
}

/// D40 receive direction: `E_K(ciphertext) XOR previous ciphertext`.
fn d40_receive(key: &[u8; 24], data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(8) {
        return Err(NfcError::InvalidArgument(
            "D40 receive data must be a non-empty multiple of 8 bytes".to_owned(),
        ));
    }
    let cipher = TdesEde3::new_from_slice(key)
        .map_err(|_| NfcError::InvalidArgument("invalid 3DES key".to_owned()))?;
    let mut previous = [0u8; 8];
    let mut output = Vec::with_capacity(data.len());
    for block in data.as_chunks::<8>().0 {
        let mut current = [0u8; 8];
        current.copy_from_slice(block);
        cipher.encrypt_block((&mut current).into());
        for (index, byte) in current.iter().enumerate() {
            output.push(byte ^ previous[index]);
        }
        previous.copy_from_slice(block);
    }
    Ok(output)
}

fn des_key_version(key: &[u8]) -> u8 {
    let mut version = 0u8;
    let mut bit = 0;
    for index in (0..key.len()).rev() {
        version |= (key[index] & 1) << bit;
        bit = (bit + 1) % 8;
    }
    version
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

/// Split a create-file payload into its optional ISO file ID and the rest.
///
/// A payload with an ISO file ID carries the two ID bytes in front of the
/// communication byte; GetFileSettings responses omit them, as on hardware.
fn split_iso_prefix(
    rest: &[u8],
    plain_length: usize,
    iso_length: usize,
) -> std::result::Result<(Option<u16>, &[u8]), [u8; 2]> {
    match rest.len() {
        length if length == plain_length => Ok((None, rest)),
        length if length == iso_length => {
            let iso_file_id = u16::from_le_bytes([rest[0], rest[1]]);
            if iso_file_id == 0 {
                return Err(STATUS_PARAMETER_ERROR);
            }
            Ok((Some(iso_file_id), &rest[2..]))
        }
        _ => Err(STATUS_ILLEGAL_COMMAND),
    }
}

fn command_body(apdu: &[u8]) -> &[u8] {
    if apdu.len() <= 5 {
        return &[];
    }
    &apdu[5..apdu.len() - 1]
}

/// Transport that serves one [`MockDesfire`] card.
pub struct MockDesfireTransport {
    info: DeviceInfo,
    card: CardInfo,
    tag: Arc<Mutex<MockDesfire>>,
}

impl MockDesfireTransport {
    pub fn new() -> Self {
        Self::with_tag(MockDesfire::default())
    }

    pub fn with_tag(tag: MockDesfire) -> Self {
        let card = CardInfo::new(tag.uid.to_vec(), [0x00, 0x44], 0x20, CardType::Iso14443);
        Self {
            info: DeviceInfo::new("Mock DESFire", "mock:desfire"),
            card,
            tag: Arc::new(Mutex::new(tag)),
        }
    }

    pub fn tag(&self) -> MutexGuard<'_, MockDesfire> {
        self.tag.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Shared card handle for tests that move the transport into a service.
    pub fn card(&self) -> Arc<Mutex<MockDesfire>> {
        self.tag.clone()
    }

    pub fn commands(&self) -> Vec<Vec<u8>> {
        self.tag().commands().to_vec()
    }
}

impl Default for MockDesfireTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for MockDesfireTransport {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        Ok(vec![self.info.clone()])
    }

    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>> {
        if conn_string != self.info.conn_string {
            return Err(NfcError::InvalidArgument(format!(
                "the mock DESFire transport has no device {conn_string}"
            )));
        }
        Ok(Box::new(MockDesfireReader {
            info: self.info.clone(),
            card: self.card.clone(),
            tag: self.tag.clone(),
            open: false,
        }))
    }
}

struct MockDesfireReader {
    info: DeviceInfo,
    card: CardInfo,
    tag: Arc<Mutex<MockDesfire>>,
    open: bool,
}

impl Reader for MockDesfireReader {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn capabilities(&self) -> nfc_transport::ReaderCapabilities {
        nfc_transport::ReaderCapabilities::default()
    }

    fn open(&mut self, conn_string: &str) -> Result<()> {
        if conn_string != self.info.conn_string {
            return Err(NfcError::InvalidArgument(format!(
                "the mock DESFire reader has no device {conn_string}"
            )));
        }
        self.open = true;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.open = false;
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        if !self.open {
            return Err(NfcError::NoReader);
        }
        Ok(self.card.clone())
    }

    fn authenticate(&mut self, _block: u8, _key_type: KeyType, _key: Key) -> Result<()> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no MIFARE sector authentication",
        ))
    }

    fn read_block(&mut self, _block: u8) -> Result<[u8; BLOCK_SIZE]> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no MIFARE blocks",
        ))
    }

    fn write_block(&mut self, _block: u8, _data: [u8; BLOCK_SIZE]) -> Result<()> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no MIFARE blocks",
        ))
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no raw byte mode",
        ))
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no raw bit mode",
        ))
    }

    fn set_raw_mode(&mut self, _enabled: bool) -> Result<()> {
        Ok(())
    }

    fn apdu_capabilities(&self) -> ApduCapabilities {
        ApduCapabilities {
            supported: true,
            max_command: 247,
        }
    }

    fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
        Ok(self
            .tag
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .handle(apdu))
    }

    fn read_page_quad(&mut self, _start_page: u8) -> Result<[u8; PAGE_QUAD_SIZE]> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no Type 2 pages",
        ))
    }

    fn write_page(&mut self, _page: u8, _data: [u8; PAGE_SIZE]) -> Result<()> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no Type 2 pages",
        ))
    }

    fn type2_version(&mut self) -> Result<[u8; TYPE2_VERSION_SIZE]> {
        Err(NfcError::Unsupported(
            "the mock DESFire card has no Type 2 version",
        ))
    }
}
