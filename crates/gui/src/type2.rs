//! Type 2 presentation model for the GTK workbench.
//!
//! Page roles, lock display, write gates, and NDEF facts come from
//! `nfc-type2`. This module only turns them into rows, labels, and parsed
//! operator input; it never reads or writes a card.

use nfc_core::{CardInfo, CardType, PAGE_SIZE};
use nfc_type2::{
    CapabilityContainer, Configuration, LockBits, LockUpdate, NdefState, PageRole, Type2Image,
    Type2Ndef, WritePolicy, check_writable, read_ndef,
};

/// The displayed Type 2 image together with its card identity and NDEF facts.
#[derive(Clone, Debug)]
pub struct Session {
    pub card: CardInfo,
    pub image: Type2Image,
    pub warnings: Vec<String>,
    ndef: std::result::Result<Type2Ndef, String>,
}

impl Session {
    pub fn new(card: CardInfo, image: Type2Image, warnings: Vec<String>) -> Self {
        let mut session = Self {
            card,
            image,
            warnings,
            ndef: Err("the NDEF state has not been parsed".to_owned()),
        };
        session.refresh_ndef();
        session
    }

    /// A loaded project image has no live selection identity; derive the
    /// standard Type 2 identity from its UID so inspection still names it.
    pub fn from_image(image: Type2Image, warnings: Vec<String>) -> Self {
        let card = CardInfo::new(image.uid.clone(), [0x00, 0x44], 0x00, CardType::Type2);
        Self::new(card, image, warnings)
    }

    pub fn refresh_ndef(&mut self) {
        self.ndef = read_ndef(&self.image).map_err(|error| error.to_string());
    }

    pub fn ndef(&self) -> std::result::Result<&Type2Ndef, &str> {
        self.ndef.as_ref().map_err(String::as_str)
    }

    pub fn locks(&self) -> std::result::Result<LockBits, String> {
        LockBits::from_image(&self.image).map_err(|error| error.to_string())
    }

    pub fn configuration(&self) -> Option<Configuration> {
        Configuration::from_image(&self.image).ok()
    }
}

/// One visible row in the page table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageRow {
    pub page: u8,
    pub role: &'static str,
    pub lock: String,
    pub data: String,
    pub advanced: bool,
    pub complete: bool,
}

/// Page facts for the inspector and the current write gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageInsight {
    pub title: String,
    pub meta: String,
    pub details: String,
    pub hex: String,
    pub ascii: String,
    pub can_write: bool,
}

/// A pending lock update with its current and resulting lock bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockPreview {
    pub static_before: [u8; 2],
    pub static_after: [u8; 2],
    pub dynamic_before: [u8; 3],
    pub dynamic_after: [u8; 3],
    pub update: LockUpdate,
}

/// Rows for the current mode. Advanced pages (CC, lock, configuration, and
/// password/PACK) stay hidden until Advanced mode is on.
pub fn page_rows(session: &Session, advanced: bool) -> Vec<PageRow> {
    let locks = session.locks().ok();
    let config = session.configuration();
    let cc_writable = session.image.cc.writable();
    (0..session.image.layout.total_pages)
        .filter(|page| {
            let role = session.image.role(*page);
            advanced
                || matches!(
                    role,
                    PageRole::UserMemory | PageRole::Manufacturer | PageRole::Reserved
                )
        })
        .map(|page| {
            let role = session.image.role(page);
            PageRow {
                page,
                role: role.label(),
                lock: lock_label(
                    &session.image,
                    locks.as_ref(),
                    config.as_ref(),
                    cc_writable,
                    page,
                ),
                data: session
                    .image
                    .page(page)
                    .map(hex4)
                    .unwrap_or_else(|| "—".to_owned()),
                advanced: role.is_advanced(),
                complete: session.image.page(page).is_some(),
            }
        })
        .collect()
}

