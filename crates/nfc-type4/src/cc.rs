//! NFC Forum Type 4 capability container and File Control TLVs.

use nfc_core::{NfcError, Result};

/// Minimum CC length: header plus one NDEF File Control TLV.
pub const CC_MIN_LENGTH: usize = 15;
/// TLV type for an NDEF File Control TLV.
pub const TAG_NDEF_FILE_CONTROL: u8 = 0x04;
/// TLV type for a Proprietary File Control TLV.
pub const TAG_PROPRIETARY_FILE_CONTROL: u8 = 0x05;
/// TLV type for a NULL block.
pub const TAG_NULL: u8 = 0x00;
/// TLV type for the Terminator block.
pub const TAG_TERMINATOR: u8 = 0xFE;
/// Length octet that selects the three-octet length form.
const LENGTH_LONG_FORM: u8 = 0xFF;
/// Minimum MLe value from the Type 4 Tag specification.
pub const MIN_MLE: u16 = 0x000F;
/// Minimum NDEF file size: two NLEN bytes plus at least one message byte.
pub const MIN_NDEF_FILE_SIZE: u16 = 0x0005;

/// One NDEF File Control TLV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NdefFileControl {
    pub file_id: u16,
    /// Maximum size of the NDEF file, including the two NLEN bytes.
    pub max_size: u16,
    pub read_access: u8,
    pub write_access: u8,
}

impl NdefFileControl {
    /// Largest NDEF message that fits after the two-byte NLEN field.
    pub const fn capacity(self) -> usize {
        (self.max_size as usize).saturating_sub(2)
    }

    pub const fn readable(self) -> bool {
        self.read_access == 0x00
    }

    pub const fn writable(self) -> bool {
        self.write_access == 0x00
    }

    /// A human-readable reason why a write is unavailable.
    pub fn write_denied_reason(self) -> Option<&'static str> {
        match self.write_access {
            0x00 => None,
            0xFF => Some("the NDEF File Control TLV marks the file read-only"),
            0x80..=0xFE => Some(
                "the NDEF file requires a proprietary security condition that this reader does not provide",
            ),
            _ => Some("the NDEF File Control TLV uses an invalid write access condition"),
        }
    }
}

/// One Proprietary File Control TLV. The file is not used by the NDEF adapter;
/// the TLV is preserved and reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProprietaryFileControl {
    pub file_id: u16,
    pub max_size: u16,
    pub read_access: u8,
    pub write_access: u8,
}

/// One TLV from the capability container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CcTlv {
    NdefFile(NdefFileControl),
    ProprietaryFile(ProprietaryFileControl),
    Unknown { tag: u8, value: Vec<u8> },
    Null,
    Terminator,
}

/// Parsed capability container. The original bytes are kept so backups and
/// displays do not re-encode unknown or proprietary TLVs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type4CapabilityContainer {
    pub cclen: u16,
    pub major_version: u8,
    pub minor_version: u8,
    pub max_le: u16,
    pub max_lc: u16,
    tlvs: Vec<CcTlv>,
    raw: Vec<u8>,
}

