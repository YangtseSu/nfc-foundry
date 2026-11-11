//! Verified Type 2 page, NDEF, lock, and configuration writes.

use nfc_core::{CancelToken, NfcError, PAGE_SIZE, Result};
use nfc_ndef::NdefMessage;
use nfc_transport::Reader;

use crate::auth::{Type2Auth, Type2AuthSession};
use crate::config::{Configuration, PasswordUpdate};
use crate::layout::{PageRole, Type2Layout};
use crate::lock::{LockBits, LockUpdate};
use crate::ndef::{encode_ndef_area, format_ndef_area};
use crate::page::Type2Image;

/// Which page classes a write may touch. Defaults allow user memory and the
/// NDEF data area only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WritePolicy {
    pub user_memory: bool,
    pub capability_container: bool,
    pub lock_bytes: bool,
    pub configuration: bool,
}

impl WritePolicy {
    pub const fn user() -> Self {
        Self {
            user_memory: true,
            capability_container: false,
            lock_bytes: false,
            configuration: false,
        }
    }

    pub const fn advanced() -> Self {
        Self {
            user_memory: true,
            capability_container: true,
            lock_bytes: true,
            configuration: true,
        }
    }
}

/// Result of one verified page write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageWriteResult {
    pub page: u8,
    pub previous: [u8; PAGE_SIZE],
    pub written: [u8; PAGE_SIZE],
    pub changed: bool,
}

/// Result of a multi-page write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteSummary {
    pub written_pages: usize,
    pub unchanged_pages: usize,
}

/// Validate that `page` may be written under `policy` and the current locks.
pub fn check_writable(image: &Type2Image, page: u8, policy: WritePolicy) -> Result<PageRole> {
    if !image.layout.contains(page) {
        return Err(NfcError::InvalidArgument(format!(
            "page {page} is outside the tag"
        )));
    }
    let role = image.layout.role(page);
    let allowed = match role {
        PageRole::UserMemory => policy.user_memory,
        PageRole::CapabilityContainer => policy.capability_container,
        PageRole::StaticLock | PageRole::DynamicLock => policy.lock_bytes,
        PageRole::Configuration | PageRole::Password | PageRole::PasswordAck => {
            policy.configuration
        }
        PageRole::Manufacturer | PageRole::Reserved => false,
    };
    if !allowed {
        return Err(NfcError::Unsupported(match role {
            PageRole::Manufacturer => "manufacturer and UID pages are read-only",
            PageRole::Reserved => "the page has no writable Type 2 role",
            _ => "this page class requires an Advanced write policy",
        }));
    }
    if role == PageRole::CapabilityContainer && !image.cc.writable() {
        return Err(NfcError::Unsupported(
            "the capability container write access forbids data-area writes",
        ));
    }
    if matches!(role, PageRole::UserMemory | PageRole::CapabilityContainer) {
        let locks = LockBits::from_image(image).unwrap_or_default();
        if locks.is_locked(&image.layout, page) {
            return Err(NfcError::Unsupported("the page is locked read-only"));
        }
    }
    if role == PageRole::Configuration {
        let config = Configuration::from_image(image)?;
        if config.cfg_locked() {
            return Err(NfcError::Unsupported(
                "the NTAG configuration is permanently locked",
            ));
        }
    }
    Ok(role)
}