fn lock_label(
    image: &Type2Image,
    locks: Option<&LockBits>,
    config: Option<&Configuration>,
    cc_writable: bool,
    page: u8,
) -> String {
    match image.role(page) {
        PageRole::Manufacturer | PageRole::Reserved => "read-only".to_owned(),
        PageRole::StaticLock | PageRole::DynamicLock => "lock page".to_owned(),
        PageRole::CapabilityContainer => {
            if cc_writable {
                "writable".to_owned()
            } else {
                "read-only".to_owned()
            }
        }
        PageRole::Configuration => match config {
            Some(config) if config.cfg_locked() => "locked".to_owned(),
            Some(_) => "writable".to_owned(),
            None => "—".to_owned(),
        },
        PageRole::Password | PageRole::PasswordAck => "write-only".to_owned(),
        PageRole::UserMemory => match locks {
            Some(locks) if locks.is_static_locked(&image.layout, page) => "static".to_owned(),
            Some(locks) if locks.is_dynamic_locked(&image.layout, page) => "dynamic".to_owned(),
            Some(_) if config.is_some_and(|config| config.requires_password(page)) => {
                "auth".to_owned()
            }
            Some(_) => "—".to_owned(),
            None => "unknown".to_owned(),
        },
    }
}

/// Inspector content for one page. `advanced` is the current mode and
/// `auth_supplied` reports whether protected-access credentials are entered;
/// both change the write gate, not the displayed facts.
pub fn inspect(
    session: &Session,
    page: u8,
    advanced: bool,
    auth_supplied: bool,
) -> Option<PageInsight> {
    if page >= session.image.layout.total_pages {
        return None;
    }
    let role = session.image.role(page);
    let data = session.image.page(page).copied();
    let locks = session.locks().ok();
    let config = session.configuration();
    let lock = lock_label(
        &session.image,
        locks.as_ref(),
        config.as_ref(),
        session.image.cc.writable(),
        page,
    );
    let mut details = Vec::new();
    match role {
        PageRole::Manufacturer => {
            if page == 0 {
                details.push(format!(
                    "UID bytes 0..2: {}",
                    hex_bytes(&session.image.uid.iter().copied().take(3).collect::<Vec<_>>())
                ));
                if let Some(data) = data {
                    details.push(format!("BCC0 check byte: {:02X}", data[3]));
                }
            } else {
                details.push(format!(
                    "UID bytes 3..6: {}",
                    hex_bytes(
                        &session.image
                            .uid
                            .iter()
                            .copied()
                            .skip(3)
                            .take(4)
                            .collect::<Vec<_>>()
                    )
                ));
            }
            details.push("Manufacturer and UID pages are read-only.".to_owned());
        }
        PageRole::StaticLock => {
            match locks {
                Some(locks) => {
                    details.push(format!(
                        "Static lock bytes: {}",
                        hex_bytes(&locks.static_lock)
                    ));
                    let locked = locked_pages(session, |page| {
                        locks.is_static_locked(&session.image.layout, page)
                    });
                    details.push(format!("Statically locked pages: {}", page_list(&locked)));
                }
                None => details.push("Static lock bytes are not readable.".to_owned()),
            }
            details.push(
                "Lock bits are one-way: a set bit can never be cleared by a page write."
                    .to_owned(),
            );
        }
        PageRole::CapabilityContainer => match data {
            Some(data) => match CapabilityContainer::parse(&data) {
                Ok(cc) => {
                    details.push(format!(
                        "CC {} · version {}.{}",
                        hex4(&data),
                        cc.major_version,
                        cc.minor_version
                    ));
                    details.push(format!(
                        "Data area {} bytes · NDEF capacity {} bytes",
                        cc.size,
                        cc.ndef_capacity()
                    ));
                    details.push(format!(
                        "Read access {:#04X} · write access {:#04X} · {}",
                        cc.read_access(),
                        cc.write_access(),
                        if cc.writable() {
                            "data area writable"
                        } else {
                            "data area read-only"
                        }
                    ));
                }
                Err(error) => details.push(format!("Invalid capability container: {error}")),
            },
            None => details.push("The capability container is not readable.".to_owned()),
        },
        PageRole::UserMemory => match session.image.layout.ndef_offset(page, &session.image.cc) {
            Some(offset) => details.push(format!(
                "NDEF data area offset {offset} of {} bytes",
                session.image.cc.size
            )),
            None => details.push("Outside the CC data area.".to_owned()),
        },
        PageRole::DynamicLock => {
            if let Some(data) = data {
                details.push(format!("Dynamic lock bytes: {}", hex_bytes(&data[..3])));
                details.push(
                    "Lock bits are one-way and page granularity follows the product."
                        .to_owned(),
                );
            }
        }
        PageRole::Configuration => match config {
            Some(config) => details.extend(configuration_lines(&config)),
            None => details.push("The NTAG configuration pages are not readable.".to_owned()),
        },
        PageRole::Password => details.push(
            "PWD is write-only: reads return zero or undefined data. Use the Protection form to program PWD, PACK, AUTH0, and ACCESS together.".to_owned(),
        ),
        PageRole::PasswordAck => details.push(
            "PACK is write-only: the value is verified with PWD_AUTH, not by reading the page."
                .to_owned(),
        ),
        PageRole::Reserved => details.push("Reserved page; no write path.".to_owned()),
    }
    if let Some(config) = &config
        && config.requires_password(page)
    {
        details.push(format!(
            "Password protection starts at page {}; this page needs PWD_AUTH before a write.",
            config.auth0
        ));
    }
    let gate = write_gate(session, page, advanced, auth_supplied);
    let can_write = gate.is_ok();
    details.push(match &gate {
        Ok(()) => "Write: ready.".to_owned(),
        Err(error) => format!("Write: {error}"),
    });
    Some(PageInsight {
        title: format!("Page {page:03}"),
        meta: format!("{} · {lock}", role.label()),
        details: details.join("\n"),
        hex: data.map(|data| hex4(&data)).unwrap_or_default(),
        ascii: data.map(|data| ascii4(&data)).unwrap_or_default(),
        can_write,
    })
}

