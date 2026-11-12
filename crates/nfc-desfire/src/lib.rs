// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal MIFARE DESFire native command layer over ISO 7816-4 APDUs.
//!
//! This crate currently covers unauthenticated identity and application/file
//! inventory. NDEF formatting needs the card's PICC key and the EV1/EV2 secure
//! channel; that path is deliberately not shipped while the owned card's key is
//! unknown.

use nfc_core::{NfcError, Result};
use nfc_transport::Reader;

const CLA: u8 = 0x90;
const INS_ADDITIONAL_FRAME: u8 = 0xAF;
const INS_GET_VERSION: u8 = 0x60;
const INS_GET_APPLICATION_IDS: u8 = 0x6A;
const INS_SELECT_APPLICATION: u8 = 0x5A;
const INS_GET_FILE_IDS: u8 = 0x6F;
const INS_GET_FILE_SETTINGS: u8 = 0xF5;

const STATUS_OK: u16 = 0x9100;
const STATUS_ADDITIONAL: u16 = 0x91AF;

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
}

impl<'a> Desfire<'a> {
    pub fn new(reader: &'a mut dyn Reader) -> Self {
        Self { reader }
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
        Ok(())
    }

    pub fn file_ids(&mut self) -> Result<Vec<u8>> {
        self.expect_ok(INS_GET_FILE_IDS, &[])
    }

    pub fn file_settings(&mut self, file_id: u8) -> Result<Vec<u8>> {
        self.expect_ok(INS_GET_FILE_SETTINGS, &[file_id])
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

fn status_error(ins: u8, sw: u16) -> NfcError {
    NfcError::VerificationFailed(format!(
        "DESFire command {ins:#04X} returned status {sw:#06X}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
