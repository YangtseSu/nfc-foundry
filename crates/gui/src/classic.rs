//! Presentation data for the Classic block table and inspector.
//!
//! The GTK view never reads a card itself. These helpers turn a [`Dump`] into
//! rows and parse the hex editor; `nfc-app` owns the actual operations.

use nfc_classic::{Block, BlockStatus, Dump, Layout};

/// One row in the Classic block table. Rows are immutable snapshots; a write
/// replaces the row with a fresh snapshot from the service result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRow {
    pub number: usize,
    pub sector: usize,
    pub kind: &'static str,
    pub status: &'static str,
    pub data: String,
    pub complete: bool,
    pub trailer: bool,
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
            complete: block.complete(),
            trailer: layout.is_trailer(block.number).unwrap_or(false),
            error: block.error.clone(),
        }
    }
}

pub fn rows_for(dump: &Dump) -> Vec<BlockRow> {
    dump.blocks
        .iter()
        .map(|block| BlockRow::from_block(block, dump.layout))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_the_layout_geometry() {
        let one_k = Dump::new(Layout::Classic1K);
        let four_k = Dump::new(Layout::Classic4K);
        assert_eq!(rows_for(&one_k).len(), 64);
        assert_eq!(rows_for(&four_k).len(), 256);
        assert_eq!(rows_for(&four_k)[128].sector, 32);
        assert!(rows_for(&four_k)[255].trailer);
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
}
