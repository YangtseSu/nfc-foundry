//! Editable Classic image with a baseline, dirty tracking, and undo.

use nfc_classic::{AccessBits, BccStatus, Block, Dump, Layout};
use nfc_core::{BLOCK_SIZE, NfcError, Result};

/// One reversible block change.
#[derive(Clone, Debug, PartialEq, Eq)]
struct UndoEntry {
    block: usize,
    previous: Block,
}

/// Hardware-free edit session over one Classic image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicWorkbench {
    baseline: Dump,
    working: Dump,
    undo: Vec<UndoEntry>,
}

/// Snapshot handed to a view. The view never mutates the model directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkbenchSnapshot {
    pub dump: Dump,
    pub dirty_blocks: Vec<usize>,
    pub undo_depth: usize,
}

impl WorkbenchSnapshot {
    pub fn is_dirty(&self, block: usize) -> bool {
        self.dirty_blocks.binary_search(&block).is_ok()
    }

    pub fn is_saved(&self) -> bool {
        self.dirty_blocks.is_empty()
    }
}

/// Result of merging a partial read into the workbench.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkbenchMergeReport {
    /// Blocks whose new data replaced an unknown working block.
    pub filled: Vec<usize>,
    /// Non-dirty blocks refreshed from the card.
    pub refreshed: Vec<usize>,
    /// Dirty edits that the card confirmed byte for byte.
    pub reconciled: Vec<usize>,
    /// Blocks already equal to the read.
    pub unchanged: usize,
    /// Dirty edits that differ from the card read and were kept.
    pub conflicts: Vec<usize>,
    /// Read blocks that were incomplete and could not be merged.
    pub ignored: usize,
}

impl WorkbenchMergeReport {
    pub fn changed(&self) -> bool {
        !self.filled.is_empty() || !self.refreshed.is_empty() || !self.reconciled.is_empty()
    }
}

/// Validation details for the working image.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkbenchValidation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub dirty_blocks: Vec<usize>,
}

impl WorkbenchValidation {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

impl ClassicWorkbench {
    /// Creates a workbench from a complete or partial image. The image must
    /// have consistent geometry.
    pub fn new(dump: Dump) -> Result<Self> {
        dump.validate_geometry()?;
        Ok(Self {
            baseline: dump.clone(),
            working: dump,
            undo: Vec::new(),
        })
    }

    pub fn layout(&self) -> Layout {
        self.working.layout
    }

    pub fn baseline(&self) -> &Dump {
        &self.baseline
    }

    pub fn working(&self) -> &Dump {
        &self.working
    }

    pub fn dirty_blocks(&self) -> Vec<usize> {
        dirty_between(&self.baseline, &self.working)
    }

    pub fn snapshot(&self) -> WorkbenchSnapshot {
        WorkbenchSnapshot {
            dump: self.working.clone(),
            dirty_blocks: self.dirty_blocks(),
            undo_depth: self.undo.len(),
        }
    }

    pub fn fingerprint(&self) -> u64 {
        dump_fingerprint(&self.working)
    }

