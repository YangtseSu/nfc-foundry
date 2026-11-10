//! NDEF record model and TNF field validation.

use nfc_core::{NfcError, Result};

/// Maximum octets in an NDEF TYPE field.
pub const MAX_TYPE_LENGTH: usize = 255;
/// Maximum octets in an NDEF ID field.
pub const MAX_ID_LENGTH: usize = 255;
/// Maximum octets in a non-chunked NDEF payload.
pub const MAX_PAYLOAD_LENGTH: usize = u32::MAX as usize;

/// Type Name Format, the low three bits of the record header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tnf {
    /// TNF 0x00: no type, ID, or payload.
    #[default]
    Empty,
    /// TNF 0x01: NFC Forum well-known type.
    WellKnown,
    /// TNF 0x02: RFC 2046 media type.
    Media,
    /// TNF 0x03: RFC 3986 absolute URI.
    AbsoluteUri,
    /// TNF 0x04: NFC Forum external type.
    External,
    /// TNF 0x05: unknown type with an omitted TYPE field.
    Unknown,
    /// TNF 0x06: continuation chunk with an omitted TYPE and ID field.
    Unchanged,
}

impl Tnf {
    /// Return the encoded three-bit TNF value.
    pub const fn bits(self) -> u8 {
        match self {
            Self::Empty => 0x00,
            Self::WellKnown => 0x01,
            Self::Media => 0x02,
            Self::AbsoluteUri => 0x03,
            Self::External => 0x04,
            Self::Unknown => 0x05,
            Self::Unchanged => 0x06,
        }
    }

    /// Decode a three-bit TNF value. The reserved value 0x07 is rejected.
    pub fn from_bits(bits: u8) -> Result<Self> {
        match bits {
            0x00 => Ok(Self::Empty),
            0x01 => Ok(Self::WellKnown),
            0x02 => Ok(Self::Media),
            0x03 => Ok(Self::AbsoluteUri),
            0x04 => Ok(Self::External),
            0x05 => Ok(Self::Unknown),
            0x06 => Ok(Self::Unchanged),
            0x07 => Err(NfcError::InvalidArgument(
                "NDEF TNF value 0x07 is reserved".to_owned(),
            )),
            other => Err(NfcError::InvalidArgument(format!(
                "NDEF TNF value 0x{other:02X} is not a three-bit field"
            ))),
        }
    }
}

/// A logical NDEF record. Message-level flags are owned by `NdefMessage`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NdefRecord {
    tnf: Tnf,
    record_type: Vec<u8>,
    id: Vec<u8>,
    payload: Vec<u8>,
}

impl NdefRecord {
    /// Create a record and validate its TNF field constraints.
    pub fn new(
        tnf: Tnf,
        record_type: impl Into<Vec<u8>>,
        id: impl Into<Vec<u8>>,
        payload: impl Into<Vec<u8>>,
    ) -> Result<Self> {
        let record = Self {
            tnf,
            record_type: record_type.into(),
            id: id.into(),
            payload: payload.into(),
        };
        record.validate()?;
        Ok(record)
    }

    /// Create the empty record (TNF 0x00).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create a well-known record with no ID.
    pub fn well_known(
        record_type: impl Into<Vec<u8>>,
        payload: impl Into<Vec<u8>>,
    ) -> Result<Self> {
        Self::new(Tnf::WellKnown, record_type, Vec::new(), payload)
    }

    /// Create a media-type record with no ID.
    pub fn media(record_type: impl Into<Vec<u8>>, payload: impl Into<Vec<u8>>) -> Result<Self> {
        Self::new(Tnf::Media, record_type, Vec::new(), payload)
    }

    /// Create an absolute-URI record with no ID.
    pub fn absolute_uri(uri: impl Into<Vec<u8>>, payload: impl Into<Vec<u8>>) -> Result<Self> {
        Self::new(Tnf::AbsoluteUri, uri, Vec::new(), payload)
    }

    /// Create an external-type record with no ID.
    pub fn external(record_type: impl Into<Vec<u8>>, payload: impl Into<Vec<u8>>) -> Result<Self> {
        Self::new(Tnf::External, record_type, Vec::new(), payload)
    }

    /// Create an unknown-type record with an omitted TYPE field.
    pub fn unknown(payload: impl Into<Vec<u8>>) -> Result<Self> {
        Self::new(Tnf::Unknown, Vec::new(), Vec::new(), payload)
    }

    /// The type name format.
    pub const fn tnf(&self) -> Tnf {
        self.tnf
    }

    /// The raw TYPE field.
    pub fn record_type(&self) -> &[u8] {
        &self.record_type
    }

    /// The raw ID field.
    pub fn id(&self) -> &[u8] {
        &self.id
    }

    /// The raw payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Validate TNF field constraints and field sizes.
    pub fn validate(&self) -> Result<()> {
        if self.record_type.len() > MAX_TYPE_LENGTH {
            return Err(NfcError::InvalidArgument(format!(
                "NDEF TYPE field is {} octets, maximum is {MAX_TYPE_LENGTH}",
                self.record_type.len()
            )));
        }
        if self.id.len() > MAX_ID_LENGTH {
            return Err(NfcError::InvalidArgument(format!(
                "NDEF ID field is {} octets, maximum is {MAX_ID_LENGTH}",
                self.id.len()
            )));
        }
        if self.payload.len() > MAX_PAYLOAD_LENGTH {
            return Err(NfcError::InvalidArgument(format!(
                "NDEF payload is {} octets, maximum is {MAX_PAYLOAD_LENGTH}",
                self.payload.len()
            )));
        }
        match self.tnf {
            Tnf::Empty => {
                if !self.record_type.is_empty() || !self.id.is_empty() || !self.payload.is_empty() {
                    return Err(NfcError::InvalidArgument(
                        "NDEF Empty record must omit TYPE, ID, and PAYLOAD".to_owned(),
                    ));
                }
            }
            Tnf::Unknown => {
                if !self.record_type.is_empty() {
                    return Err(NfcError::InvalidArgument(
                        "NDEF Unknown record must omit the TYPE field".to_owned(),
                    ));
                }
            }
            Tnf::Unchanged => {
                return Err(NfcError::InvalidArgument(
                    "NDEF Unchanged TNF is only valid in a chunk continuation".to_owned(),
                ));
            }
            Tnf::WellKnown | Tnf::Media | Tnf::AbsoluteUri | Tnf::External => {
                if self.record_type.is_empty() {
                    return Err(NfcError::InvalidArgument(format!(
                        "NDEF {:?} record requires a non-empty TYPE field",
                        self.tnf
                    )));
                }
            }
        }
        Ok(())
    }

    /// Build a record whose parts were already checked by the decoder.
    pub(crate) fn from_decoded(
        tnf: Tnf,
        record_type: Vec<u8>,
        id: Vec<u8>,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            tnf,
            record_type,
            id,
            payload,
        }
    }
}
