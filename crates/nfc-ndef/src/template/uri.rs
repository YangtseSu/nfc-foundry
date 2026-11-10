//! NFC Forum URI RTD (`U`).

use nfc_core::{NfcError, Result};

use crate::record::{NdefRecord, Tnf};

const TYPE: &[u8] = b"U";

/// NFC Forum URI identifier prefixes, indexed by the payload code octet.
pub const URI_PREFIXES: [&str; 36] = [
    "",
    "http://www.",
    "https://www.",
    "http://",
    "https://",
    "tel:",
    "mailto:",
    "ftp://anonymous:anonymous@",
    "ftp://ftp.",
    "ftps://",
    "sftp://",
    "smb://",
    "nfs://",
    "ftp://",
    "dav://",
    "news:",
    "telnet://",
    "imap:",
    "rtsp://",
    "urn:",
    "pop:",
    "sip:",
    "sips:",
    "tftp:",
    "btspp://",
    "btl2cap://",
    "btgoep://",
    "tcpobex://",
    "irdaobex://",
    "file://",
    "urn:epc:id:",
    "urn:epc:tag:",
    "urn:epc:pat:",
    "urn:epc:raw:",
    "urn:epc:",
    "urn:nfc:",
];

/// A decoded NFC Forum URI record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UriRecord {
    /// The fully expanded internationalized resource identifier.
    pub uri: String,
}

impl UriRecord {
    /// Create a URI record. The URI must not be empty and must not contain
    /// control characters.
    pub fn new(uri: impl Into<String>) -> Result<Self> {
        let uri = uri.into();
        validate_uri(&uri)?;
        Ok(Self { uri })
    }

    /// Encode the URI record.
    pub fn to_record(&self) -> Result<NdefRecord> {
        NdefRecord::well_known(TYPE, Self::encode_payload(&self.uri)?)
    }

    /// Decode a URI record.
    pub fn from_record(record: &NdefRecord) -> Result<Self> {
        if record.tnf() != Tnf::WellKnown || record.record_type() != TYPE {
            return Err(NfcError::InvalidArgument(
                "record is not an NFC Forum URI record".to_owned(),
            ));
        }
        Ok(Self {
            uri: Self::decode_payload(record.payload())?,
        })
    }

    /// Encode the URI RTD payload. The DPP carrier record reuses this payload.
    pub(crate) fn encode_payload(uri: &str) -> Result<Vec<u8>> {
        validate_uri(uri)?;
        let mut best: Option<(usize, &str)> = None;
        for (index, prefix) in URI_PREFIXES.iter().enumerate().skip(1) {
            if uri.starts_with(prefix)
                && best.is_none_or(|(_, current)| prefix.len() > current.len())
            {
                best = Some((index, prefix));
            }
        }
        let (code, rest) = match best {
            Some((index, prefix)) => (index as u8, &uri[prefix.len()..]),
            None => (0, uri),
        };
        let mut payload = Vec::with_capacity(1 + rest.len());
        payload.push(code);
        payload.extend_from_slice(rest.as_bytes());
        Ok(payload)
    }

    /// Decode the URI RTD payload.
    pub(crate) fn decode_payload(payload: &[u8]) -> Result<String> {
        let code = *payload.first().ok_or_else(|| {
            NfcError::InvalidArgument(
                "URI record payload is missing the identifier code".to_owned(),
            )
        })?;
        let prefix = URI_PREFIXES.get(usize::from(code)).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "URI record identifier code 0x{code:02X} is reserved"
            ))
        })?;
        let data = std::str::from_utf8(&payload[1..]).map_err(|_| {
            NfcError::InvalidArgument("URI record payload is not valid UTF-8".to_owned())
        })?;
        if data.bytes().any(|byte| byte < 0x20 || byte == 0x7F) {
            return Err(NfcError::InvalidArgument(
                "URI record payload contains a control character".to_owned(),
            ));
        }
        let uri = format!("{prefix}{data}");
        validate_uri(&uri)?;
        Ok(uri)
    }
}

fn validate_uri(uri: &str) -> Result<()> {
    if uri.is_empty() {
        return Err(NfcError::InvalidArgument(
            "URI record content must not be empty".to_owned(),
        ));
    }
    if uri.bytes().any(|byte| byte < 0x20 || byte == 0x7F) {
        return Err(NfcError::InvalidArgument(
            "URI record content contains a control character".to_owned(),
        ));
    }
    Ok(())
}
