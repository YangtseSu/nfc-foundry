//! File control information and PPSE directory parsing.
//!
//! The parser is deliberately tolerant about which template carries a field:
//! cards move the application label, priority, and version between the
//! proprietary template and the issuer discretionary data. Fields are collected
//! from the whole FCI and the first occurrence wins.

use nfc_core::{NfcError, Result};

use crate::tlv::{self, Tlv};

/// One application entry from the PPSE directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub aid: Vec<u8>,
    pub label: Option<String>,
    pub priority: Option<u8>,
}

/// Fields from one application's FCI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub df_name: Vec<u8>,
    pub label: Option<String>,
    pub priority: Option<u8>,
    /// Application version from `9F08`; the T-Union profile uses one byte.
    pub version: Option<Vec<u8>>,
    /// Application metadata from `9F0C`.
    pub meta: Option<Vec<u8>>,
}

/// Parse an application FCI that starts with the `6F` template.
pub fn parse_application(fci: &[u8]) -> Result<Application> {
    let outer = tlv::parse(fci)?;
    let template = tlv::find(&outer, 0x6F)
        .ok_or_else(|| NfcError::Io("the application FCI has no 6F template".to_owned()))?;
    let (fields, lossy_error) = tlv::collect_lossy(template.value);
    let df_name = first(&fields, 0x84).ok_or_else(|| {
        lossy_error
            .unwrap_or_else(|| NfcError::Io("the application FCI has no 84 DF name".to_owned()))
    })?;
    let label = first(&fields, 0x50).and_then(|value| String::from_utf8(value).ok());
    let priority = first(&fields, 0x87).and_then(|value| value.first().copied());
    let version = first(&fields, 0x9F08);
    let meta = first(&fields, 0x9F0C);
    Ok(Application {
        df_name,
        label,
        priority,
        version,
        meta,
    })
}

/// Parse a PPSE FCI into the advertised application entries.
///
/// Entries are keyed by `4F`; `50` and `87` attach to the most recent `4F` in
/// depth-first order. A directory with no entries returns an empty list.
pub fn parse_ppse(fci: &[u8]) -> Result<Vec<DirectoryEntry>> {
    let outer = tlv::parse(fci)?;
    let template = tlv::find(&outer, 0x6F)
        .ok_or_else(|| NfcError::Io("the PPSE FCI has no 6F template".to_owned()))?;
    let (fields, lossy_error) = tlv::collect_lossy(template.value);
    let mut entries: Vec<DirectoryEntry> = Vec::new();
    for (tag, value) in fields {
        match tag {
            0x4F => entries.push(DirectoryEntry {
                aid: value,
                label: None,
                priority: None,
            }),
            0x50 => {
                if let Some(entry) = entries.last_mut() {
                    entry.label = String::from_utf8(value).ok();
                }
            }
            0x87 => {
                if let Some(entry) = entries.last_mut() {
                    entry.priority = value.first().copied();
                }
            }
            _ => {}
        }
    }
    if entries.is_empty()
        && let Some(error) = lossy_error
    {
        return Err(error);
    }
    Ok(entries)
}

/// Check whether a directory contains an AID.
pub fn contains_aid(entries: &[DirectoryEntry], aid: &[u8]) -> bool {
    entries.iter().any(|entry| entry.aid == aid)
}

fn first(fields: &[(u32, Vec<u8>)], tag: u32) -> Option<Vec<u8>> {
    fields
        .iter()
        .find(|(field_tag, _)| *field_tag == tag)
        .map(|(_, value)| value.clone())
}

/// Helper for tests and callers that already have parsed TLV objects.
pub fn find_tlv<'a, 'b>(objects: &'b [Tlv<'a>], tag: u32) -> Option<&'b Tlv<'a>> {
    tlv::find(objects, tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPSE: &[u8] = &[
        0x6F, 0x49, 0x84, 0x0E, b'2', b'P', b'A', b'Y', b'.', b'S', b'Y', b'S', b'.', b'D', b'D',
        b'F', b'0', b'1', 0xA5, 0x37, 0xBF, 0x0C, 0x34, 0x61, 0x19, 0x4F, 0x08, 0xA0, 0x00, 0x00,
        0x06, 0x32, 0x01, 0x01, 0x06, 0x50, 0x0A, b'M', b'O', b'T', b'_', b'T', b'_', b'C', b'A',
        b'S', b'H', 0x87, 0x01, 0x01, 0x61, 0x17, 0x4F, 0x08, 0xA0, 0x00, 0x00, 0x06, 0x32, 0x01,
        0x01, 0x05, 0x50, 0x08, b'M', b'O', b'T', b'_', b'T', b'_', b'E', b'P', 0x87, 0x01, 0x02,
    ];

    #[test]
    fn parses_ppse_entries_with_labels_and_priority() {
        let entries = parse_ppse(PPSE).expect("PPSE");
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].aid,
            vec![0xA0, 0x00, 0x00, 0x06, 0x32, 0x01, 0x01, 0x06]
        );
        assert_eq!(entries[0].label.as_deref(), Some("MOT_T_CASH"));
        assert_eq!(entries[0].priority, Some(1));
        assert_eq!(
            entries[1].aid,
            vec![0xA0, 0x00, 0x00, 0x06, 0x32, 0x01, 0x01, 0x05]
        );
        assert_eq!(entries[1].label.as_deref(), Some("MOT_T_EP"));
        assert_eq!(entries[1].priority, Some(2));
        assert!(contains_aid(&entries, &entries[1].aid));
    }

    #[test]
    fn parses_application_fci() {
        let fci = [
            0x6F, 0x31, 0x84, 0x08, 0xA0, 0x00, 0x00, 0x06, 0x32, 0x01, 0x01, 0x05, 0xA5, 0x25,
            0x9F, 0x08, 0x01, 0x02, 0x9F, 0x0C, 0x1E, 0x01, 0x00, 0x00, 0x10, 0xFF, 0xFF, 0xFF,
            0xFF, 0x02, 0x01, 0x03, 0x10, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x20,
            0x22, 0x01, 0x02, 0x20, 0x72, 0x01, 0x02, 0x50, 0x00,
        ];
        let application = parse_application(&fci).expect("FCI");
        assert_eq!(
            application.df_name,
            vec![0xA0, 0x00, 0x00, 0x06, 0x32, 0x01, 0x01, 0x05]
        );
        assert_eq!(application.version.as_deref(), Some([0x02].as_slice()));
        assert_eq!(application.meta.as_ref().map(Vec::len), Some(30));
    }

    #[test]
    fn rejects_fci_without_template_or_df_name() {
        assert!(parse_application(&[0x84, 0x01, 0x00]).is_err());
        assert!(parse_application(&[0x6F, 0x03, 0x84, 0x01, 0x00]).is_ok());
        assert!(parse_application(&[0x6F, 0x03, 0x87, 0x01, 0x01]).is_err());
    }
}