impl Type4CapabilityContainer {
    /// Parse a complete CC file. The caller must read exactly `CCLEN` bytes;
    /// trailing bytes are rejected rather than silently ignored.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < CC_MIN_LENGTH {
            return Err(NfcError::InvalidArgument(format!(
                "a Type 4 capability container is at least {CC_MIN_LENGTH} bytes, got {}",
                bytes.len()
            )));
        }
        let cclen = u16::from_be_bytes([bytes[0], bytes[1]]);
        if usize::from(cclen) != bytes.len() {
            return Err(NfcError::InvalidArgument(format!(
                "the capability container declares {cclen} bytes but {} were read",
                bytes.len()
            )));
        }
        if cclen < CC_MIN_LENGTH as u16 {
            return Err(NfcError::InvalidArgument(format!(
                "the capability container length {cclen} is below {CC_MIN_LENGTH}"
            )));
        }
        let major_version = bytes[2] >> 4;
        let minor_version = bytes[2] & 0x0F;
        if major_version != 2 || minor_version != 0 {
            return Err(NfcError::Unsupported(
                "this Type 4 mapping version is not supported; only version 2.0 is implemented",
            ));
        }
        let max_le = u16::from_be_bytes([bytes[3], bytes[4]]);
        let max_lc = u16::from_be_bytes([bytes[5], bytes[6]]);
        if max_le < MIN_MLE {
            return Err(NfcError::InvalidArgument(format!(
                "the capability container MLe {max_le} is below {MIN_MLE}"
            )));
        }
        if max_lc == 0 {
            return Err(NfcError::InvalidArgument(
                "the capability container MLc is zero".to_owned(),
            ));
        }

        let mut tlvs = Vec::new();
        let mut offset = 7;
        while offset < bytes.len() {
            let tag = bytes[offset];
            offset += 1;
            match tag {
                TAG_NULL => {
                    tlvs.push(CcTlv::Null);
                    continue;
                }
                TAG_TERMINATOR => {
                    tlvs.push(CcTlv::Terminator);
                    break;
                }
                _ => {}
            }
            let Some(&first) = bytes.get(offset) else {
                return Err(NfcError::InvalidArgument(format!(
                    "CC TLV {tag:#04X} has no length octet"
                )));
            };
            offset += 1;
            let length = if first == LENGTH_LONG_FORM {
                if offset + 2 > bytes.len() {
                    return Err(NfcError::InvalidArgument(format!(
                        "CC TLV {tag:#04X} has a truncated long length"
                    )));
                }
                let value = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
                offset += 2;
                if value < u16::from(LENGTH_LONG_FORM) {
                    return Err(NfcError::InvalidArgument(format!(
                        "CC TLV {tag:#04X} uses the long length form for {value} bytes"
                    )));
                }
                usize::from(value)
            } else {
                usize::from(first)
            };
            if offset + length > bytes.len() {
                return Err(NfcError::InvalidArgument(format!(
                    "CC TLV {tag:#04X} claims {length} bytes but the container ends early"
                )));
            }
            let value = &bytes[offset..offset + length];
            offset += length;
            match tag {
                TAG_NDEF_FILE_CONTROL => {
                    if length != 6 {
                        return Err(NfcError::InvalidArgument(format!(
                            "an NDEF File Control TLV is 6 bytes, got {length}"
                        )));
                    }
                    tlvs.push(CcTlv::NdefFile(parse_file_control(value)?));
                }
                TAG_PROPRIETARY_FILE_CONTROL => {
                    if length != 6 {
                        return Err(NfcError::InvalidArgument(format!(
                            "a Proprietary File Control TLV is 6 bytes, got {length}"
                        )));
                    }
                    tlvs.push(CcTlv::ProprietaryFile(parse_proprietary_control(value)?));
                }
                _ => tlvs.push(CcTlv::Unknown {
                    tag,
                    value: value.to_vec(),
                }),
            }
        }

        if !matches!(tlvs.first(), Some(CcTlv::NdefFile(_))) {
            return Err(NfcError::InvalidArgument(
                "the first capability container TLV is not an NDEF File Control TLV".to_owned(),
            ));
        }

        Ok(Self {
            cclen,
            major_version,
            minor_version,
            max_le,
            max_lc,
            tlvs,
            raw: bytes.to_vec(),
        })
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub fn tlvs(&self) -> &[CcTlv] {
        &self.tlvs
    }

    pub fn ndef_file(&self) -> Result<NdefFileControl> {
        self.ndef_files().next().ok_or_else(|| {
            NfcError::InvalidArgument(
                "the capability container has no NDEF File Control TLV".to_owned(),
            )
        })
    }

    pub fn ndef_files(&self) -> impl Iterator<Item = NdefFileControl> + '_ {
        self.tlvs.iter().filter_map(|tlv| match tlv {
            CcTlv::NdefFile(file) => Some(*file),
            _ => None,
        })
    }

    pub fn proprietary_files(&self) -> impl Iterator<Item = ProprietaryFileControl> + '_ {
        self.tlvs.iter().filter_map(|tlv| match tlv {
            CcTlv::ProprietaryFile(file) => Some(*file),
            _ => None,
        })
    }

    pub fn writable(&self) -> bool {
        self.ndef_file().is_ok_and(NdefFileControl::writable)
    }

    pub fn read_only_reason(&self) -> Option<&'static str> {
        self.ndef_file()
            .ok()
            .and_then(NdefFileControl::write_denied_reason)
    }
}

fn parse_file_control(value: &[u8]) -> Result<NdefFileControl> {
    let file_id = u16::from_be_bytes([value[0], value[1]]);
    if !matches!(file_id, 0xE100 | 0xE101 | 0xE104..=0xE10F) {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF file ID {file_id:04X} is outside the Type 4 NDEF file range"
        )));
    }
    let max_size = u16::from_be_bytes([value[2], value[3]]);
    if max_size < MIN_NDEF_FILE_SIZE {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF file size {max_size} is below {MIN_NDEF_FILE_SIZE}"
        )));
    }
    Ok(NdefFileControl {
        file_id,
        max_size,
        read_access: value[4],
        write_access: value[5],
    })
}

