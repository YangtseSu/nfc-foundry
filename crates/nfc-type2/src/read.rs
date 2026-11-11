//! Reader-backed Type 2 memory reads.

use nfc_core::{CancelToken, CardInfo, CardType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result};
use nfc_transport::Reader;

use crate::auth::Type2AuthSession;
use crate::cc::CapabilityContainer;
use crate::config::Configuration;
use crate::layout::{PageRole, Type2Layout};
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
///
/// The configuration pages are read first. If they are protected, or if the
/// protection covers user pages, `access` authenticates once before the first
/// protected read. A protected tag without credentials is reported as
/// `AuthenticationRequired` instead of a partial image.
pub fn read_tag(
    reader: &mut dyn Reader,
    card: &CardInfo,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<Type2ReadResult> {
    cancel.check()?;
    if card.card_type != CardType::Type2 {
        return Err(NfcError::Unsupported(
            "the selected card is not an NFC Forum Type 2 tag",
        ));
    }
    // Authenticate before the first READ when credentials are supplied. A
    // protected READ that is refused can leave the tag ignoring PWD_AUTH until
    // the next field cycle on some readers, so probing first is not safe.
    if access.has_credentials() {
        access.authenticate(reader, cancel)?;
    }
    let first = reader.read_page_quad(0)?;
    let cc = CapabilityContainer::from_page(page_from_quad(&first, 3))?;
    let layout = Type2Layout::from_cc(&cc)?;
    let mut image = Type2Image::new(layout, cc, card.uid.clone());
    image.set_page(0, page_from_quad(&first, 0))?;
    image.set_page(1, page_from_quad(&first, 1))?;
    image.set_page(2, page_from_quad(&first, 2))?;
    image.set_page(3, page_from_quad(&first, 3))?;

    let config = if layout.config.is_some() {
        Some(read_configuration(reader, &mut image, access, cancel)?)
    } else {
        None
    };
    if let Some(config) = config
        && config.read_and_write_protected()
        && let Some(first) = config
            .protected_from()
            .filter(|first| *first < layout.total_pages)
        && first <= layout.user_last
    {
        if !access.has_credentials() {
            return Err(protected_tag_error(first));
        }
        access.authenticate(reader, cancel)?;
    }

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
                        store_page(&mut image, page, page_from_quad(&data, offset))?;
                    }
                }
            }
            Err(error) => {
                if !access.has_credentials() && protected_read_page(&config, start, &layout) {
                    return Err(protected_tag_error(
                        config
                            .and_then(Configuration::protected_from)
                            .unwrap_or(start),
                    ));
                }
                if !access.has_credentials() && matches!(error, NfcError::AuthenticationFailed) {
                    return Err(NfcError::AuthenticationRequired(
                        "the tag rejected an unauthenticated read; supply the Type 2 password and PACK"
                            .to_owned(),
                    ));
                }
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

/// Read the NTAG configuration pages before the main image pass.
///
/// A failed configuration read without credentials is reported as a protected
/// tag: the page cannot be distinguished from an authentication failure, and a
/// partial image must not claim the protection state is known.
fn read_configuration(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<Configuration> {
    let pages = image.layout.config.ok_or(NfcError::Unsupported(
        "this Type 2 tag has no configuration pages",
    ))?;
    for page in [pages.cfg0, pages.cfg1] {
        cancel.check()?;
        let start = safe_quad_start(page, image.layout.total_pages);
        let quad = match reader.read_page_quad(start) {
            Ok(quad) => quad,
            Err(error) => {
                if !access.has_credentials() {
                    return Err(NfcError::AuthenticationRequired(format!(
                        "the NTAG configuration could not be read ({error}); supply the Type 2 password and PACK if the tag is protected"
                    )));
                }
                access.authenticate(reader, cancel)?;
                reader.read_page_quad(start)?
            }
        };
        image.set_page(page, page_from_quad(&quad, page - start))?;
    }
    Configuration::from_image(image)
}

/// Read one page through a single READ command. The returned quad is aligned
/// down to a multiple of four.
pub fn read_page(reader: &mut dyn Reader, page: u8) -> Result<[u8; PAGE_SIZE]> {
    let start = page & !0x03;
    let quad = reader.read_page_quad(start)?;
    Ok(page_from_quad(&quad, page - start))
}

/// Read one page with a final overlapping quad so a READ never crosses the
/// end of the tag.
pub(crate) fn read_page_bounded(
    reader: &mut dyn Reader,
    page: u8,
    total_pages: u8,
) -> Result<[u8; PAGE_SIZE]> {
    let start = safe_quad_start(page, total_pages);
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

fn safe_quad_start(page: u8, total_pages: u8) -> u8 {
    let start = page & !0x03;
    if start.saturating_add(4) <= total_pages {
        start
    } else {
        total_pages.saturating_sub(4)
    }
}

fn protected_read_page(config: &Option<Configuration>, start: u8, layout: &Type2Layout) -> bool {
    config.as_ref().is_some_and(|config| {
        (start..start.saturating_add(4))
            .any(|page| layout.contains(page) && config.requires_password_for_read(page))
    })
}

fn protected_tag_error(first: u8) -> NfcError {
    NfcError::AuthenticationRequired(format!(
        "the tag is password protected from page {first}; supply the Type 2 password and PACK"
    ))
}

fn store_page(image: &mut Type2Image, page: u8, data: [u8; PAGE_SIZE]) -> Result<()> {
    let data = match image.layout.role(page) {
        // PWD and PACK are write-only on NTAG21x. Discard anything a reader
        // returns so the secret cannot enter the image, a dump, or the GUI.
        PageRole::Password | PageRole::PasswordAck => [0; PAGE_SIZE],
        _ => data,
    };
    image.set_page(page, data)
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

    fn protected_reader() -> MockReader {
        let mut reader = reader_with_ntag213();
        reader.set_password([1, 2, 3, 4], [0xAA, 0x55]);
        reader.set_page(41, [0x00, 0x00, 0x00, 0x20]);
        reader.set_page(42, [0x80, 0x00, 0x00, 0x00]);
        reader
    }

    #[test]
    fn reads_every_page_of_an_ntag213() {
        let mut reader = reader_with_ntag213();
        let mut access = Type2AuthSession::none();
        let result =
            read_tag(&mut reader, &card(), &mut access, &CancelToken::new()).expect("read");
        assert!(result.image.is_complete());
        assert_eq!(result.image.layout.total_pages, 45);
        assert_eq!(result.image.page(44), Some(&[0; 4]));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn failed_quads_stay_unknown_with_a_warning() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:ntag"), Some(card()));
        reader.set_page(0, [0; 4]);
        reader.set_page(1, [0; 4]);
        reader.set_page(2, [0; 4]);
        reader.set_page(3, [0xE1, 0x10, 0x12, 0x00]);
        for page in 40..45u8 {
            reader.set_page(page, [0; 4]);
        }
        let mut access = Type2AuthSession::none();
        let result =
            read_tag(&mut reader, &card(), &mut access, &CancelToken::new()).expect("read");
        assert_eq!(result.image.known_pages(), 9);
        assert!(!result.image.is_complete());
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn protected_tags_require_credentials_and_authenticate_once() {
        let mut reader = protected_reader();
        let mut access = Type2AuthSession::none();
        let error = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect_err("protected tag");
        assert!(matches!(error, NfcError::AuthenticationRequired(_)));
        assert_eq!(reader.password_auths(), 0);

        let auth = crate::Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let result = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read");
        assert!(result.image.is_complete());
        assert_eq!(reader.password_auths(), 1);
        assert_eq!(result.image.page(32), Some(&[32; 4]));
    }

    #[test]
    fn wrong_credentials_fail_before_the_image_is_returned() {
        let mut reader = protected_reader();
        let auth = crate::Type2Auth::new([9, 9, 9, 9], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        assert!(matches!(
            read_tag(&mut reader, &card(), &mut access, &CancelToken::new()),
            Err(NfcError::AuthenticationFailed)
        ));
    }

    #[test]
    fn password_pages_are_masked_in_the_image() {
        let mut reader = reader_with_ntag213();
        reader.set_page(43, [0xDE, 0xAD, 0xBE, 0xEF]);
        reader.set_page(44, [0xCA, 0xFE, 0x00, 0x00]);
        let mut access = Type2AuthSession::none();
        let result =
            read_tag(&mut reader, &card(), &mut access, &CancelToken::new()).expect("read");
        assert_eq!(result.image.page(43), Some(&[0; 4]));
        assert_eq!(result.image.page(44), Some(&[0; 4]));
    }

    #[test]
    fn quads_cover_the_final_partial_group() {
        assert_eq!(quad_starts(16).collect::<Vec<_>>(), vec![0, 4, 8, 12]);
        assert_eq!(
            quad_starts(45).collect::<Vec<_>>(),
            vec![0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 41]
        );
        assert_eq!(safe_quad_start(132, 135), 131);
        assert_eq!(safe_quad_start(131, 135), 128);
    }

    #[test]
    fn non_type2_card_is_rejected() {
        let mut reader = reader_with_ntag213();
        let classic = CardInfo::new(vec![1; 4], [0, 4], 0x08, CardType::MifareClassic1K);
        let mut access = Type2AuthSession::none();
        assert!(read_tag(&mut reader, &classic, &mut access, &CancelToken::new()).is_err());
    }
}