/// Authenticate before a multi-page mutation when any target page needs a
/// password, so a wrong credential cannot leave an earlier page written.
fn authenticate_for_pages(
    reader: &mut dyn Reader,
    image: &Type2Image,
    pages: impl IntoIterator<Item = u8>,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<()> {
    let Some(config) = Configuration::from_image(image).ok() else {
        return Ok(());
    };
    if pages.into_iter().any(|page| config.requires_password(page)) {
        access.authenticate(reader, cancel)?;
    }
    Ok(())
}

/// Write one page, read it back, compare the expected post-write value, and
/// update the in-memory image. A failed read-back is an error; a write that
/// was ignored is detected here.
pub fn write_page_verified(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    page: u8,
    data: [u8; PAGE_SIZE],
    policy: WritePolicy,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<PageWriteResult> {
    cancel.check()?;
    let role = check_writable(image, page, policy)?;
    if Configuration::from_image(image).is_ok_and(|config| config.requires_password(page)) {
        access.authenticate(reader, cancel)?;
    }
    let previous = *image.page(page).ok_or_else(|| {
        NfcError::VerificationFailed(format!("page {page} was not readable before the write"))
    })?;
    if role == PageRole::DynamicLock && data[3] != 0 {
        return Err(NfcError::InvalidArgument(
            "the dynamic lock page byte 3 is RFUI and must be zero".to_owned(),
        ));
    }
    let expected = expected_after_write(role, previous, data)?;
    let secret = matches!(role, PageRole::Password | PageRole::PasswordAck);
    if expected == previous && !secret {
        return Ok(PageWriteResult {
            page,
            previous,
            written: previous,
            changed: false,
        });
    }
    let expected_cc = if role == PageRole::CapabilityContainer {
        let cc = crate::cc::CapabilityContainer::parse(&expected)?;
        let layout = Type2Layout::from_cc(&cc)?;
        if layout.product != image.layout.product {
            return Err(NfcError::InvalidArgument(
                "a capability container write cannot change the tag product or data-area size"
                    .to_owned(),
            ));
        }
        Some(cc)
    } else {
        None
    };
    cancel.check()?;
    reader.write_page(page, data)?;
    cancel.check()?;
    if secret {
        // PWD and PACK read as zero, so a zero PACK cannot be compared with
        // the previous read-back. The write is always issued and the caller
        // verifies the secret with PWD_AUTH. Keep the in-memory image equal to
        // what the tag would report.
        image.set_page(page, [0; PAGE_SIZE])?;
        return Ok(PageWriteResult {
            page,
            previous,
            written: [0; PAGE_SIZE],
            changed: true,
        });
    }
    let read_back = crate::read::read_page_bounded(reader, page, image.layout.total_pages)?;
    let matches = if role == PageRole::DynamicLock {
        read_back[..3] == expected[..3]
    } else {
        read_back == expected
    };
    if !matches {
        return Err(NfcError::VerificationFailed(format!(
            "page {page} read back as {:02X?}, expected {:02X?}",
            read_back, expected
        )));
    }
    let mut observed = read_back;
    if role == PageRole::DynamicLock {
        observed[3] = previous[3];
    }
    if let Some(cc) = expected_cc {
        image.cc = cc;
    }
    image.set_page(page, observed)?;
    Ok(PageWriteResult {
        page,
        previous,
        written: observed,
        changed: true,
    })
}

fn expected_after_write(
    role: PageRole,
    previous: [u8; PAGE_SIZE],
    data: [u8; PAGE_SIZE],
) -> Result<[u8; PAGE_SIZE]> {
    let mut expected = data;
    match role {
        PageRole::StaticLock => {
            expected[0] = previous[0];
            expected[1] = previous[1];
            expected[2] |= previous[2];
            expected[3] |= previous[3];
        }
        PageRole::DynamicLock => {
            expected[0] |= previous[0];
            expected[1] |= previous[1];
            expected[2] |= previous[2];
            // Byte 3 is RFUI and reads as BDh on NTAG21x. Keep the observed
            // value so a reader that reports zero also compares cleanly.
            expected[3] = previous[3];
        }
        PageRole::CapabilityContainer => {
            for index in 0..PAGE_SIZE {
                expected[index] |= previous[index];
            }
        }
        _ => {}
    }
    Ok(expected)
}

/// Write a complete NDEF data area, page by page.
pub fn write_area(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    area: &[u8],
    policy: WritePolicy,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<WriteSummary> {
    if !area.len().is_multiple_of(PAGE_SIZE) {
        return Err(NfcError::InvalidArgument(
            "the NDEF area must end on a page boundary".to_owned(),
        ));
    }
    if area.len() > usize::from(image.cc.size) {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF area is {} bytes but the tag holds {}",
            area.len(),
            image.cc.size
        )));
    }
    let page_count = area.len() / PAGE_SIZE;
    authenticate_for_pages(
        reader,
        image,
        (0..page_count).map(|index| image.layout.user_first.saturating_add(index as u8)),
        access,
        cancel,
    )?;
    let mut summary = WriteSummary::default();
    for (index, chunk) in area.chunks(PAGE_SIZE).enumerate() {
        cancel.check()?;
        let page = image
            .layout
            .user_first
            .checked_add(index as u8)
            .ok_or_else(|| NfcError::InvalidArgument("NDEF page overflows".to_owned()))?;
        let mut data = [0u8; PAGE_SIZE];
        data.copy_from_slice(chunk);
        let result = write_page_verified(reader, image, page, data, policy, access, cancel)?;
        if result.changed {
            summary.written_pages += 1;
        } else {
            summary.unchanged_pages += 1;
        }
    }
    Ok(summary)
}

