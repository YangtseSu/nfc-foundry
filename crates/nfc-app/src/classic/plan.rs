//! Classic write planning and one-shot preflight tokens.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nfc_classic::{Dump, Layout};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, CardType, KeyType, NfcError, Result};

use super::backup::BackupStore;
use super::keys::KeyStore;
use super::uid::{ClassicUidPlan, UID_LEN};

/// How long a preflight plan may be used for a write.
pub const WRITE_PLAN_TTL: Duration = Duration::from_secs(120);

/// One-shot token for a preflight plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WriteToken(u64);

impl WriteToken {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for WriteToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:08X}", self.0)
    }
}

/// Block selection for a restore or workbench write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RestoreSelection {
    /// Every known block. This is the ordinary full-card restore.
    #[default]
    All,
    /// Only blocks the source changed relative to its baseline.
    ChangesOnly,
    /// Exactly these block numbers.
    Blocks(Vec<usize>),
}

impl RestoreSelection {
    pub fn label(&self) -> String {
        match self {
            Self::All => "all known blocks".to_owned(),
            Self::ChangesOnly => "changed blocks only".to_owned(),
            Self::Blocks(blocks) => format!("{} selected block(s)", blocks.len()),
        }
    }
}

/// One block the plan will write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedWriteBlock {
    pub block: usize,
    pub sector: usize,
    pub trailer: bool,
    pub old: [u8; BLOCK_SIZE],
    pub new: [u8; BLOCK_SIZE],
    /// Key types the current access conditions permit for this block.
    pub write_keys: WriteKeyTypes,
    pub access_note: String,
}

impl PlannedWriteBlock {
    pub const fn role_label(&self) -> &'static str {
        if self.trailer {
            "sector trailer"
        } else {
            "data block"
        }
    }
}

/// Accepted key types for one planned block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteKeyTypes {
    pub a: bool,
    pub b: bool,
}

impl WriteKeyTypes {
    pub const fn any(self) -> bool {
        self.a || self.b
    }

    pub const fn allows(self, key_type: KeyType) -> bool {
        match key_type {
            KeyType::A => self.a,
            KeyType::B => self.b,
        }
    }

    pub fn label(self) -> &'static str {
        match (self.a, self.b) {
            (true, true) => "Key A or B",
            (true, false) => "Key A",
            (false, true) => "Key B",
            (false, false) => "no key",
        }
    }
}

/// A validated write plan returned by preflight.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicWritePlan {
    pub token: WriteToken,
    pub layout: Layout,
    pub card: CardInfo,
    pub selection: RestoreSelection,
    /// Selected block numbers in ascending order, including unchanged and
    /// unknown selections.
    pub selected_blocks: Vec<usize>,
    /// Blocks that will actually be written, in execution order.
    pub blocks: Vec<PlannedWriteBlock>,
    pub unchanged_blocks: usize,
    pub skipped_unknown_blocks: usize,
    pub warnings: Vec<String>,
    /// Unix seconds after which the token is refused.
    pub expires_at: u64,
    /// Fingerprint of the source image the plan was built from.
    pub source_fingerprint: u64,
    /// Fingerprint of the card backup the plan was built from.
    pub card_fingerprint: u64,
    /// The continue policy the plan was built with.
    pub continue_on_failure: bool,
    /// Guarded UID write executed only after the full restore and its
    /// post-restore sector-0 verification succeed.
    pub uid: Option<ClassicUidPlan>,
}

impl ClassicWritePlan {
    pub fn trailer_writes(&self) -> usize {
        self.blocks.iter().filter(|block| block.trailer).count()
    }
}

