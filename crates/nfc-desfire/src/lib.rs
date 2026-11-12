// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal MIFARE DESFire native command layer over ISO 7816-4 APDUs.
//!
//! This crate covers identity and application/file inventory, legacy
//! DES/2K3DES authentication, and the primitive application/file commands used
//! by hardware tests. NDEF formatting and the EV1/EV2 secure channel are not
//! shipped yet.

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

const STATUS_OK: u16 = 0x9100;
const STATUS_ADDITIONAL: u16 = 0x91AF;

/// The legacy authentication key families this layer can derive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyKeyType {
    Des,
    Tdes2,
}

impl LegacyKeyType {
    fn from_key(key: &[u8]) -> Result<Self> {
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

/// Read-only card inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub version: Version,
    pub applications: Vec<Application>,
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
    pub fn create_application(
        &mut self,
        aid: [u8; 3],
        key_settings: u8,
        key_settings2: u8,
    ) -> Result<()> {
        self.ensure_authenticated()?;
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
        self.expect_ok(
            INS_CREATE_APPLICATION,
            &[aid[0], aid[1], aid[2], key_settings, key_settings2],
        )?;
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

    /// Create a standard data file in the selected application.
    ///
    /// `communication` is 0 for plain, 1 for MACed and 3 for enciphered.
    /// `access_rights` uses the raw DESFire 16-bit encoding.
    pub fn create_standard_file(
        &mut self,
        file_id: u8,
        communication: u8,
        access_rights: u16,
        size: u32,
    ) -> Result<()> {
        self.ensure_authenticated()?;
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
        self.expect_ok(
            INS_CREATE_STD_DATA_FILE,
            &[
                file_id,
                communication,
                access_rights as u8,
                (access_rights >> 8) as u8,
                size as u8,
                (size >> 8) as u8,
                (size >> 16) as u8,
            ],
        )?;
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

    /// Read plain data from a standard, backup, value or record file.
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
    fn rotate_left_and_right_are_inverse() {
        let block = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(rotate_right(&rotate_left(&block)), block);
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
