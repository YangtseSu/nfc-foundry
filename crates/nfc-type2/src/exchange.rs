//! Project JSON for partial and complete Type 2 images.

use nfc_core::{NfcError, PAGE_SIZE, Result};
use serde::{Deserialize, Serialize};

use crate::cc::CapabilityContainer;
use crate::layout::{PageRole, Type2Layout};
use crate::page::Type2Image;

const FORMAT: &str = "nfc-foundry-type2";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DumpFile {
    format: String,
    version: u32,
    uid: String,
    cc: String,
    pages: Vec<Option<String>>,
}

/// Encode a Type 2 image as project JSON. Unknown pages are `null`.
pub fn export(image: &Type2Image) -> Result<Vec<u8>> {
    let file = DumpFile {
        format: FORMAT.to_owned(),
        version: VERSION,
        uid: image.uid_hex(),
        cc: hex(&image.cc.to_bytes()),
        pages: image
            .pages()
            .map(|(page, data)| data.map(|data| hex(&masked_page(&image.layout, page, *data))))
            .collect(),
    };
    serde_json::to_vec_pretty(&file)
        .map_err(|error| NfcError::Io(format!("could not encode the Type 2 dump: {error}")))
}

/// Decode and validate a project JSON Type 2 image.
pub fn load(bytes: &[u8]) -> Result<Type2Image> {
    let file: DumpFile = serde_json::from_slice(bytes)
        .map_err(|error| NfcError::Io(format!("could not parse the Type 2 dump: {error}")))?;
    if file.format != FORMAT {
        return Err(NfcError::InvalidArgument(format!(
            "unsupported Type 2 dump format {:?}",
            file.format
        )));
    }
    if file.version != VERSION {
        return Err(NfcError::InvalidArgument(format!(
            "unsupported Type 2 dump version {}",
            file.version
        )));
    }
    let cc_bytes = parse_hex(&file.cc, 4)?;
    let cc = CapabilityContainer::parse(&cc_bytes)?;
    let layout = Type2Layout::from_cc(&cc)?;
    if file.pages.len() != layout.page_count() {
        return Err(NfcError::InvalidArgument(format!(
            "the Type 2 dump has {} page slots but the {} layout needs {}",
            file.pages.len(),
            layout.product.name(),
            layout.page_count()
        )));
    }
    let mut pages = Vec::with_capacity(file.pages.len());
    for (index, value) in file.pages.iter().enumerate() {
        let page = match value {
            Some(value) => {
                let data = parse_hex(value, PAGE_SIZE)?;
                let mut page = [0u8; PAGE_SIZE];
                page.copy_from_slice(&data);
                Some(masked_page(&layout, index as u8, page))
            }
            None => None,
        };
        pages.push(page);
    }
    let uid = parse_uid(&file.uid)?;
    let image = Type2Image::from_pages(layout, cc, uid, pages)?;
    if let Some(page) = image.page(3)
        && page != &cc.to_bytes()
    {
        return Err(NfcError::InvalidArgument(
            "the Type 2 dump capability container does not match page 3".to_owned(),
        ));
    }
    Ok(image)
}

impl Type2Image {
    pub fn uid_hex(&self) -> String {
        self.uid
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn masked_page(layout: &Type2Layout, page: u8, data: [u8; PAGE_SIZE]) -> [u8; PAGE_SIZE] {
    if matches!(
        layout.role(page),
        PageRole::Password | PageRole::PasswordAck
    ) {
        [0; PAGE_SIZE]
    } else {
        data
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

fn parse_hex(value: &str, expected: usize) -> Result<Vec<u8>> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    if compact.len() != expected * 2 {
        return Err(NfcError::InvalidArgument(format!(
            "expected {} hexadecimal digits, got {}",
            expected * 2,
            compact.len()
        )));
    }
    let mut bytes = Vec::with_capacity(expected);
    for index in 0..expected {
        let high = nibble(compact.as_bytes()[index * 2])?;
        let low = nibble(compact.as_bytes()[index * 2 + 1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn parse_uid(value: &str) -> Result<Vec<u8>> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    if compact.is_empty() || !compact.len().is_multiple_of(2) {
        return Err(NfcError::InvalidArgument(
            "the Type 2 dump UID is not a whole number of bytes".to_owned(),
        ));
    }
    let bytes = parse_hex(&compact, compact.len() / 2)?;
    if !matches!(bytes.len(), 4 | 7 | 10) {
        return Err(NfcError::InvalidArgument(format!(
            "the Type 2 dump UID is {} bytes, expected 4, 7, or 10",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn nibble(character: u8) -> Result<u8> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(NfcError::InvalidArgument(
            "the Type 2 dump contains a non-hexadecimal character".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::cc::CapabilityContainer;
    use crate::layout::Type2Layout;

    use super::*;

    fn image() -> Type2Image {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0x00]).expect("CC");
        let layout = Type2Layout::from_cc(&cc).expect("layout");
        let mut image = Type2Image::new(layout, cc, vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80]);
        image.set_page(3, [0xE1, 0x10, 0x12, 0x00]).expect("CC");
        image.set_page(4, [1, 2, 3, 4]).expect("page 4");
        image
    }

    #[test]
    fn round_trips_partial_images() {
        let source = image();
        let bytes = export(&source).expect("export");
        let loaded = load(&bytes).expect("load");
        assert_eq!(loaded, source);
        assert!(loaded.page(5).is_none());
    }

    #[test]
    fn password_pages_are_masked_in_dumps_and_loads() {
        let mut source = image();
        source
            .set_page(43, [0xDE, 0xAD, 0xBE, 0xEF])
            .expect("PWD page");
        source
            .set_page(44, [0xCA, 0xFE, 0x00, 0x00])
            .expect("PACK page");
        let bytes = export(&source).expect("export");
        let text = String::from_utf8(bytes.clone()).expect("utf8");
        assert!(!text.contains("DEADBEEF"), "{text}");
        assert!(!text.contains("CAFE"), "{text}");
        let loaded = load(&bytes).expect("load");
        assert_eq!(loaded.page(43), Some(&[0; 4]));
        assert_eq!(loaded.page(44), Some(&[0; 4]));
    }

    #[test]
    fn rejects_bad_versions_and_page_counts() {
        let source = image();
        let text = String::from_utf8(export(&source).expect("export")).expect("utf8");
        let bad_version = text.replace("\"version\": 1", "\"version\": 2");
        assert!(load(bad_version.as_bytes()).is_err());
        let bad_pages = text.replace("\"pages\": [", "\"pages\": [null, ");
        assert!(load(bad_pages.as_bytes()).is_err());
        let bad_cc = text.replace("E1101200", "E1103E00");
        assert!(load(bad_cc.as_bytes()).is_err());
    }

    #[test]
    fn rejects_a_disagreeing_page_three() {
        let source = image();
        let text = String::from_utf8(export(&source).expect("export")).expect("utf8");
        let pages = text.find("\"pages\"").expect("pages field");
        let (head, tail) = text.split_at(pages);
        let tail = tail.replacen("\"E1101200\"", "\"E1101201\"", 1);
        let bad_page = format!("{head}{tail}");
        assert!(load(bad_page.as_bytes()).is_err());
    }
}
