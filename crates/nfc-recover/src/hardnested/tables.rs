// SPDX-FileCopyrightText: 2015, 2016 piwi
// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Precalculated hardnested state tables.
//!
//! The table layout is the PM3 `client/resources/hardnested_tables/` layout:
//! `bitflip_<odd-even>_<bitflip>_states.bin[.lz4]`. Each file starts with a
//! 32-bit little-endian state count followed by 2^24 state bits. Tables whose
//! state count is at least 99.01% of the state space carry no useful
//! information and are ignored, as in PM3.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use lz4_flex::frame::FrameDecoder;
use nfc_core::{NfcError, Result};

use super::bitset::{BitSet24, STATE_COUNT, WORD_COUNT};

pub const NUM_BITFLIPS: usize = 0x400;
pub const BITFLIP_2ND_BYTE: u16 = 0x0200;
const IGNORE_BITFLIP_THRESHOLD: f64 = 0.9901;

#[derive(Clone)]
pub struct BitflipTable {
    pub bits: BitSet24,
    pub count: u32,
}

pub struct Tables {
    even: Vec<Option<BitflipTable>>,
    odd: Vec<Option<BitflipTable>>,
    counts: [[u32; NUM_BITFLIPS]; 2],
    /// Effective bitflips in PM3 application order: first-byte masks sorted by
    /// state-count product, then second-byte masks.
    pub all_effective: Vec<u16>,
    pub first_byte_effective: usize,
}

impl Tables {
    pub fn load(dir: &Path) -> Result<Self> {
        if !dir.is_dir() {
            return Err(NfcError::Io(format!(
                "hardnested table directory {} does not exist",
                dir.display()
            )));
        }

        let mut even = vec![None; NUM_BITFLIPS];
        let mut odd = vec![None; NUM_BITFLIPS];
        let mut counts = [[STATE_COUNT as u32; NUM_BITFLIPS]; 2];

        for (odd_even, counts_row) in counts.iter_mut().enumerate() {
            for bitflip in 0x001..NUM_BITFLIPS {
                let bitflip = bitflip as u16;
                let Some((bits, count)) = load_table(dir, odd_even, bitflip)? else {
                    continue;
                };
                let index = usize::from(bitflip);
                counts_row[index] = count;
                let table = BitflipTable { bits, count };
                if odd_even == 0 {
                    even[index] = Some(table);
                } else {
                    odd[index] = Some(table);
                }
            }
        }

        let mut effective_even: Vec<u16> = (0x001..NUM_BITFLIPS as u16)
            .filter(|bitflip| even[*bitflip as usize].is_some())
            .collect();
        let mut effective_odd: Vec<u16> = (0x001..NUM_BITFLIPS as u16)
            .filter(|bitflip| odd[*bitflip as usize].is_some())
            .collect();
        if effective_even.is_empty() || effective_odd.is_empty() {
            return Err(NfcError::Io(format!(
                "hardnested table directory {} contains no usable state tables",
                dir.display()
            )));
        }

        effective_even.sort_unstable();
        effective_odd.sort_unstable();
        let mut all_effective = Vec::with_capacity(effective_even.len() + effective_odd.len());
        let (mut i, mut j) = (0, 0);
        while i < effective_even.len() || j < effective_odd.len() {
            match (effective_even.get(i), effective_odd.get(j)) {
                (Some(&left), Some(&right)) if left == right => {
                    all_effective.push(left);
                    i += 1;
                    j += 1;
                }
                (Some(&left), Some(&right)) if left < right => {
                    all_effective.push(left);
                    i += 1;
                }
                (Some(_), Some(_)) => {
                    all_effective.push(effective_odd[j]);
                    j += 1;
                }
                (Some(&left), None) => {
                    all_effective.push(left);
                    i += 1;
                }
                (None, Some(&right)) => {
                    all_effective.push(right);
                    j += 1;
                }
                (None, None) => break,
            }
        }

        let first_byte_effective = all_effective
            .iter()
            .take_while(|bitflip| **bitflip & BITFLIP_2ND_BYTE == 0)
            .count();

        let product = |bitflip: u16| -> u64 {
            u64::from(counts[0][bitflip as usize]) * u64::from(counts[1][bitflip as usize])
        };
        all_effective[..first_byte_effective]
            .sort_unstable_by_key(|bitflip| (product(*bitflip), *bitflip));
        all_effective[first_byte_effective..]
            .sort_unstable_by_key(|bitflip| (product(*bitflip), *bitflip));

        Ok(Self {
            even,
            odd,
            counts,
            all_effective,
            first_byte_effective,
        })
    }

    pub fn table(&self, odd_even: usize, bitflip: u16) -> Option<&BitflipTable> {
        match odd_even {
            0 => self.even[bitflip as usize].as_ref(),
            1 => self.odd[bitflip as usize].as_ref(),
            _ => None,
        }
    }

    pub fn count(&self, odd_even: usize, bitflip: u16) -> u32 {
        self.counts[odd_even][bitflip as usize]
    }

    pub fn effective_count(&self) -> usize {
        self.all_effective.len()
    }
}

