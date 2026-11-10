//! NTAG21x configuration pages: MIRROR, AUTH0, ACCESS, PWD, and PACK.

use nfc_core::{NfcError, PAGE_SIZE, Result};

use crate::page::Type2Image;

/// Parsed configuration pages.
///
/// PWD and PACK are never readable from the tag; their fields are zero when
/// derived from an image. Write requests carry the secret separately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Configuration {
    pub mirror: u8,
    pub mirror_page: u8,
    pub auth0: u8,
    pub access: u8,
    pub password: [u8; PAGE_SIZE],
    pub pack: [u8; 2],
}

impl Configuration {
    pub fn from_image(image: &Type2Image) -> Result<Self> {
        let pages = image.layout.config.ok_or(NfcError::Unsupported(
            "this Type 2 tag has no NTAG configuration pages",
        ))?;
        let cfg0 = *image.page(pages.cfg0).ok_or_else(|| {
            NfcError::InvalidArgument(format!("configuration page {} is not readable", pages.cfg0))
        })?;
        let cfg1 = *image.page(pages.cfg1).ok_or_else(|| {
            NfcError::InvalidArgument(format!("configuration page {} is not readable", pages.cfg1))
        })?;
        let pwd = image.page(pages.pwd).copied().unwrap_or([0; PAGE_SIZE]);
        let pack = image.page(pages.pack).copied().unwrap_or([0; PAGE_SIZE]);
        Ok(Self {
            mirror: cfg0[0],
            mirror_page: cfg0[2],
            auth0: cfg0[3],
            access: cfg1[0],
            password: pwd,
            pack: [pack[0], pack[1]],
        })
    }

    pub const fn cfg_locked(self) -> bool {
        self.access & 0x40 != 0
    }

    pub const fn read_and_write_protected(self) -> bool {
        self.access & 0x80 != 0
    }

    pub const fn auth_lim(self) -> u8 {
        self.access & 0x07
    }

    /// First page protected by password verification, when enabled.
    pub const fn protected_from(self) -> Option<u8> {
        if self.auth0 == 0xFF {
            None
        } else {
            Some(self.auth0)
        }
    }

    pub const fn requires_password(self, page: u8) -> bool {
        matches!(self.protected_from(), Some(first) if page >= first)
    }

    pub const fn requires_password_for_read(self, page: u8) -> bool {
        self.requires_password(page) && self.read_and_write_protected()
    }

    /// Build the two configuration pages for a write request. RFUI bits stay
    /// zero and the mirror configuration is preserved unless overridden.
    pub fn config_pages(mirror: u8, mirror_page: u8, auth0: u8, access: u8) -> [[u8; 4]; 2] {
        [
            [mirror, 0x00, mirror_page, auth0],
            [access & 0xC7, 0x00, 0x00, 0x00],
        ]
    }

    pub fn password_page(password: [u8; PAGE_SIZE]) -> [u8; PAGE_SIZE] {
        password
    }

    pub fn pack_page(pack: [u8; 2]) -> [u8; PAGE_SIZE] {
        [pack[0], pack[1], 0x00, 0x00]
    }

    /// Expected read-back of a configuration page. PWD and PACK read as zero.
    pub fn readback(page_role: crate::layout::PageRole, written: [u8; 4]) -> [u8; 4] {
        match page_role {
            crate::layout::PageRole::Password | crate::layout::PageRole::PasswordAck => [0; 4],
            _ => written,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cc::CapabilityContainer;
    use crate::layout::PageRole;
    use crate::layout::Type2Layout;

    use super::*;

    fn image() -> Type2Image {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0x00]).expect("CC");
        let layout = Type2Layout::from_cc(&cc).expect("layout");
        let mut image = Type2Image::new(layout, cc, vec![0; 7]);
        image.set_page(41, [0x00, 0x00, 0x04, 0x20]).expect("cfg0");
        image.set_page(42, [0xC5, 0x00, 0x00, 0x00]).expect("cfg1");
        image
    }

    #[test]
    fn parses_auth0_access_and_protection() {
        let config = Configuration::from_image(&image()).expect("configuration");
        assert_eq!(config.auth0, 0x20);
        assert_eq!(config.mirror_page, 0x04);
        assert_eq!(config.access, 0xC5);
        assert!(config.cfg_locked());
        assert!(config.read_and_write_protected());
        assert_eq!(config.auth_lim(), 5);
        assert_eq!(config.protected_from(), Some(0x20));
        assert!(config.requires_password(0x20));
        assert!(config.requires_password_for_read(0x20));
        assert!(!config.requires_password(0x1F));
    }

    #[test]
    fn disabled_password_protection_is_explicit() {
        let mut image = image();
        image.set_page(41, [0, 0, 0, 0xFF]).expect("cfg0");
        let config = Configuration::from_image(&image).expect("configuration");
        assert_eq!(config.protected_from(), None);
        assert!(!config.requires_password(0));
    }

    #[test]
    fn config_page_builders_clear_rfui_and_capability_private_bits() {
        let [cfg0, cfg1] = Configuration::config_pages(0xFF, 0x04, 0x20, 0xFF);
        assert_eq!(cfg0, [0xFF, 0x00, 0x04, 0x20]);
        assert_eq!(cfg1, [0xC7, 0x00, 0x00, 0x00]);
        assert_eq!(
            Configuration::readback(PageRole::Password, [1, 2, 3, 4]),
            [0; 4]
        );
        assert_eq!(
            Configuration::readback(PageRole::Configuration, [1, 2, 3, 4]),
            [1, 2, 3, 4]
        );
    }
}
