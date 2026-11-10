//! MIFARE Application Directory 1 and 2 parsing and encoding.
//!
//! The layout follows AN10787. AID bytes are stored application code first,
//! function cluster second. The CRC covers the info byte followed by every
//! stored AID byte.

use nfc_classic::{Dump, Layout};
use nfc_core::{BLOCK_SIZE, NfcError, Result};

/// Function cluster code reserved for NDEF data.
pub const NFC_CLUSTER: u8 = 0xE1;
/// Application code reserved for NDEF data.
pub const NFC_APPLICATION: u8 = 0x03;
/// Public Key A of the MAD sectors.
pub const MAD_KEY_A: [u8; 6] = [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
/// Access bits of a MAD sector with write access granted.
pub const MAD_ACCESS_BITS: [u8; 3] = [0x78, 0x77, 0x88];
/// GPB of a MAD1 sector.
pub const MAD1_GPB: u8 = 0xC1;
/// GPB of a MAD2 sector 0.
pub const MAD2_GPB: u8 = 0xC2;

const MAD_DA: u8 = 0x80;
const MAD_VERSION_MASK: u8 = 0x03;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadVersion {
    Mad1,
    Mad2,
}

impl MadVersion {
    pub const fn gpb(self) -> u8 {
        match self {
            Self::Mad1 => MAD1_GPB,
            Self::Mad2 => MAD2_GPB,
        }
    }

    /// Number of directory entries addressable by this MAD version.
    pub const fn aid_count(self, layout: Layout) -> usize {
        match (self, layout) {
            (Self::Mad1, Layout::Classic1K | Layout::Classic4K) => 15,
            (Self::Mad2, Layout::Classic4K) => 38,
            (Self::Mad2, Layout::Classic1K) => 0,
        }
    }
}

/// One directory entry. Bytes are stored application code first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MadAid {
    pub function_cluster: u8,
    pub application: u8,
}

impl MadAid {
    pub const NFC: Self = Self {
        function_cluster: NFC_CLUSTER,
        application: NFC_APPLICATION,
    };
    pub const FREE: Self = Self {
        function_cluster: 0,
        application: 0,
    };

    pub const fn from_stored(bytes: [u8; 2]) -> Self {
        Self {
            application: bytes[0],
            function_cluster: bytes[1],
        }
    }

    pub const fn stored(self) -> [u8; 2] {
        [self.application, self.function_cluster]
    }

    pub const fn is_nfc(self) -> bool {
        self.function_cluster == NFC_CLUSTER && self.application == NFC_APPLICATION
    }
}

/// A parsed MIFARE Application Directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mad {
    pub version: MadVersion,
    pub info: u8,
    /// AID per sector, in ascending sector order. Sector 0 and the MAD2 sector
    /// 16 are not directory entries.
    pub entries: Vec<(u8, MadAid)>,
    pub warnings: Vec<String>,
}

