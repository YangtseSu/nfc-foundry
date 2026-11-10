//! Wi-Fi Easy Connect (DPP) bootstrapping record template.

use nfc_core::{NfcError, Result};

use crate::record::{NdefRecord, Tnf};
use crate::template::uri::UriRecord;

const MEDIA_TYPE: &[u8] = b"application/vnd.wfa.dpp";

/// A Wi-Fi DPP bootstrapping URI and its two NFC carrier forms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DppBootstrap {
    uri: String,
}

impl DppBootstrap {
    /// Validate and wrap a DPP bootstrapping URI.
    pub fn new(uri: impl Into<String>) -> Result<Self> {
        let uri = uri.into();
        validate_uri(&uri)?;
        Ok(Self { uri })
    }

    /// The bootstrapping URI.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// Encode the standard NFC Forum URI record carrier.
    pub fn to_uri_record(&self) -> Result<NdefRecord> {
        UriRecord::new(&self.uri)?.to_record()
    }

    /// Encode the Wi-Fi DPP Bootstrapping Carrier Configuration record
    /// (`application/vnd.wfa.dpp`), whose payload is the URI-record payload.
    pub fn to_media_record(&self) -> Result<NdefRecord> {
        let payload = UriRecord::encode_payload(&self.uri)?;
        NdefRecord::media(MEDIA_TYPE, payload)
    }

    /// Decode a URI-record carrier.
    pub fn from_uri_record(record: &NdefRecord) -> Result<Self> {
        let uri = UriRecord::from_record(record)?.uri;
        Self::new(uri)
    }

    /// Decode a Wi-Fi DPP carrier configuration record.
    pub fn from_media_record(record: &NdefRecord) -> Result<Self> {
        if record.tnf() != Tnf::Media || record.record_type() != MEDIA_TYPE {
            return Err(NfcError::InvalidArgument(
                "record is not an application/vnd.wfa.dpp media record".to_owned(),
            ));
        }
        Self::new(UriRecord::decode_payload(record.payload())?)
    }
}

fn validate_uri(uri: &str) -> Result<()> {
    if !uri.starts_with("DPP:") {
        return Err(NfcError::InvalidArgument(
            "DPP bootstrapping URI must start with \"DPP:\"".to_owned(),
        ));
    }
    if !uri.ends_with(";;") {
        return Err(NfcError::InvalidArgument(
            "DPP bootstrapping URI must end with \";;\"".to_owned(),
        ));
    }
    if uri.len() <= 5 {
        return Err(NfcError::InvalidArgument(
            "DPP bootstrapping URI has no body".to_owned(),
        ));
    }
    if !uri.contains(";K:") {
        return Err(NfcError::InvalidArgument(
            "DPP bootstrapping URI is missing the public key field".to_owned(),
        ));
    }
    if uri.bytes().any(|byte| !(0x20..=0x7E).contains(&byte)) {
        return Err(NfcError::InvalidArgument(
            "DPP bootstrapping URI must contain printable ASCII only".to_owned(),
        ));
    }
    Ok(())
}