    /// Replaces one complete block. A no-op edit is accepted but adds no undo
    /// entry. Manufacturer block 0 is not editable.
    pub fn edit_block(&mut self, block: usize, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let current = self.working.blocks.get(block).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "block {block} is outside the {} layout",
                self.working.layout.name()
            ))
        })?;
        if block == 0 {
            return Err(NfcError::Unsupported(
                "manufacturer block 0 is read-only in the workbench",
            ));
        }
        if current.complete() && current.data == data {
            return Ok(());
        }
        self.push_undo(block);
        self.working.blocks[block].set_read(data);
        Ok(())
    }

    /// Edits one block from exactly 16 ASCII characters.
    pub fn edit_ascii_block(&mut self, block: usize, text: &str) -> Result<()> {
        let bytes = text.as_bytes();
        if bytes.len() != BLOCK_SIZE || !bytes.iter().all(u8::is_ascii) {
            return Err(NfcError::InvalidArgument(format!(
                "ASCII block must be exactly {BLOCK_SIZE} ASCII characters"
            )));
        }
        let mut data = [0u8; BLOCK_SIZE];
        data.copy_from_slice(bytes);
        self.edit_block(block, data)
    }

    /// Undoes the most recent block edit.
    pub fn undo(&mut self) -> Result<()> {
        let entry = self
            .undo
            .pop()
            .ok_or_else(|| NfcError::InvalidArgument("there is no edit to undo".to_owned()))?;
        if let Some(block) = self.working.blocks.get_mut(entry.block) {
            *block = entry.previous;
        }
        Ok(())
    }

    /// Discards every edit and restores the baseline image.
    pub fn revert(&mut self) -> Result<()> {
        self.working = self.baseline.clone();
        self.undo.clear();
        Ok(())
    }

    /// Marks the current working image as the saved baseline.
    pub fn mark_saved(&mut self) {
        self.baseline = self.working.clone();
        self.undo.clear();
    }

    /// Updates the baseline for blocks the card accepted and read back.
    pub fn mark_written(&mut self, blocks: &[usize]) -> Result<()> {
        for &number in blocks {
            let Some(block) = self.working.blocks.get_mut(number) else {
                return Err(NfcError::InvalidArgument(format!(
                    "block {number} is outside the {} layout",
                    self.working.layout.name()
                )));
            };
            block.status = nfc_classic::BlockStatus::Verified;
            self.baseline.blocks[number] = block.clone();
        }
        Ok(())
    }

    /// Merges a partial or complete read. Dirty blocks are kept on conflict.
    pub fn merge_read(&mut self, read: &Dump) -> Result<WorkbenchMergeReport> {
        read.validate_geometry()?;
        if read.layout != self.working.layout {
            return Err(NfcError::InvalidArgument(format!(
                "read layout {} does not match the workbench layout {}",
                read.layout.name(),
                self.working.layout.name()
            )));
        }
        if let (Some(workbench), Some(read)) = (&self.working.card, &read.card)
            && !workbench.same_identity(read)
        {
            return Err(NfcError::CardChanged);
        }
        let mut report = WorkbenchMergeReport::default();
        for (number, source) in read.blocks.iter().enumerate() {
            if !source.complete() {
                report.ignored += 1;
                continue;
            }
            let working_complete = self.working.blocks[number].complete();
            let working_data = self.working.blocks[number].data;
            let dirty = !blocks_equal(&self.baseline.blocks[number], &self.working.blocks[number]);
            if working_complete && working_data == source.data {
                if dirty {
                    // The card already holds the edit; clear the dirty mark
                    // without touching the working bytes.
                    self.baseline.blocks[number] = self.working.blocks[number].clone();
                    report.reconciled.push(number);
                } else {
                    report.unchanged += 1;
                }
                continue;
            }
            if dirty {
                report.conflicts.push(number);
                continue;
            }
            let sector = self.working.blocks[number].sector;
            self.push_undo(number);
            let mut merged = source.clone();
            merged.number = number;
            merged.sector = sector;
            self.working.blocks[number] = merged.clone();
            if working_complete {
                report.refreshed.push(number);
            } else {
                report.filled.push(number);
            }
            self.baseline.blocks[number] = merged;
        }
        Ok(report)
    }

    /// Merges one freshly read block. The block reads as complete data.
    pub fn merge_block(
        &mut self,
        block: usize,
        data: [u8; BLOCK_SIZE],
    ) -> Result<WorkbenchMergeReport> {
        if self.working.blocks.get(block).is_none() {
            return Err(NfcError::InvalidArgument(format!(
                "block {block} is outside the {} layout",
                self.working.layout.name()
            )));
        }
        let mut read = Dump::new(self.working.layout);
        read.blocks[block].set_read(data);
        if let Some(card) = &self.working.card {
            read.card = Some(card.clone());
        }
        self.merge_read(&read)
    }

    /// Validates the working image without touching a reader.
    pub fn validate(&self) -> WorkbenchValidation {
        let mut validation = WorkbenchValidation {
            dirty_blocks: self.dirty_blocks(),
            ..WorkbenchValidation::default()
        };
        if let Err(error) = self.working.validate_geometry() {
            validation.errors.push(error.to_string());
            return validation;
        }
        let mut unknown_blocks = 0;
        for block in &self.working.blocks {
            if !block.complete() {
                unknown_blocks += 1;
            }
        }
        if unknown_blocks > 0 {
            validation.warnings.push(format!(
                "{unknown_blocks} block(s) are unknown; they cannot be written"
            ));
        }
        for sector in 0..self.working.layout.sector_count() {
            let trailer = self
                .working
                .layout
                .trailer_block(sector)
                .expect("sector is within the layout");
            let block = &self.working.blocks[trailer];
            let access_mask = 0b0000_0001_1100_0000;
            if block.known_mask & access_mask != access_mask {
                validation
                    .warnings
                    .push(format!("sector {sector} trailer access bytes are unknown"));
                continue;
            }
            if let Err(error) = AccessBits::decode([block.data[6], block.data[7], block.data[8]]) {
                validation
                    .errors
                    .push(format!("sector {sector} trailer: {error}"));
            }
        }
        let uid_length = self.working.card.as_ref().map(|card| card.uid.len());
        match uid_length {
            Some(length) => match self.working.validate_bcc(length) {
                Ok(BccStatus::Valid) | Ok(BccStatus::NotApplicable) => {}
                Ok(BccStatus::Invalid { expected, actual }) => validation.errors.push(format!(
                    "manufacturer BCC is {actual:02X}, expected {expected:02X}"
                )),
                Err(error) => validation.warnings.push(error.to_string()),
            },
            None => validation.warnings.push(
                "card UID length is unknown; the manufacturer BCC was not checked".to_owned(),
            ),
        }
        validation
    }

    fn push_undo(&mut self, block: usize) {
        self.undo.push(UndoEntry {
            block,
            previous: self.working.blocks[block].clone(),
        });
    }
}

