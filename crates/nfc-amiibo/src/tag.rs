//! Raw NTAG215 images and the internal plaintext layout.

use std::fmt;

use nfc_core::{NfcError, Result};

use crate::crypto;
use crate::identity::AmiiboIdentity;
use crate::keys::AmiiboKeys;

/// A complete NTAG215 image: 135 pages of four bytes.
pub const TAG_SIZE: usize = 540;
/// A plaintext image: the 520-byte crypto buffer plus the raw 20-byte tail.
pub const PLAIN_SIZE: usize = 540;
/// The Amiibo UID length.
pub const UID_SIZE: usize = 7;

const CRYPTO_SIZE: usize = 520;
const UID_OFFSET: usize = 0x1D4;
const IDENTITY_OFFSET: usize = 0x1DC;
const CC: [u8; 4] = [0xF1, 0x10, 0xFF, 0xEE];
const CANONICAL_TAIL: [u8; 20] = [
    0x01, 0x00, 0x0F, 0xBD, // dynamic lock + RFUI
    0x00, 0x00, 0x00, 0x04, // AUTH0 = 04
    0x5F, 0x00, 0x00, 0x00, // ACCESS = 5F
    0x00, 0x00, 0x00, 0x00, // PWD
    0x00, 0x00, 0x00, 0x00, // PACK
];

/// The application-level UID BCC values.
pub fn bcc0(uid: &[u8; UID_SIZE]) -> u8 {
    0x88 ^ uid[0] ^ uid[1] ^ uid[2]
}

pub fn bcc1(uid: &[u8; UID_SIZE]) -> u8 {
    uid[3] ^ uid[4] ^ uid[5] ^ uid[6]
}

fn uid_from_pages(page0: &[u8], page1: &[u8]) -> [u8; UID_SIZE] {
    [
        page0[0], page0[1], page0[2], page1[0], page1[1], page1[2], page1[3],
    ]
}

/// One 540-byte raw NTAG215 image.
///
/// Construction validates the seven-byte UID and both BCC bytes. It does not
/// verify the Amiibo signatures; [`unpack`] does that with the user's keys.
#[derive(Clone, PartialEq, Eq)]
pub struct AmiiboTag {
    bytes: [u8; TAG_SIZE],
}

impl AmiiboTag {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != TAG_SIZE {
            return Err(NfcError::InvalidArgument(format!(
                "an NTAG215 image is {TAG_SIZE} bytes; got {}",
                bytes.len()
            )));
        }
        let mut tag = [0u8; TAG_SIZE];
        tag.copy_from_slice(bytes);
        let tag = Self { bytes: tag };
        tag.uid()?;
        Ok(tag)
    }

    pub fn as_bytes(&self) -> &[u8; TAG_SIZE] {
        &self.bytes
    }

    pub fn into_bytes(self) -> [u8; TAG_SIZE] {
        self.bytes
    }

    /// Seven-byte UID from pages 0 and 1.
    pub fn uid(&self) -> Result<[u8; UID_SIZE]> {
        let uid = uid_from_pages(&self.bytes[..4], &self.bytes[4..8]);
        if self.bytes[3] != bcc0(&uid) || self.bytes[8] != bcc1(&uid) {
            return Err(NfcError::InvalidArgument(
                "the NTAG215 UID pages have invalid BCC bytes".to_owned(),
            ));
        }
        if uid[0] != 0x04 {
            return Err(NfcError::InvalidArgument(
                "the NTAG215 UID does not use the NXP manufacturer byte 04".to_owned(),
            ));
        }
        Ok(uid)
    }

    pub fn page(&self, page: u8) -> Result<[u8; 4]> {
        let start = usize::from(page)
            .checked_mul(4)
            .filter(|start| *start + 4 <= TAG_SIZE)
            .ok_or_else(|| NfcError::InvalidArgument(format!("page {page} is outside NTAG215")))?;
        let mut data = [0u8; 4];
        data.copy_from_slice(&self.bytes[start..start + 4]);
        Ok(data)
    }

    pub fn set_page(&mut self, page: u8, data: [u8; 4]) -> Result<()> {
        let start = usize::from(page)
            .checked_mul(4)
            .filter(|start| *start + 4 <= TAG_SIZE)
            .ok_or_else(|| NfcError::InvalidArgument(format!("page {page} is outside NTAG215")))?;
        self.bytes[start..start + 4].copy_from_slice(&data);
        Ok(())
    }

    pub fn capability_container(&self) -> [u8; 4] {
        let mut cc = [0u8; 4];
        cc.copy_from_slice(&self.bytes[0x0C..0x10]);
        cc
    }

    /// Raw tail: dynamic lock page and configuration pages.
    pub fn tail(&self) -> [u8; 20] {
        let mut tail = [0u8; 20];
        tail.copy_from_slice(&self.bytes[0x208..0x21C]);
        tail
    }
}

impl fmt::Debug for AmiiboTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmiiboTag")
            .field("uid", &self.uid().map(|uid| format!("{uid:02X?}")))
            .field("bytes", &TAG_SIZE)
            .finish()
    }
}