impl Mad {
    /// Parses the MAD, returning `None` when the MAD-available bit is clear.
    pub fn read(dump: &Dump) -> Result<Option<Self>> {
        let layout = dump.layout;
        let trailer = dump_block(dump, layout.trailer_block(0)?, "MAD sector trailer")?;
        let gpb = trailer[9];
        if gpb & MAD_DA == 0 {
            return Ok(None);
        }
        let version = match gpb & MAD_VERSION_MASK {
            1 => MadVersion::Mad1,
            2 => MadVersion::Mad2,
            0 | 3 => {
                return Err(NfcError::InvalidArgument(format!(
                    "MAD version code {} is not supported",
                    gpb & MAD_VERSION_MASK
                )));
            }
            _ => unreachable!("MAD version mask is two bits"),
        };
        if version == MadVersion::Mad2 && layout != Layout::Classic4K {
            return Err(NfcError::InvalidArgument(
                "MAD2 requires a MIFARE Classic 4K layout".to_owned(),
            ));
        }

        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        let block1 = dump_block(dump, 1, "MAD block 1")?;
        let block2 = dump_block(dump, 2, "MAD block 2")?;
        let mut sector_zero = Vec::with_capacity(31);
        sector_zero.push(block1[1]);
        sector_zero.extend_from_slice(&block1[2..]);
        sector_zero.extend_from_slice(&block2);
        check_crc(block1[0], &sector_zero, "MAD1")?;

        let info = block1[1];
        entries.extend(directory_entries(&block1, &block2, 1));
        if version == MadVersion::Mad2 {
            let block_0x40 = dump_block(dump, 0x40, "MAD2 block 0x40")?;
            let block_0x41 = dump_block(dump, 0x41, "MAD2 block 0x41")?;
            let block_0x42 = dump_block(dump, 0x42, "MAD2 block 0x42")?;
            let trailer_0x10 = dump_block(dump, 0x43, "MAD2 sector trailer")?;
            if trailer_0x10[9] != 0 {
                warnings.push(format!(
                    "MAD2 sector 16 GPB is {:02X}, expected 00",
                    trailer_0x10[9]
                ));
            }
            let mut sector_sixteen = Vec::with_capacity(47);
            sector_sixteen.push(block_0x40[1]);
            sector_sixteen.extend_from_slice(&block_0x40[2..]);
            sector_sixteen.extend_from_slice(&block_0x41);
            sector_sixteen.extend_from_slice(&block_0x42);
            check_crc(block_0x40[0], &sector_sixteen, "MAD2")?;
            entries.extend(directory_entries(&block_0x40, &block_0x41, 17));
            entries.extend(eight_entries(&block_0x42, 32));
        }

        Ok(Some(Self {
            version,
            info,
            entries,
            warnings,
        }))
    }

    pub fn ndef_sectors(&self) -> Vec<u8> {
        self.entries
            .iter()
            .filter_map(|(sector, aid)| aid.is_nfc().then_some(*sector))
            .collect()
    }

    pub fn free_sectors(&self) -> Vec<u8> {
        self.entries
            .iter()
            .filter_map(|(sector, aid)| (*aid == MadAid::FREE).then_some(*sector))
            .collect()
    }

    pub fn aid(&self, sector: u8) -> Option<MadAid> {
        self.entries
            .iter()
            .find_map(|(entry_sector, aid)| (*entry_sector == sector).then_some(*aid))
    }

    /// Blank directory containing only NFC AIDs, ready for the format path.
    pub fn blank(layout: Layout, version: MadVersion, ndef_sectors: &[u8]) -> Result<Self> {
        if version == MadVersion::Mad2 && layout != Layout::Classic4K {
            return Err(NfcError::InvalidArgument(
                "MAD2 requires a MIFARE Classic 4K layout".to_owned(),
            ));
        }
        let aid_count = version.aid_count(layout);
        let mut entries = Vec::with_capacity(aid_count);
        let mut sector = 1u8;
        for _ in 0..aid_count {
            if sector == 16 {
                sector += 1;
            }
            entries.push((
                sector,
                if ndef_sectors.contains(&sector) {
                    MadAid::NFC
                } else {
                    MadAid::FREE
                },
            ));
            sector += 1;
        }
        Ok(Self {
            version,
            info: 0x00,
            entries,
            warnings: Vec::new(),
        })
    }

