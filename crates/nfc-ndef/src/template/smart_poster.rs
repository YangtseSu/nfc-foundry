//! NFC Forum Smart Poster RTD (`Sp`).

use nfc_core::{NfcError, Result};

use crate::message::NdefMessage;
use crate::record::{NdefRecord, Tnf};
use crate::template::text::TextRecord;
use crate::template::uri::UriRecord;

const TYPE: &[u8] = b"Sp";
const ACTION_TYPE: &[u8] = b"act";
const URI_TYPE: &[u8] = b"U";
const TEXT_TYPE: &[u8] = b"T";

/// Recommended action for a Smart Poster resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartPosterAction {
    /// Execute the URI action.
    Execute,
    /// Save the URI for later.
    Save,
    /// Open the URI for editing.
    Edit,
}

impl SmartPosterAction {
    const fn code(self) -> u8 {
        match self {
            Self::Execute => 0,
            Self::Save => 1,
            Self::Edit => 2,
        }
    }

    fn from_code(code: u8) -> Result<Self> {
        match code {
            0 => Ok(Self::Execute),
            1 => Ok(Self::Save),
            2 => Ok(Self::Edit),
            other => Err(NfcError::InvalidArgument(format!(
                "Smart Poster action value {other} is not defined"
            ))),
        }
    }
}

/// A decoded Smart Poster record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartPoster {
    /// The resource URI.
    pub uri: String,
    /// Optional text titles, typically one per language.
    pub titles: Vec<TextRecord>,
    /// Optional recommended action.
    pub action: Option<SmartPosterAction>,
    /// Nested records that are not URI, Text, or Action records, preserved in
    /// original order.
    pub additional: Vec<NdefRecord>,
}

impl SmartPoster {
    /// Create a Smart Poster with a resource URI and no titles or action.
    pub fn new(uri: impl Into<String>) -> Result<Self> {
        Ok(Self {
            uri: UriRecord::new(uri)?.uri,
            titles: Vec::new(),
            action: None,
            additional: Vec::new(),
        })
    }

    /// Add a text title.
    pub fn with_title(mut self, title: TextRecord) -> Self {
        self.titles.push(title);
        self
    }

    /// Set the recommended action.
    pub fn with_action(mut self, action: SmartPosterAction) -> Self {
        self.action = Some(action);
        self
    }

    /// Encode the Smart Poster record, nesting the URI, titles, action, and
    /// additional records as an NDEF message.
    pub fn to_record(&self) -> Result<NdefRecord> {
        let mut records = vec![UriRecord::new(&self.uri)?.to_record()?];
        for title in &self.titles {
            records.push(title.to_record()?);
        }
        if let Some(action) = self.action {
            records.push(NdefRecord::well_known(ACTION_TYPE, [action.code()])?);
        }
        records.extend(self.additional.iter().cloned());
        let nested = NdefMessage::new(records)?;
        NdefRecord::well_known(TYPE, nested.encode()?)
    }

    /// Decode a Smart Poster record.
    pub fn from_record(record: &NdefRecord) -> Result<Self> {
        if record.tnf() != Tnf::WellKnown || record.record_type() != TYPE {
            return Err(NfcError::InvalidArgument(
                "record is not an NFC Forum Smart Poster record".to_owned(),
            ));
        }
        let nested = NdefMessage::decode(record.payload())?;
        let mut uri: Option<String> = None;
        let mut titles = Vec::new();
        let mut action = None;
        let mut additional = Vec::new();

        for record in nested.records() {
            if record.tnf() == Tnf::WellKnown && record.record_type() == URI_TYPE {
                if uri.is_some() {
                    return Err(NfcError::InvalidArgument(
                        "Smart Poster contains more than one URI record".to_owned(),
                    ));
                }
                uri = Some(UriRecord::from_record(record)?.uri);
            } else if record.tnf() == Tnf::WellKnown && record.record_type() == TEXT_TYPE {
                titles.push(TextRecord::from_record(record)?);
            } else if record.tnf() == Tnf::WellKnown && record.record_type() == ACTION_TYPE {
                if action.is_some() {
                    return Err(NfcError::InvalidArgument(
                        "Smart Poster contains more than one Action record".to_owned(),
                    ));
                }
                let payload = record.payload();
                if payload.len() != 1 {
                    return Err(NfcError::InvalidArgument(format!(
                        "Smart Poster Action payload is {} octets, expected 1",
                        payload.len()
                    )));
                }
                action = Some(SmartPosterAction::from_code(payload[0])?);
            } else {
                additional.push(record.clone());
            }
        }

        Ok(Self {
            uri: uri.ok_or_else(|| {
                NfcError::InvalidArgument("Smart Poster requires a URI record".to_owned())
            })?,
            titles,
            action,
            additional,
        })
    }
}
