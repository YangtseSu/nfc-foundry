//! Presentation data for the Classic block table and inspector.
//!
//! The GTK view never reads a card itself. These helpers turn a [`Dump`] into
//! rows and parse the hex editor; `nfc-app` owns the actual operations.

use nfc_classic::{
    AccessBits, BccStatus, Block, BlockKind, BlockStatus, DiffState, Dump, DumpDiff, KeyMask,
    Layout, value,
};
use nfc_core::KeyType;

/// Largest supported sector count; the GUI preallocates one child model per
/// sector and only fills the rows of the loaded layout.
pub const MAX_SECTORS: usize = 40;

/// One row in the Classic block table. Rows are immutable snapshots; a write
/// replaces the row with a fresh snapshot from the service result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRow {
    pub number: usize,
    pub sector: usize,
    pub kind: &'static str,
    pub status: &'static str,
    pub data: String,
    pub ascii: String,
    pub complete: bool,
    pub trailer: bool,
    pub dirty: bool,
    pub error: Option<String>,
}

impl BlockRow {
    pub fn from_block(block: &Block, layout: Layout) -> Self {
        Self {
            number: block.number,
            sector: block.sector,
            kind: kind_label(block.number, layout),
            status: status_label(block.status),
            data: hex_with_mask(&block.data, block.known_mask),
            ascii: ascii_with_mask(&block.data, block.known_mask),
            complete: block.complete(),
            trailer: layout.is_trailer(block.number).unwrap_or(false),
            dirty: false,
            error: block.error.clone(),
        }
    }

    /// Status shown in the table; an unsaved edit wins over the read status.
    pub fn display_status(&self) -> &'static str {
        if self.dirty { "edited" } else { self.status }
    }
}

/// Rows with the workbench dirty set applied.
pub fn rows_for_dirty(dump: &Dump, dirty: &[usize]) -> Vec<BlockRow> {
    dump.blocks
        .iter()
        .map(|block| {
            let mut row = BlockRow::from_block(block, dump.layout);
            row.dirty = dirty.binary_search(&block.number).is_ok();
            row
        })
        .collect()
}

/// One sector header above its block rows. The header is a presentation
/// summary: it never replaces or hides an image byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectorRow {
    pub sector: usize,
    pub first_block: usize,
    pub block_count: usize,
    pub key_a: bool,
    pub key_b: bool,
    pub edited: usize,
}

/// One item of the grouped block table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableRow {
    Sector(SectorRow),
    Block(BlockRow),
}

impl TableRow {
    pub fn sector_row(&self) -> Option<&SectorRow> {
        match self {
            Self::Sector(row) => Some(row),
            Self::Block(_) => None,
        }
    }

    pub fn block_row(&self) -> Option<&BlockRow> {
        match self {
            Self::Block(row) => Some(row),
            Self::Sector(_) => None,
        }
    }
}

/// Sector headers with verified key and unsaved-edit counts. `key_a`/`key_b`
/// mean the dump metadata holds a key that authenticated that sector.
pub fn sector_rows(dump: &Dump, dirty: &[usize]) -> Vec<SectorRow> {
    (0..dump.layout.sector_count())
        .map(|sector| {
            let first_block = dump
                .layout
                .first_block(sector)
                .expect("sector within the layout");
            let block_count = dump
                .layout
                .blocks_in_sector(sector)
                .expect("sector within the layout");
            let mut key_a = false;
            let mut key_b = false;
            for key in &dump.metadata.keys {
                if key.sector != sector {
                    continue;
                }
                match key.key_type {
                    KeyType::A => key_a = true,
                    KeyType::B => key_b = true,
                }
            }
            let edited = dirty
                .iter()
                .filter(|number| {
                    dump.blocks
                        .get(**number)
                        .is_some_and(|block| block.sector == sector)
                })
                .count();
            SectorRow {
                sector,
                first_block,
                block_count,
                key_a,
                key_b,
                edited,
            }
        })
        .collect()
}

pub fn kind_label(number: usize, layout: Layout) -> &'static str {
    if number == 0 {
        "M"
    } else if layout.is_trailer(number).unwrap_or(false) {
        "T"
    } else {
        "D"
    }
}

