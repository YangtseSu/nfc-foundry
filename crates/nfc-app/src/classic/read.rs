//! Card reading: full dumps and single blocks.

use nfc_classic::{Dump, DumpKey, Layout};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, CardType, Key, KeyType, NfcError, Result};

use crate::operation::Operation;

use super::keys::{AuthenticatedKey, KeySource, KeyStore, common_keys};

/// Options for a full-card read.
pub struct ClassicRead<'a> {
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicRead<'a> {
    pub fn new(keys: &'a KeyStore) -> Self {
        Self {
            keys,
            include_common_keys: true,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// A partial or complete Classic dump with explicit warnings and the keys that
/// authenticated each read sector.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicReadResult {
    pub dump: Dump,
    pub keys: Vec<AuthenticatedKey>,
    pub warnings: Vec<String>,
}

/// Result of reading one block after authenticating its sector.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicBlockReadResult {
    pub card: CardInfo,
    pub data: [u8; BLOCK_SIZE],
    pub key: AuthenticatedKey,
}

/// Maps a selected card to its Classic layout. Other families are rejected
/// instead of guessing a layout from the UID.
pub fn layout_for_card(card: &CardInfo) -> Result<Layout> {
    match card.card_type {
        CardType::MifareClassic1K => Ok(Layout::Classic1K),
        CardType::MifareClassic4K => Ok(Layout::Classic4K),
        _ => Err(NfcError::Unsupported(
            "the selected card is not a MIFARE Classic 1K or 4K card",
        )),
    }
}

/// Candidate keys for one sector, including common factory keys when enabled.
/// Verified keys for this card are tried first.
pub fn candidate_keys(
    keys: &KeyStore,
    card: &CardInfo,
    include_common_keys: bool,
    sector: usize,
) -> Vec<(KeyType, Key, KeySource)> {
    let mut candidates = keys.candidates_for_card(card, sector);
    if include_common_keys {
        for key in common_keys() {
            for key_type in [KeyType::A, KeyType::B] {
                let candidate = (key_type, key, KeySource::Common);
                if !candidates
                    .iter()
                    .any(|existing| existing.0 == candidate.0 && existing.1 == candidate.1)
                {
                    candidates.push(candidate);
                }
            }
        }
    }
    candidates
}

/// Reads the complete card, stopping at the first non-recoverable card error.
/// Authentication failures are recorded per block and do not stop the read.
pub fn read_dump(
    operation: &mut Operation<'_>,
    keys: &KeyStore,
    include_common_keys: bool,
) -> Result<ClassicReadResult> {
    let started_at = now_seconds();
    let card = operation.card().clone();
    let layout = layout_for_card(&card)?;
    let mut dump = Dump::new(layout).with_card(card.clone());
    let mut authenticated_keys = Vec::new();
    let mut warnings = Vec::new();

    let mut unread_sectors = Vec::new();
    for sector in 0..layout.sector_count() {
        operation.cancel().check()?;
        let first = layout
            .first_block(sector)
            .expect("sector is within the layout");
        let count = layout
            .blocks_in_sector(sector)
            .expect("sector is within the layout");
        let mut authenticated = None;
        for (key_type, key, source) in candidate_keys(keys, &card, include_common_keys, sector) {
            operation.cancel().check()?;
            match operation.authenticate(first as u8, key_type, key) {
                Ok(()) => {
                    authenticated = Some(AuthenticatedKey {
                        sector,
                        key_type,
                        key,
                        source,
                    });
                    break;
                }
                Err(NfcError::AuthenticationFailed) => continue,
                Err(error) if is_fatal_read_error(&error) => return Err(error),
                Err(error) => {
                    warnings.push(format!("sector {sector}: {error}"));
                    break;
                }
            }
        }
        let Some(authenticated) = authenticated else {
            unread_sectors.push(sector);
            warnings.push(format!("sector {sector}: no candidate key authenticated"));
            for block in first..first + count {
                dump.blocks[block].status = nfc_classic::BlockStatus::AuthFailed;
                dump.blocks[block].error = Some("no candidate key authenticated".to_owned());
            }
            continue;
        };
        let mut read_any = false;
        for block in first..first + count {
            operation.cancel().check()?;
            match operation.read_block(block as u8) {
                Ok(data) => {
                    dump.blocks[block].set_read(data);
                    read_any = true;
                }
                Err(error) if is_fatal_read_error(&error) => return Err(error),
                Err(error) => {
                    dump.blocks[block].status = nfc_classic::BlockStatus::ReadFailed;
                    dump.blocks[block].error = Some(error.to_string());
                }
            }
        }
        if read_any {
            dump.metadata.keys.push(DumpKey {
                sector: authenticated.sector,
                key_type: authenticated.key_type,
                key: authenticated.key,
                source: Some(authenticated.source.stored().to_owned()),
            });
            authenticated_keys.push(authenticated);
        }
    }

    if !unread_sectors.is_empty() {
        warnings.push(format!(
            "{} of {} sectors did not authenticate: {}",
            unread_sectors.len(),
            layout.sector_count(),
            unread_sectors
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let readability = dump.readability();
    if readability.readable_blocks < readability.total_blocks {
        warnings.push(format!(
            "partial dump: {}/{} blocks readable, skipped sectors: {}",
            readability.readable_blocks,
            readability.total_blocks,
            readability
                .skipped_sectors
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if readability.readable_blocks == 0 {
        return Err(NfcError::AuthenticationFailed);
    }
    dump.metadata.started_at = Some(started_at);
    dump.metadata.finished_at = Some(now_seconds());
    Ok(ClassicReadResult {
        dump,
        keys: authenticated_keys,
        warnings,
    })
}

/// Reads one block after authenticating its sector.
pub fn read_block(
    operation: &mut Operation<'_>,
    block: usize,
    keys: &KeyStore,
    include_common_keys: bool,
) -> Result<ClassicBlockReadResult> {
    let card = operation.card().clone();
    let layout = layout_for_card(&card)?;
    if block >= layout.block_count() {
        return Err(NfcError::InvalidArgument(format!(
            "block {block} is outside the {} layout",
            layout.name()
        )));
    }
    let sector = layout
        .sector_for_block(block)
        .expect("block is within the layout");
    let first = layout
        .first_block(sector)
        .expect("sector is within the layout");
    let key = authenticate_sector(operation, &card, first, sector, keys, include_common_keys)?;
    Ok(ClassicBlockReadResult {
        card,
        data: operation.read_block(block as u8)?,
        key,
    })
}

/// Authenticates one sector with the first candidate key the card accepts.
fn authenticate_sector(
    operation: &mut Operation<'_>,
    card: &CardInfo,
    first_block: usize,
    sector: usize,
    keys: &KeyStore,
    include_common_keys: bool,
) -> Result<AuthenticatedKey> {
    for (key_type, key, source) in candidate_keys(keys, card, include_common_keys, sector) {
        operation.cancel().check()?;
        match operation.authenticate(first_block as u8, key_type, key) {
            Ok(()) => {
                return Ok(AuthenticatedKey {
                    sector,
                    key_type,
                    key,
                    source,
                });
            }
            Err(NfcError::AuthenticationFailed) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(NfcError::AuthenticationFailed)
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_fatal_read_error(error: &NfcError) -> bool {
    matches!(
        error,
        NfcError::Cancelled
            | NfcError::NoCard
            | NfcError::NoReader
            | NfcError::Timeout(_)
            | NfcError::CardChanged
            | NfcError::Unsupported(_)
    )
}
