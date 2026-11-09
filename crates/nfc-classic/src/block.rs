use nfc_core::{BLOCK_SIZE, CardInfo, NfcError, Result};

use crate::Layout;
use crate::access::AccessBits;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlockStatus {
    #[default]
    Unread,
    Read,
    AuthFailed,
    ReadFailed,
    Synthetic,
    Verified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub number: usize,
    pub sector: usize,
    pub data: [u8; BLOCK_SIZE],
    pub known_mask: u16,
    pub status: BlockStatus,
    pub error: Option<String>,
}

impl Block {
    pub fn new(layout: Layout, number: usize) -> Result<Self> {
        let sector = layout.sector_for_block(number)?;
        Ok(Self {
            number,
            sector,
            data: [0; BLOCK_SIZE],
            known_mask: 0,
            status: BlockStatus::Unread,
            error: None,
        })
    }

    /// Marks all 16 bytes as known card data.
    pub fn set_read(&mut self, data: [u8; BLOCK_SIZE]) {
        self.data = data;
        self.known_mask = u16::MAX;
        self.status = BlockStatus::Read;
        self.error = None;
    }

    pub fn complete(&self) -> bool {
        self.known_mask == u16::MAX
    }
}

/// A geometry-aware in-memory image. Unavailable bytes keep their block
/// address and a zero known-mask bit, so an unknown byte can never be confused
/// with a known zero byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dump {
    pub layout: Layout,
    pub card: Option<CardInfo>,
    pub blocks: Vec<Block>,
}

impl Dump {
    pub fn new(layout: Layout) -> Self {
        let blocks = (0..layout.block_count())
            .map(|number| {
                let sector = layout
                    .sector_for_block(number)
                    .expect("every block number in the layout has a sector");
                Block {
                    number,
                    sector,
                    data: [0; BLOCK_SIZE],
                    known_mask: 0,
                    status: BlockStatus::Unread,
                    error: None,
                }
            })
            .collect();
        Self {
            layout,
            card: None,
            blocks,
        }
    }

    /// Parses a complete raw image. Only the two supported sizes are accepted;
    /// trailer structure and BCC are not validated here.
    pub fn from_raw(raw: &[u8]) -> Result<Self> {
        let layout = match raw.len() {
            size if size == Layout::Classic1K.dump_size() => Layout::Classic1K,
            size if size == Layout::Classic4K.dump_size() => Layout::Classic4K,
            size => {
                return Err(NfcError::InvalidArgument(format!(
                    "raw MIFARE Classic dump must be exactly {} or {} bytes, got {size}",
                    Layout::Classic1K.dump_size(),
                    Layout::Classic4K.dump_size()
                )));
            }
        };
        let mut dump = Self::new(layout);
        for (number, block) in dump.blocks.iter_mut().enumerate() {
            let offset = number * BLOCK_SIZE;
            block.set_read(
                raw[offset..offset + BLOCK_SIZE]
                    .try_into()
                    .expect("block-sized slice"),
            );
        }
        Ok(dump)
    }

    pub fn with_card(mut self, card: CardInfo) -> Self {
        self.card = Some(card);
        self
    }

    /// Serializes a complete image. Partial images are rejected; unknown bytes
    /// are never zero-filled.
    pub fn raw(&self) -> Result<Vec<u8>> {
        if !self.complete() {
            return Err(NfcError::VerificationFailed(
                "MIFARE Classic dump is incomplete".to_owned(),
            ));
        }
        let mut raw = Vec::with_capacity(self.layout.dump_size());
        for block in &self.blocks {
            raw.extend_from_slice(&block.data);
        }
        Ok(raw)
    }

    pub fn validate_geometry(&self) -> Result<()> {
        if self.blocks.len() != self.layout.block_count() {
            return Err(NfcError::InvalidArgument(format!(
                "dump has {} blocks, expected {} for {}",
                self.blocks.len(),
                self.layout.block_count(),
                self.layout.name()
            )));
        }
        for (number, block) in self.blocks.iter().enumerate() {
            let expected_sector = self
                .layout
                .sector_for_block(number)
                .expect("index within the layout");
            if block.number != number || block.sector != expected_sector {
                return Err(NfcError::InvalidArgument(format!(
                    "block {number} has inconsistent address: number {} sector {}",
                    block.number, block.sector
                )));
            }
        }
        Ok(())
    }

    pub fn complete(&self) -> bool {
        self.validate_geometry().is_ok() && self.blocks.iter().all(Block::complete)
    }

    pub fn readability(&self) -> Readability {
        let total_blocks = self.layout.block_count();
        let mut readable = vec![false; total_blocks];
        for (number, block) in self.blocks.iter().take(total_blocks).enumerate() {
            if block.number == number && block.complete() {
                readable[number] = true;
            }
        }
        let readable_blocks = readable.iter().filter(|readable| **readable).count();
        let mut skipped_sectors = Vec::new();
        let mut readable_sectors = 0;
        for sector in 0..self.layout.sector_count() {
            let first = self
                .layout
                .first_block(sector)
                .expect("sector within the layout");
            let count = self
                .layout
                .blocks_in_sector(sector)
                .expect("sector within the layout");
            if readable[first..first + count].iter().all(|known| *known) {
                readable_sectors += 1;
            } else {
                skipped_sectors.push(sector);
            }
        }
        Readability {
            readable_blocks,
            total_blocks,
            readable_sectors,
            total_sectors: self.layout.sector_count(),
            skipped_sectors,
        }
    }

