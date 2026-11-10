//! Product layouts for NTAG21x and MIFARE Ultralight Type 2 tags.

use nfc_core::{NfcError, Result};

use crate::cc::CapabilityContainer;

/// Type 2 product family derived from the capability container data area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type2Product {
    Ultralight,
    Ntag213,
    Ntag215,
    Ntag216,
    /// A Type 2 tag whose CC size does not match a known NTAG layout.
    Generic,
}

impl Type2Product {
    pub fn from_cc_size(size: u16) -> Self {
        match size {
            48 => Self::Ultralight,
            144 => Self::Ntag213,
            496 => Self::Ntag215,
            872 => Self::Ntag216,
            _ => Self::Generic,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Ultralight => "MIFARE Ultralight",
            Self::Ntag213 => "NTAG213",
            Self::Ntag215 => "NTAG215",
            Self::Ntag216 => "NTAG216",
            Self::Generic => "NFC Forum Type 2",
        }
    }

    /// Number of pages covered by one dynamic lock bit.
    pub const fn dynamic_lock_granularity(self) -> u16 {
        match self {
            Self::Ultralight => 0,
            Self::Ntag213 => 2,
            Self::Ntag215 | Self::Ntag216 => 16,
            Self::Generic => 0,
        }
    }
}

/// Configuration page addresses for NTAG21x tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigPages {
    pub cfg0: u8,
    pub cfg1: u8,
    pub pwd: u8,
    pub pack: u8,
}

/// The role of one page in the memory map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageRole {
    /// Pages 0 and 1: serial number and BCC bytes.
    Manufacturer,
    /// Page 2: UID bytes 0..2, internal byte, and static lock bytes.
    StaticLock,
    /// Page 3: capability container, or the Ultralight OTP page.
    CapabilityContainer,
    UserMemory,
    /// NTAG dynamic lock bytes.
    DynamicLock,
    Configuration,
    Password,
    PasswordAck,
    Reserved,
}

impl PageRole {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Manufacturer => "manufacturer",
            Self::StaticLock => "static lock",
            Self::CapabilityContainer => "CC",
            Self::UserMemory => "user memory",
            Self::DynamicLock => "dynamic lock",
            Self::Configuration => "configuration",
            Self::Password => "password",
            Self::PasswordAck => "PACK",
            Self::Reserved => "reserved",
        }
    }

    /// User memory and the capability container are the only pages that a
    /// normal NDEF workflow writes. Everything else needs an Advanced action.
    pub const fn is_advanced(self) -> bool {
        !matches!(self, Self::UserMemory)
    }
}

/// Page geometry for one Type 2 tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Type2Layout {
    pub product: Type2Product,
    pub total_pages: u8,
    pub user_first: u8,
    pub user_last: u8,
    pub dynamic_lock: Option<u8>,
    pub config: Option<ConfigPages>,
}

impl Type2Layout {
    /// Map a capability container to the product page geometry.
    pub fn from_cc(cc: &CapabilityContainer) -> Result<Self> {
        let product = Type2Product::from_cc_size(cc.size);
        let layout = match product {
            Type2Product::Ultralight => Self {
                product,
                total_pages: 16,
                user_first: 4,
                user_last: 15,
                dynamic_lock: None,
                config: None,
            },
            Type2Product::Ntag213 => Self {
                product,
                total_pages: 45,
                user_first: 4,
                user_last: 39,
                dynamic_lock: Some(40),
                config: Some(ConfigPages {
                    cfg0: 41,
                    cfg1: 42,
                    pwd: 43,
                    pack: 44,
                }),
            },
            Type2Product::Ntag215 => Self {
                product,
                total_pages: 135,
                user_first: 4,
                user_last: 129,
                dynamic_lock: Some(130),
                config: Some(ConfigPages {
                    cfg0: 131,
                    cfg1: 132,
                    pwd: 133,
                    pack: 134,
                }),
            },
            Type2Product::Ntag216 => Self {
                product,
                total_pages: 231,
                user_first: 4,
                user_last: 225,
                dynamic_lock: Some(226),
                config: Some(ConfigPages {
                    cfg0: 227,
                    cfg1: 228,
                    pwd: 229,
                    pack: 230,
                }),
            },
            Type2Product::Generic => {
                let user_pages = cc.size.div_ceil(4);
                let total_pages = u8::try_from(user_pages)
                    .ok()
                    .and_then(|pages| pages.checked_add(4))
                    .filter(|pages| *pages > 4 && usize::from(*pages) <= 256)
                    .ok_or_else(|| {
                        NfcError::InvalidArgument(format!(
                            "the Type 2 capability container size {} bytes cannot be mapped to pages",
                            cc.size
                        ))
                    })?;
                Self {
                    product,
                    total_pages,
                    user_first: 4,
                    user_last: total_pages - 1,
                    dynamic_lock: None,
                    config: None,
                }
            }
        };
        if layout.total_pages <= layout.user_last {
            return Err(NfcError::InvalidArgument(
                "the Type 2 layout ends before the user memory".to_owned(),
            ));
        }
        Ok(layout)
    }

    pub const fn page_count(&self) -> usize {
        self.total_pages as usize
    }

    pub const fn user_pages(&self) -> u16 {
        (self.user_last - self.user_first + 1) as u16
    }

    pub const fn user_bytes(&self) -> u16 {
        self.user_pages() * 4
    }

    pub const fn contains(&self, page: u8) -> bool {
        page < self.total_pages
    }