pub const fn status_label(status: BlockStatus) -> &'static str {
    match status {
        BlockStatus::Unread => "unread",
        BlockStatus::Read => "read",
        BlockStatus::AuthFailed => "no key",
        BlockStatus::ReadFailed => "read failed",
        BlockStatus::Synthetic => "synthetic",
        BlockStatus::Verified => "verified",
    }
}

/// Formats all 16 bytes, using `??` for bytes that were not read. Unknown
/// bytes are never displayed as zeroes.
pub fn hex_with_mask(data: &[u8; 16], mask: u16) -> String {
    let mut parts = Vec::with_capacity(16);
    for (index, byte) in data.iter().enumerate() {
        if mask & (1 << index) != 0 {
            parts.push(format!("{byte:02X}"));
        } else {
            parts.push("??".to_owned());
        }
    }
    parts.join(" ")
}

/// Printable ASCII for one block. Bytes outside `0x20..=0x7E` and unknown
/// bytes render as `.`; an unknown byte is never shown as a zero glyph.
pub fn ascii_with_mask(data: &[u8; 16], mask: u16) -> String {
    data.iter()
        .enumerate()
        .map(|(index, byte)| {
            if mask & (1 << index) != 0 && (0x20..=0x7E).contains(byte) {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

/// Parses exactly 16 bytes from space-separated or compact hexadecimal.
pub fn parse_hex_16(text: &str) -> Result<[u8; 16], String> {
    let compact: String = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if compact.len() != 32 {
        return Err(format!(
            "expected 32 hexadecimal digits, got {}",
            compact.len()
        ));
    }
    let mut data = [0u8; 16];
    for (index, byte) in data.iter_mut().enumerate() {
        let pair = &compact[index * 2..index * 2 + 2];
        *byte = u8::from_str_radix(pair, 16)
            .map_err(|_| format!("invalid hexadecimal byte {pair:?}"))?;
    }
    Ok(data)
}

pub fn display_layout(layout: Layout) -> &'static str {
    match layout {
        Layout::Classic1K => "MIFARE Classic 1K",
        Layout::Classic4K => "MIFARE Classic 4K",
    }
}

/// Inspector detail for one block: decoded access conditions, value-block
/// contents, BCC state, and warnings. Empty lines separate sections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockInsight {
    pub details: String,
    pub warnings: Vec<String>,
}

pub fn inspect_block(dump: &Dump, number: usize) -> Option<BlockInsight> {
    let block = dump.blocks.get(number)?;
    let mut lines = Vec::new();
    let mut warnings = Vec::new();
    if !block.complete() {
        warnings.push("Some bytes were not read; unknown bytes are never treated as zero.".into());
    }
    if number == 0 {
        lines.push(manufacturer_bcc(dump).unwrap_or_else(|error| {
            warnings.push(error);
            "BCC: not checked".to_owned()
        }));
    }
    if number == 0 || block.complete() {
        let sector = block.sector;
        match dump.access_bits(sector) {
            Ok(bits) => {
                let group = dump
                    .layout
                    .access_group_for_block(number)
                    .expect("block within layout");
                if number == 0 {
                    lines.push(format!(
                        "Sector 0 access bits {} · group 0 · {} · {}",
                        access_hex(&bits),
                        condition_code(bits.groups[0]),
                        data_permissions(bits.groups[0])
                    ));
                } else if dump.layout.is_trailer(number).unwrap_or(false) {
                    lines.push(trailer_access(dump.layout, sector, &bits, block));
                } else {
                    lines.push(format!(
                        "Access group {group} · {} · {}",
                        group_blocks(dump.layout, sector, group),
                        condition_code(bits.groups[group])
                    ));
                    lines.push(data_permissions(bits.groups[group]));
                }
            }
            Err(error) => warnings.push(error.to_string()),
        }
        if number != 0 && !dump.layout.is_trailer(number).unwrap_or(false) {
            let data = block.data;
            if let Ok(value) = value::decode(data) {
                lines.push(format!(
                    "Value block · value {} · address {}",
                    value.value, value.address
                ));
            }
        }
    }
    Some(BlockInsight {
        details: lines.join("\n"),
        warnings,
    })
}

fn manufacturer_bcc(dump: &Dump) -> Result<String, String> {
    let Some(card) = &dump.card else {
        return Ok("BCC: no card identity; cannot check".to_owned());
    };
    match dump
        .validate_bcc(card.uid.len())
        .map_err(|error| error.to_string())?
    {
        BccStatus::Valid => Ok(format!("BCC {:02X} · valid", dump.blocks[0].data[4])),
        BccStatus::Invalid { expected, actual } => Err(format!(
            "BCC {actual:02X} does not match the UID (expected {expected:02X})"
        )),
        BccStatus::NotApplicable => Ok("BCC: not applicable for this UID length".to_owned()),
    }
}

fn trailer_access(layout: Layout, sector: usize, bits: &AccessBits, block: &Block) -> String {
    let mut text = format!(
        "Sector {sector} trailer · access bytes {} {}\n",
        access_hex(bits),
        condition_codes(bits)
    );
    for group in 0..3 {
        text.push_str(&format!(
            "Group {group} · {} · {} · {}\n",
            group_blocks(layout, sector, group),
            condition_code(bits.groups[group]),
            data_permissions(bits.groups[group])
        ));
    }
    let permissions = bits.groups[3].trailer_access();
    text.push_str(&format!(
        "Group 3 · {} · {} · Key A write {} · access write {} · Key B write {} · Key B readable {}",
        group_blocks(layout, sector, 3),
        condition_code(bits.groups[3]),
        mask_label(permissions.write_key_a),
        mask_label(permissions.write_access),
        mask_label(permissions.write_key_b),
        if permissions.key_b_readable {
            "yes"
        } else {
            "no"
        }
    ));
    text.push_str(&format!(
        "\nKey A {} · Key B {} · GPB {:02X}",
        hex_bytes(&block.data[0..6]),
        hex_bytes(&block.data[10..16]),
        block.data[9]
    ));
    text
}

fn access_hex(bits: &AccessBits) -> String {
    let [b6, b7, b8] = bits.encode();
    format!("{b6:02X} {b7:02X} {b8:02X}")
}

fn condition_codes(bits: &AccessBits) -> String {
    let codes = bits
        .groups
        .iter()
        .map(|condition| condition_code(*condition))
        .collect::<Vec<_>>()
        .join(" / ");
    format!("groups {codes}")
}

fn condition_code(condition: nfc_classic::AccessCondition) -> String {
    format!("C1C2C3 {:03b}", condition.code())
}

fn data_permissions(condition: nfc_classic::AccessCondition) -> String {
    format!(
        "read {} · write {} · increment {} · decrement {}",
        mask_label(condition.data_read_keys()),
        mask_label(condition.data_write_keys()),
        mask_label(condition.data_increment_keys()),
        mask_label(condition.data_decrement_keys())
    )
}

fn mask_label(mask: KeyMask) -> &'static str {
    match (mask.allows_a(), mask.allows_b()) {
        (true, true) => "A|B",
        (true, false) => "A",
        (false, true) => "B",
        (false, false) => "never",
    }
}

fn group_blocks(layout: Layout, sector: usize, group: usize) -> String {
    let Ok(first) = layout.first_block(sector) else {
        return "unknown".to_owned();
    };
    let Ok(count) = layout.blocks_in_sector(sector) else {
        return "unknown".to_owned();
    };
    let trailer = first + count - 1;
    if group == 3 {
        return format!("block {trailer}");
    }
    let range = if count == 16 {
        let start = first + group * 5;
        (start, start + 4)
    } else {
        (first + group, first + group)
    };
    if range.0 == range.1 {
        format!("block {}", range.0)
    } else {
        format!("blocks {}-{}", range.0, range.1)
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// One save-format choice in the dump dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveFormatOption {
    pub format: nfc_classic::DumpFormat,
    pub label: &'static str,
    pub enabled: bool,
}

/// Save choices with a completeness-sensitive default. Auto stays first and
/// resolves to raw for a complete image or project for a partial image.
pub fn save_format_options(dump: &Dump) -> Vec<SaveFormatOption> {
    use nfc_classic::DumpFormat;
    [
        (DumpFormat::Auto, "Automatic (raw or project)"),
        (DumpFormat::Raw, "Proxmark3 raw (.bin)"),
        (DumpFormat::Proxmark3, "Proxmark3 JSON (.mfc.json)"),
        (DumpFormat::Mct, "MCT text (.mct)"),
        (
            DumpFormat::Project,
            "NFC Foundry project (.nfc-foundry.json)",
        ),
    ]
    .into_iter()
    .map(|(format, label)| SaveFormatOption {
        format,
        label,
        enabled: format.can_represent(dump),
    })
    .collect()
}

/// One row in the diff table. Equal blocks are summarized only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub number: usize,
    pub sector: usize,
    pub kind: &'static str,
    pub state: &'static str,
    pub detail: String,
}

pub fn diff_rows(diff: &DumpDiff) -> Vec<DiffRow> {
    diff.blocks
        .iter()
        .filter(|block| block.state != DiffState::Equal)
        .map(|block| DiffRow {
            number: block.block,
            sector: block.sector,
            kind: diff_kind_label(block.kind),
            state: diff_state_label(block.state),
            detail: diff_detail(block),
        })
        .collect()
}

fn diff_kind_label(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Manufacturer => "M",
        BlockKind::Data => "D",
        BlockKind::Trailer => "T",
    }
}

