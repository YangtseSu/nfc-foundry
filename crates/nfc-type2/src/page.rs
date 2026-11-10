//! Page memory image with explicit unknown pages.

use nfc_core::{NfcError, PAGE_SIZE, Result};

use crate::cc::CapabilityContainer;
use crate::layout::{PageRole, Type2Layout};

/// One four-byte Type 2 page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub number: u8,
    pub data: [u8; PAGE_SIZE],
}

/// A partial or complete Type 2 memory image.
///
/// Unknown pages stay `None`; a read that failed is never turned into zero
/// bytes. The image is the single source for page inspection, backups, NDEF
/// extraction, and restore validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type2Image {
    pub layout: Type2Layout,
    pub cc: CapabilityContainer,
    pub uid: Vec<u8>,
    pages: Vec<Option<[u8; PAGE_SIZE]>>,
}

impl Type2Image {
    pub fn new(layout: Type2Layout, cc: CapabilityContainer, uid: Vec<u8>) -> Self {
        Self {
            layout,
            cc,
            uid,
            pages: vec![None; layout.page_count()],
        }
    }

    pub fn from_pages(
        layout: Type2Layout,
        cc: CapabilityContainer,
        uid: Vec<u8>,
        pages: Vec<Option<[u8; PAGE_SIZE]>>,
    ) -> Result<Self> {
        if pages.len() != layout.page_count() {
            return Err(NfcError::InvalidArgument(format!(
                "the Type 2 image has {} pages but the layout needs {}",
                pages.len(),
                layout.page_count()
            )));
        }
        Ok(Self {
            layout,
            cc,
            uid,
            pages,
        })
    }

    pub fn page(&self, page: u8) -> Option<&[u8; PAGE_SIZE]> {
        self.pages.get(usize::from(page))?.as_ref()
    }

    pub fn set_page(&mut self, page: u8, data: [u8; PAGE_SIZE]) -> Result<()> {
        let slot = self
            .pages
            .get_mut(usize::from(page))
            .ok_or_else(|| NfcError::InvalidArgument(format!("page {page} is outside the tag")))?;
        *slot = Some(data);
        Ok(())
    }

    pub fn clear_page(&mut self, page: u8) {
        if let Some(slot) = self.pages.get_mut(usize::from(page)) {
            *slot = None;
        }
    }

    pub fn pages(&self) -> impl Iterator<Item = (u8, Option<&[u8; PAGE_SIZE]>)> {
        self.pages
            .iter()
            .enumerate()
            .map(|(number, page)| (number as u8, page.as_ref()))
    }

    pub fn known_pages(&self) -> usize {
        self.pages.iter().filter(|page| page.is_some()).count()
    }

    pub fn is_complete(&self) -> bool {
        self.pages.iter().all(Option::is_some)
    }

    pub fn role(&self, page: u8) -> PageRole {
        self.layout.role(page)
    }

    /// Concatenate the NDEF data area. Returns `None` when any page in the
    /// area is unknown; a partial area is never treated as a message.
    pub fn ndef_area(&self) -> Option<Vec<u8>> {
        let size = usize::from(self.cc.size);
        let mut output = Vec::with_capacity(size);
        let mut page = self.layout.user_first;
        while output.len() < size {
            let data = self.page(page)?;
            output.extend_from_slice(data);
            page = page.checked_add(1)?;
        }
        output.truncate(size);
        Some(output)
    }

    /// Store a complete NDEF data area, page by page. The area length must fit
    /// the capability container and end on a page boundary.
    pub fn set_ndef_area(&mut self, area: &[u8]) -> Result<()> {
        if area.len() > usize::from(self.cc.size) {
            return Err(NfcError::InvalidArgument(format!(
                "NDEF area is {} bytes but the capability container allows {}",
                area.len(),
                self.cc.size
            )));
        }
        if !area.len().is_multiple_of(PAGE_SIZE) {
            return Err(NfcError::InvalidArgument(
                "the NDEF area must end on a page boundary".to_owned(),
            ));
        }
        let pages = area.len() / PAGE_SIZE;
        for index in 0..pages {
            let page = self
                .layout
                .user_first
                .checked_add(index as u8)
                .ok_or_else(|| NfcError::InvalidArgument("NDEF page overflows".to_owned()))?;
            let mut data = [0u8; PAGE_SIZE];
            data.copy_from_slice(&area[index * PAGE_SIZE..(index + 1) * PAGE_SIZE]);
            self.set_page(page, data)?;
        }
        Ok(())
    }

    pub fn page_role(&self, page: u8) -> PageRole {
        self.layout.role(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Type2Image {
        let cc = CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0x00]).expect("CC");
        let layout = Type2Layout::from_cc(&cc).expect("layout");
        Type2Image::new(layout, cc, vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80])
    }

    #[test]
    fn unknown_pages_stay_unknown() {
        let mut image = image();
        assert!(!image.is_complete());
        assert!(image.ndef_area().is_none());
        image.set_page(4, [1, 2, 3, 4]).expect("page 4");
        assert_eq!(image.page(4), Some(&[1, 2, 3, 4]));
        assert_eq!(image.known_pages(), 1);
        assert!(!image.is_complete());
    }

    #[test]
    fn ndef_area_requires_every_page_and_truncates_to_cc_size() {
        let mut image = image();
        for page in 4..=39 {
            image.set_page(page, [page; 4]).expect("page");
        }
        let area = image.ndef_area().expect("complete area");
        assert_eq!(area.len(), 144);
        assert_eq!(&area[..8], &[4, 4, 4, 4, 5, 5, 5, 5]);
        image.clear_page(39);
        assert!(image.ndef_area().is_none());
    }

    #[test]
    fn rejects_out_of_range_pages_and_odd_areas() {
        let mut image = image();
        assert!(image.set_page(45, [0; 4]).is_err());
        assert!(image.set_ndef_area(&[0; 5]).is_err());
        assert!(image.set_ndef_area(&[0; 148]).is_err());
    }
}