/// Stable 64-bit fingerprint of the image bytes and identity.
pub fn dump_fingerprint(dump: &Dump) -> u64 {
    let mut hash = FNV_OFFSET;
    fnv_bytes(&mut hash, dump.layout.name().as_bytes());
    match &dump.card {
        Some(card) => {
            fnv_bytes(&mut hash, b"card");
            fnv_bytes(&mut hash, &card.uid);
            fnv_bytes(&mut hash, &card.atqa);
            fnv_bytes(&mut hash, &[card.sak]);
        }
        None => fnv_bytes(&mut hash, b"no-card"),
    }
    for block in &dump.blocks {
        fnv_bytes(&mut hash, &block.data);
        fnv_bytes(&mut hash, &block.known_mask.to_le_bytes());
    }
    hash
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv_bytes(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn blocks_equal(left: &Block, right: &Block) -> bool {
    left.known_mask == right.known_mask && left.data == right.data
}

fn dirty_between(baseline: &Dump, working: &Dump) -> Vec<usize> {
    baseline
        .blocks
        .iter()
        .zip(&working.blocks)
        .enumerate()
        .filter(|(_, (left, right))| !blocks_equal(left, right))
        .map(|(number, _)| number)
        .collect()
}

#[cfg(test)]
mod tests {
    use nfc_classic::{Layout, bcc_for_uid};
    use nfc_core::{CardInfo, CardType};

    use super::*;

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0xA2, 0x24, 0x8A],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )
    }

    fn complete_dump() -> Dump {
        let mut dump = Dump::new(Layout::Classic1K).with_card(card());
        for block in 0..dump.blocks.len() {
            dump.blocks[block].set_read([block as u8; BLOCK_SIZE]);
        }
        dump.blocks[0].data[0..4].copy_from_slice(&[0x04, 0xA2, 0x24, 0x8A]);
        dump.blocks[0].data[4] = bcc_for_uid(&[0x04, 0xA2, 0x24, 0x8A]).expect("bcc");
        for block in (3..64).step_by(4) {
            dump.blocks[block].data[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
        }
        dump
    }

    #[test]
    fn edit_undo_revert_and_dirty_tracking() {
        let mut workbench = ClassicWorkbench::new(complete_dump()).expect("workbench");
        assert!(workbench.dirty_blocks().is_empty());
        workbench.edit_block(4, [0xAA; BLOCK_SIZE]).expect("edit");
        assert_eq!(workbench.dirty_blocks(), vec![4]);
        assert_eq!(workbench.snapshot().undo_depth, 1);
        workbench
            .edit_ascii_block(5, "0123456789ABCDEF")
            .expect("ascii");
        assert_eq!(workbench.dirty_blocks(), vec![4, 5]);
        workbench.undo().expect("undo ascii");
        assert_eq!(workbench.dirty_blocks(), vec![4]);
        workbench.undo().expect("undo data");
        assert!(workbench.dirty_blocks().is_empty());
        assert!(workbench.undo().is_err());

        workbench.edit_block(6, [0x55; BLOCK_SIZE]).expect("edit");
        workbench.revert().expect("revert");
        assert!(workbench.dirty_blocks().is_empty());
        assert_eq!(workbench.snapshot().undo_depth, 0);
        assert!(workbench.edit_block(0, [0; BLOCK_SIZE]).is_err());
        assert!(workbench.edit_ascii_block(4, "short").is_err());
    }

    #[test]
    fn saved_and_written_baselines_clear_only_the_relevant_blocks() {
        let mut workbench = ClassicWorkbench::new(complete_dump()).expect("workbench");
        workbench.edit_block(4, [1; BLOCK_SIZE]).expect("edit");
        workbench.edit_block(5, [2; BLOCK_SIZE]).expect("edit");
        workbench.mark_written(&[4]).expect("written");
        assert_eq!(workbench.dirty_blocks(), vec![5]);
        assert_eq!(
            workbench.working().blocks[4].status,
            nfc_classic::BlockStatus::Verified
        );
        workbench.mark_saved();
        assert!(workbench.dirty_blocks().is_empty());
        assert_eq!(workbench.snapshot().undo_depth, 0);
    }

    #[test]
    fn merge_fills_unknown_blocks_and_preserves_dirty_conflicts() {
        let mut dump = complete_dump();
        for block in [4usize, 5, 6] {
            dump.blocks[block].known_mask = 0;
        }
        let mut workbench = ClassicWorkbench::new(dump).expect("workbench");
        workbench
            .edit_block(5, [0xEE; BLOCK_SIZE])
            .expect("dirty edit");
        let mut read = Dump::new(Layout::Classic1K).with_card(card());
        read.blocks[4].set_read([0x11; BLOCK_SIZE]);
        read.blocks[5].set_read([0x22; BLOCK_SIZE]);
        read.blocks[6].set_read([0x33; BLOCK_SIZE]);
        let report = workbench.merge_read(&read).expect("merge");
        assert_eq!(report.filled, vec![4, 6]);
        assert_eq!(report.conflicts, vec![5]);
        assert_eq!(workbench.working().blocks[5].data, [0xEE; BLOCK_SIZE]);
        assert_eq!(workbench.working().blocks[4].data, [0x11; BLOCK_SIZE]);
        assert_eq!(workbench.dirty_blocks(), vec![5]);
    }

    #[test]
    fn merge_reconciles_a_dirty_block_the_card_already_holds() {
        let mut workbench = ClassicWorkbench::new(complete_dump()).expect("workbench");
        workbench.edit_block(4, [0x44; BLOCK_SIZE]).expect("edit");
        let report = workbench.merge_block(4, [0x44; BLOCK_SIZE]).expect("merge");
        assert_eq!(report.reconciled, vec![4]);
        assert!(workbench.dirty_blocks().is_empty());
    }

    #[test]
    fn validation_reports_trailers_bcc_and_unknown_blocks() {
        let mut workbench = ClassicWorkbench::new(complete_dump()).expect("workbench");
        assert!(
            workbench.validate().is_valid(),
            "{:?}",
            workbench.validate()
        );
        workbench.working.blocks[7].data[6] ^= 1;
        assert!(!workbench.validate().is_valid());
        workbench.working.blocks[4].known_mask = 0;
        assert!(
            workbench
                .validate()
                .warnings
                .iter()
                .any(|warning| warning.contains("unknown"))
        );
    }

    #[test]
    fn fingerprint_changes_with_edits_and_identity() {
        let workbench = ClassicWorkbench::new(complete_dump()).expect("workbench");
        let before = workbench.fingerprint();
        let mut other = workbench.clone();
        other.edit_block(4, [0xFF; BLOCK_SIZE]).expect("edit");
        assert_ne!(before, other.fingerprint());
        let mut other_card = workbench.working().clone();
        other_card.card = Some(CardInfo::new(
            vec![0x04, 0xA2, 0x24, 0x8B],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        ));
        assert_ne!(before, dump_fingerprint(&other_card));
    }
}
