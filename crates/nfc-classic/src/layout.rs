use nfc_core::{NfcError, Result};

/// Supported MIFARE Classic memory geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Classic1K,
    Classic4K,
}

impl Layout {
    pub const fn sector_count(self) -> usize {
        match self {
            Self::Classic1K => 16,
            Self::Classic4K => 40,
        }
    }

    pub const fn block_count(self) -> usize {
        match self {
            Self::Classic1K => 64,
            Self::Classic4K => 256,
        }
    }

    pub const fn dump_size(self) -> usize {
        self.block_count() * nfc_core::BLOCK_SIZE
    }

    pub fn sector_for_block(self, block: usize) -> Result<usize> {
        if block >= self.block_count() {
            return Err(NfcError::InvalidArgument(format!(
                "block {block} is outside the {:?} layout",
                self
            )));
        }
        match self {
            Self::Classic1K => Ok(block / 4),
            Self::Classic4K if block < 128 => Ok(block / 4),
            Self::Classic4K => Ok(32 + (block - 128) / 16),
        }
    }

    pub fn first_block(self, sector: usize) -> Result<usize> {
        if sector >= self.sector_count() {
            return Err(NfcError::InvalidArgument(format!(
                "sector {sector} is outside the {:?} layout",
                self
            )));
        }
        match self {
            Self::Classic1K => Ok(sector * 4),
            Self::Classic4K if sector < 32 => Ok(sector * 4),
            Self::Classic4K => Ok(128 + (sector - 32) * 16),
        }
    }

    pub fn blocks_in_sector(self, sector: usize) -> Result<usize> {
        if sector >= self.sector_count() {
            return Err(NfcError::InvalidArgument(format!(
                "sector {sector} is outside the {:?} layout",
                self
            )));
        }
        match self {
            Self::Classic1K => Ok(4),
            Self::Classic4K if sector < 32 => Ok(4),
            Self::Classic4K => Ok(16),
        }
    }

    pub fn trailer_block(self, sector: usize) -> Result<usize> {
        let first = self.first_block(sector)?;
        let count = self.blocks_in_sector(sector)?;
        Ok(first + count - 1)
    }

    pub fn is_trailer(self, block: usize) -> Result<bool> {
        let sector = self.sector_for_block(block)?;
        Ok(self.trailer_block(sector)? == block)
    }

    /// C1/C2/C3 group for a block. Group three is the sector trailer; large
    /// 4K sectors use five blocks per data group.
    pub fn access_group_for_block(self, block: usize) -> Result<usize> {
        let sector = self.sector_for_block(block)?;
        let first = self.first_block(sector)?;
        let count = self.blocks_in_sector(sector)?;
        let local = block - first;
        if local + 1 == count {
            return Ok(3);
        }
        if count == 16 {
            Ok(local / 5)
        } else {
            Ok(local)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_1k_geometry_is_stable() {
        assert_eq!(Layout::Classic1K.sector_count(), 16);
        assert_eq!(Layout::Classic1K.block_count(), 64);
        assert_eq!(Layout::Classic1K.first_block(1).expect("block"), 4);
        assert_eq!(Layout::Classic1K.trailer_block(1).expect("trailer"), 7);
        assert_eq!(
            Layout::Classic1K.access_group_for_block(6).expect("group"),
            2
        );
    }

    #[test]
    fn classic_4k_large_sectors_use_sixteen_blocks() {
        assert_eq!(Layout::Classic4K.sector_count(), 40);
        assert_eq!(Layout::Classic4K.block_count(), 256);
        assert_eq!(Layout::Classic4K.first_block(32).expect("block"), 128);
        assert_eq!(Layout::Classic4K.blocks_in_sector(32).expect("count"), 16);
        assert_eq!(Layout::Classic4K.trailer_block(39).expect("trailer"), 255);
        assert_eq!(
            Layout::Classic4K
                .access_group_for_block(142)
                .expect("group"),
            2
        );
    }

    #[test]
    fn rejects_blocks_outside_the_layout() {
        assert!(Layout::Classic1K.sector_for_block(64).is_err());
        assert!(Layout::Classic4K.first_block(40).is_err());
    }
}
