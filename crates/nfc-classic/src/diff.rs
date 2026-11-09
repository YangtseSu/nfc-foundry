use nfc_core::{CardInfo, Key, NfcError, Result};

use crate::access::AccessBits;
use crate::{Dump, Layout};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffState {
    Equal,
    Changed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Manufacturer,
    Data,
    Trailer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteDiff {
    pub offset: usize,
    pub before: u8,
    pub after: u8,
    pub before_known: bool,
    pub after_known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrailerDiff {
    pub valid: bool,
    pub error: Option<String>,
    pub key_a_changed: bool,
    pub key_b_changed: bool,
    pub access_changed: bool,
    pub gpb_changed: bool,
    pub before_key_a: Key,
    pub after_key_a: Key,
    pub before_key_b: Key,
    pub after_key_b: Key,
    pub before_access: [u8; 3],
    pub after_access: [u8; 3],
    pub before_gpb: u8,
    pub after_gpb: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockDiff {
    pub block: usize,
    pub sector: usize,
    pub kind: BlockKind,
    pub state: DiffState,
    pub before_known_mask: u16,
    pub after_known_mask: u16,
    /// Byte differences and bytes whose known state differs. Equal known bytes
    /// are omitted.
    pub bytes: Vec<ByteDiff>,
    pub trailer: Option<TrailerDiff>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpDiff {
    pub layout: Layout,
    pub before_card: Option<CardInfo>,
    pub after_card: Option<CardInfo>,
    /// True only when both images carry a known identity and those identities
    /// differ. An unavailable identity is not reported as a change.
    pub identity_changed: bool,
    pub changed_blocks: usize,
    pub equal_blocks: usize,
    pub unknown_blocks: usize,
    pub trailer_changes: usize,
    pub blocks: Vec<BlockDiff>,
}

/// Compares two images block-wise without touching a reader. A block with any
/// unknown byte on either side is `Unknown`, even when its known bytes match.
pub fn diff(before: &Dump, after: &Dump) -> Result<DumpDiff> {
    if before.layout != after.layout {
        return Err(NfcError::InvalidArgument(format!(
            "dump layouts do not match: {} and {}",
            before.layout.name(),
            after.layout.name()
        )));
    }
    before.validate_geometry()?;
    after.validate_geometry()?;
    let identity_changed = match (&before.card, &after.card) {
        (Some(before), Some(after)) => !before.same_identity(after),
        _ => false,
    };
    let mut result = DumpDiff {
        layout: before.layout,
        before_card: before.card.clone(),
        after_card: after.card.clone(),
        identity_changed,
        changed_blocks: 0,
        equal_blocks: 0,
        unknown_blocks: 0,
        trailer_changes: 0,
        blocks: Vec::with_capacity(before.layout.block_count()),
    };
    for (number, (left, right)) in before.blocks.iter().zip(&after.blocks).enumerate() {
        let all_known = left.complete() && right.complete();
        let mut equal = all_known;
        let mut bytes = Vec::new();
        for offset in 0..nfc_core::BLOCK_SIZE {
            let mask = 1u16 << offset;
            let before_known = left.known_mask & mask != 0;
            let after_known = right.known_mask & mask != 0;
            if left.data[offset] != right.data[offset] || before_known != after_known {
                bytes.push(ByteDiff {
                    offset,
                    before: left.data[offset],
                    after: right.data[offset],
                    before_known,
                    after_known,
                });
                equal = false;
            }
        }
        let state = if !all_known {
            result.unknown_blocks += 1;
            DiffState::Unknown
        } else if equal {
            result.equal_blocks += 1;
            DiffState::Equal
        } else {
            result.changed_blocks += 1;
            DiffState::Changed
        };
        let kind = block_kind(before.layout, number)?;
        let trailer = if kind == BlockKind::Trailer {
            let trailer = trailer_diff(left, right);
            if trailer.key_a_changed
                || trailer.key_b_changed
                || trailer.access_changed
                || trailer.gpb_changed
            {
                result.trailer_changes += 1;
            }
            Some(trailer)
        } else {
            None
        };
        result.blocks.push(BlockDiff {
            block: number,
            sector: left.sector,
            kind,
            state,
            before_known_mask: left.known_mask,
            after_known_mask: right.known_mask,
            bytes,
            trailer,
        });
    }
    Ok(result)
}

fn block_kind(layout: Layout, block: usize) -> Result<BlockKind> {
    if block == 0 {
        return Ok(BlockKind::Manufacturer);
    }
    if layout.is_trailer(block)? {
        Ok(BlockKind::Trailer)
    } else {
        Ok(BlockKind::Data)
    }
}

fn trailer_diff(before: &crate::Block, after: &crate::Block) -> TrailerDiff {
    let before_key_a = Key::new(before.data[0..6].try_into().expect("six key bytes"));
    let after_key_a = Key::new(after.data[0..6].try_into().expect("six key bytes"));
    let before_key_b = Key::new(before.data[10..16].try_into().expect("six key bytes"));
    let after_key_b = Key::new(after.data[10..16].try_into().expect("six key bytes"));
    let before_access = [before.data[6], before.data[7], before.data[8]];
    let after_access = [after.data[6], after.data[7], after.data[8]];
    let before_gpb = before.data[9];
    let after_gpb = after.data[9];
    let key_a_known = before.known_mask & 0x003F == 0x003F && after.known_mask & 0x003F == 0x003F;
    let key_b_known = before.known_mask & 0xFC00 == 0xFC00 && after.known_mask & 0xFC00 == 0xFC00;
    let access_known = before.known_mask & 0x01C0 == 0x01C0 && after.known_mask & 0x01C0 == 0x01C0;
    let gpb_known = before.known_mask & 0x0200 != 0 && after.known_mask & 0x0200 != 0;
    let mut valid = false;
    let mut error_parts = Vec::new();
    if access_known {
        match AccessBits::decode(before_access) {
            Ok(_) => {}
            Err(error) => {
                error_parts.push(format!("before: {error}"));
            }
        }
        match AccessBits::decode(after_access) {
            Ok(_) => {}
            Err(error) => {
                error_parts.push(format!("after: {error}"));
            }
        }
        valid = error_parts.is_empty();
    } else {
        error_parts.push("access bytes are unknown".to_owned());
    }
    TrailerDiff {
        valid,
        error: (!error_parts.is_empty()).then(|| error_parts.join("; ")),
        key_a_changed: key_a_known && before_key_a != after_key_a,
        key_b_changed: key_b_known && before_key_b != after_key_b,
        access_changed: access_known && before_access != after_access,
        gpb_changed: gpb_known && before_gpb != after_gpb,
        before_key_a,
        after_key_a,
        before_key_b,
        after_key_b,
        before_access,
        after_access,
        before_gpb,
        after_gpb,
    }
}

#[cfg(test)]
mod tests {
    use nfc_core::{BLOCK_SIZE, CardInfo, CardType};

    use super::*;

    fn dump(layout: Layout) -> Dump {
        let mut dump = Dump::new(layout);
        for sector in 0..layout.sector_count() {
            let trailer = layout.trailer_block(sector).expect("trailer");
            dump.blocks[trailer].data[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
            dump.blocks[trailer].known_mask = u16::MAX;
        }
        for block in &mut dump.blocks {
            block.known_mask = u16::MAX;
            block.status = crate::BlockStatus::Read;
        }
        dump
    }

    #[test]
    fn identical_complete_dumps_are_equal() {
        let before = dump(Layout::Classic1K);
        let after = before.clone();
        let result = diff(&before, &after).expect("diff");
        assert_eq!(result.equal_blocks, 64);
        assert_eq!(result.changed_blocks, 0);
        assert_eq!(result.unknown_blocks, 0);
        assert_eq!(result.trailer_changes, 0);
        assert!(
            result
                .blocks
                .iter()
                .all(|block| block.state == DiffState::Equal)
        );
    }

    #[test]
    fn changed_bytes_and_trailer_fields_are_classified() {
        let before = dump(Layout::Classic1K);
        let mut after = before.clone();
        after.blocks[4].data[3] = 0x55;
        after.blocks[7].data[0] ^= 0x0F;
        after.blocks[7].data[9] ^= 0x01;
        after.blocks[7].data[10] ^= 0x0F;
        let result = diff(&before, &after).expect("diff");
        assert_eq!(result.changed_blocks, 2);
        assert_eq!(result.equal_blocks, 62);
        assert_eq!(result.blocks[4].state, DiffState::Changed);
        assert_eq!(result.blocks[4].bytes.len(), 1);
        assert_eq!(result.blocks[4].bytes[0].offset, 3);
        let trailer = result.blocks[7].trailer.as_ref().expect("trailer diff");
        assert!(trailer.key_a_changed);
        assert!(trailer.key_b_changed);
        assert!(trailer.gpb_changed);
        assert!(!trailer.access_changed);
        assert!(trailer.valid);
        assert!(trailer.error.is_none());
        assert_eq!(result.trailer_changes, 1);
    }

    #[test]
    fn unknown_bytes_force_unknown_state() {
        let mut before = dump(Layout::Classic1K);
        before.blocks[5].known_mask &= !0b100;
        let after = dump(Layout::Classic1K);
        let result = diff(&before, &after).expect("diff");
        assert_eq!(result.blocks[5].state, DiffState::Unknown);
        assert_eq!(result.unknown_blocks, 1);
        assert_eq!(result.equal_blocks, 63);
        assert_eq!(result.blocks[5].bytes[0].offset, 2);
        assert!(!result.blocks[5].bytes[0].before_known);
        assert!(result.blocks[5].bytes[0].after_known);
    }

    #[test]
    fn unknown_data_never_equals_a_known_zero_block() {
        let mut before = dump(Layout::Classic1K);
        before.blocks[9].known_mask = 0;
        before.blocks[9].data = [0; BLOCK_SIZE];
        let after = dump(Layout::Classic1K);
        let result = diff(&before, &after).expect("diff");
        assert_eq!(result.blocks[9].state, DiffState::Unknown);
        assert_ne!(result.blocks[9].state, DiffState::Equal);
    }

    #[test]
    fn identity_change_is_reported_only_when_both_are_known() {
        let base = dump(Layout::Classic1K);
        let card = |uid| CardInfo::new(uid, [0, 4], 8, CardType::MifareClassic1K);
        let mut before = base.clone();
        before.card = Some(card(vec![1, 2, 3, 4]));
        let mut after = base.clone();
        after.card = Some(card(vec![1, 2, 3, 5]));
        assert!(diff(&before, &after).expect("diff").identity_changed);
        after.card = Some(card(vec![1, 2, 3, 4]));
        assert!(!diff(&before, &after).expect("diff").identity_changed);
        after.card = None;
        assert!(!diff(&before, &after).expect("diff").identity_changed);
    }

    #[test]
    fn layout_mismatch_is_rejected() {
        let before = dump(Layout::Classic1K);
        let after = dump(Layout::Classic4K);
        assert!(diff(&before, &after).is_err());
    }
}