    pub fn role(&self, page: u8) -> PageRole {
        if page >= self.total_pages {
            return PageRole::Reserved;
        }
        if page < 2 {
            return PageRole::Manufacturer;
        }
        if page == 2 {
            return PageRole::StaticLock;
        }
        if page == 3 {
            return PageRole::CapabilityContainer;
        }
        if (self.user_first..=self.user_last).contains(&page) {
            return PageRole::UserMemory;
        }
        if Some(page) == self.dynamic_lock {
            return PageRole::DynamicLock;
        }
        if let Some(config) = self.config {
            if page == config.cfg0 || page == config.cfg1 {
                return PageRole::Configuration;
            }
            if page == config.pwd {
                return PageRole::Password;
            }
            if page == config.pack {
                return PageRole::PasswordAck;
            }
        }
        PageRole::Reserved
    }

    /// Static lock bit for a page. Pages 3..=15 map onto lock bytes 0 and 1.
    pub fn static_lock_position(&self, page: u8) -> Option<(usize, u8)> {
        if !(3..=15).contains(&page) {
            return None;
        }
        if page <= 7 {
            Some((0, page))
        } else {
            Some((1, page - 8))
        }
    }

    /// Dynamic lock bit for a user page, when the product has dynamic locks.
    pub fn dynamic_lock_position(&self, page: u8) -> Option<(usize, u8)> {
        match self.product {
            Type2Product::Ntag213 if (16..=39).contains(&page) => {
                let index = (page - 16) / 2;
                Some(((index / 8) as usize, index % 8))
            }
            Type2Product::Ntag215 if (16..=129).contains(&page) => {
                let index = if page <= 127 { (page - 16) / 16 } else { 7 };
                Some((0, index))
            }
            Type2Product::Ntag216 if (16..=225).contains(&page) => {
                let index = if page <= 143 {
                    (page - 16) / 16
                } else {
                    8 + (page - 144) / 16
                };
                Some(((index / 8) as usize, index % 8))
            }
            _ => None,
        }
    }

    /// Byte offset of the page in the NDEF data area, if it is inside it.
    pub fn ndef_offset(&self, page: u8, cc: &CapabilityContainer) -> Option<usize> {
        if page < self.user_first {
            return None;
        }
        let offset = usize::from(page - self.user_first) * 4 + usize::from(self.user_first - 4) * 4;
        (offset < usize::from(cc.size)).then_some(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(size: u8) -> CapabilityContainer {
        CapabilityContainer::parse(&[0xE1, 0x10, size, 0x00]).expect("CC")
    }

    #[test]
    fn maps_known_products() {
        let ntag213 = Type2Layout::from_cc(&cc(0x12)).expect("NTAG213");
        assert_eq!(ntag213.product, Type2Product::Ntag213);
        assert_eq!(ntag213.total_pages, 45);
        assert_eq!(ntag213.user_bytes(), 144);
        assert_eq!(ntag213.role(40), PageRole::DynamicLock);
        assert_eq!(ntag213.role(41), PageRole::Configuration);
        assert_eq!(ntag213.role(43), PageRole::Password);
        assert_eq!(ntag213.role(44), PageRole::PasswordAck);

        let ntag215 = Type2Layout::from_cc(&cc(0x3E)).expect("NTAG215");
        assert_eq!(ntag215.total_pages, 135);
        assert_eq!(ntag215.user_bytes(), 504);

        let ntag216 = Type2Layout::from_cc(&cc(0x6D)).expect("NTAG216");
        assert_eq!(ntag216.total_pages, 231);
        assert_eq!(ntag216.user_bytes(), 888);

        let ultralight = Type2Layout::from_cc(&cc(0x06)).expect("Ultralight");
        assert_eq!(ultralight.product, Type2Product::Ultralight);
        assert_eq!(ultralight.total_pages, 16);
        assert_eq!(ultralight.user_bytes(), 48);
    }

    #[test]
    fn generic_layout_rounds_up_to_pages() {
        let layout = Type2Layout::from_cc(&cc(0x0A)).expect("80-byte tag");
        assert_eq!(layout.product, Type2Product::Generic);
        assert_eq!(layout.user_bytes(), 80);
        assert_eq!(layout.total_pages, 24);
        assert_eq!(layout.role(23), PageRole::UserMemory);
    }

    #[test]
    fn static_lock_positions_follow_the_datasheet() {
        let layout = Type2Layout::from_cc(&cc(0x3E)).expect("NTAG215");
        assert_eq!(layout.static_lock_position(3), Some((0, 3)));
        assert_eq!(layout.static_lock_position(4), Some((0, 4)));
        assert_eq!(layout.static_lock_position(7), Some((0, 7)));
        assert_eq!(layout.static_lock_position(8), Some((1, 0)));
        assert_eq!(layout.static_lock_position(15), Some((1, 7)));
        assert_eq!(layout.static_lock_position(16), None);
    }

    #[test]
    fn dynamic_lock_positions_follow_each_product() {
        let ntag213 = Type2Layout::from_cc(&cc(0x12)).expect("NTAG213");
        assert_eq!(ntag213.dynamic_lock_position(16), Some((0, 0)));
        assert_eq!(ntag213.dynamic_lock_position(31), Some((0, 7)));
        assert_eq!(ntag213.dynamic_lock_position(39), Some((1, 3)));

        let ntag215 = Type2Layout::from_cc(&cc(0x3E)).expect("NTAG215");
        assert_eq!(ntag215.dynamic_lock_position(16), Some((0, 0)));
        assert_eq!(ntag215.dynamic_lock_position(129), Some((0, 7)));

        let ntag216 = Type2Layout::from_cc(&cc(0x6D)).expect("NTAG216");
        assert_eq!(ntag216.dynamic_lock_position(143), Some((0, 7)));
        assert_eq!(ntag216.dynamic_lock_position(144), Some((1, 0)));
        assert_eq!(ntag216.dynamic_lock_position(225), Some((1, 5)));
    }
}