fn diff_state_label(state: DiffState) -> &'static str {
    match state {
        DiffState::Equal => "equal",
        DiffState::Changed => "changed",
        DiffState::Unknown => "unknown",
    }
}

fn diff_detail(block: &nfc_classic::BlockDiff) -> String {
    let mut parts = Vec::new();
    if let Some(trailer) = &block.trailer {
        if trailer.key_a_changed {
            parts.push(format!(
                "Key A {} → {}",
                trailer.before_key_a.to_hex(),
                trailer.after_key_a.to_hex()
            ));
        }
        if trailer.key_b_changed {
            parts.push(format!(
                "Key B {} → {}",
                trailer.before_key_b.to_hex(),
                trailer.after_key_b.to_hex()
            ));
        }
        if trailer.access_changed {
            parts.push(format!(
                "access {} → {}",
                hex_bytes(&trailer.before_access),
                hex_bytes(&trailer.after_access)
            ));
        }
        if trailer.gpb_changed {
            parts.push(format!(
                "GPB {:02X} → {:02X}",
                trailer.before_gpb, trailer.after_gpb
            ));
        }
        if let Some(error) = &trailer.error {
            parts.push(error.clone());
        }
    }
    if block.state == DiffState::Unknown {
        parts.push("unknown bytes on one or both sides".to_owned());
    }
    for byte in block.bytes.iter().take(4) {
        let before = if byte.before_known {
            format!("{:02X}", byte.before)
        } else {
            "??".to_owned()
        };
        let after = if byte.after_known {
            format!("{:02X}", byte.after)
        } else {
            "??".to_owned()
        };
        parts.push(format!("[{:02}] {before} → {after}", byte.offset));
    }
    if block.bytes.len() > 4 {
        parts.push(format!("+{} more", block.bytes.len() - 4));
    }
    parts.join("; ")
}