fn configuration_lines(config: &Configuration) -> Vec<String> {
    let mut lines = vec![format!(
        "MIRROR {:#04X} · mirror page {}",
        config.mirror, config.mirror_page
    )];
    lines.push(format!(
        "AUTH0 {} · ACCESS {:#04X} · AUTH_LIM {}",
        config.auth0,
        config.access,
        config.auth_lim()
    ));
    lines.push(format!(
        "CFGLCK {} · PROT {} · protected from {}",
        if config.cfg_locked() { "on" } else { "off" },
        if config.read_and_write_protected() {
            "on"
        } else {
            "off"
        },
        config
            .protected_from()
            .map(|page| page.to_string())
            .unwrap_or_else(|| "never".to_owned())
    ));
    lines.push("PWD and PACK are never readable.".to_owned());
    lines
}

/// Refuse a page write under the current mode before any card operation.
pub fn write_gate(
    session: &Session,
    page: u8,
    advanced: bool,
    auth_supplied: bool,
) -> std::result::Result<(), String> {
    let role = session.image.role(page);
    if matches!(
        role,
        PageRole::CapabilityContainer
            | PageRole::StaticLock
            | PageRole::DynamicLock
            | PageRole::Configuration
            | PageRole::Password
            | PageRole::PasswordAck
    ) && !advanced
    {
        return Err("Advanced mode is required for this page.".to_owned());
    }
    if session.image.layout.config.is_some() && session.configuration().is_none() {
        return Err(
            "The NTAG configuration pages are not readable; protection state is unknown."
                .to_owned(),
        );
    }
    let policy = if advanced {
        WritePolicy::advanced()
    } else {
        WritePolicy::user()
    };
    check_writable(&session.image, page, policy).map_err(|error| error.to_string())?;
    if auth_supplied {
        return Ok(());
    }
    if let Some(config) = session.configuration()
        && config.requires_password(page)
    {
        return Err(format!(
            "Password protection starts at page {}; enter the access password and PACK in the Protection tab.",
            config.auth0
        ));
    }
    Ok(())
}

/// The pages covered by the capability container data area.
pub fn ndef_area_pages(image: &Type2Image) -> Vec<u8> {
    let count = usize::from(image.cc.size) / PAGE_SIZE;
    (0..count)
        .map(|index| image.layout.user_first.saturating_add(index as u8))
        .take_while(|page| *page <= image.layout.user_last)
        .collect()
}

/// Pages in the NDEF data area that are locked read-only.
pub fn ndef_locked_pages(session: &Session) -> std::result::Result<Vec<u8>, String> {
    let locks = session.locks()?;
    Ok(ndef_area_pages(&session.image)
        .into_iter()
        .filter(|page| locks.is_locked(&session.image.layout, *page))
        .collect())
}

/// First page in the NDEF data area covered by password protection.
pub fn ndef_password_page(session: &Session) -> Option<u8> {
    let config = session.configuration()?;
    ndef_area_pages(&session.image)
        .into_iter()
        .find(|page| config.requires_password(*page))
}

