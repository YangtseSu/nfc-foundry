// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scripted DESFire card for service and workflow tests.
//!
//! The card answers native DESFire APDUs for legacy DES/2K3DES, 3K3DES, and
//! AES EV1 authentication, GetKeyVersion, and GetCardUID. EV2, file, and
//! write commands are not simulated; they return `IllegalCommand`.
//!
//! This module is compiled for tests and for callers that enable the
//! `mock-card` feature. It is not part of the production command path.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use des::TdesEde3;
use des::cipher::{BlockCipherEncrypt, KeyInit};
use nfc_core::{
    BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result,
    TYPE2_VERSION_SIZE,
};
use nfc_transport::{ApduCapabilities, ApduResponse, DeviceInfo, Reader, Transport};

use crate::{
    AuthKeyType, INS_ADDITIONAL_FRAME, INS_GET_CARD_UID, INS_GET_KEY_VERSION,
    INS_SELECT_APPLICATION, aes_cbc_decrypt, aes_cbc_encrypt, cbc_decrypt_iv, cbc_encrypt_iv,
    derive_session_key, ev1_cmac, expand_tdes_key, last_block, rotate_left,
};

const STATUS_OK: [u8; 2] = [0x91, 0x00];
const STATUS_ADDITIONAL: [u8; 2] = [0x91, 0xAF];
const STATUS_AUTHENTICATION_ERROR: [u8; 2] = [0x91, 0xAE];
const STATUS_ILLEGAL_COMMAND: [u8; 2] = [0x91, 0x1C];
const STATUS_PERMISSION_DENIED: [u8; 2] = [0x91, 0x9D];

const DEFAULT_UID: [u8; 7] = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];

enum AuthState {
    Idle,
    Challenge { rnd_b: Vec<u8>, challenge: Vec<u8> },
    Session { session_key: Vec<u8>, iv: Vec<u8> },
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
    state: AuthState,
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
            state: AuthState::Idle,
            commands: Vec::new(),
        }
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
            INS_SELECT_APPLICATION => {
                self.state = AuthState::Idle;
                ApduResponse::new(Vec::new(), STATUS_OK)
            }
            INS_GET_KEY_VERSION => self.reply_key_version(ins, body),
            INS_GET_CARD_UID => self.reply_card_uid(ins),
            INS_ADDITIONAL_FRAME => self.finish_authentication(body),
            command if command == self.key_type.command() => self.begin_authentication(body),
            _ => ApduResponse::new(Vec::new(), STATUS_ILLEGAL_COMMAND),
        }
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