/// Read, encode, and write one NDEF message while preserving other TLVs.
pub fn write_ndef(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    message: &NdefMessage,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<WriteSummary> {
    let area = encode_ndef_area(image, message)?;
    write_area(reader, image, &area, WritePolicy::user(), access, cancel)
}

/// Replace the NDEF data area with an empty formatted tag.
pub fn format_ndef(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<WriteSummary> {
    let area = format_ndef_area(image)?;
    write_area(reader, image, &area, WritePolicy::user(), access, cancel)
}

/// Lock a set of pages. Lock bits are one-way, so every bit is verified.
pub fn lock_pages(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    pages: &[u8],
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<LockUpdate> {
    let current = LockBits::from_image(image)?;
    let update = current.pending(&image.layout, pages)?;
    if update.is_empty() {
        return Ok(update);
    }
    let mut targets = Vec::new();
    if update.static_changed {
        targets.push(2);
    }
    if update.dynamic_changed
        && let Some(page) = image.layout.dynamic_lock
    {
        targets.push(page);
    }
    authenticate_for_pages(reader, image, targets, access, cancel)?;
    if update.static_changed {
        let mut data = current.static_lock;
        data[0] |= update.static_lock[0];
        data[1] |= update.static_lock[1];
        let page = 2u8;
        let mut payload = *image
            .page(page)
            .ok_or_else(|| NfcError::InvalidArgument("page 2 is not readable".to_owned()))?;
        payload[2] = data[0];
        payload[3] = data[1];
        write_page_verified(
            reader,
            image,
            page,
            payload,
            WritePolicy::advanced(),
            access,
            cancel,
        )?;
    }
    if update.dynamic_changed {
        let page = image
            .layout
            .dynamic_lock
            .ok_or(NfcError::Unsupported("this tag has no dynamic lock page"))?;
        let mut payload = [0u8; PAGE_SIZE];
        payload[0] = current.dynamic_lock[0] | update.dynamic_lock[0];
        payload[1] = current.dynamic_lock[1] | update.dynamic_lock[1];
        payload[2] = current.dynamic_lock[2] | update.dynamic_lock[2];
        write_page_verified(
            reader,
            image,
            page,
            payload,
            WritePolicy::advanced(),
            access,
            cancel,
        )?;
    }
    Ok(update)
}

/// Program PWD/PACK and the AUTH0/ACCESS configuration, then verify the new
/// password with a fresh PWD_AUTH. PWD and PACK are written before AUTH0 so a
/// protected tag is never left with an unknown secret.
///
/// `current` carries the password of an already protected tag. It is required
/// before the first write when the current configuration protects any
/// configuration page. `auth0 = 0xFF` with `access = 0` clears protection.
pub fn set_password(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    current: &mut Type2AuthSession<'_>,
    update: PasswordUpdate,
    cancel: &CancelToken,
) -> Result<()> {
    let PasswordUpdate {
        auth0,
        access,
        password,
        pack,
    } = update;
    let pages = image.layout.config.ok_or(NfcError::Unsupported(
        "this tag has no NTAG configuration pages",
    ))?;
    let config = Configuration::from_image(image)?;
    if config.cfg_locked() {
        return Err(NfcError::Unsupported(
            "the NTAG configuration is permanently locked",
        ));
    }
    if auth0 != 0xFF && !(image.layout.user_first..=image.layout.user_last).contains(&auth0) {
        return Err(NfcError::InvalidArgument(format!(
            "AUTH0 must be a user-memory page ({}..={}) or 255 to disable protection",
            image.layout.user_first, image.layout.user_last
        )));
    }
    if access & 0x78 != 0 {
        return Err(NfcError::InvalidArgument(
            "ACCESS contains the CFGLCK bit or reserved bits; only PROT and AUTHLIM may be set"
                .to_owned(),
        ));
    }
    let protected_now = [pages.pwd, pages.pack, pages.cfg0, pages.cfg1]
        .into_iter()
        .any(|page| config.requires_password(page));
    if protected_now {
        current.authenticate(reader, cancel)?;
    }

    let mut committed = Vec::new();
    if let Err(error) = write_and_record(
        reader,
        image,
        pages.pwd,
        Configuration::password_page(password),
        current,
        cancel,
        &mut committed,
    ) {
        return Err(password_failure(error, &committed, pages, config, update));
    }
    if let Err(error) = write_and_record(
        reader,
        image,
        pages.pack,
        Configuration::pack_page(pack),
        current,
        cancel,
        &mut committed,
    ) {
        return Err(password_failure(error, &committed, pages, config, update));
    }

    // The new credentials replace the old session for the configuration
    // writes. This also proves PWD/PACK were programmed before AUTH0 changes.
    let new_auth = Type2Auth::new(password, pack);
    let mut new_session = Type2AuthSession::new(&new_auth);
    if let Err(error) = new_session.authenticate(reader, cancel) {
        return Err(password_failure(error, &committed, pages, config, update));
    }
    let [cfg0, cfg1] =
        Configuration::config_pages(config.mirror, config.mirror_page, auth0, access);
    if let Err(error) = write_and_record(
        reader,
        image,
        pages.cfg1,
        cfg1,
        &mut new_session,
        cancel,
        &mut committed,
    ) {
        return Err(password_failure(error, &committed, pages, config, update));
    }
    if let Err(error) = write_and_record(
        reader,
        image,
        pages.cfg0,
        cfg0,
        &mut new_session,
        cancel,
        &mut committed,
    ) {
        return Err(password_failure(error, &committed, pages, config, update));
    }

    // Re-authenticate after AUTH0/ACCESS and require the new PACK, then read
    // both configuration pages back as the tag now reports them.
    new_session.reset();
    if let Err(error) = new_session.authenticate(reader, cancel) {
        return Err(password_failure(error, &committed, pages, config, update));
    }
    for (page, expected) in [(pages.cfg0, cfg0), (pages.cfg1, cfg1)] {
        match crate::read::read_page_bounded(reader, page, image.layout.total_pages) {
            Ok(observed) if observed == expected => {}
            Ok(observed) => {
                return Err(password_failure(
                    NfcError::VerificationFailed(format!(
                        "page {page} read back as {:02X?}, expected {:02X?}",
                        observed, expected
                    )),
                    &committed,
                    pages,
                    config,
                    update,
                ));
            }
            Err(error) => {
                return Err(password_failure(error, &committed, pages, config, update));
            }
        }
    }
    Ok(())
}

fn write_and_record(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    page: u8,
    data: [u8; PAGE_SIZE],
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
    committed: &mut Vec<u8>,
) -> Result<()> {
    write_page_verified(
        reader,
        image,
        page,
        data,
        WritePolicy::advanced(),
        access,
        cancel,
    )?;
    committed.push(page);
    Ok(())
}

fn password_failure(
    error: NfcError,
    committed: &[u8],
    pages: crate::layout::ConfigPages,
    current: Configuration,
    update: PasswordUpdate,
) -> NfcError {
    if committed.is_empty() && matches!(error, NfcError::Cancelled) {
        return error;
    }
    let list = if committed.is_empty() {
        "no page".to_owned()
    } else {
        format!(
            "page(s) {}",
            committed
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let secret = match (
        committed.contains(&pages.pwd),
        committed.contains(&pages.pack),
    ) {
        (true, true) => "PWD and PACK may now hold the new values".to_owned(),
        (true, false) => {
            "PWD may hold the new value; PACK may still hold the previous value".to_owned()
        }
        (false, true) => {
            "PACK may hold the new value; PWD may still hold the previous value".to_owned()
        }
        (false, false) => "PWD and PACK may still hold the previous values".to_owned(),
    };
    let configuration = if committed.contains(&pages.cfg0) {
        format!(
            "AUTH0 now reads {} and ACCESS now reads {:02X}",
            update.auth0, update.access
        )
    } else if committed.contains(&pages.cfg1) {
        format!(
            "ACCESS now reads {:02X}; AUTH0 remains {:02X}",
            update.access, current.auth0
        )
    } else {
        format!(
            "AUTH0 {:02X} and ACCESS {:02X} remain the previous values",
            current.auth0, current.access
        )
    };
    NfcError::VerificationFailed(format!(
        "password configuration stopped after {list}: {secret}; {configuration}; {error}. Authenticate with the known password before any further change."
    ))
}

/// Options for restoring a Type 2 image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RestoreOptions {
    pub restore_user_memory: bool,
    pub restore_capability_container: bool,
    pub restore_configuration: bool,
    pub restore_lock_bytes: bool,
}

/// Restore a dumped image onto the selected tag.
///
/// The product, capability container, and page count must match. Manufacturer
/// pages are never written. User memory is written first, configuration next,
/// the capability container after it, and one-way lock bytes last.
pub fn restore_image(
    reader: &mut dyn Reader,
    image: &mut Type2Image,
    source: &Type2Image,
    options: RestoreOptions,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<WriteSummary> {
    if source.layout != image.layout {
        return Err(NfcError::InvalidArgument(format!(
            "the dump is for {} but the card is {}",
            source.layout.product.name(),
            image.layout.product.name()
        )));
    }
    if source.cc.to_bytes() != image.cc.to_bytes() {
        return Err(NfcError::InvalidArgument(
            "the dump capability container does not match the card".to_owned(),
        ));
    }
    let plan = restore_plan(&image.layout, options);
    for page in &plan {
        if source.page(*page).is_none() {
            return Err(NfcError::InvalidArgument(format!(
                "the dump has no data for page {page}"
            )));
        }
    }
    authenticate_for_pages(reader, image, plan.iter().copied(), access, cancel)?;
    let mut summary = WriteSummary::default();
    for page in plan {
        cancel.check()?;
        let data = *source
            .page(page)
            .ok_or_else(|| NfcError::InvalidArgument(format!("page {page} is unknown")))?;
        let change = write_page_verified(
            reader,
            image,
            page,
            data,
            WritePolicy::advanced(),
            access,
            cancel,
        )?;
        if change.changed {
            summary.written_pages += 1;
        } else {
            summary.unchanged_pages += 1;
        }
    }
    Ok(summary)
}

/// Pages a restore may touch, in safe write order.
fn restore_plan(layout: &Type2Layout, options: RestoreOptions) -> Vec<u8> {
    let mut plan = Vec::new();
    if options.restore_user_memory {
        plan.extend(layout.user_first..=layout.user_last);
    }
    if options.restore_configuration
        && let Some(config) = layout.config
    {
        // PWD and PACK are never readable, so a dump cannot restore them.
        plan.extend([config.cfg0, config.cfg1]);
    }
    if options.restore_capability_container {
        plan.push(3);
    }
    if options.restore_lock_bytes {
        if let Some(page) = layout.dynamic_lock {
            plan.push(page);
        }
        plan.push(2);
    }
    plan
}

#[cfg(test)]
mod tests {
    use nfc_core::{CardInfo, CardType};
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::{MockCall, MockFault, MockReader};

    use crate::cc::CapabilityContainer;
    use crate::read::read_tag;

    use super::*;

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )
    }

    fn reader() -> MockReader {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:ntag"), Some(card()));
        for page in 0..45u8 {
            reader.set_page(page, [page; 4]);
        }
        reader.set_page(2, [2, 2, 0x00, 0x00]);
        reader.set_page(3, [0xE1, 0x10, 0x12, 0x00]);
        reader.set_page(40, [0x00, 0x00, 0x00, 0xBD]);
        reader.set_page(41, [0x00, 0x00, 0x00, 0xFF]);
        reader.set_page(42, [0x00, 0x00, 0x00, 0x00]);
        reader.set_page(43, [0xFF; 4]);
        reader.set_page(44, [0x00, 0x00, 0x00, 0x00]);
        reader
    }

    fn protected_reader() -> MockReader {
        let mut reader = reader();
        for page in 4..40u8 {
            reader.set_page(page, [0; 4]);
        }
        reader.set_page(4, [0x03, 0x00, 0xFE, 0x00]);
        reader.set_password([1, 2, 3, 4], [0xAA, 0x55]);
        reader.set_page(41, [0x00, 0x00, 0x00, 0x20]);
        reader.set_page(42, [0x80, 0x00, 0x00, 0x00]);
        reader
    }

    fn read(reader: &mut MockReader) -> Type2Image {
        let mut access = Type2AuthSession::none();
        read_tag(reader, &card(), &mut access, &CancelToken::new())
            .expect("read")
            .image
    }

    fn image() -> (MockReader, Type2Image) {
        let mut reader = reader();
        let image = read(&mut reader);
        (reader, image)
    }

    fn update(auth0: u8, access: u8, password: [u8; 4], pack: [u8; 2]) -> PasswordUpdate {
        PasswordUpdate {
            auth0,
            access,
            password,
            pack,
        }
    }

    #[test]
    fn user_page_write_is_verified_and_updates_the_image() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        let result = write_page_verified(
            &mut reader,
            &mut image,
            8,
            [0xAA; 4],
            WritePolicy::user(),
            &mut access,
            &CancelToken::new(),
        )
        .expect("write");
        assert!(result.changed);
        assert_eq!(image.page(8), Some(&[0xAA; 4]));
        assert_eq!(reader.page(8), Some([0xAA; 4]));
    }

    #[test]
    fn ignored_page_write_is_caught_by_read_back() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        reader.fail_next(MockCall::WritePage, MockFault::WriteIgnored);
        assert!(matches!(
            write_page_verified(
                &mut reader,
                &mut image,
                8,
                [0xAA; 4],
                WritePolicy::user(),
                &mut access,
                &CancelToken::new(),
            ),
            Err(NfcError::VerificationFailed(_))
        ));
    }

    #[test]
    fn protected_pages_require_an_advanced_policy() {
        let (_reader, image) = image();
        assert!(check_writable(&image, 3, WritePolicy::user()).is_err());
        assert!(check_writable(&image, 2, WritePolicy::user()).is_err());
        assert!(check_writable(&image, 40, WritePolicy::user()).is_err());
        assert!(check_writable(&image, 0, WritePolicy::advanced()).is_err());
        assert!(check_writable(&image, 3, WritePolicy::advanced()).is_ok());
    }

    #[test]
    fn capability_container_write_updates_the_parsed_cc() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        write_page_verified(
            &mut reader,
            &mut image,
            3,
            [0xE1, 0x10, 0x12, 0xF0],
            WritePolicy::advanced(),
            &mut access,
            &CancelToken::new(),
        )
        .expect("CC write");
        assert!(!image.cc.writable());
        assert_eq!(image.page(3), Some(&[0xE1, 0x10, 0x12, 0xF0]));
        assert_eq!(reader.page(3), Some([0xE1, 0x10, 0x12, 0xF0]));
    }

    #[test]
    fn capability_container_size_change_is_refused_before_the_write() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        assert!(matches!(
            write_page_verified(
                &mut reader,
                &mut image,
                3,
                [0xE1, 0x10, 0x3E, 0x00],
                WritePolicy::advanced(),
                &mut access,
                &CancelToken::new(),
            ),
            Err(NfcError::InvalidArgument(_))
        ));
        assert_eq!(reader.page(3), Some([0xE1, 0x10, 0x12, 0x00]));
        assert_eq!(image.cc.size, 144);
    }

    #[test]
    fn locked_pages_are_rejected() {
        let (_reader, mut image) = image();
        image.set_page(2, [0, 0, 0x10, 0x00]).expect("lock page");
        assert!(check_writable(&image, 4, WritePolicy::user()).is_err());
        assert!(check_writable(&image, 5, WritePolicy::user()).is_ok());
    }

    #[test]
    fn protected_page_write_requires_authentication() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        assert_eq!(reader.password_auths(), 1);

        let mut none = Type2AuthSession::none();
        let error = write_page_verified(
            &mut reader,
            &mut image,
            32,
            [0xA5; 4],
            WritePolicy::user(),
            &mut none,
            &CancelToken::new(),
        )
        .expect_err("missing credentials");
        assert!(matches!(error, NfcError::AuthenticationRequired(_)));
        assert_eq!(reader.page(32), Some([0; 4]));

        write_page_verified(
            &mut reader,
            &mut image,
            32,
            [0xA5; 4],
            WritePolicy::user(),
            &mut access,
            &CancelToken::new(),
        )
        .expect("authenticated write");
        assert_eq!(reader.page(32), Some([0xA5; 4]));
        assert_eq!(reader.password_auths(), 1);
    }

    #[test]
    fn protected_ndef_write_authenticates_once() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        let message = NdefMessage::new(vec![
            nfc_ndef::TextRecord::new("hello", "en")
                .expect("text")
                .to_record()
                .expect("record"),
        ])
        .expect("message");
        let summary = write_ndef(
            &mut reader,
            &mut image,
            &message,
            &mut access,
            &CancelToken::new(),
        )
        .expect("write NDEF");
        assert!(summary.written_pages > 0);
        assert_eq!(reader.password_auths(), 1);
        assert!(matches!(
            crate::ndef::read_ndef(&image).expect("read NDEF").state,
            crate::ndef::NdefState::Message(_)
        ));
    }

    #[test]
    fn ndef_write_and_format_round_trip() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        format_ndef(&mut reader, &mut image, &mut access, &CancelToken::new())
            .expect("initial format");
        let message = NdefMessage::new(vec![
            nfc_ndef::TextRecord::new("hello", "en")
                .expect("text")
                .to_record()
                .expect("record"),
        ])
        .expect("message");
        let summary = write_ndef(
            &mut reader,
            &mut image,
            &message,
            &mut access,
            &CancelToken::new(),
        )
        .expect("write NDEF");
        assert!(summary.written_pages > 0);
        let ndef = crate::ndef::read_ndef(&image).expect("read NDEF");
        assert!(matches!(ndef.state, crate::ndef::NdefState::Message(_)));
        write_page_verified(
            &mut reader,
            &mut image,
            4,
            [0xFF; 4],
            WritePolicy::user(),
            &mut access,
            &CancelToken::new(),
        )
        .expect("corrupt NDEF");
        let summary =
            format_ndef(&mut reader, &mut image, &mut access, &CancelToken::new()).expect("format");
        assert!(summary.written_pages > 0);
        assert_eq!(
            crate::ndef::read_ndef(&image).expect("read").state,
            crate::ndef::NdefState::Empty
        );
    }

    #[test]
    fn lock_write_sets_bits_without_clearing_uid_bytes() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        let update = lock_pages(
            &mut reader,
            &mut image,
            &[3, 16],
            &mut access,
            &CancelToken::new(),
        )
        .expect("lock");
        assert!(update.static_changed);
        assert!(update.dynamic_changed);
        assert_eq!(image.page(2).expect("page 2")[0..2], [2, 2]);
        assert_eq!(image.page(2).expect("page 2")[2], 0x08);
        assert_eq!(image.page(40).expect("dynamic")[0], 0x01);
        assert_eq!(image.page(40).expect("dynamic")[3], 0xBD);
    }

    #[test]
    fn password_write_verifies_with_pwd_auth() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        set_password(
            &mut reader,
            &mut image,
            &mut access,
            update(0x20, 0x00, [1, 2, 3, 4], [0xAA, 0x55]),
            &CancelToken::new(),
        )
        .expect("password");
        let config = Configuration::from_image(&image).expect("config");
        assert_eq!(config.auth0, 0x20);
    }

    #[test]
    fn a_zero_pack_is_written_instead_of_skipped_as_unchanged() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        set_password(
            &mut reader,
            &mut image,
            &mut access,
            update(0x20, 0x80, [1, 2, 3, 4], [0xAA, 0x55]),
            &CancelToken::new(),
        )
        .expect("initial password");
        let old = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&old);
        set_password(
            &mut reader,
            &mut image,
            &mut access,
            update(0x20, 0x80, [1, 2, 3, 4], [0x00, 0x00]),
            &CancelToken::new(),
        )
        .expect("zero PACK");
        assert_eq!(reader.page(44).expect("PACK page")[..2], [0x00, 0x00]);
        let zero = Type2Auth::new([1, 2, 3, 4], [0x00, 0x00]);
        let mut session = Type2AuthSession::new(&zero);
        session
            .authenticate(&mut reader, &CancelToken::new())
            .expect("zero PACK authenticates");
    }

    #[test]
    fn protected_password_change_uses_current_credentials() {
        let mut reader = protected_reader();
        let old = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&old);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;

        let mut none = Type2AuthSession::none();
        let error = set_password(
            &mut reader,
            &mut image,
            &mut none,
            update(0x20, 0x80, [5, 6, 7, 8], [0x12, 0x34]),
            &CancelToken::new(),
        )
        .expect_err("current credentials are required");
        assert!(matches!(error, NfcError::AuthenticationRequired(_)));
        assert_eq!(reader.page(43), Some([0xFF; 4]));

        set_password(
            &mut reader,
            &mut image,
            &mut access,
            update(0x20, 0x80, [5, 6, 7, 8], [0x12, 0x34]),
            &CancelToken::new(),
        )
        .expect("change password");
        let config = Configuration::from_image(&image).expect("config");
        assert_eq!(config.auth0, 0x20);
        assert!(config.read_and_write_protected());
        assert_eq!(reader.page(43), Some([5, 6, 7, 8]));
        assert_eq!(reader.page(44).expect("pack page")[..2], [0x12, 0x34]);
    }

    #[test]
    fn protection_can_be_cleared_with_the_current_password() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        set_password(
            &mut reader,
            &mut image,
            &mut access,
            update(0xFF, 0x00, [5, 6, 7, 8], [0x12, 0x34]),
            &CancelToken::new(),
        )
        .expect("clear protection");
        let config = Configuration::from_image(&image).expect("config");
        assert_eq!(config.auth0, 0xFF);
        assert_eq!(config.access, 0x00);

        let mut none = Type2AuthSession::none();
        let read = read_tag(&mut reader, &card(), &mut none, &CancelToken::new())
            .expect("unprotected read");
        assert!(read.image.is_complete());
    }

    #[test]
    fn a_protected_reconfiguration_without_credentials_fails_before_mutation() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        let before: Vec<_> = (0..45).map(|page| reader.page(page)).collect();
        let mut none = Type2AuthSession::none();
        let error = set_password(
            &mut reader,
            &mut image,
            &mut none,
            update(0x20, 0x80, [5, 6, 7, 8], [0x12, 0x34]),
            &CancelToken::new(),
        )
        .expect_err("missing current password");
        assert!(matches!(error, NfcError::AuthenticationRequired(_)));
        let after: Vec<_> = (0..45).map(|page| reader.page(page)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn a_wrong_password_never_writes_the_unprotected_pages_first() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        let before: Vec<_> = (4..40).map(|page| reader.page(page)).collect();
        let wrong = Type2Auth::new([1, 2, 3, 4], [0x00, 0x00]);
        let mut wrong = Type2AuthSession::new(&wrong);
        let message = NdefMessage::new(vec![
            nfc_ndef::TextRecord::new("wrong", "en")
                .expect("text")
                .to_record()
                .expect("record"),
        ])
        .expect("message");
        assert!(matches!(
            write_ndef(
                &mut reader,
                &mut image,
                &message,
                &mut wrong,
                &CancelToken::new(),
            ),
            Err(NfcError::VerificationFailed(_))
        ));
        let after: Vec<_> = (4..40).map(|page| reader.page(page)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn a_protected_lock_update_fails_before_the_static_lock_page() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        let before = reader.page(2).expect("page 2");
        let mut none = Type2AuthSession::none();
        assert!(matches!(
            lock_pages(
                &mut reader,
                &mut image,
                &[4, 16],
                &mut none,
                &CancelToken::new(),
            ),
            Err(NfcError::AuthenticationRequired(_))
        ));
        assert_eq!(reader.page(2), Some(before));
    }

    #[test]
    fn a_protected_restore_fails_before_any_page() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        let mut source = image.clone();
        for page in 4..40u8 {
            source.set_page(page, [0xEE; 4]).expect("source page");
        }
        let before: Vec<_> = (4..40).map(|page| reader.page(page)).collect();
        let mut none = Type2AuthSession::none();
        assert!(matches!(
            restore_image(
                &mut reader,
                &mut image,
                &source,
                RestoreOptions {
                    restore_user_memory: true,
                    ..RestoreOptions::default()
                },
                &mut none,
                &CancelToken::new(),
            ),
            Err(NfcError::AuthenticationRequired(_))
        ));
        let after: Vec<_> = (4..40).map(|page| reader.page(page)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn protected_write_stops_on_cancellation_before_the_first_page() {
        let mut reader = protected_reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut access = Type2AuthSession::new(&auth);
        let mut image = read_tag(&mut reader, &card(), &mut access, &CancelToken::new())
            .expect("authenticated read")
            .image;
        let before: Vec<_> = (0..45).map(|page| reader.page(page)).collect();
        let cancel = CancelToken::new();
        cancel.cancel();
        let error = write_page_verified(
            &mut reader,
            &mut image,
            32,
            [0xA5; 4],
            WritePolicy::user(),
            &mut access,
            &cancel,
        )
        .expect_err("cancelled write");
        assert!(matches!(error, NfcError::Cancelled));
        let after: Vec<_> = (0..45).map(|page| reader.page(page)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn restore_rejects_a_different_capability_container() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        let other_cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x3E, 0x00]).expect("CC");
        let source = Type2Image::new(
            Type2Layout::from_cc(&other_cc).expect("layout"),
            other_cc,
            vec![0; 7],
        );
        assert!(
            restore_image(
                &mut reader,
                &mut image,
                &source,
                RestoreOptions {
                    restore_user_memory: true,
                    ..RestoreOptions::default()
                },
                &mut access,
                &CancelToken::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn restore_writes_changed_user_pages_only() {
        let (mut reader, mut image) = image();
        let mut access = Type2AuthSession::none();
        let source = image.clone();
        let summary = restore_image(
            &mut reader,
            &mut image,
            &source,
            RestoreOptions {
                restore_user_memory: true,
                ..RestoreOptions::default()
            },
            &mut access,
            &CancelToken::new(),
        )
        .expect("restore");
        assert_eq!(summary.written_pages, 0);
        assert_eq!(summary.unchanged_pages, 36);
    }
}
