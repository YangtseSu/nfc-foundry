//! vCard record template using the `text/vcard` media type.

use nfc_core::{NfcError, Result};

use crate::record::{NdefRecord, Tnf};

const PRIMARY_MEDIA_TYPE: &[u8] = b"text/vcard";
const LEGACY_MEDIA_TYPE: &[u8] = b"text/x-vcard";

/// A small vCard 3.0 profile with the properties commonly used on NFC tags.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VCard {
    /// Formatted name (`FN`), required.
    pub formatted_name: String,
    /// Organization (`ORG`).
    pub organization: Option<String>,
    /// Telephone number (`TEL`).
    pub phone: Option<String>,
    /// Email address (`EMAIL`).
    pub email: Option<String>,
    /// Web address (`URL`).
    pub url: Option<String>,
}

impl VCard {
    /// Create a vCard with a formatted name.
    pub fn new(formatted_name: impl Into<String>) -> Result<Self> {
        let card = Self {
            formatted_name: formatted_name.into(),
            ..Self::default()
        };
        card.validate()?;
        Ok(card)
    }

    /// Validate required fields and reject empty optional values.
    pub fn validate(&self) -> Result<()> {
        if self.formatted_name.trim().is_empty() {
            return Err(NfcError::InvalidArgument(
                "vCard formatted name must not be empty".to_owned(),
            ));
        }
        for (name, value) in [
            ("organization", &self.organization),
            ("phone", &self.phone),
            ("email", &self.email),
            ("url", &self.url),
        ] {
            if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
                return Err(NfcError::InvalidArgument(format!(
                    "vCard {name} must not be empty when present"
                )));
            }
        }
        Ok(())
    }

    /// Encode the vCard as a `text/vcard` media record.
    pub fn to_record(&self) -> Result<NdefRecord> {
        self.validate()?;
        let mut text = String::new();
        text.push_str("BEGIN:VCARD\r\n");
        text.push_str("VERSION:3.0\r\n");
        push_property(&mut text, "FN", &self.formatted_name);
        if let Some(value) = &self.organization {
            push_property(&mut text, "ORG", value);
        }
        if let Some(value) = &self.phone {
            push_property(&mut text, "TEL", value);
        }
        if let Some(value) = &self.email {
            push_property(&mut text, "EMAIL", value);
        }
        if let Some(value) = &self.url {
            push_property(&mut text, "URL", value);
        }
        text.push_str("END:VCARD\r\n");
        NdefRecord::media(PRIMARY_MEDIA_TYPE, text.into_bytes())
    }

    /// Decode a `text/vcard` or `text/x-vcard` media record.
    pub fn from_record(record: &NdefRecord) -> Result<Self> {
        if record.tnf() != Tnf::Media
            || (record.record_type() != PRIMARY_MEDIA_TYPE
                && record.record_type() != LEGACY_MEDIA_TYPE)
        {
            return Err(NfcError::InvalidArgument(
                "record is not a text/vcard media record".to_owned(),
            ));
        }
        let text = std::str::from_utf8(record.payload()).map_err(|_| {
            NfcError::InvalidArgument("vCard payload is not valid UTF-8".to_owned())
        })?;
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let lines = unfold_lines(text);
        if lines.first().map(String::as_str) != Some("BEGIN:VCARD") {
            return Err(NfcError::InvalidArgument(
                "vCard payload does not start with BEGIN:VCARD".to_owned(),
            ));
        }
        if lines.last().map(String::as_str) != Some("END:VCARD") {
            return Err(NfcError::InvalidArgument(
                "vCard payload does not end with END:VCARD".to_owned(),
            ));
        }

        let mut card = Self::default();
        let mut version_seen = false;
        for line in &lines[1..lines.len() - 1] {
            let Some((name, value)) = line.split_once(':') else {
                return Err(NfcError::InvalidArgument(format!(
                    "vCard property line has no colon: {line:?}"
                )));
            };
            let name = name.split(';').next().unwrap_or(name).to_ascii_uppercase();
            let value = unescape(value)?;
            match name.as_str() {
                "VERSION" => {
                    if !value.starts_with("3.0") {
                        return Err(NfcError::InvalidArgument(format!(
                            "vCard version {value:?} is not supported, expected 3.0"
                        )));
                    }
                    version_seen = true;
                }
                "FN" => card.formatted_name = value,
                "ORG" => card.organization = Some(value),
                "TEL" => card.phone = Some(value),
                "EMAIL" => card.email = Some(value),
                "URL" => card.url = Some(value),
                _ => {}
            }
        }
        if !version_seen {
            return Err(NfcError::InvalidArgument(
                "vCard payload is missing VERSION:3.0".to_owned(),
            ));
        }
        card.validate()?;
        Ok(card)
    }
}

fn push_property(output: &mut String, name: &str, value: &str) {
    output.push_str(name);
    output.push(':');
    output.push_str(&escape(value));
    output.push_str("\r\n");
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            ';' => escaped.push_str("\\;"),
            ',' => escaped.push_str("\\,"),
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                escaped.push_str("\\n");
            }
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn unescape(value: &str) -> Result<String> {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let escaped = characters.next().ok_or_else(|| {
            NfcError::InvalidArgument("vCard value ends with an incomplete escape".to_owned())
        })?;
        match escaped {
            'n' | 'N' => output.push('\n'),
            '\\' => output.push('\\'),
            ';' => output.push(';'),
            ',' => output.push(','),
            other => output.push(other),
        }
    }
    Ok(output)
}

fn unfold_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(continuation) = line.strip_prefix([' ', '\t']) {
            if let Some(previous) = lines.last_mut() {
                previous.push_str(continuation);
            }
        } else {
            lines.push(line.to_owned());
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}