/// One decrypted image in the internal plaintext layout.
///
/// The first 520 bytes are plaintext; the final 20 bytes are the raw tail
/// carried through so encrypt reproduces the source dump.
#[derive(Clone, PartialEq, Eq)]
pub struct AmiiboPlain {
    bytes: [u8; PLAIN_SIZE],
}

impl AmiiboPlain {
    /// Accept the 520-byte plaintext-only form or the 540-byte carried-tail
    /// form. A missing tail receives the canonical NTAG215 values.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut plain = [0u8; PLAIN_SIZE];
        match bytes.len() {
            CRYPTO_SIZE => {
                plain[..CRYPTO_SIZE].copy_from_slice(bytes);
                plain[0x208..0x21C].copy_from_slice(&CANONICAL_TAIL);
            }
            PLAIN_SIZE => plain.copy_from_slice(bytes),
            length => {
                return Err(NfcError::InvalidArgument(format!(
                    "a plaintext Amiibo image is {CRYPTO_SIZE} or {PLAIN_SIZE} bytes; got {length}"
                )));
            }
        }
        let plain = Self { bytes: plain };
        plain.uid()?;
        Ok(plain)
    }

    pub fn as_bytes(&self) -> &[u8; PLAIN_SIZE] {
        &self.bytes
    }

    pub fn into_bytes(self) -> [u8; PLAIN_SIZE] {
        self.bytes
    }

    pub fn uid(&self) -> Result<[u8; UID_SIZE]> {
        let pages = &self.bytes[UID_OFFSET..UID_OFFSET + 8];
        let uid = uid_from_pages(&pages[..4], &pages[4..8]);
        if pages[3] != bcc0(&uid) || self.bytes[0] != bcc1(&uid) {
            return Err(NfcError::InvalidArgument(
                "the plaintext Amiibo UID pages have invalid BCC bytes".to_owned(),
            ));
        }
        if uid[0] != 0x04 {
            return Err(NfcError::InvalidArgument(
                "the plaintext Amiibo UID does not use the NXP manufacturer byte 04".to_owned(),
            ));
        }
        Ok(uid)
    }

    /// Validate the fields this workflow depends on before signing or writing.
    pub fn validate(&self) -> Result<()> {
        let uid = self.uid()?;
        let cc = &self.bytes[4..8];
        if cc != CC {
            return Err(NfcError::InvalidArgument(format!(
                "the plaintext capability container is {:02X?}; an Amiibo expects {CC:02X?}",
                cc
            )));
        }
        let identity = self.identity()?;
        if identity.format_version != 2 {
            return Err(NfcError::Unsupported(
                "this dump is not Amiibo format version 2",
            ));
        }
        if self.bytes[0] != bcc1(&uid) {
            return Err(NfcError::InvalidArgument(
                "the plaintext page-2 BCC1 does not match the UID".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn identity(&self) -> Result<AmiiboIdentity> {
        AmiiboIdentity::parse(
            &self.bytes[IDENTITY_OFFSET..IDENTITY_OFFSET + 8],
            &self.bytes,
        )
    }

    /// Replace the internal UID pages and page-2 BCC1 with another tag UID.
    /// This does not encrypt or sign; call [`encrypt`] afterwards.
    pub fn rebind_uid(&self, uid: &[u8; UID_SIZE]) -> Result<Self> {
        if uid[0] != 0x04 {
            return Err(NfcError::InvalidArgument(
                "an Amiibo UID must use the NXP manufacturer byte 04".to_owned(),
            ));
        }
        let mut plain = self.clone();
        plain.bytes[0] = bcc1(uid);
        plain.bytes[UID_OFFSET] = uid[0];
        plain.bytes[UID_OFFSET + 1] = uid[1];
        plain.bytes[UID_OFFSET + 2] = uid[2];
        plain.bytes[UID_OFFSET + 3] = bcc0(uid);
        plain.bytes[UID_OFFSET + 4] = uid[3];
        plain.bytes[UID_OFFSET + 5] = uid[4];
        plain.bytes[UID_OFFSET + 6] = uid[5];
        plain.bytes[UID_OFFSET + 7] = uid[6];
        plain.uid()?;
        Ok(plain)
    }
}

impl fmt::Debug for AmiiboPlain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmiiboPlain")
            .field("uid", &self.uid().map(|uid| format!("{uid:02X?}")))
            .field("app_data", &"<user data>")
            .finish()
    }
}

/// Decrypt a raw image and verify both signatures.
pub fn unpack(keys: &AmiiboKeys, tag: &AmiiboTag) -> Result<AmiiboPlain> {
    let plain = crypto::decrypt(keys, tag.as_bytes())?;
    let plain = AmiiboPlain { bytes: plain };
    plain.validate()?;
    Ok(plain)
}

/// Encrypt and sign an internal plaintext image.
pub fn pack(keys: &AmiiboKeys, plain: &AmiiboPlain) -> Result<AmiiboTag> {
    plain.validate()?;
    let bytes = crypto::encrypt(keys, plain.as_bytes())?;
    AmiiboTag::from_bytes(&bytes)
}
