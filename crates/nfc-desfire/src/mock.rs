// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scripted DESFire card for service and workflow tests.
//!
//! The card answers native DESFire APDUs for legacy DES/2K3DES, 3K3DES, and
//! AES EV1 authentication, GetKeyVersion, GetCardUID, GetFileSettings,
//! ReadData, WriteData, CommitTransaction, and AbortTransaction. EV2 and
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
    INS_COMMIT_TRANSACTION, INS_CREATE_APPLICATION, INS_CREATE_BACKUP_DATA_FILE,
    INS_CREATE_CYCLIC_RECORD_FILE, INS_CREATE_LINEAR_RECORD_FILE, INS_CREATE_STD_DATA_FILE,
    INS_CREATE_TRANSACTION_MAC_FILE, INS_CREATE_VALUE_FILE, INS_DELETE_APPLICATION,
    INS_DELETE_FILE, INS_GET_APPLICATION_IDS, INS_GET_CARD_UID, INS_GET_FILE_IDS,
    INS_GET_FILE_SETTINGS, INS_GET_FREE_MEMORY, INS_GET_KEY_VERSION, INS_GET_VALUE,
    INS_GET_VERSION, INS_READ_DATA, INS_SELECT_APPLICATION, INS_WRITE_DATA, aes_cbc_decrypt,
    aes_cbc_encrypt, cbc_decrypt_iv, cbc_encrypt_iv, derive_session_key, ev1_cmac, expand_tdes_key,
    last_block, rotate_left,
};

const STATUS_OK: [u8; 2] = [0x91, 0x00];
const STATUS_ADDITIONAL: [u8; 2] = [0x91, 0xAF];
const STATUS_NO_MEMORY: [u8; 2] = [0x91, 0x0E];
const STATUS_APPLICATION_NOT_FOUND: [u8; 2] = [0x91, 0xA0];
const STATUS_AUTHENTICATION_ERROR: [u8; 2] = [0x91, 0xAE];
const STATUS_ILLEGAL_COMMAND: [u8; 2] = [0x91, 0x1C];
const STATUS_PARAMETER_ERROR: [u8; 2] = [0x91, 0x9E];
const STATUS_PERMISSION_DENIED: [u8; 2] = [0x91, 0x9D];
const STATUS_BOUNDARY_ERROR: [u8; 2] = [0x91, 0xBE];
const STATUS_FILE_NOT_FOUND: [u8; 2] = [0x91, 0xDE];
const STATUS_FILE_EXISTS: [u8; 2] = [0x91, 0xF0];
const STATUS_DUPLICATE: [u8; 2] = [0x91, 0xEE];

const PICC: [u8; 3] = [0x00; 3];
const DEFAULT_APP: [u8; 3] = [0xF0, 0x0D, 0x01];

const DEFAULT_UID: [u8; 7] = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];

enum AuthState {
    Idle,
    Challenge { rnd_b: Vec<u8>, challenge: Vec<u8> },
    Session { session_key: Vec<u8>, iv: Vec<u8> },
}

struct MockFile {
    settings: Vec<u8>,
    data: Vec<u8>,
    /// Current value for a Value file, returned by GetValue.
    value: i32,
}

#[derive(Default)]
struct MockApp {
    files: BTreeMap<u8, MockFile>,
}

/// A card-side DESFire simulator.
pub struct MockDesfire {
    key: Vec<u8>,
    key_type: AuthKeyType,
    key_no: u8,
    key_version: u8,
    uid: [u8; 7],
    corrupt_mac: bool,
    corrupt_uid: bool,
    fail_commit: bool,
    corrupt_readback: bool,
    wrote: bool,
    state: AuthState,
    applications: BTreeMap<[u8; 3], MockApp>,
    selected: [u8; 3],
    free_memory: u32,
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
        Self {
            key,
            key_type,
            key_no: 0,
            key_version: 0,
            uid: DEFAULT_UID,
            corrupt_mac: false,
            corrupt_uid: false,
            fail_commit: false,
            corrupt_readback: false,
            wrote: false,
            state: AuthState::Idle,
            applications: BTreeMap::new(),
            selected: PICC,
            free_memory: 8192,
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
                    value: 0,
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
                value: 0,
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
        self.key_no = key_no;
        self
    }

