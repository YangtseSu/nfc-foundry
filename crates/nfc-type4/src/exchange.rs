//! Type 4 project JSON for backups and inspection.

use nfc_core::{CardInfo, NfcError, Result};
use serde::{Deserialize, Serialize};

use crate::read::Type4ReadResult;

const FORMAT: &str = "nfc-foundry-type4";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    format: String,
    version: u32,
    uid: String,
    cc: String,
    ndef: String,
}

/// Encode the CC and the current NDEF file bytes as project JSON. The NDEF
/// field starts with the two-byte NLEN value.
pub fn export(card: &CardInfo, read: &Type4ReadResult) -> Result<Vec<u8>> {
    let file = ProjectFile {
        format: FORMAT.to_owned(),
        version: VERSION,
        uid: card.uid_hex(),
        cc: hex(read.cc.raw()),
        ndef: hex(read.ndef.file_bytes()),
    };
    serde_json::to_vec_pretty(&file)
        .map_err(|error| NfcError::Io(format!("could not encode the Type 4 project: {error}")))
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::{MockReader, MockType4Tag};

    use crate::read::read_tag;

    use super::*;

    #[test]
    fn exports_cc_and_ndef_bytes() {
        let card = CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            [0x00, 0x44],
            0x20,
            nfc_core::CardType::Iso14443,
        );
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:type4"), Some(card.clone()));
        reader.set_type4_tag(MockType4Tag::ntag424());
        let read = read_tag(&mut reader, &card, &nfc_core::CancelToken::new()).expect("read");
        let bytes = export(&card, &read).expect("export");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(value["format"], FORMAT);
        assert_eq!(value["version"], VERSION);
        assert_eq!(value["uid"], "04 11 22 33 44 55 66");
        assert_eq!(
            value["cc"].as_str().expect("cc").len(),
            read.cc.raw().len() * 2
        );
        assert_eq!(value["ndef"], "0000");
    }
}
