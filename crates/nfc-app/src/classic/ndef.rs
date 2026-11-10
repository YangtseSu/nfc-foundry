//! Classic NDEF read, write, and format workflows.

use nfc_classic::{AccessBits, Dump, KeyMask, Layout};
use nfc_classic_ndef::{
    ClassicNdefInfo, MAD_ACCESS_BITS, MAD_KEY_A, Mad, MadVersion, NFC_GPB_RO, NFC_GPB_RW,
    NFC_KEY_A, NFC_RO_ACCESS_BITS, NFC_RW_ACCESS_BITS, NdefState, encode_ndef_area,
    format_ndef_area, length_field_bytes, read_ndef,
};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, Key, KeyType, NfcError, Result};
use nfc_ndef::NdefMessage;

use crate::operation::Operation;

use super::backup::{BackupRecord, BackupStore};
use super::keys::KeyStore;
use super::read::candidate_keys;
use super::write::{AllowedTypes, backup_label, check_expected_card};

const FACTORY_KEY_B: [u8; 6] = [0xFF; 6];

/// Options for reading the Classic NDEF mapping.
pub struct ClassicNdefRead<'a> {
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicNdefRead<'a> {
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

/// A full Classic image with its parsed NDEF mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicNdefReadResult {
    pub dump: Dump,
    pub info: ClassicNdefInfo,
    pub warnings: Vec<String>,
}