    pub fn with_key_version(mut self, key_version: u8) -> Self {
        self.key_version = key_version;
        self
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
        let body = command_body(apdu);
        match ins {
            INS_SELECT_APPLICATION => self.select_application(body),
            INS_GET_VERSION => self.reply_version(ins),
            INS_GET_APPLICATION_IDS => self.reply_application_ids(ins),
            INS_GET_FILE_IDS => self.reply_file_ids(ins),
            INS_GET_FREE_MEMORY => self.reply_free_memory(ins),
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
            INS_READ_DATA => self.reply_read_data(ins, body),
            INS_WRITE_DATA => self.apply_write_data(ins, body),
            INS_COMMIT_TRANSACTION => {
                if self.fail_commit {
                    ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED)
                } else {
                    ApduResponse::new(Vec::new(), STATUS_OK)
                }
            }
            INS_ABORT_TRANSACTION => ApduResponse::new(Vec::new(), STATUS_OK),
            INS_GET_KEY_VERSION => self.reply_key_version(ins, body),
            INS_GET_CARD_UID => self.reply_card_uid(ins),
            INS_ADDITIONAL_FRAME => self.finish_authentication(body),
            command if command == self.key_type.command() => self.begin_authentication(body),
            _ => ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND),
        }
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
        let AuthState::Session { session_key, iv } = &mut self.state else {
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
        let (settings, required, value) = match ins {
            INS_CREATE_STD_DATA_FILE | INS_CREATE_BACKUP_DATA_FILE => {
                if rest.len() != 6 {
                    return Err(STATUS_ILLEGAL_COMMAND);
                }
                let size = u24(&rest[3..6]);
                let mut settings = vec![if ins == INS_CREATE_BACKUP_DATA_FILE {
                    0x01
                } else {
                    0x00
                }];
                settings.extend_from_slice(rest);
                (settings, size, 0)
            }
            INS_CREATE_VALUE_FILE => {
                if rest.len() != 16 {
                    return Err(STATUS_ILLEGAL_COMMAND);
                }
                let initial = i32::from_le_bytes(rest[11..15].try_into().expect("value bytes"));
                // GetFileSettings reports the LimitedCredit amount, not the
                // current value; the card leaves it at zero after creation.
                let mut settings = vec![0x02];
                settings.extend_from_slice(&rest[..11]);
                settings.extend_from_slice(&[0, 0, 0, 0]);
                settings.push(rest[15]);
                (settings, 13, initial)
            }
            INS_CREATE_LINEAR_RECORD_FILE | INS_CREATE_CYCLIC_RECORD_FILE => {
                if rest.len() != 9 {
                    return Err(STATUS_ILLEGAL_COMMAND);
                }
                let record_size = u24(&rest[3..6]);
                let max_records = u24(&rest[6..9]);
                let mut settings = vec![if ins == INS_CREATE_CYCLIC_RECORD_FILE {
                    0x04
                } else {
                    0x03
                }];
                settings.extend_from_slice(rest);
                settings.extend_from_slice(&[0, 0, 0]);
                (settings, record_size.saturating_mul(max_records), 0)
            }
            INS_CREATE_TRANSACTION_MAC_FILE => {
                if rest.len() != 21 || rest[3] != 0x02 {
                    return Err(STATUS_ILLEGAL_COMMAND);
                }
                let mut settings = vec![0x05];
                settings.extend_from_slice(&rest[..3]);
                settings.push(rest[3]);
                settings.push(rest[20]);
                (settings, 18, 0)
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
                value,
            },
        );
        Ok(())
    }

    fn secure_reply_enciphered(&mut self, body: Vec<u8>) -> ApduResponse {
        let AuthState::Session { session_key, iv } = &mut self.state else {
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
        let expected = match self.key_type {
            AuthKeyType::AesEv2 => vec![self.key_no, 0x00],
            _ => vec![self.key_no],
        };
        if body != expected {
            return ApduResponse::new(Vec::new(), STATUS_AUTHENTICATION_ERROR);
        }
        if matches!(self.key_type, AuthKeyType::AesEv2) {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
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
                let key = expand_tdes_key(&self.key).expect("valid legacy key");
                cbc_encrypt_iv(&key, &[0u8; 8], &rnd_b).expect("challenge encryption")
            }
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(&self.key).expect("valid 3K3DES key");
                cbc_encrypt_iv(&key, &[0u8; 8], &rnd_b).expect("challenge encryption")
            }
            AuthKeyType::Aes => {
                aes_cbc_encrypt(&self.key, &[0u8; 16], &rnd_b).expect("challenge encryption")
            }
            AuthKeyType::AesEv2 => unreachable!("EV2 is refused above"),
        };
        self.state = AuthState::Challenge {
            rnd_b: rnd_b.clone(),
            challenge: challenge.clone(),
        };
        ApduResponse::new(challenge, STATUS_ADDITIONAL)
    }

    fn finish_authentication(&mut self, body: &[u8]) -> ApduResponse {
        let AuthState::Challenge { rnd_b, challenge } =
            std::mem::replace(&mut self.state, AuthState::Idle)
        else {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        };
        let length = rnd_b.len();
        if body.len() != length * 2 {
            return ApduResponse::new(Vec::new(), STATUS_AUTHENTICATION_ERROR);
        }
        let plain = match self.key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                let key = expand_tdes_key(&self.key).expect("valid legacy key");
                d40_receive(&key, body).expect("legacy receive")
            }
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(&self.key).expect("valid 3K3DES key");
                cbc_decrypt_iv(
                    &key,
                    last_block(&challenge, 8).expect("challenge block"),
                    body,
                )
                .expect("3K3DES receive")
            }
            AuthKeyType::Aes => aes_cbc_decrypt(&self.key, &challenge, body).expect("AES receive"),
            AuthKeyType::AesEv2 => unreachable!("EV2 is refused before this step"),
        };
        if plain[length..] != rotate_left(&rnd_b) {
            return ApduResponse::new(Vec::new(), STATUS_AUTHENTICATION_ERROR);
        }
        let rnd_a = &plain[..length];
        let rotated = rotate_left(rnd_a);
        let response = match self.key_type {
            AuthKeyType::Des | AuthKeyType::Tdes2 => {
                let key = expand_tdes_key(&self.key).expect("valid legacy key");
                cbc_encrypt_iv(&key, &[0u8; 8], &rotated).expect("response encryption")
            }
            AuthKeyType::Tdes3 => {
                let key = expand_tdes_key(&self.key).expect("valid 3K3DES key");
                cbc_encrypt_iv(&key, last_block(body, 8).expect("request block"), &rotated)
                    .expect("response encryption")
            }
            AuthKeyType::Aes => aes_cbc_encrypt(
                &self.key,
                last_block(body, 16).expect("request block"),
                &rotated,
            )
            .expect("response encryption"),
            AuthKeyType::AesEv2 => unreachable!("EV2 is refused before this step"),
        };
        let session_key = derive_session_key(self.key_type, rnd_a, &rnd_b);
        let iv = match self.key_type {
            AuthKeyType::Tdes3 => vec![0u8; 8],
            AuthKeyType::Aes => vec![0u8; 16],
            _ => Vec::new(),
        };
        self.state = AuthState::Session { session_key, iv };
        ApduResponse::new(response, STATUS_OK)
    }

    fn reply_key_version(&mut self, ins: u8, body: &[u8]) -> ApduResponse {
        if !matches!(self.state, AuthState::Session { .. }) {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        if body != [self.key_no] {
            return ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND);
        }
        self.secure_reply(ins, body, vec![self.key_version])
    }

    fn reply_card_uid(&mut self, ins: u8) -> ApduResponse {
        if !matches!(self.state, AuthState::Session { .. }) {
            return ApduResponse::new(Vec::new(), STATUS_PERMISSION_DENIED);
        }
        let uid = self.uid.to_vec();
        let AuthState::Session { session_key, iv } = &mut self.state else {
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
        let AuthState::Session { session_key, iv } = &mut self.state else {
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

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
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
