//! Static and dynamic lock bytes.

use nfc_core::{NfcError, Result};

use crate::layout::Type2Layout;
use crate::page::Type2Image;

/// Static lock bytes (page 2, bytes 2..4) and dynamic lock bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockBits {
    pub static_lock: [u8; 2],
    pub dynamic_lock: [u8; 3],
}

impl LockBits {
    /// Read the lock bytes from a memory image. Dynamic bytes are all zero for
    /// products without a dynamic lock page.
    pub fn from_image(image: &Type2Image) -> Result<Self> {
        let static_page = image
            .page(2)
            .ok_or_else(|| NfcError::InvalidArgument("page 2 is not readable".to_owned()))?;
        let dynamic_lock = match image.layout.dynamic_lock {
            Some(page) => {
                let data = image.page(page).ok_or_else(|| {
                    NfcError::InvalidArgument(format!("dynamic lock page {page} is not readable"))
                })?;
                [data[0], data[1], data[2]]
            }
            None => [0; 3],
        };
        Ok(Self {
            static_lock: [static_page[2], static_page[3]],
            dynamic_lock,
        })
    }

    pub fn is_static_locked(&self, layout: &Type2Layout, page: u8) -> bool {
        layout
            .static_lock_position(page)
            .is_some_and(|(byte, bit)| {
                self.static_lock
                    .get(byte)
                    .is_some_and(|value| value & (1 << bit) != 0)
            })
    }

    pub fn is_dynamic_locked(&self, layout: &Type2Layout, page: u8) -> bool {
        layout
            .dynamic_lock_position(page)
            .is_some_and(|(byte, bit)| {
                self.dynamic_lock
                    .get(byte)
                    .is_some_and(|value| value & (1 << bit) != 0)
            })
    }

    pub fn is_locked(&self, layout: &Type2Layout, page: u8) -> bool {
        self.is_static_locked(layout, page) || self.is_dynamic_locked(layout, page)
    }

    /// The minimum bytes that must be written to lock `pages`. Static locks are
    /// written through page 2, dynamic locks through the dynamic lock page.
    pub fn pending(&self, layout: &Type2Layout, pages: &[u8]) -> Result<LockUpdate> {
        let mut update = LockUpdate::default();
        for &page in pages {
            if !layout.contains(page) {
                return Err(NfcError::InvalidArgument(format!(
                    "page {page} is outside the tag"
                )));
            }
            if let Some((byte, bit)) = layout.static_lock_position(page) {
                update.static_lock[byte] |= 1 << bit;
                update.static_changed = true;
                continue;
            }
            if let Some((byte, bit)) = layout.dynamic_lock_position(page) {
                update.dynamic_lock[byte] |= 1 << bit;
                update.dynamic_changed = true;
                continue;
            }
            return Err(NfcError::Unsupported(
                "this page cannot be locked with a Type 2 lock byte",
            ));
        }
        update.static_changed &= update.static_lock != self.static_lock;
        update.dynamic_changed &= update.dynamic_lock != self.dynamic_lock;
        Ok(update)
    }

    /// Reject a requested lock value that tries to clear an already set bit.
    pub fn validate_irreversible(&self, requested: LockBits) -> Result<()> {
        for (index, (old, new)) in self
            .static_lock
            .iter()
            .zip(requested.static_lock.iter())
            .enumerate()
        {
            if old & !new != 0 {
                return Err(NfcError::InvalidArgument(format!(
                    "static lock byte {index} can only set bits, not clear them"
                )));
            }
        }
        for (index, (old, new)) in self
            .dynamic_lock
            .iter()
            .zip(requested.dynamic_lock.iter())
            .enumerate()
        {
            if old & !new != 0 {
                return Err(NfcError::InvalidArgument(format!(
                    "dynamic lock byte {index} can only set bits, not clear them"
                )));
            }
        }
        Ok(())
    }
}

/// One pending lock change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockUpdate {
    pub static_lock: [u8; 2],
    pub dynamic_lock: [u8; 3],
    pub static_changed: bool,
    pub dynamic_changed: bool,
}

impl LockUpdate {
    pub fn is_empty(&self) -> bool {
        !self.static_changed && !self.dynamic_changed
    }
}

#[cfg(test)]
mod tests {
    use crate::cc::CapabilityContainer;
    use crate::layout::Type2Layout;

    use super::*;

    fn ntag215() -> (Type2Layout, Type2Image) {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x3E, 0x00]).expect("CC");
        let layout = Type2Layout::from_cc(&cc).expect("layout");
        let image = Type2Image::new(layout, cc, vec![0; 7]);
        (layout, image)
    }

    #[test]
    fn static_and_dynamic_lock_checks_use_the_layout_positions() {
        let (layout, _) = ntag215();
        let locks = LockBits {
            static_lock: [0x08, 0x00],
            dynamic_lock: [0x01, 0x00, 0x00],
        };
        assert!(locks.is_static_locked(&layout, 3));
        assert!(!locks.is_static_locked(&layout, 4));
        assert!(locks.is_dynamic_locked(&layout, 16));
        assert!(!locks.is_dynamic_locked(&layout, 32));
    }

    #[test]
    fn pending_lock_update_sets_only_requested_bits() {
        let (layout, _) = ntag215();
        let locks = LockBits::default();
        let update = locks.pending(&layout, &[3, 16, 16]).expect("lock update");
        assert_eq!(update.static_lock, [0x08, 0x00]);
        assert_eq!(update.dynamic_lock, [0x01, 0x00, 0x00]);
        assert!(update.static_changed);
        assert!(update.dynamic_changed);
        assert!(!update.is_empty());
        assert!(locks.pending(&layout, &[2]).is_err());
    }

    #[test]
    fn requested_locks_cannot_clear_bits() {
        let current = LockBits {
            static_lock: [0x08, 0x00],
            dynamic_lock: [0x01, 0x00, 0x00],
        };
        let same = current;
        current.validate_irreversible(same).expect("same value");
        let cleared = LockBits {
            static_lock: [0x00, 0x00],
            dynamic_lock: [0x01, 0x00, 0x00],
        };
        assert!(current.validate_irreversible(cleared).is_err());
        let added = LockBits {
            static_lock: [0x0C, 0x00],
            dynamic_lock: [0x01, 0x00, 0x00],
        };
        current.validate_irreversible(added).expect("added bits");
    }
}