    /// Encodes every MAD data block. Sector trailers are not included.
    pub fn encode(&self, layout: Layout) -> Result<Vec<(usize, [u8; BLOCK_SIZE])>> {
        if self.version == MadVersion::Mad2 && layout != Layout::Classic4K {
            return Err(NfcError::InvalidArgument(
                "MAD2 requires a MIFARE Classic 4K layout".to_owned(),
            ));
        }
        let mut block1 = [0u8; BLOCK_SIZE];
        let mut block2 = [0u8; BLOCK_SIZE];
        block1[1] = self.info;
        for (sector, aid) in &self.entries {
            let slot = match *sector {
                1..=7 => Some((&mut block1, usize::from(*sector - 1) * 2 + 2)),
                8..=15 => Some((&mut block2, usize::from(*sector - 8) * 2)),
                17..=39 if self.version == MadVersion::Mad2 => continue,
                _ => None,
            };
            let Some((block, offset)) = slot else {
                return Err(NfcError::InvalidArgument(format!(
                    "sector {sector} has no MAD1 entry"
                )));
            };
            block[offset..offset + 2].copy_from_slice(&aid.stored());
        }
        let mut crc_input = Vec::with_capacity(31);
        crc_input.push(block1[1]);
        crc_input.extend_from_slice(&block1[2..]);
        crc_input.extend_from_slice(&block2);
        block1[0] = crc8(&crc_input);
        let mut blocks = vec![(1, block1), (2, block2)];

        if self.version == MadVersion::Mad2 {
            let mut block0 = [0u8; BLOCK_SIZE];
            let mut block1 = [0u8; BLOCK_SIZE];
            let mut block2 = [0u8; BLOCK_SIZE];
            block0[1] = self.info;
            for (sector, aid) in &self.entries {
                let slot = match *sector {
                    1..=15 => continue,
                    17..=23 => Some((&mut block0, usize::from(*sector - 17) * 2 + 2)),
                    24..=31 => Some((&mut block1, usize::from(*sector - 24) * 2)),
                    32..=39 => Some((&mut block2, usize::from(*sector - 32) * 2)),
                    _ => None,
                };
                let Some((block, offset)) = slot else {
                    return Err(NfcError::InvalidArgument(format!(
                        "sector {sector} has no MAD2 entry"
                    )));
                };
                block[offset..offset + 2].copy_from_slice(&aid.stored());
            }
            let mut crc_input = Vec::with_capacity(47);
            crc_input.push(block0[1]);
            crc_input.extend_from_slice(&block0[2..]);
            crc_input.extend_from_slice(&block1);
            crc_input.extend_from_slice(&block2);
            block0[0] = crc8(&crc_input);
            blocks.extend([(0x40, block0), (0x41, block1), (0x42, block2)]);
        }
        Ok(blocks)
    }
}

fn directory_entries(
    block_a: &[u8; BLOCK_SIZE],
    block_b: &[u8; BLOCK_SIZE],
    first: u8,
) -> Vec<(u8, MadAid)> {
    let mut entries = Vec::with_capacity(15);
    for index in 0..7 {
        let offset = (index + 1) * 2;
        entries.push((
            first + index as u8,
            MadAid::from_stored([block_a[offset], block_a[offset + 1]]),
        ));
    }
    for index in 0..8 {
        let offset = index * 2;
        entries.push((
            first + 7 + index as u8,
            MadAid::from_stored([block_b[offset], block_b[offset + 1]]),
        ));
    }
    entries
}

fn eight_entries(block: &[u8; BLOCK_SIZE], first: u8) -> Vec<(u8, MadAid)> {
    (0..8)
        .map(|index| {
            let offset = index * 2;
            (
                first + index as u8,
                MadAid::from_stored([block[offset], block[offset + 1]]),
            )
        })
        .collect()
}

fn dump_block(dump: &Dump, number: usize, label: &str) -> Result<[u8; BLOCK_SIZE]> {
    let block = dump
        .blocks
        .get(number)
        .ok_or_else(|| NfcError::InvalidArgument(format!("{label} is outside the dump")))?;
    if !block.complete() {
        return Err(NfcError::VerificationFailed(format!(
            "{label} (block {number}) is not readable"
        )));
    }
    Ok(block.data)
}

fn check_crc(expected: u8, input: &[u8], label: &str) -> Result<()> {
    let actual = crc8(input);
    if actual != expected {
        return Err(NfcError::InvalidArgument(format!(
            "{label} CRC is {expected:02X}, computed {actual:02X}"
        )));
    }
    Ok(())
}

/// CRC-8/MIFARE-MAD: poly 0x1D, init 0xC7, no reflection, no final XOR.
pub fn crc8(input: &[u8]) -> u8 {
    let mut crc = 0xC7u8;
    for byte in input {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x1D
            } else {
                crc << 1
            };
        }
    }
    crc
}
