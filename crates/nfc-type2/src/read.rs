//! Reader-backed Type 2 memory reads.

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result};
use nfc_transport::Reader;

use crate::cc::CapabilityContainer;
use crate::layout::Type2Layout;
use crate::page::Type2Image;

/// A partial or complete Type 2 read with explicit warnings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type2ReadResult {
    pub image: Type2Image,
    pub warnings: Vec<String>,
}

/// Read the complete Type 2 memory area. The first four pages are mandatory
/// because they carry the capability container; later failures stay explicit.
pub fn read_tag(
    reader: &mut dyn Reader,
    card: &CardInfo,
    cancel: &CancelToken,
) -> Result<Type2ReadResult> {
    cancel.check()?;
    if card.card_type != CardType::Type2 {
        return Err(NfcError::Unsupported(
            "the selected card is not an NFC Forum Type 2 tag",
        ));
    }
    let first = reader.read_page_quad(0)?;
    let cc = CapabilityContainer::from_page(page_from_quad(&first, 3))?;
    let layout = Type2Layout::from_cc(&cc)?;
    let mut image = Type2Image::new(layout, cc, card.uid.clone());
    image.set_page(0, page_from_quad(&first, 0))?;
    image.set_page(1, page_from_quad(&first, 1))?;
    image.set_page(2, page_from_quad(&first, 2))?;
    image.set_page(3, page_from_quad(&first, 3))?;

    let mut warnings = Vec::new();
    for start in quad_starts(layout.total_pages) {
        if start == 0 {
            continue;
        }
        cancel.check()?;
        match reader.read_page_quad(start) {
            Ok(data) => {
                for offset in 0..4u8 {
                    let page = start.saturating_add(offset);
                    if layout.contains(page) {
                        image.set_page(page, page_from_quad(&data, offset))?;
                    }
                }
            }
            Err(error) => {
                warnings.push(format!(
                    "pages {start}..={}: {error}",
                    start.saturating_add(3).min(layout.total_pages - 1)
                ));
            }
        }
    }

    if image.known_pages() == 0 {
        return Err(NfcError::Io(
            "no Type 2 page could be read from the tag".to_owned(),
        ));
    }
    Ok(Type2ReadResult { image, warnings })
}

/// Read one page through a single READ command. The returned quad is aligned
/// down to a multiple of four.
pub fn read_page(reader: &mut dyn Reader, page: u8) -> Result<[u8; PAGE_SIZE]> {
    let start = page & !0x03;
    let quad = reader.read_page_quad(start)?;
    Ok(page_from_quad(&quad, page - start))
}

/// All start pages needed to cover `total_pages`, using overlapping final
/// quads because a Type 2 READ always returns four pages.
pub fn quad_starts(total_pages: u8) -> impl Iterator<Item = u8> {
    let mut next = Some(0u8);
    std::iter::from_fn(move || {
        let start = next?;
        let following = start.saturating_add(4);
        next = if following.saturating_add(4) <= total_pages {
            Some(following)
        } else if following < total_pages {
            Some(total_pages.saturating_sub(4))
        } else {
            None
        };
        Some(start)
    })
}

fn page_from_quad(quad: &[u8; PAGE_QUAD_SIZE], offset: u8) -> [u8; PAGE_SIZE] {
    let index = usize::from(offset) * PAGE_SIZE;
    let mut page = [0u8; PAGE_SIZE];
    page.copy_from_slice(&quad[index..index + PAGE_SIZE]);
    page
}

#[cfg(test)]
mod tests {
    use nfc_core::{CardInfo, CardType};
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::MockReader;

    use super::*;

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )
    }

    fn reader_with_ntag213() -> MockReader {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:ntag"), Some(card()));
        for page in 0..45u8 {
            reader.set_page(page, [page; 4]);
        }
        reader.set_page(3, [0xE1, 0x10, 0x12, 0x00]);
        reader
    }

    #[test]
    fn reads_every_page_of_an_ntag213() {
        let mut reader = reader_with_ntag213();
        let result = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        assert!(result.image.is_complete());
        assert_eq!(result.image.layout.total_pages, 45);
        assert_eq!(result.image.page(44), Some(&[44; 4]));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn failed_quads_stay_unknown_with_a_warning() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:ntag"), Some(card()));
        reader.set_page(0, [0; 4]);
        reader.set_page(1, [0; 4]);
        reader.set_page(2, [0; 4]);
        reader.set_page(3, [0xE1, 0x10, 0x12, 0x00]);
        let result = read_tag(&mut reader, &card(), &CancelToken::new()).expect("read");
        assert_eq!(result.image.known_pages(), 4);
        assert!(!result.image.is_complete());
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn quads_cover_the_final_partial_group() {
        assert_eq!(quad_starts(16).collect::<Vec<_>>(), vec![0, 4, 8, 12]);
        assert_eq!(
            quad_starts(45).collect::<Vec<_>>(),
            vec![0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 41]
        );
    }

    #[test]
    fn non_type2_card_is_rejected() {
        let mut reader = reader_with_ntag213();
        let classic = CardInfo::new(vec![1; 4], [0, 4], 0x08, CardType::MifareClassic1K);
        assert!(read_tag(&mut reader, &classic, &CancelToken::new()).is_err());
    }
}
