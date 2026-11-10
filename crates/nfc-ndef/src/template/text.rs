//! NFC Forum Text RTD (`T`).

use nfc_core::{NfcError, Result};

use crate::record::{NdefRecord, Tnf};

const TYPE: &[u8] = b"T";
const UTF16_FLAG: u8 = 0x80;
const LANGUAGE_MASK: u8 = 0x3F;

/// Transfer encoding used by a Text record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextEncoding {
    /// UTF-8 without a byte-order mark.
    #[default]
    Utf8,
    /// UTF-16 with a big-endian byte-order mark.
    Utf16,
}

/// A decoded NFC Forum Text record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRecord {
    /// The text content.
    pub text: String,
    /// IANA language code, 1 to 63 US-ASCII characters.
    pub language: String,
    /// Encoding used on the wire.
    pub encoding: TextEncoding,
}

impl TextRecord {
    /// Create a UTF-8 Text record.
    pub fn new(text: impl Into<String>, language: impl Into<String>) -> Result<Self> {
        let record = Self {
            text: text.into(),
            language: language.into(),
            encoding: TextEncoding::Utf8,
        };
        record.validate()?;
        Ok(record)
    }

    /// Create a Text record with an explicit transfer encoding.
    pub fn with_encoding(
        text: impl Into<String>,
        language: impl Into<String>,
        encoding: TextEncoding,
    ) -> Result<Self> {
        let record = Self {
            text: text.into(),
            language: language.into(),
            encoding,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate the language tag.
    pub fn validate(&self) -> Result<()> {
        validate_language(&self.language)
    }

    /// Encode the Text record.
    pub fn to_record(&self) -> Result<NdefRecord> {
        self.validate()?;
        let language = self.language.as_bytes();
        let mut payload = Vec::with_capacity(1 + language.len() + self.text.len());
        let encoding_bit = if self.encoding == TextEncoding::Utf16 {
            UTF16_FLAG
        } else {
            0
        };
        payload.push(encoding_bit | language.len() as u8);
        payload.extend_from_slice(language);
        match self.encoding {
            TextEncoding::Utf8 => payload.extend_from_slice(self.text.as_bytes()),
            TextEncoding::Utf16 => {
                payload.extend_from_slice(&[0xFE, 0xFF]);
                for unit in self.text.encode_utf16() {
                    payload.extend_from_slice(&unit.to_be_bytes());
                }
            }
        }
        NdefRecord::well_known(TYPE, payload)
    }

    /// Decode a Text record.
    pub fn from_record(record: &NdefRecord) -> Result<Self> {
        require_text_record(record)?;
        let payload = record.payload();
        let status = *payload.first().ok_or_else(|| {
            NfcError::InvalidArgument("Text record payload is missing the status octet".to_owned())
        })?;
        let language_length = usize::from(status & LANGUAGE_MASK);
        if language_length == 0 {
            return Err(NfcError::InvalidArgument(
                "Text record language code is empty".to_owned(),
            ));
        }
        let language_end = 1 + language_length;
        let language_bytes = payload.get(1..language_end).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "Text record language code declares {language_length} octets but the payload is {} octets",
                payload.len()
            ))
        })?;
        let language = std::str::from_utf8(language_bytes)
            .map_err(|_| NfcError::InvalidArgument("Text record language is not UTF-8".to_owned()))?
            .to_owned();
        validate_language(&language)?;

        let body = &payload[language_end..];
        let (text, encoding) = if status & UTF16_FLAG != 0 {
            (decode_utf16(body)?, TextEncoding::Utf16)
        } else {
            let text = std::str::from_utf8(body).map_err(|_| {
                NfcError::InvalidArgument("Text record payload is not valid UTF-8".to_owned())
            })?;
            (text.to_owned(), TextEncoding::Utf8)
        };
        Ok(Self {
            text,
            language,
            encoding,
        })
    }
}

pub(crate) fn validate_language(language: &str) -> Result<()> {
    let bytes = language.as_bytes();
    if bytes.is_empty() || bytes.len() > usize::from(LANGUAGE_MASK) {
        return Err(NfcError::InvalidArgument(format!(
            "language code must be 1 to {LANGUAGE_MASK} octets, got {}",
            bytes.len()
        )));
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
    {
        return Err(NfcError::InvalidArgument(
            "language code must contain only US-ASCII letters, digits, and hyphens".to_owned(),
        ));
    }
    Ok(())
}

fn decode_utf16(body: &[u8]) -> Result<String> {
    let (data, little_endian) = if body.starts_with(&[0xFF, 0xFE]) {
        (&body[2..], true)
    } else if body.starts_with(&[0xFE, 0xFF]) {
        (&body[2..], false)
    } else {
        (body, false)
    };
    if !data.len().is_multiple_of(2) {
        return Err(NfcError::InvalidArgument(
            "Text record UTF-16 payload has an odd number of octets".to_owned(),
        ));
    }
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes(*pair)
            } else {
                u16::from_be_bytes(*pair)
            }
        })
        .collect();
    String::from_utf16(&units)
        .map_err(|_| NfcError::InvalidArgument("Text record contains invalid UTF-16".to_owned()))
}

fn require_text_record(record: &NdefRecord) -> Result<()> {
    if record.tnf() != Tnf::WellKnown || record.record_type() != TYPE {
        return Err(NfcError::InvalidArgument(
            "record is not an NFC Forum Text record".to_owned(),
        ));
    }
    Ok(())
}