/// Whether a restore action is available: a loaded complete-or-partial image,
/// a live selected card, and no running operation. The service enforces its
/// own gates; this is only the button state.
pub fn restore_ready(live: bool, busy: bool, loaded: bool) -> bool {
    live && !busy && loaded
}

/// Capacity refusal for a composed message. The service remains authoritative.
pub fn ndef_capacity_error(bytes: usize, capacity: usize) -> Option<String> {
    (bytes > capacity).then(|| {
        format!("The message needs {bytes} bytes; the NDEF area holds at most {capacity}.")
    })
}

/// Hex preview with a byte limit; the ellipsis marks truncated output.
pub fn hex_preview(bytes: &[u8], limit: usize) -> String {
    let shown = bytes
        .iter()
        .take(limit)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > limit {
        format!("{shown} …")
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_the_layout_geometry() {
        let one_k = Dump::new(Layout::Classic1K);
        let four_k = Dump::new(Layout::Classic4K);
        assert_eq!(rows_for_dirty(&one_k, &[]).len(), 64);
        assert_eq!(rows_for_dirty(&four_k, &[]).len(), 256);
        assert_eq!(rows_for_dirty(&four_k, &[])[128].sector, 32);
        assert!(rows_for_dirty(&four_k, &[])[255].trailer);
    }

    #[test]
    fn unknown_bytes_are_not_zero_filled() {
        let mut block = Block::new(Layout::Classic1K, 1).expect("block");
        block.data[..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        block.known_mask = 0x000F;
        block.status = BlockStatus::ReadFailed;
        let row = BlockRow::from_block(&block, Layout::Classic1K);
        assert_eq!(row.data, "DE AD BE EF ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ??");
        assert!(!row.complete);
    }

    #[test]
    fn ascii_renders_printable_bytes_and_dots_for_unknown_or_control() {
        let mut data = [0u8; 16];
        data[..8].copy_from_slice(b"AB 09\x00\x7F~");
        data[8..].fill(b'Z');
        assert_eq!(ascii_with_mask(&data, u16::MAX), "AB 09..~ZZZZZZZZ");
        assert_eq!(ascii_with_mask(&data, 0x00FF), "AB 09..~........");
        assert_eq!(ascii_with_mask(&data, 0), "................");
    }

    #[test]
    fn sector_rows_count_blocks_keys_and_edits() {
        let mut dump = complete_dump();
        let one_k = sector_rows(&dump, &[]);
        assert_eq!(one_k.len(), 16);
        assert_eq!(one_k[0].first_block, 0);
        assert_eq!(one_k[0].block_count, 4);
        assert!(!one_k[0].key_a && !one_k[0].key_b);
        assert_eq!(one_k[0].edited, 0);

        dump.metadata.keys = vec![
            nfc_classic::DumpKey {
                sector: 0,
                key_type: KeyType::A,
                key: nfc_core::Key::from_hex("FFFFFFFFFFFF").expect("key"),
                source: None,
            },
            nfc_classic::DumpKey {
                sector: 1,
                key_type: KeyType::B,
                key: nfc_core::Key::from_hex("A0A1A2A3A4A5").expect("key"),
                source: None,
            },
        ];
        let rows = sector_rows(&dump, &[1, 4, 5, 7]);
        assert!(rows[0].key_a && !rows[0].key_b);
        assert!(!rows[1].key_a && rows[1].key_b);
        assert_eq!(rows[0].edited, 1);
        assert_eq!(rows[1].edited, 3);

        let four_k = Dump::new(Layout::Classic4K);
        let rows = sector_rows(&four_k, &[]);
        assert_eq!(rows.len(), 40);
        assert_eq!(rows[31].block_count, 4);
        assert_eq!(rows[32].first_block, 128);
        assert_eq!(rows[32].block_count, 16);
        assert_eq!(rows[39].first_block, 240);
    }

    #[test]
    fn table_rows_expose_the_sector_of_both_variants() {
        let dump = complete_dump();
        let sector = TableRow::Sector(sector_rows(&dump, &[])[2].clone());
        let block = TableRow::Block(BlockRow::from_block(&dump.blocks[9], Layout::Classic1K));
        assert_eq!(sector.sector_row().map(|row| row.sector), Some(2));
        assert_eq!(block.block_row().map(|row| row.sector), Some(2));
        assert!(sector.block_row().is_none());
        assert!(block.sector_row().is_none());
    }

    #[test]
    fn hex_parser_accepts_grouped_hex_and_rejects_bad_input() {
        let data =
            parse_hex_16("04 A2 24 8A 08 08 00 04 00 00 00 00 00 00 00 00").expect("valid hex");
        assert_eq!(data[0], 0x04);
        assert_eq!(data[15], 0x00);
        assert!(parse_hex_16("04 A2").is_err());
        assert!(parse_hex_16("04 A2 24 8A 08 08 00 04 00 00 00 00 00 00 00 GG").is_err());
    }

    #[test]
    fn labels_cover_manufacturer_trailer_and_data_blocks() {
        assert_eq!(kind_label(0, Layout::Classic1K), "M");
        assert_eq!(kind_label(3, Layout::Classic1K), "T");
        assert_eq!(kind_label(4, Layout::Classic1K), "D");
    }

    fn complete_dump() -> Dump {
        let mut raw = vec![0u8; Layout::Classic1K.dump_size()];
        raw[0..4].copy_from_slice(&[1, 2, 3, 4]);
        raw[4] = nfc_classic::bcc_for_uid(&[1, 2, 3, 4]).expect("bcc");
        for sector in 0..Layout::Classic1K.sector_count() {
            let block = Layout::Classic1K.trailer_block(sector).expect("trailer");
            let offset = block * 16;
            raw[offset + 6..offset + 9].copy_from_slice(&[0xFF, 0x07, 0x80]);
            raw[offset + 9] = 0x69;
        }
        Dump::from_raw(&raw)
            .expect("complete image")
            .with_card(nfc_core::CardInfo::new(
                vec![1, 2, 3, 4],
                [0x00, 0x04],
                0x08,
                nfc_core::CardType::MifareClassic1K,
            ))
    }

    #[test]
    fn inspect_decodes_bcc_value_block_and_trailer() {
        let mut dump = complete_dump();
        dump.blocks[4].set_read(value::encode(value::ValueBlock {
            value: 1234,
            address: 4,
        }));
        dump.blocks[5].set_read(value::encode(value::ValueBlock {
            value: -7,
            address: 5,
        }));
        let manufacturer = inspect_block(&dump, 0).expect("block 0");
        assert!(manufacturer.details.contains("BCC"));
        assert!(manufacturer.details.contains("valid"));
        let data = inspect_block(&dump, 4).expect("block 4");
        assert!(
            data.details
                .contains("Value block · value 1234 · address 4")
        );
        assert!(data.details.contains("Access group 0 · block 4"));
        let trailer = inspect_block(&dump, 7).expect("trailer");
        assert!(trailer.details.contains("Sector 1 trailer"));
        assert!(trailer.details.contains("Group 0"));
        assert!(trailer.details.contains("Key A"));
    }

    #[test]
    fn inspect_reports_invalid_trailer_and_unknown_bytes() {
        let mut dump = complete_dump();
        dump.blocks[7].data[6] = 0x00;
        dump.blocks[8].known_mask = 0x000F;
        let trailer = inspect_block(&dump, 7).expect("trailer");
        assert!(!trailer.warnings.is_empty());
        let data = inspect_block(&dump, 8).expect("block 8");
        assert!(
            data.warnings
                .iter()
                .any(|warning| warning.contains("not read"))
        );
    }

    #[test]
    fn save_options_follow_completeness() {
        let complete = complete_dump();
        let partial = Dump::new(Layout::Classic1K);
        let complete_options = save_format_options(&complete);
        assert_eq!(complete_options[0].format, nfc_classic::DumpFormat::Auto);
        assert!(complete_options.iter().all(|option| option.enabled));
        let partial_options = save_format_options(&partial);
        for option in &partial_options {
            match option.format {
                nfc_classic::DumpFormat::Raw
                | nfc_classic::DumpFormat::Proxmark3
                | nfc_classic::DumpFormat::Mct => assert!(!option.enabled, "{option:?}"),
                nfc_classic::DumpFormat::Auto | nfc_classic::DumpFormat::Project => {
                    assert!(option.enabled)
                }
            }
        }
        assert_eq!(
            nfc_classic::DumpFormat::Auto.default_file_name(),
            "card-dump.bin"
        );
    }

    #[test]
    fn ndef_capacity_and_preview_follow_the_bytes() {
        assert_eq!(ndef_capacity_error(93, 93), None);
        let error = ndef_capacity_error(94, 93).expect("overflow");
        assert!(error.contains("94"));
        assert!(error.contains("93"));
        assert_eq!(hex_preview(&[0xD1, 0x01], 4), "D1 01");
        assert_eq!(hex_preview(&[0xD1, 0x01, 0x02], 2), "D1 01 …");
    }

    #[test]
    fn diff_rows_keep_only_informative_blocks() {
        let before = complete_dump();
        let mut after = before.clone();
        after.blocks[4].set_read([0xAA; 16]);
        after.blocks[5].known_mask = 0;
        let rows = diff_rows(&nfc_classic::diff::diff(&before, &after).expect("diff"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].state, "changed");
        assert!(rows[0].detail.contains("[00]"));
        assert_eq!(rows[1].state, "unknown");
        assert!(rows[1].detail.contains("unknown bytes"));
    }
}