/// Read-only request that produces a write plan.
pub struct ClassicWritePreflight<'a> {
    pub dump: Dump,
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    pub expected_card: Option<CardInfo>,
    pub selection: RestoreSelection,
    /// Source baseline for `ChangesOnly`. When absent, the live card is the
    /// baseline.
    pub baseline: Option<Dump>,
    pub continue_on_failure: bool,
    /// Target manufacturer-block UID. Only a full restore may carry one.
    pub uid_target: Option<[u8; UID_LEN]>,
    /// Advanced mode is required when `uid_target` is set.
    pub advanced: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicWritePreflight<'a> {
    pub fn new(dump: Dump, keys: &'a KeyStore) -> Self {
        Self {
            dump,
            keys,
            include_common_keys: true,
            expected_card: None,
            selection: RestoreSelection::All,
            baseline: None,
            continue_on_failure: false,
            uid_target: None,
            advanced: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn expected_card(mut self, card: CardInfo) -> Self {
        self.expected_card = Some(card);
        self
    }

    pub fn selection(mut self, selection: RestoreSelection) -> Self {
        self.selection = selection;
        self
    }

    pub fn baseline(mut self, baseline: Dump) -> Self {
        self.baseline = Some(baseline);
        self
    }

    pub fn continue_on_failure(mut self, continue_on_failure: bool) -> Self {
        self.continue_on_failure = continue_on_failure;
        self
    }

    /// Adds a UID target to a full restore. The UID is written only after the
    /// restore is complete and sector 0 has been verified.
    pub fn uid_target(mut self, uid: [u8; UID_LEN]) -> Self {
        self.uid_target = Some(uid);
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Request that executes a previously built plan.
pub struct ClassicWriteExecution<'a> {
    pub token: WriteToken,
    /// Current source image. Its fingerprint must match the preflight source.
    pub dump: Dump,
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    pub backup: &'a BackupStore,
    pub confirmed: bool,
    /// Required when the plan carries a UID target.
    pub advanced: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicWriteExecution<'a> {
    pub fn new(token: WriteToken, dump: Dump, keys: &'a KeyStore, backup: &'a BackupStore) -> Self {
        Self {
            token,
            dump,
            keys,
            include_common_keys: true,
            backup,
            confirmed: false,
            advanced: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn confirmed(mut self, confirmed: bool) -> Self {
        self.confirmed = confirmed;
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Parses `1,4,5-7` into a sorted, deduplicated block list.
pub fn parse_block_list(text: &str) -> Result<Vec<usize>> {
    let mut blocks = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = parse_block_number(start)?;
            let end: usize = parse_block_number(end)?;
            if start > end {
                return Err(NfcError::InvalidArgument(format!(
                    "block range {start}-{end} is reversed"
                )));
            }
            blocks.extend(start..=end);
        } else {
            blocks.push(parse_block_number(part)?);
        }
    }
    if blocks.is_empty() {
        return Err(NfcError::InvalidArgument(
            "block list is empty; use e.g. 1,4,5-7".to_owned(),
        ));
    }
    blocks.sort_unstable();
    blocks.dedup();
    Ok(blocks)
}

fn parse_block_number(text: &str) -> Result<usize> {
    text.trim().parse::<usize>().map_err(|_| {
        NfcError::InvalidArgument(format!("block list entry {text:?} is not a block number"))
    })
}

/// Validates that `dump` can be written to `card`'s layout.
pub fn validate_source(dump: &Dump, card: &CardInfo) -> Result<Layout> {
    dump.validate_geometry()?;
    let layout = match card.card_type {
        CardType::MifareClassic1K => Layout::Classic1K,
        CardType::MifareClassic4K => Layout::Classic4K,
        _ => {
            return Err(NfcError::Unsupported(
                "the selected card is not a MIFARE Classic 1K or 4K card",
            ));
        }
    };
    if dump.layout != layout {
        return Err(NfcError::InvalidArgument(format!(
            "dump layout {} does not match the card layout {}",
            dump.layout.name(),
            layout.name()
        )));
    }
    if let Some(source_card) = &dump.card
        && !source_card.same_identity(card)
    {
        return Err(NfcError::CardChanged);
    }
    Ok(layout)
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use nfc_core::NfcError;

    use super::*;

    #[test]
    fn block_lists_parse_ranges_and_reject_bad_input() {
        assert_eq!(
            parse_block_list("1,4,5-7").expect("list"),
            vec![1, 4, 5, 6, 7]
        );
        assert_eq!(parse_block_list(" 7 , 1,1 ").expect("list"), vec![1, 7]);
        assert!(matches!(
            parse_block_list("8-3"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_block_list("one"),
            Err(NfcError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_block_list(""),
            Err(NfcError::InvalidArgument(_))
        ));
    }

    #[test]
    fn write_key_labels_cover_all_slots() {
        assert_eq!(WriteKeyTypes { a: true, b: true }.label(), "Key A or B");
        assert_eq!(WriteKeyTypes { a: true, b: false }.label(), "Key A");
        assert_eq!(WriteKeyTypes { a: false, b: true }.label(), "Key B");
        assert!(!WriteKeyTypes { a: false, b: false }.any());
    }
}