    /// Decodes the access bytes of one sector trailer. Bytes 6..9 must be known;
    /// masked key bytes do not prevent this check.
    pub fn access_bits(&self, sector: usize) -> Result<AccessBits> {
        let trailer = self.layout.trailer_block(sector)?;
        let block = self
            .blocks
            .get(trailer)
            .ok_or_else(|| NfcError::InvalidArgument("dump does not contain the trailer".into()))?;
        let access_mask = 0b0000_0001_1100_0000;
        if block.known_mask & access_mask != access_mask {
            return Err(NfcError::VerificationFailed(format!(
                "sector {sector} trailer access bytes are unknown"
            )));
        }
        AccessBits::decode([block.data[6], block.data[7], block.data[8]])
            .map_err(|error| NfcError::InvalidArgument(format!("sector {sector} trailer: {error}")))
    }

    /// Validates the access-bit complements of every sector trailer. Trailers
    /// whose access bytes are unknown are reported, not treated as valid.
    pub fn validate_trailers(&self) -> Result<()> {
        self.validate_geometry()?;
        for sector in 0..self.layout.sector_count() {
            self.access_bits(sector)?;
        }
        Ok(())
    }
}

/// Counts fully known blocks and sectors. A sector is readable only when every
/// one of its blocks is fully known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readability {
    pub readable_blocks: usize,
    pub total_blocks: usize,
    pub readable_sectors: usize,
    pub total_sectors: usize,
    pub skipped_sectors: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_raw(layout: Layout) -> Vec<u8> {
        let mut raw = vec![0u8; layout.dump_size()];
        raw[0..4].copy_from_slice(&[1, 2, 3, 4]);
        raw[4] = raw[0] ^ raw[1] ^ raw[2] ^ raw[3];
        for sector in 0..layout.sector_count() {
            let block = layout.trailer_block(sector).expect("trailer");
            let offset = block * BLOCK_SIZE;
            raw[offset + 6..offset + 9].copy_from_slice(&[0xFF, 0x07, 0x80]);
        }
        raw
    }

    #[test]
    fn raw_round_trip_is_exact_for_both_layouts() {
        for layout in Layout::ALL {
            let raw = complete_raw(layout);
            let dump = Dump::from_raw(&raw).expect("complete raw dump");
            assert!(dump.complete());
            assert_eq!(dump.readability().readable_blocks, layout.block_count());
            assert_eq!(dump.readability().readable_sectors, layout.sector_count());
            assert!(dump.readability().skipped_sectors.is_empty());
            assert_eq!(dump.raw().expect("raw"), raw);
        }
    }

    #[test]
    fn raw_parser_rejects_other_sizes() {
        for size in [0, 1023, 1025, 4095, 4097] {
            assert!(Dump::from_raw(&vec![0u8; size]).is_err(), "size {size}");
        }
    }

    #[test]
    fn partial_dump_never_becomes_raw() {
        let dump = Dump::new(Layout::Classic1K);
        assert!(!dump.complete());
        assert!(dump.raw().is_err());
        let readability = dump.readability();
        assert_eq!(readability.readable_blocks, 0);
        assert_eq!(readability.skipped_sectors.len(), 16);
    }

    #[test]
    fn readability_counts_only_fully_known_sectors() {
        let mut dump = Dump::from_raw(&complete_raw(Layout::Classic1K)).expect("dump");
        dump.blocks[5].known_mask = 0;
        let readability = dump.readability();
        assert_eq!(readability.readable_blocks, 63);
        assert_eq!(readability.readable_sectors, 15);
        assert_eq!(readability.skipped_sectors, vec![1]);
    }

    #[test]
    fn trailer_validation_requires_access_bytes_but_not_masked_keys() {
        let mut dump = Dump::from_raw(&complete_raw(Layout::Classic1K)).expect("dump");
        dump.validate_trailers().expect("valid trailers");
        dump.blocks[7].known_mask &= !0b0000_0000_0011_1111;
        dump.validate_trailers()
            .expect("masked Key A does not hide access bits");
        dump.blocks[7].known_mask &= !0b0000_0001_1100_0000;
        assert!(dump.validate_trailers().is_err());
    }

    #[test]
    fn trailer_errors_name_the_sector() {
        let mut dump = Dump::from_raw(&complete_raw(Layout::Classic1K)).expect("dump");
        dump.blocks[7].data[6] ^= 1;
        let error = dump.validate_trailers().expect_err("invalid trailer");
        assert!(error.to_string().contains("sector 1"), "{error}");
    }

    #[test]
    fn classic_4k_validates_the_last_trailer() {
        let mut dump = Dump::from_raw(&complete_raw(Layout::Classic4K)).expect("dump");
        dump.validate_trailers().expect("valid trailers");
        dump.blocks[255].data[8] ^= 1;
        assert!(dump.validate_trailers().is_err());
    }
}