/// Options for writing the mandatory NDEF Message TLV.
pub struct ClassicNdefWrite<'a> {
    pub message: NdefMessage,
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    /// Identity of the image the write was composed from.
    pub expected_card: Option<CardInfo>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> ClassicNdefWrite<'a> {
    pub fn new(message: NdefMessage, keys: &'a KeyStore, backup: &'a BackupStore) -> Self {
        Self {
            message,
            keys,
            include_common_keys: true,
            expected_card: None,
            backup,
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

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of a verified NDEF write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicNdefWriteResult {
    pub dump: Dump,
    pub info: ClassicNdefInfo,
    pub backup: BackupRecord,
    pub written_blocks: usize,
    pub changed_blocks: usize,
}

/// Options for formatting NFC sectors.
pub struct ClassicNdefFormat<'a> {
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    pub backup: &'a BackupStore,
    pub read_only: bool,
    pub confirm: bool,
    /// Identity of the image the format was composed from.
    pub expected_card: Option<CardInfo>,
    pub cancel: CancelToken,
}

impl<'a> ClassicNdefFormat<'a> {
    pub fn new(keys: &'a KeyStore, backup: &'a BackupStore) -> Self {
        Self {
            keys,
            include_common_keys: true,
            backup,
            read_only: false,
            confirm: false,
            expected_card: None,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Formatting replaces keys and access bits; the caller must confirm.
    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn expected_card(mut self, card: CardInfo) -> Self {
        self.expected_card = Some(card);
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of a verified format.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicNdefFormatResult {
    pub dump: Dump,
    pub info: ClassicNdefInfo,
    pub backup: BackupRecord,
    pub written_blocks: usize,
    pub trailer_writes: usize,
}

/// Reads the card and parses the MAD/NFC sector mapping.
pub(crate) fn read(
    operation: &mut Operation<'_>,
    request: ClassicNdefRead<'_>,
) -> Result<ClassicNdefReadResult> {
    let read = super::read::read_dump(operation, request.keys, request.include_common_keys)?;
    let info = read_ndef(&read.dump)?;
    Ok(ClassicNdefReadResult {
        dump: read.dump,
        info,
        warnings: read.warnings,
    })
}

/// Parses a dump that was already read; no card I/O.
pub(crate) fn inspect(dump: &Dump) -> Result<ClassicNdefInfo> {
    read_ndef(dump)
}

/// Writes the mandatory NDEF Message TLV with a backup and verified blocks.
pub(crate) fn write(
    operation: &mut Operation<'_>,
    request: ClassicNdefWrite<'_>,
) -> Result<ClassicNdefWriteResult> {
    request.cancel.check()?;
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let read = super::read::read_dump(operation, request.keys, request.include_common_keys)?;
    let info = read_ndef(&read.dump)?;
    let layout = info.layout.as_ref().ok_or_else(|| {
        NfcError::InvalidArgument(
            "the card has no MAD; format it for Classic NDEF first".to_owned(),
        )
    })?;
    if layout.sectors.is_empty() {
        return Err(NfcError::InvalidArgument(
            "the MAD contains no NFC sectors; format the card first".to_owned(),
        ));
    }
    if layout.read_only || !layout.writable {
        return Err(NfcError::Unsupported(
            "the NFC sectors are read-only; NDEF writes are disabled",
        ));
    }
    if !matches!(info.ndef.state, NdefState::Empty | NdefState::Message(_)) {
        return Err(NfcError::InvalidArgument(
            "the card has no mandatory NDEF Message TLV; format it first".to_owned(),
        ));
    }
    let area = layout.area(&read.dump)?;
    let target = encode_ndef_area(layout, &area, &request.message)?;

    let mut changed = Vec::new();
    for (block, before, after) in block_slices(layout, &area, &target)? {
        if before != after {
            changed.push(block);
        }
    }
    if changed.is_empty() {
        return Err(NfcError::InvalidArgument(
            "the requested NDEF message is already stored".to_owned(),
        ));
    }
    ensure_writable(
        operation,
        &read.dump,
        &changed,
        request.keys,
        request.include_common_keys,
        &request.cancel,
    )?;

    let backup = request.backup.save(
        &read.dump,
        &backup_label(&read.dump, read.dump.layout, "ndef-write"),
    )?;
    let message_len = request.message.encoded_len()?;
    let width = length_field_bytes(message_len);
    let mut cleared = target.clone();
    let length_start = info
        .ndef
        .tlv_offset
        .checked_add(1)
        .ok_or_else(|| NfcError::InvalidArgument("NDEF TLV offset overflows".to_owned()))?;
    match width {
        1 => {
            let byte = cleared.get_mut(length_start).ok_or_else(|| {
                NfcError::InvalidArgument("NDEF TLV length field is truncated".to_owned())
            })?;
            *byte = 0;
        }
        3 => {
            let field = cleared
                .get_mut(length_start..length_start + 3)
                .ok_or_else(|| {
                    NfcError::InvalidArgument("NDEF TLV length field is truncated".to_owned())
                })?;
            field.copy_from_slice(&[0xFF, 0x00, 0x00]);
        }
        _ => unreachable!("length fields are one or three bytes"),
    }
    let mut written_blocks = 0;
    let clear_writes: Vec<(usize, [u8; BLOCK_SIZE])> = block_slices(layout, &area, &cleared)?
        .into_iter()
        .filter_map(|(block, before, after)| (before != after).then_some((block, after)))
        .collect();
    written_blocks += write_plan(
        operation,
        &read.dump,
        &clear_writes,
        request.keys,
        request.include_common_keys,
        &request.cancel,
    )?;
    let mut final_writes: Vec<(usize, [u8; BLOCK_SIZE])> = block_slices(layout, &cleared, &target)?
        .into_iter()
        .filter_map(|(block, before, after)| (before != after).then_some((block, after)))
        .collect();
    let mut length_blocks = Vec::new();
    for offset in length_start..length_start + width {
        if let Some((block, _)) = layout.location(offset) {
            length_blocks.push(block);
        }
    }
    final_writes.sort_by_key(|(block, _)| length_blocks.contains(block));
    written_blocks += write_plan(
        operation,
        &read.dump,
        &final_writes,
        request.keys,
        request.include_common_keys,
        &request.cancel,
    )?;
    operation.check_card()?;

    let verification =
        super::read::read_dump(operation, request.keys, request.include_common_keys)?;
    let verified = read_ndef(&verification.dump)?;
    if verified.ndef.state != NdefState::Message(request.message.clone()) {
        return Err(NfcError::VerificationFailed(
            "the NDEF Message TLV did not match after the write".to_owned(),
        ));
    }
    Ok(ClassicNdefWriteResult {
        dump: verification.dump,
        info: verified,
        backup,
        written_blocks,
        changed_blocks: changed.len(),
    })
}

/// Formats every addressable NFC sector, MAD sectors included, with a backup.
pub(crate) fn format(
    operation: &mut Operation<'_>,
    request: ClassicNdefFormat<'_>,
) -> Result<ClassicNdefFormatResult> {
    request.cancel.check()?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "formatting replaces sector keys and access bits; confirmation is required".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let read = super::read::read_dump(operation, request.keys, request.include_common_keys)?;
    if !read.dump.complete() {
        return Err(NfcError::VerificationFailed(
            "format requires a complete card read; unreadable sectors were left alone".to_owned(),
        ));
    }
    let layout = read.dump.layout;
    let (version, nfc_sectors) = match layout {
        Layout::Classic1K => (MadVersion::Mad1, (1..=15).collect::<Vec<u8>>()),
        Layout::Classic4K => (
            MadVersion::Mad2,
            (1..=15).chain(17..=39).collect::<Vec<u8>>(),
        ),
    };
    let mad = Mad::blank(layout, version, &nfc_sectors)?;
    let mut mad_blocks = mad.encode(layout)?;
    mad_blocks.sort_by_key(|(block, _)| *block);

    let mut data_plan: Vec<(usize, [u8; BLOCK_SIZE])> = Vec::new();
    for (block, data) in mad_blocks {
        data_plan.push((block, data));
    }
    let data_blocks = nfc_sectors.iter().try_fold(0usize, |total, sector| {
        let count = layout.blocks_in_sector(usize::from(*sector))?;
        Ok::<_, NfcError>(total + count - 1)
    })?;
    let area = format_ndef_area(data_blocks * BLOCK_SIZE)?;
    let mut area_offset = 0;
    for sector in &nfc_sectors {
        let first = layout.first_block(usize::from(*sector))?;
        let count = layout.blocks_in_sector(usize::from(*sector))?;
        for block in first..first + count - 1 {
            let chunk: [u8; BLOCK_SIZE] = area[area_offset..area_offset + BLOCK_SIZE]
                .try_into()
                .expect("block-sized chunk");
            data_plan.push((block, chunk));
            area_offset += BLOCK_SIZE;
        }
    }

    let mut trailer_plan =
        Vec::with_capacity(usize::from(version == MadVersion::Mad2) + nfc_sectors.len() + 1);
    trailer_plan.push((
        layout.trailer_block(0)?,
        trailer(
            MAD_KEY_A,
            MAD_ACCESS_BITS,
            version.gpb(),
            existing_key_b(&read.dump, layout, 0),
        ),
    ));
    if version == MadVersion::Mad2 {
        trailer_plan.push((
            layout.trailer_block(16)?,
            trailer(
                MAD_KEY_A,
                MAD_ACCESS_BITS,
                0x00,
                existing_key_b(&read.dump, layout, 16),
            ),
        ));
    }
    for sector in &nfc_sectors {
        let sector_number = usize::from(*sector);
        trailer_plan.push((
            layout.trailer_block(sector_number)?,
            if request.read_only {
                trailer(
                    NFC_KEY_A,
                    NFC_RO_ACCESS_BITS,
                    NFC_GPB_RO,
                    existing_key_b(&read.dump, layout, sector_number),
                )
            } else {
                trailer(
                    NFC_KEY_A,
                    NFC_RW_ACCESS_BITS,
                    NFC_GPB_RW,
                    existing_key_b(&read.dump, layout, sector_number),
                )
            },
        ));
    }

    ensure_writable(
        operation,
        &read.dump,
        &data_plan
            .iter()
            .map(|(block, _)| *block)
            .chain(trailer_plan.iter().map(|(block, _)| *block))
            .collect::<Vec<_>>(),
        request.keys,
        request.include_common_keys,
        &request.cancel,
    )?;

    let backup = request
        .backup
        .save(&read.dump, &backup_label(&read.dump, layout, "ndef-format"))?;
    let mut written_blocks = write_plan(
        operation,
        &read.dump,
        &data_plan,
        request.keys,
        request.include_common_keys,
        &request.cancel,
    )?;
    for (block, data) in &trailer_plan {
        let mask = write_mask(&read.dump, *block)?;
        authenticate_for_mask(
            operation,
            request.keys,
            request.include_common_keys,
            read.dump.layout.sector_for_block(*block)?,
            read.dump
                .layout
                .first_block(read.dump.layout.sector_for_block(*block)?)?,
            mask,
        )?;
        operation.write_block_unverified(*block as u8, *data)?;
        let new_key = Key::new(data[..6].try_into().expect("trailer Key A is six bytes"));
        operation.authenticate(*block as u8, KeyType::A, new_key)?;
        let read_back = operation.read_block(*block as u8)?;
        // Key A is never readable and Key B is usually masked, so compare the
        // access bytes and GPB. The new Key A is already proven by the
        // authentication above; a trailer that grants Key B writes is also
        // verified by authenticating with the intended Key B.
        if read_back[6..10] != data[6..10] {
            return Err(NfcError::VerificationFailed(format!(
                "block {block} access bytes did not match after the trailer write"
            )));
        }
        let access = AccessBits::decode([data[6], data[7], data[8]])?;
        if access.groups[3].full_trailer_write_keys().allows_b() {
            let key_b = Key::new(data[10..16].try_into().expect("Key B is six bytes"));
            operation.authenticate(*block as u8, KeyType::B, key_b)?;
        }
        written_blocks += 1;
    }
    operation.check_card()?;

    let verification = super::read::read_dump(operation, request.keys, true)?;
    let verified = read_ndef(&verification.dump)?;
    let verified_layout = verified.layout.as_ref().ok_or_else(|| {
        NfcError::VerificationFailed("the formatted card has no readable MAD".to_owned())
    })?;
    if verified_layout.sectors != nfc_sectors {
        return Err(NfcError::VerificationFailed(
            "the formatted NFC sector map did not read back".to_owned(),
        ));
    }
    if verified.ndef.state != NdefState::Empty {
        return Err(NfcError::VerificationFailed(
            "the formatted card does not contain an empty NDEF Message TLV".to_owned(),
        ));
    }
    if verified_layout.read_only != request.read_only {
        return Err(NfcError::VerificationFailed(
            "the formatted read-only state did not read back".to_owned(),
        ));
    }
    Ok(ClassicNdefFormatResult {
        dump: verification.dump,
        info: verified,
        backup,
        written_blocks,
        trailer_writes: trailer_plan.len(),
    })
}

/// Returns the write permissions for one block from the current trailer.
fn write_mask(dump: &Dump, block: usize) -> Result<KeyMask> {
    let layout = dump.layout;
    let sector = layout.sector_for_block(block)?;
    let access = dump.access_bits(sector)?;
    if layout.is_trailer(block)? {
        Ok(access.groups[3].full_trailer_write_keys())
    } else {
        let group = layout.access_group_for_block(block)?;
        Ok(access.groups[group].data_write_keys())
    }
}

/// Confirms every target block is writable with a key the card accepts.
fn ensure_writable(
    operation: &mut Operation<'_>,
    dump: &Dump,
    blocks: &[usize],
    keys: &KeyStore,
    include_common_keys: bool,
    cancel: &CancelToken,
) -> Result<()> {
    let mut seen = Vec::new();
    for block in blocks {
        cancel.check()?;
        let sector = dump.layout.sector_for_block(*block)?;
        let mask = write_mask(dump, *block)?;
        if mask.is_empty() {
            return Err(NfcError::Unsupported(
                "the current access bits do not permit writing every NDEF block",
            ));
        }
        if seen.contains(&(sector, mask.bits())) {
            continue;
        }
        seen.push((sector, mask.bits()));
        let first = dump.layout.first_block(sector)?;
        authenticate_for_mask(operation, keys, include_common_keys, sector, first, mask)?;
    }
    Ok(())
}

/// Writes each planned block in order, authenticating each sector as needed.
fn write_plan(
    operation: &mut Operation<'_>,
    dump: &Dump,
    writes: &[(usize, [u8; BLOCK_SIZE])],
    keys: &KeyStore,
    include_common_keys: bool,
    cancel: &CancelToken,
) -> Result<usize> {
    let mut written = 0;
    let mut current: Option<(usize, KeyType, Key)> = None;
    for (block, data) in writes {
        cancel.check()?;
        let sector = dump.layout.sector_for_block(*block)?;
        let mask = write_mask(dump, *block)?;
        let allowed = AllowedTypes::from(mask);
        if !allowed.any() {
            return Err(NfcError::Unsupported(
                "the current access bits do not permit this write",
            ));
        }
        let usable = current.is_some_and(|(current_sector, key_type, _)| {
            current_sector == sector && allowed.allows(key_type)
        });
        if !usable {
            let (key_type, key) = authenticate_for_mask(
                operation,
                keys,
                include_common_keys,
                sector,
                dump.layout.first_block(sector)?,
                mask,
            )?;
            current = Some((sector, key_type, key));
        }
        operation.write_block_verified(*block as u8, *data)?;
        written += 1;
    }
    Ok(written)
}

type BlockChange = (usize, [u8; BLOCK_SIZE], [u8; BLOCK_SIZE]);

/// Splits two equal data areas into `(block, before, after)` slices.
fn block_slices(
    layout: &nfc_classic_ndef::ClassicNdefLayout,
    before: &[u8],
    after: &[u8],
) -> Result<Vec<BlockChange>> {
    if before.len() != layout.data_len() || after.len() != layout.data_len() {
        return Err(NfcError::InvalidArgument(
            "the NDEF data areas do not match the mapping".to_owned(),
        ));
    }
    Ok(layout
        .data_blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let start = index * BLOCK_SIZE;
            (
                *block,
                before[start..start + BLOCK_SIZE]
                    .try_into()
                    .expect("block-sized slice"),
                after[start..start + BLOCK_SIZE]
                    .try_into()
                    .expect("block-sized slice"),
            )
        })
        .collect())
}

fn authenticate_for_mask(
    operation: &mut Operation<'_>,
    keys: &KeyStore,
    include_common_keys: bool,
    sector: usize,
    first_block: usize,
    mask: KeyMask,
) -> Result<(KeyType, Key)> {
    let allowed = AllowedTypes::from(mask);
    let candidates: Vec<(KeyType, Key)> = candidate_keys(keys, include_common_keys, sector)
        .into_iter()
        .filter(|(key_type, _)| allowed.allows(*key_type))
        .collect();
    if candidates.is_empty() {
        return Err(NfcError::AuthenticationFailed);
    }
    for (key_type, key) in candidates {
        match operation.authenticate(first_block as u8, key_type, key) {
            Ok(()) => return Ok((key_type, key)),
            Err(NfcError::AuthenticationFailed) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(NfcError::AuthenticationFailed)
}

/// Reads the current Key B so a format does not silently replace a secret with
/// the factory value. A masked Key B falls back to `FF FF FF FF FF FF`.
fn existing_key_b(dump: &Dump, layout: Layout, sector: usize) -> [u8; 6] {
    let Some(block) = layout
        .trailer_block(sector)
        .ok()
        .and_then(|number| dump.blocks.get(number))
    else {
        return FACTORY_KEY_B;
    };
    if block.known_mask & 0xFC00 != 0xFC00 {
        return FACTORY_KEY_B;
    }
    block.data[10..16].try_into().expect("Key B is six bytes")
}

fn trailer(key_a: [u8; 6], access: [u8; 3], gpb: u8, key_b: [u8; 6]) -> [u8; BLOCK_SIZE] {
    let mut data = [0xFF; BLOCK_SIZE];
    data[..6].copy_from_slice(&key_a);
    data[6..9].copy_from_slice(&access);
    data[9] = gpb;
    data[10..].copy_from_slice(&key_b);
    data
}
