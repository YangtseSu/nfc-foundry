//! NFC Forum Type 2 GET_VERSION product descriptor.

use nfc_core::{NfcError, Result, TYPE2_VERSION_SIZE};

use crate::layout::Type2Product;

const NXP_VENDOR_ID: u8 = 0x04;
const PRODUCT_TYPE_ULTRALIGHT: u8 = 0x03;
const PRODUCT_TYPE_NTAG: u8 = 0x04;
const PRODUCT_MAJOR_EV1: u8 = 0x01;
const PRODUCT_MAJOR_NTAG21X: u8 = 0x01;

/// The eight-byte response to the Type 2 GET_VERSION command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Type2Version {
    pub header: u8,
    pub vendor_id: u8,
    pub product_type: u8,
    pub product_subtype: u8,
    pub major_version: u8,
    pub minor_version: u8,
    pub storage_size: u8,
    pub protocol_type: u8,
}

impl Type2Version {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != TYPE2_VERSION_SIZE {
            return Err(NfcError::InvalidArgument(format!(
                "a Type 2 GET_VERSION response is {TYPE2_VERSION_SIZE} bytes, got {}",
                bytes.len()
            )));
        }
        Ok(Self {
            header: bytes[0],
            vendor_id: bytes[1],
            product_type: bytes[2],
            product_subtype: bytes[3],
            major_version: bytes[4],
            minor_version: bytes[5],
            storage_size: bytes[6],
            protocol_type: bytes[7],
        })
    }

    /// The product whose page geometry is fixed by the version response.
    ///
    /// The storage-size byte is matched exactly. The datasheet encodes ranges
    /// with the low bit, but every product supported here has a known exact
    /// size, so an unexpected byte is not guessed into a layout.
    pub const fn product(self) -> Option<Type2Product> {
        if self.header != 0x00
            || self.vendor_id != NXP_VENDOR_ID
            || self.minor_version != 0x00
            || self.protocol_type != 0x03
        {
            return None;
        }
        match (self.product_type, self.major_version, self.storage_size) {
            (PRODUCT_TYPE_ULTRALIGHT, PRODUCT_MAJOR_EV1, 0x0B) => {
                Some(Type2Product::UltralightEv1Ul11)
            }
            (PRODUCT_TYPE_ULTRALIGHT, PRODUCT_MAJOR_EV1, 0x0E) => {
                Some(Type2Product::UltralightEv1Ul21)
            }
            (PRODUCT_TYPE_NTAG, PRODUCT_MAJOR_NTAG21X, 0x0F) => Some(Type2Product::Ntag213),
            (PRODUCT_TYPE_NTAG, PRODUCT_MAJOR_NTAG21X, 0x11) => Some(Type2Product::Ntag215),
            (PRODUCT_TYPE_NTAG, PRODUCT_MAJOR_NTAG21X, 0x13) => Some(Type2Product::Ntag216),
            _ => None,
        }
    }

    /// True when the version response identifies an Ultralight EV1 variant.
    pub const fn is_ultralight_ev1(self) -> bool {
        matches!(
            self.product(),
            Some(Type2Product::UltralightEv1Ul11 | Type2Product::UltralightEv1Ul21)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Type2Product;

    #[test]
    fn identifies_ultralight_ev1_variants() {
        let ul11 = Type2Version::parse(&[0x00, 0x04, 0x03, 0x01, 0x01, 0x00, 0x0B, 0x03])
            .expect("UL11 version");
        assert_eq!(ul11.product(), Some(Type2Product::UltralightEv1Ul11));
        assert!(ul11.is_ultralight_ev1());

        let ul21 = Type2Version::parse(&[0x00, 0x04, 0x03, 0x02, 0x01, 0x00, 0x0E, 0x03])
            .expect("UL21 version");
        assert_eq!(ul21.product(), Some(Type2Product::UltralightEv1Ul21));
    }

    #[test]
    fn identifies_ntag21x_variants() {
        for (storage, product) in [
            (0x0F, Type2Product::Ntag213),
            (0x11, Type2Product::Ntag215),
            (0x13, Type2Product::Ntag216),
        ] {
            let version = Type2Version::parse(&[0x00, 0x04, 0x04, 0x02, 0x01, 0x00, storage, 0x03])
                .expect("NTAG version");
            assert_eq!(version.product(), Some(product));
            assert!(!version.is_ultralight_ev1());
        }
    }

    #[test]
    fn rejects_unknown_or_malformed_versions() {
        assert!(Type2Version::parse(&[0x00, 0x04, 0x03, 0x01, 0x01, 0x00, 0x0B]).is_err());
        let unknown = Type2Version::parse(&[0x00, 0x04, 0x03, 0x01, 0x02, 0x00, 0x0B, 0x03])
            .expect("version");
        assert_eq!(unknown.product(), None);
        let vendor = Type2Version::parse(&[0x00, 0x05, 0x03, 0x01, 0x01, 0x00, 0x0B, 0x03])
            .expect("version");
        assert_eq!(vendor.product(), None);
    }
}