pub fn ndef_state_label(state: &NdefState) -> String {
    match state {
        NdefState::Unformatted => "unformatted".to_owned(),
        NdefState::Empty => "empty".to_owned(),
        NdefState::Message(message) => format!("{} record(s)", message.records().len()),
    }
}

/// Parse a decimal page list such as `4,8,16-20`.
pub fn parse_page_list(text: &str) -> std::result::Result<Vec<u8>, String> {
    let mut pages = Vec::new();
    for token in text
        .split([',', ';', ' ', '\n', '\t'])
        .filter(|token| !token.is_empty())
    {
        if let Some((start, end)) = token.split_once('-') {
            let start = parse_page(start)?;
            let end = parse_page(end)?;
            if start > end {
                return Err(format!("page range {token} is reversed"));
            }
            pages.extend(start..=end);
        } else {
            pages.push(parse_page(token)?);
        }
    }
    if pages.is_empty() {
        return Err("Enter at least one page.".to_owned());
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

fn parse_page(text: &str) -> std::result::Result<u8, String> {
    text.parse::<u8>()
        .map_err(|_| format!("{text:?} is not a page number"))
}

/// Compute the lock bytes a request would set without touching the card.
pub fn lock_preview(session: &Session, pages: &[u8]) -> std::result::Result<LockPreview, String> {
    let locks = session.locks()?;
    let update = locks
        .pending(&session.image.layout, pages)
        .map_err(|error| error.to_string())?;
    let mut static_after = locks.static_lock;
    for (index, byte) in static_after.iter_mut().enumerate() {
        *byte |= update.static_lock[index];
    }
    let mut dynamic_after = locks.dynamic_lock;
    for (index, byte) in dynamic_after.iter_mut().enumerate() {
        *byte |= update.dynamic_lock[index];
    }
    Ok(LockPreview {
        static_before: locks.static_lock,
        static_after,
        dynamic_before: locks.dynamic_lock,
        dynamic_after,
        update,
    })
}

/// Configuration facts for the Protection screen.
pub fn configuration_text(session: &Session) -> String {
    match session.configuration() {
        Some(config) => configuration_lines(&config).join("\n"),
        None => "This tag has no readable NTAG configuration pages.".to_owned(),
    }
}

/// Pages whose lock bits are set, using the given predicate.
fn locked_pages(session: &Session, predicate: impl Fn(u8) -> bool) -> Vec<u8> {
    (0..session.image.layout.total_pages)
        .filter(|page| predicate(*page))
        .collect()
}

fn page_list(pages: &[u8]) -> String {
    if pages.is_empty() {
        "none".to_owned()
    } else {
        pages
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn hex4(data: &[u8; PAGE_SIZE]) -> String {
    hex_bytes(data)
}

pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn ascii4(data: &[u8; PAGE_SIZE]) -> String {
    data.iter()
        .map(|byte| {
            if (0x20..=0x7E).contains(byte) {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

/// Parse eight hex digits with optional spaces or colons.
pub fn parse_hex4(text: &str) -> std::result::Result<[u8; PAGE_SIZE], String> {
    parse_hex_bytes::<PAGE_SIZE>(text, "page")
}

/// Parse a fixed number of bytes from hex digits, spaces, or colons.
pub fn parse_hex_bytes<const N: usize>(
    text: &str,
    label: &str,
) -> std::result::Result<[u8; N], String> {
    let compact: String = text
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != N * 2 {
        return Err(format!(
            "{label} is {} hexadecimal digits, got {}",
            N * 2,
            compact.len()
        ));
    }
    let mut data = [0u8; N];
    for (index, byte) in data.iter_mut().enumerate() {
        let high = nibble(compact.as_bytes()[index * 2])?;
        let low = nibble(compact.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Ok(data)
}

/// Parse a byte as decimal or `0x`-prefixed hexadecimal.
pub fn parse_u8(text: &str) -> std::result::Result<u8, String> {
    let text = text.trim();
    let value = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16),
        None => text.parse::<u8>(),
    };
    value.map_err(|_| format!("{text:?} is not a byte value (0..=255)"))
}

fn nibble(character: u8) -> std::result::Result<u8, String> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(format!(
            "{:?} is not a hexadecimal digit",
            character as char
        )),
    }
}

#[cfg(test)]
mod tests {
    use nfc_core::PAGE_SIZE;
    use nfc_type2::{PageRole, Type2Layout};

    use super::*;

    fn card(uid: u8) -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, uid],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )
    }

    fn image() -> Type2Image {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0x00]).expect("CC");
        let layout = Type2Layout::from_cc(&cc).expect("layout");
        let mut image = Type2Image::new(layout, cc, card(0x80).uid);
        for page in 0..45u8 {
            image.set_page(page, [page; PAGE_SIZE]).expect("page");
        }
        image.set_page(3, [0xE1, 0x10, 0x12, 0x00]).expect("CC");
        let mut area = [0u8; 144];
        area[..3].copy_from_slice(&[0x03, 0x00, 0xFE]);
        for (index, page) in (4..40u8).enumerate() {
            let mut data = [0u8; PAGE_SIZE];
            data.copy_from_slice(&area[index * PAGE_SIZE..(index + 1) * PAGE_SIZE]);
            image.set_page(page, data).expect("area");
        }
        image.set_page(2, [0x80, 0x48, 0x00, 0x00]).expect("lock");
        image
            .set_page(40, [0x00, 0x00, 0x00, 0xBD])
            .expect("dynamic");
        image.set_page(41, [0x00, 0x00, 0x00, 0xFF]).expect("cfg0");
        image.set_page(42, [0x00, 0x00, 0x00, 0x00]).expect("cfg1");
        image.set_page(43, [0x00; PAGE_SIZE]).expect("pwd");
        image.set_page(44, [0x00; PAGE_SIZE]).expect("pack");
        image
    }

    fn session() -> Session {
        Session::new(card(0x80), image(), Vec::new())
    }

    #[test]
    fn rows_hide_advanced_pages_until_enabled() {
        let session = session();
        let basic = page_rows(&session, false);
        assert_eq!(basic.len(), 38);
        assert!(!basic.iter().any(|row| row.page == 3));
        assert!(!basic.iter().any(|row| row.page == 40));
        assert!(basic.iter().any(|row| row.page == 0));
        assert_eq!(basic[0].role, "manufacturer");
        let advanced = page_rows(&session, true);
        assert_eq!(advanced.len(), 45);
        assert!(advanced.iter().any(|row| row.page == 43));
    }

    #[test]
    fn inspection_reports_roles_cc_locks_and_ndef() {
        let session = session();
        let cc = inspect(&session, 3, true, false).expect("CC");
        assert!(cc.details.contains("Data area 144 bytes"));
        assert!(cc.details.contains("NDEF capacity 141 bytes"));
        let user = inspect(&session, 4, false, false).expect("user");
        assert!(user.details.contains("NDEF data area offset 0"));
        assert!(user.can_write);
        let config = inspect(&session, 41, true, false).expect("config");
        assert!(config.details.contains("AUTH0 255"));
        let password = inspect(&session, 43, true, false).expect("password");
        assert!(password.details.contains("write-only"));
        assert!(password.can_write);
        let manufacturer = inspect(&session, 0, true, false).expect("uid");
        assert!(!manufacturer.can_write);
        assert!(
            write_gate(&session, 0, true, false)
                .expect_err("manufacturer page")
                .contains("manufacturer")
        );
    }

    #[test]
    fn locked_and_read_only_pages_are_refused() {
        let mut session = session();
        session
            .image
            .set_page(2, [0x80, 0x48, 0x10, 0x00])
            .expect("lock");
        assert!(write_gate(&session, 4, false, false).is_err());
        assert!(write_gate(&session, 5, false, false).is_ok());

        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0xF0]).expect("CC");
        let layout = Type2Layout::from_cc(&cc).expect("layout");
        let mut tag = Type2Image::new(layout, cc, card(0x80).uid);
        for page in 0..45u8 {
            if let Some(data) = session.image.page(page) {
                tag.set_page(page, *data).expect("page");
            }
        }
        tag.set_page(3, [0xE1, 0x10, 0x12, 0xF0]).expect("CC");
        let session = Session::new(card(0x80), tag, Vec::new());
        let ndef = session.ndef().expect("NDEF");
        assert!(ndef.read_only);
        assert!(write_gate(&session, 3, true, false).is_err());
    }

    #[test]
    fn parses_pages_hex_and_lock_updates() {
        assert_eq!(
            parse_page_list("4, 6-8; 8").expect("pages"),
            vec![4, 6, 7, 8]
        );
        assert!(parse_page_list("").is_err());
        assert!(parse_page_list("8-4").is_err());
        assert_eq!(
            parse_hex4("AA bb:01 02").expect("hex"),
            [0xAA, 0xBB, 0x01, 0x02]
        );
        assert!(parse_hex4("AA").is_err());
        assert_eq!(ascii4(&[b'A', 0, b'Z', 0xFF]), "A.Z.");

        let session = session();
        let preview = lock_preview(&session, &[4, 16]).expect("preview");
        assert!(preview.update.static_changed);
        assert!(preview.update.dynamic_changed);
        assert_eq!(preview.static_before, [0x00, 0x00]);
        assert_eq!(preview.static_after, [0x10, 0x00]);
        assert_eq!(preview.dynamic_after, [0x01, 0x00, 0x00]);
    }

    #[test]
    fn ndef_area_and_password_gates_follow_the_layout() {
        let mut session = session();
        assert_eq!(ndef_area_pages(&session.image).len(), 36);
        assert!(ndef_locked_pages(&session).expect("locks").is_empty());
        assert_eq!(ndef_password_page(&session), None);

        session
            .image
            .set_page(42, [0x00, 0x00, 0x00, 0x00])
            .expect("cfg1");
        session
            .image
            .set_page(41, [0x00, 0x00, 0x00, 0x20])
            .expect("cfg0");
        session.refresh_ndef();
        assert_eq!(ndef_password_page(&session), Some(32));
        assert!(write_gate(&session, 32, false, false).is_err());

        session
            .image
            .set_page(2, [0x80, 0x48, 0x00, 0x10])
            .expect("lock");
        assert_eq!(ndef_locked_pages(&session).expect("locks"), vec![12]);
    }

    #[test]
    fn mock_products_render_their_layouts() {
        for (size, total, user_last, dynamic, cfg, pwd, pack, capacity) in [
            (0x12u8, 45u8, 39u8, 40u8, 41u8, 43u8, 44u8, 141usize),
            (0x3E, 135, 129, 130, 131, 133, 134, 491),
            (0x6D, 231, 225, 226, 227, 229, 230, 867),
        ] {
            let cc = CapabilityContainer::parse(&[0xE1, 0x10, size, 0x00]).expect("CC");
            let layout = Type2Layout::from_cc(&cc).expect("layout");
            let mut image = Type2Image::new(layout, cc, card(0x80).uid);
            for page in 0..layout.total_pages {
                image.set_page(page, [0; PAGE_SIZE]).expect("page");
            }
            image.set_page(3, cc.to_bytes()).expect("CC page");
            let mut area = vec![0u8; usize::from(cc.size)];
            area[..3].copy_from_slice(&[0x03, 0x00, 0xFE]);
            for index in 0..(usize::from(cc.size) / PAGE_SIZE) {
                let mut data = [0u8; PAGE_SIZE];
                data.copy_from_slice(&area[index * PAGE_SIZE..(index + 1) * PAGE_SIZE]);
                image.set_page(4 + index as u8, data).expect("NDEF page");
            }
            let session = Session::new(card(0x80), image, Vec::new());
            assert_eq!(session.image.layout.total_pages, total);
            assert_eq!(session.image.layout.user_last, user_last);
            assert_eq!(session.image.role(dynamic), PageRole::DynamicLock);
            assert_eq!(session.image.role(cfg), PageRole::Configuration);
            assert_eq!(session.image.role(pwd), PageRole::Password);
            assert_eq!(session.image.role(pack), PageRole::PasswordAck);
            assert_eq!(page_rows(&session, true).len(), usize::from(total));
            assert_eq!(
                session.ndef().expect("NDEF").capacity,
                capacity,
                "capacity for CC size {size:#04X}"
            );
        }
    }

    #[test]
    fn loaded_images_keep_their_uid_and_ndef_state() {
        let session = Session::from_image(image(), vec!["partial".to_owned()]);
        assert_eq!(session.card.card_type, CardType::Type2);
        assert_eq!(session.card.uid.len(), 7);
        assert!(matches!(
            session.ndef().expect("NDEF").state,
            NdefState::Empty
        ));
        assert_eq!(session.image.role(40), PageRole::DynamicLock);
    }
}