fn table_path(dir: &Path, odd_even: usize, bitflip: u16, extension: &str) -> PathBuf {
    dir.join(format!(
        "bitflip_{odd_even}_{bitflip:03x}_states.bin{extension}"
    ))
}

fn load_table(dir: &Path, odd_even: usize, bitflip: u16) -> Result<Option<(BitSet24, u32)>> {
    let compressed = table_path(dir, odd_even, bitflip, ".lz4");
    let raw = table_path(dir, odd_even, bitflip, "");

    let data = if compressed.is_file() {
        let file = File::open(&compressed).map_err(|error| {
            NfcError::Io(format!(
                "cannot open hardnested table {}: {error}",
                compressed.display()
            ))
        })?;
        let mut decoder = FrameDecoder::new(file);
        let mut data = Vec::with_capacity(WORD_COUNT * 4 + 4);
        decoder.read_to_end(&mut data).map_err(|error| {
            NfcError::Io(format!(
                "cannot decompress hardnested table {}: {error}",
                compressed.display()
            ))
        })?;
        data
    } else if raw.is_file() {
        std::fs::read(&raw).map_err(|error| {
            NfcError::Io(format!(
                "cannot read hardnested table {}: {error}",
                raw.display()
            ))
        })?
    } else {
        return Ok(None);
    };

    if data.len() != WORD_COUNT * 4 + 4 {
        return Err(NfcError::Io(format!(
            "hardnested table {} has {} bytes instead of {}",
            compressed.display(),
            data.len(),
            WORD_COUNT * 4 + 4
        )));
    }

    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if f64::from(count) / STATE_COUNT as f64 >= IGNORE_BITFLIP_THRESHOLD {
        return Ok(None);
    }

    let words: Vec<u32> = data[4..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    let bits = BitSet24::from_words(words);
    let actual = bits.count_ones();
    if actual != count {
        return Err(NfcError::Io(format!(
            "hardnested table {} declares {count} states but contains {actual}",
            if compressed.is_file() {
                compressed.display().to_string()
            } else {
                raw.display().to_string()
            }
        )));
    }
    Ok(Some((bits, count)))
}

/// Finds the table directory without relying on the process working
/// directory. An explicit path wins, then `NFC_FOUNDRY_HARDNESTED_TABLES`, the
/// installed resource directory, and finally a development checkout.
pub fn resolve_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = explicit {
        candidates.push(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("NFC_FOUNDRY_HARDNESTED_TABLES") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("NFC_FOUNDRY_RESOURCES") {
        candidates.push(PathBuf::from(path).join("hardnested_tables"));
    }
    candidates.push(PathBuf::from("/usr/share/nfc-foundry/hardnested_tables"));

    let mut push_ancestors = |start: &Path| {
        for ancestor in start.ancestors() {
            candidates.push(ancestor.join("resources/hardnested_tables"));
            candidates.push(ancestor.join("share/nfc-foundry/hardnested_tables"));
        }
    };
    if let Ok(cwd) = std::env::current_dir() {
        push_ancestors(&cwd);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        push_ancestors(parent);
    }

    for candidate in candidates {
        if contains_tables(&candidate) {
            return Ok(candidate);
        }
    }
    Err(NfcError::Io(
        "hardnested tables not found; install /usr/share/nfc-foundry/hardnested_tables or set NFC_FOUNDRY_HARDNESTED_TABLES"
            .to_owned(),
    ))
}

fn contains_tables(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("bitflip_") && name.contains("_states.bin")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_tables() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/hardnested_tables")
            .canonicalize()
            .expect("workspace hardnested tables")
    }

    #[test]
    fn loads_the_pm3_effective_tables() {
        let tables = Tables::load(&workspace_tables()).expect("tables");
        let even = (0x001..NUM_BITFLIPS as u16)
            .filter(|bitflip| tables.table(0, *bitflip).is_some())
            .count();
        let odd = (0x001..NUM_BITFLIPS as u16)
            .filter(|bitflip| tables.table(1, *bitflip).is_some())
            .count();
        assert_eq!(even, 145);
        assert_eq!(odd, 96);
        assert_eq!(tables.effective_count(), 239);
        assert_eq!(
            tables.first_byte_effective,
            tables
                .all_effective
                .iter()
                .take_while(|bitflip| **bitflip & BITFLIP_2ND_BYTE == 0)
                .count()
        );
        assert!(tables.first_byte_effective > 0);
        for &bitflip in &tables.all_effective {
            let has_table =
                tables.table(0, bitflip).is_some() || tables.table(1, bitflip).is_some();
            assert!(has_table, "bitflip {bitflip:03x} has no table");
        }
    }

    #[test]
    fn table_state_counts_match_their_bitsets() {
        let tables = Tables::load(&workspace_tables()).expect("tables");
        for &bitflip in &tables.all_effective {
            for odd_even in 0..2 {
                if let Some(table) = tables.table(odd_even, bitflip) {
                    assert_eq!(table.bits.count_ones(), table.count);
                }
            }
        }
    }

    #[test]
    fn missing_directory_is_reported() {
        let error = Tables::load(Path::new("/nonexistent/nfc-hardnested"))
            .err()
            .expect("missing directory");
        assert!(error.to_string().contains("does not exist"));
    }
}