fn parse_proprietary_control(value: &[u8]) -> Result<ProprietaryFileControl> {
    let file_id = u16::from_be_bytes([value[0], value[1]]);
    let max_size = u16::from_be_bytes([value[2], value[3]]);
    if max_size < 3 {
        return Err(NfcError::InvalidArgument(format!(
            "the proprietary file size {max_size} is below 3"
        )));
    }
    Ok(ProprietaryFileControl {
        file_id,
        max_size,
        read_access: value[4],
        write_access: value[5],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ntag424_cc() -> Vec<u8> {
        vec![
            0x00, 0x17, 0x20, 0x01, 0x00, 0x00, 0xFF, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
            0x00, 0x05, 0x06, 0xE1, 0x05, 0x00, 0x80, 0x82, 0x83,
        ]
    }

    #[test]
    fn parses_ntag424_and_desfire_containers() {
        let cc = Type4CapabilityContainer::parse(&ntag424_cc()).expect("NTAG 424 CC");
        assert_eq!(cc.cclen, 23);
        assert_eq!((cc.major_version, cc.minor_version), (2, 0));
        assert_eq!((cc.max_le, cc.max_lc), (0x0100, 0x00FF));
        assert_eq!(
            cc.ndef_file().expect("NDEF file"),
            NdefFileControl {
                file_id: 0xE104,
                max_size: 0x0100,
                read_access: 0,
                write_access: 0,
            }
        );
        assert_eq!(cc.ndef_file().expect("NDEF file").capacity(), 254);
        assert!(cc.writable());
        assert_eq!(cc.proprietary_files().count(), 1);
        assert_eq!(cc.raw(), ntag424_cc());

        let desfire = Type4CapabilityContainer::parse(&[
            0x00, 0x0F, 0x20, 0x00, 0x3A, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
            0x00,
        ])
        .expect("DESFire CC");
        assert_eq!(desfire.cclen, 15);
        assert_eq!(desfire.ndef_file().expect("NDEF file").max_size, 0x0100);
        assert!(desfire.writable());
    }

    #[test]
    fn reports_read_only_and_proprietary_write_access() {
        let mut bytes = ntag424_cc();
        bytes[14] = 0xFF;
        let cc = Type4CapabilityContainer::parse(&bytes).expect("CC");
        assert!(!cc.writable());
        assert_eq!(
            cc.read_only_reason(),
            Some("the NDEF File Control TLV marks the file read-only")
        );

        bytes[14] = 0x82;
        let cc = Type4CapabilityContainer::parse(&bytes).expect("CC");
        assert!(!cc.writable());
        assert!(
            cc.read_only_reason()
                .expect("reason")
                .contains("proprietary")
        );
    }

    #[test]
    fn rejects_malformed_containers() {
        assert!(Type4CapabilityContainer::parse(&ntag424_cc()[..14]).is_err());
        let mut bad = ntag424_cc();
        bad[0] = 0x00;
        bad[1] = 0x10;
        assert!(Type4CapabilityContainer::parse(&bad).is_err());
        let mut version = ntag424_cc();
        version[2] = 0x30;
        assert!(matches!(
            Type4CapabilityContainer::parse(&version),
            Err(NfcError::Unsupported(_))
        ));
        let mut mle = ntag424_cc();
        mle[3] = 0x00;
        mle[4] = 0x0E;
        assert!(Type4CapabilityContainer::parse(&mle).is_err());
        let mut first = ntag424_cc();
        first[7] = 0x05;
        assert!(Type4CapabilityContainer::parse(&first).is_err());
        let mut size = ntag424_cc();
        size[11] = 0x00;
        size[12] = 0x04;
        assert!(Type4CapabilityContainer::parse(&size).is_err());
        let mut file_id = ntag424_cc();
        file_id[9] = 0xE1;
        file_id[10] = 0x03;
        assert!(Type4CapabilityContainer::parse(&file_id).is_err());
    }

    #[test]
    fn preserves_unknown_tlvs_and_long_lengths() {
        let mut bytes = vec![0x00, 0x00, 0x20, 0x00, 0x0F, 0x00, 0x01];
        bytes.extend_from_slice(&[0x04, 0x06, 0xE1, 0x04, 0x00, 0x20, 0x00, 0x00]);
        bytes.extend_from_slice(&[0xFD, 0xFF, 0x00, 0xFF]);
        bytes.extend_from_slice(&[0xAA; 0xFF]);
        bytes.extend_from_slice(&[0xFE]);
        let length = bytes.len() as u16;
        bytes[0..2].copy_from_slice(&length.to_be_bytes());
        let cc = Type4CapabilityContainer::parse(&bytes).expect("CC");
        assert!(
            cc.tlvs()
                .iter()
                .any(|tlv| matches!(tlv, CcTlv::Unknown { tag: 0xFD, .. }))
        );
        assert!(matches!(cc.tlvs().last(), Some(CcTlv::Terminator)));
    }
}
