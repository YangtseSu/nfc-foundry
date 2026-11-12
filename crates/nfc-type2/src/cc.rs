//! NFC Forum Type 2 capability container (page 3).

use nfc_core::{NfcError, Result};

pub const CC_LENGTH: usize = 4;
/// Type 2 capability containers start with the magic number `E1`.
pub const CC_MAGIC: u8 = 0xE1;
pub const CC_MAJOR_VERSION: u8 = 1;
pub const CC_MINOR_VERSION_MAX: u8 = 2;
pub const CC_SIZE_GRANULARITY: usize = 8;

/// Capability container from page 3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapabilityContainer {
    pub major_version: u8,
    pub minor_version: u8,
    /// NDEF data area size in bytes, derived from CC byte 2.
    pub size: u16,
    pub access: u8,
}

impl CapabilityContainer {
    /// Parse and validate a four-byte CC. The data area size is
    /// `CC[2] * 8`; a zero-size area is never accepted.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != CC_LENGTH {
            return Err(NfcError::InvalidArgument(format!(
                "a Type 2 capability container is {CC_LENGTH} bytes, got {}",
                bytes.len()
            )));
        }
        if bytes[0] != CC_MAGIC {
            return Err(NfcError::InvalidArgument(format!(
                "the Type 2 capability container magic {:#04X} is not {CC_MAGIC:#04X}",
                bytes[0]
            )));
        }
        let major_version = bytes[1] >> 4;
        let minor_version = bytes[1] & 0x0F;
        if major_version != CC_MAJOR_VERSION || minor_version > CC_MINOR_VERSION_MAX {
            return Err(NfcError::InvalidArgument(format!(
                "unsupported Type 2 capability container version {major_version}.{minor_version}"
            )));
        }
        if bytes[2] == 0 {
            return Err(NfcError::InvalidArgument(
                "the Type 2 capability container reports a zero-size data area".to_owned(),
            ));
        }
        Ok(Self {
            major_version,
            minor_version,
            size: u16::from(bytes[2]) * CC_SIZE_GRANULARITY as u16,
            access: bytes[3],
        })
    }

    pub fn from_page(page: [u8; 4]) -> Result<Self> {
        Self::parse(&page)
    }

    /// A parsed container always has a non-zero data area. The default value
    /// is used for an unformatted tag whose page 3 is not a CC.
    pub const fn is_present(self) -> bool {
        self.size != 0
    }

    pub fn to_bytes(self) -> [u8; CC_LENGTH] {
        [
            CC_MAGIC,
            (self.major_version << 4) | (self.minor_version & 0x0F),
            (self.size / CC_SIZE_GRANULARITY as u16) as u8,
            self.access,
        ]
    }

    /// Read access nibble: 0 is free access.
    pub const fn read_access(self) -> u8 {
        self.access & 0x0F
    }

    /// Write access nibble: 0 is free access.
    pub const fn write_access(self) -> u8 {
        self.access >> 4
    }

    /// The CC write-access property restricts writing the data area.
    pub const fn writable(self) -> bool {
        self.write_access() == 0
    }

    pub const fn read_only(self) -> bool {
        self.read_access() != 0
    }

    /// Largest NDEF message that fits the CC data area with its TLV header and
    /// a Terminator.
    pub fn ndef_capacity(self) -> usize {
        nfc_ndef::ndef_capacity(usize::from(self.size)) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ntag_capability_containers() {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0x00]).expect("NTAG213 CC");
        assert_eq!(cc.size, 144);
        assert_eq!(cc.ndef_capacity(), 141);
        assert!(cc.writable());
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x3E, 0x00]).expect("NTAG215 CC");
        assert_eq!(cc.size, 496);
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x6D, 0x00]).expect("NTAG216 CC");
        assert_eq!(cc.size, 872);
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x06, 0x00]).expect("Ultralight CC");
        assert_eq!(cc.size, 48);
    }

    #[test]
    fn rejects_malformed_capability_containers() {
        assert!(CapabilityContainer::parse(&[0xE2, 0x10, 0x12, 0x00]).is_err());
        assert!(CapabilityContainer::parse(&[0xE1, 0x10, 0x00, 0x00]).is_err());
        assert!(CapabilityContainer::parse(&[0xE1, 0x10, 0x12]).is_err());
        assert!(CapabilityContainer::parse(&[0xE1, 0x20, 0x12, 0x00]).is_err());
    }

    #[test]
    fn rounds_trip_and_expands_access_bits() {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0xF0]).expect("CC");
        assert_eq!(cc.to_bytes(), [0xE1, 0x10, 0x12, 0xF0]);
        assert_eq!(cc.read_access(), 0);
        assert_eq!(cc.write_access(), 0x0F);
        assert!(!cc.writable());
    }
}
