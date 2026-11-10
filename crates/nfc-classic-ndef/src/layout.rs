//! NDEF sector mapping over the MAD and NFC sector GPBs.
//!
//! The NFC Forum Type MIFARE Classic mapping has no capability-container
//! record. The mapping version and read/write access live in the GPB of each
//! NFC sector trailer, and the MAD identifies the NFC sectors.

use nfc_classic::{AccessBits, Dump, Layout};
use nfc_core::{BLOCK_SIZE, NfcError, Result};
use nfc_ndef::{NdefMessage, Tlv, ndef_capacity};

use crate::mad::{Mad, MadVersion};

/// Public Key A of the NFC sectors.
pub const NFC_KEY_A: [u8; 6] = [0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7];
/// Access bits of an NFC sector with read and write access.
pub const NFC_RW_ACCESS_BITS: [u8; 3] = [0x7F, 0x07, 0x88];
/// Access bits of an NFC sector with read-only access.
pub const NFC_RO_ACCESS_BITS: [u8; 3] = [0x07, 0x8F, 0x0F];
/// GPB for mapping version 1.0 with read/write access.
pub const NFC_GPB_RW: u8 = 0x40;
/// GPB for mapping version 1.0 with read-only access.
pub const NFC_GPB_RO: u8 = 0x43;

const MAPPING_MAJOR: u8 = 1;

/// Mapping version from GPB bits 4..7.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MappingVersion {
    pub major: u8,
    pub minor: u8,
}

impl MappingVersion {
    pub const V1_0: Self = Self { major: 1, minor: 0 };

    pub const fn from_gpb(gpb: u8) -> Self {
        Self {
            major: gpb >> 6,
            minor: (gpb >> 4) & 0x03,
        }
    }

    pub const fn gpb(self) -> u8 {
        self.major << 6 | self.minor << 4
    }
}

/// State of the mandatory NDEF Message TLV.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NdefState {
    Unformatted,
    Empty,
    Message(NdefMessage),
}

/// NDEF sectors and their data area.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicNdefLayout {
    /// Sectors carrying the NFC AID, in ascending order.
    pub sectors: Vec<u8>,
    /// Absolute data block numbers in logical NDEF order.
    pub data_blocks: Vec<usize>,
    pub version: MappingVersion,
    pub read_only: bool,
    pub writable: bool,
    pub warnings: Vec<String>,
}

impl ClassicNdefLayout {
    /// Parses the NFC sector map. `None` means the MAD-available bit is clear.
    pub fn parse(dump: &Dump) -> Result<Option<Self>> {
        let Some(mad) = Mad::read(dump)? else {
            return Ok(None);
        };
        let sectors = mad.ndef_sectors();
        if sectors.is_empty() {
            return Ok(Some(Self {
                sectors,
                data_blocks: Vec::new(),
                version: MappingVersion::V1_0,
                read_only: false,
                writable: false,
                warnings: mad.warnings,
            }));
        }
        check_contiguous(dump.layout, &sectors, mad.version)?;

        let mut warnings = mad.warnings;
        let mut data_blocks = Vec::new();
        let mut version: Option<MappingVersion> = None;
        let mut read_only = None;
        let mut started = false;
        for sector in &sectors {
            let trailer = sector_trailer(dump, *sector)?;
            let access =
                AccessBits::decode([trailer[6], trailer[7], trailer[8]]).map_err(|error| {
                    NfcError::InvalidArgument(format!(
                        "sector {sector} NDEF trailer access bits are invalid: {error}"
                    ))
                })?;
            let gpb = trailer[9];
            let read_access = (gpb >> 2) & 0x03;
            let write_access = gpb & 0x03;
            let accessible = read_access == 0 && (write_access == 0 || write_access == 3);
            if !accessible {
                if !started {
                    warnings.push(format!(
                        "sector {sector} is a proprietary NFC sector and is skipped"
                    ));
                    continue;
                }
                return Err(NfcError::InvalidArgument(format!(
                    "proprietary NFC sector {sector} follows the usable NDEF sectors"
                )));
            }
            let sector_version = MappingVersion::from_gpb(gpb);
            if sector_version.major != MAPPING_MAJOR {
                return Err(NfcError::InvalidArgument(format!(
                    "sector {sector} uses unsupported NDEF mapping version {}.{}",
                    sector_version.major, sector_version.minor
                )));
            }
            if let Some(version) = version {
                if version != sector_version {
                    return Err(NfcError::InvalidArgument(format!(
                        "sector {sector} uses mapping version {}.{} but earlier sectors use {}.{}",
                        sector_version.major, sector_version.minor, version.major, version.minor
                    )));
                }
            } else {
                version = Some(sector_version);
            }
            let read_only = read_only.get_or_insert(write_access == 3);
            if *read_only != (write_access == 3) {
                return Err(NfcError::InvalidArgument(
                    "NFC sectors disagree on the NDEF read-only state".to_owned(),
                ));
            }
            let first = dump
                .layout
                .first_block(usize::from(*sector))
                .expect("sector is within the layout");
            let trailer_block = dump
                .layout
                .trailer_block(usize::from(*sector))
                .expect("sector is within the layout");
            for block in first..trailer_block {
                let group = dump
                    .layout
                    .access_group_for_block(block)
                    .expect("data block has an access group");
                if !access.groups[group].data_read_keys().allows_a() {
                    return Err(NfcError::InvalidArgument(format!(
                        "sector {sector} block {block} is not readable with the public NFC key A"
                    )));
                }
                data_blocks.push(block);
            }
            started = true;
        }
        if !started {
            return Ok(Some(Self {
                sectors,
                data_blocks,
                version: version.unwrap_or(MappingVersion::V1_0),
                read_only: false,
                writable: false,
                warnings,
            }));
        }
        Ok(Some(Self {
            sectors,
            data_blocks,
            version: version.unwrap_or(MappingVersion::V1_0),
            read_only: read_only.unwrap_or(false),
            writable: read_only == Some(false),
            warnings,
        }))
    }

    pub fn data_len(&self) -> usize {
        self.data_blocks.len() * BLOCK_SIZE
    }

    /// Reads the complete logical data area. Unknown bytes are rejected.
    pub fn area(&self, dump: &Dump) -> Result<Vec<u8>> {
        let mut area = Vec::with_capacity(self.data_len());
        for number in &self.data_blocks {
            let block = dump.blocks.get(*number).ok_or_else(|| {
                NfcError::InvalidArgument(format!("block {number} is outside the dump"))
            })?;
            if !block.complete() {
                return Err(NfcError::VerificationFailed(format!(
                    "block {number} in the NDEF data area is not readable"
                )));
            }
            area.extend_from_slice(&block.data);
        }
        Ok(area)
    }

    /// Maps a logical data-area offset to an absolute block and byte offset.
    pub fn location(&self, offset: usize) -> Option<(usize, usize)> {
        if offset >= self.data_len() {
            return None;
        }
        let index = offset / BLOCK_SIZE;
        Some((*self.data_blocks.get(index)?, offset % BLOCK_SIZE))
    }
}

/// Mandatory NDEF Message TLV facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicNdef {
    pub state: NdefState,
    /// Offset of the mandatory NDEF Message TLV from the data-area start.
    pub tlv_offset: usize,
    pub length_field_bytes: usize,
    /// Largest NDEF message that fits from the mandatory TLV onward.
    pub capacity: usize,
    pub message_bytes: usize,
}

impl ClassicNdef {
    pub fn message(&self) -> Option<&NdefMessage> {
        match &self.state {
            NdefState::Message(message) => Some(message),
            _ => None,
        }
    }
}

/// Parsed mapping plus NDEF facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicNdefInfo {
    /// `None` when the card carries no MAD.
    pub layout: Option<ClassicNdefLayout>,
    pub ndef: ClassicNdef,
    pub warnings: Vec<String>,
}

/// Detects the NFC sectors and reads the mandatory NDEF Message TLV.
pub fn read_ndef(dump: &Dump) -> Result<ClassicNdefInfo> {
    let layout = ClassicNdefLayout::parse(dump)?;
    let warnings = layout
        .as_ref()
        .map(|layout| layout.warnings.clone())
        .unwrap_or_default();
    let ndef = match &layout {
        Some(layout) if !layout.data_blocks.is_empty() => read_ndef_with_layout(dump, layout)?,
        _ => ClassicNdef {
            state: NdefState::Unformatted,
            tlv_offset: 0,
            length_field_bytes: 0,
            capacity: 0,
            message_bytes: 0,
        },
    };
    Ok(ClassicNdefInfo {
        layout,
        ndef,
        warnings,
    })
}

pub fn read_ndef_with_layout(dump: &Dump, layout: &ClassicNdefLayout) -> Result<ClassicNdef> {
    let area = layout.area(dump)?;
    let tlvs = parse_tlvs(&area)?;
    let Some(ndef) = tlvs
        .iter()
        .find(|tlv| matches!(tlv.kind, ClassicTlvKind::Ndef(_)))
    else {
        return Ok(ClassicNdef {
            state: NdefState::Unformatted,
            tlv_offset: 0,
            length_field_bytes: 0,
            capacity: 0,
            message_bytes: 0,
        });
    };
    let ClassicTlvKind::Ndef(value) = &ndef.kind else {
        unreachable!("filtered for NDEF TLVs");
    };
    let state = if value.is_empty() {
        NdefState::Empty
    } else {
        NdefState::Message(NdefMessage::decode(value).map_err(|error| {
            NfcError::InvalidArgument(format!(
                "the Classic NDEF Message TLV at byte {} is invalid: {error}",
                ndef.offset
            ))
        })?)
    };
    let message_bytes = ndef_message_len(&state)?;
    Ok(ClassicNdef {
        state,
        tlv_offset: ndef.offset,
        length_field_bytes: ndef.length_field_bytes,
        capacity: ndef_capacity(area.len() - ndef.offset),
        message_bytes,
    })
}

fn ndef_message_len(state: &NdefState) -> Result<usize> {
    match state {
        NdefState::Message(message) => message.encoded_len(),
        NdefState::Empty | NdefState::Unformatted => Ok(0),
    }
}

/// Builds a complete data area containing `message` in the mandatory NDEF TLV.
/// Bytes before that TLV are preserved; later TLVs are replaced by the write
/// procedure's Terminator and zero fill.
pub fn encode_ndef_area(
    layout: &ClassicNdefLayout,
    area: &[u8],
    message: &NdefMessage,
) -> Result<Vec<u8>> {
    message.validate()?;
    if layout.data_len() != area.len() {
        return Err(NfcError::InvalidArgument(
            "the NDEF data area length does not match the mapping".to_owned(),
        ));
    }
    let tlvs = parse_tlvs(area)?;
    let mandatory = tlvs
        .iter()
        .find(|tlv| matches!(tlv.kind, ClassicTlvKind::Ndef(_)))
        .ok_or_else(|| {
            NfcError::InvalidArgument(
                "the card has no mandatory NDEF Message TLV; format it first".to_owned(),
            )
        })?;
    let capacity = ndef_capacity(area.len() - mandatory.offset);
    let message = message.encode()?;
    if message.len() > capacity {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF message is {} bytes but the NFC sectors hold at most {capacity}",
            message.len()
        )));
    }
    let mut output = Vec::with_capacity(area.len());
    output.extend_from_slice(&area[..mandatory.offset]);
    output.extend_from_slice(&Tlv::Ndef(message).encode()?);
    if output.len() < area.len() {
        output.push(0xFE);
    }
    if output.len() > area.len() {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF Message TLV and Terminator need {} bytes but the data area is {}",
            output.len(),
            area.len()
        )));
    }
    output.resize(area.len(), 0x00);
    Ok(output)
}

/// Formatted but empty data area: `03 00 FE` plus zero fill.
pub fn format_ndef_area(len: usize) -> Result<Vec<u8>> {
    if len < 3 {
        return Err(NfcError::InvalidArgument(
            "the NDEF data area needs at least three bytes".to_owned(),
        ));
    }
    let mut area = vec![0u8; len];
    area[..3].copy_from_slice(&[0x03, 0x00, 0xFE]);
    Ok(area)
}

/// Length-field width for an NDEF message of `len` bytes.
pub const fn length_field_bytes(len: usize) -> usize {
    if len < nfc_ndef::tlv::LENGTH_LONG_FORM as usize {
        1
    } else {
        3
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ClassicTlvKind {
    Null,
    Ndef(Vec<u8>),
    Proprietary(Vec<u8>),
    Reserved { tag: u8, value: Vec<u8> },
    Terminator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClassicTlv {
    kind: ClassicTlvKind,
    offset: usize,
    length_field_bytes: usize,
}

fn parse_tlvs(area: &[u8]) -> Result<Vec<ClassicTlv>> {
    let mut tlvs = Vec::new();
    let mut offset = 0;
    while offset < area.len() {
        let tag = area[offset];
        if tag == 0x00 {
            tlvs.push(ClassicTlv {
                kind: ClassicTlvKind::Null,
                offset,
                length_field_bytes: 0,
            });
            offset += 1;
            continue;
        }
        if tag == 0xFE {
            tlvs.push(ClassicTlv {
                kind: ClassicTlvKind::Terminator,
                offset,
                length_field_bytes: 0,
            });
            break;
        }
        if tag == 0xFF {
            return Err(NfcError::InvalidArgument(format!(
                "TLV at byte {offset}: 0xFF is not a valid TLV type"
            )));
        }
        let first = *area.get(offset + 1).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "TLV 0x{tag:02X} at byte {offset}: length field is truncated"
            ))
        })?;
        let (length, length_field_bytes) = if first == 0xFF {
            let bytes = area.get(offset + 1..offset + 4).ok_or_else(|| {
                NfcError::InvalidArgument(format!(
                    "TLV 0x{tag:02X} at byte {offset}: long length is truncated"
                ))
            })?;
            let length = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
            if length == usize::from(u16::MAX) {
                return Err(NfcError::InvalidArgument(format!(
                    "TLV 0x{tag:02X} at byte {offset}: length 0xFFFF is reserved"
                )));
            }
            if length < usize::from(nfc_ndef::tlv::LENGTH_LONG_FORM) {
                return Err(NfcError::InvalidArgument(format!(
                    "TLV 0x{tag:02X} at byte {offset}: long length {length} must use a short field"
                )));
            }
            (length, 3)
        } else {
            (usize::from(first), 1)
        };
        let value_start = offset + 1 + length_field_bytes;
        let value_end = value_start.checked_add(length).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "TLV 0x{tag:02X} at byte {offset}: length overflows"
            ))
        })?;
        let value = area.get(value_start..value_end).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "TLV 0x{tag:02X} at byte {offset}: value needs {length} bytes but the data area ends"
            ))
        })?;
        let kind = match tag {
            0x03 => ClassicTlvKind::Ndef(value.to_vec()),
            0xFD => ClassicTlvKind::Proprietary(value.to_vec()),
            other => ClassicTlvKind::Reserved {
                tag: other,
                value: value.to_vec(),
            },
        };
        tlvs.push(ClassicTlv {
            kind,
            offset,
            length_field_bytes,
        });
        offset = value_end;
    }
    Ok(tlvs)
}

fn check_contiguous(layout: Layout, sectors: &[u8], version: MadVersion) -> Result<()> {
    for pair in sectors.windows(2) {
        let [previous, next] = [pair[0], pair[1]];
        let contiguous = next == previous + 1
            || (layout == Layout::Classic4K
                && version == MadVersion::Mad2
                && previous == 15
                && next == 17);
        if !contiguous {
            return Err(NfcError::InvalidArgument(format!(
                "NFC sectors are not contiguous: {previous} is followed by {next}"
            )));
        }
    }
    Ok(())
}

fn sector_trailer(dump: &Dump, sector: u8) -> Result<[u8; BLOCK_SIZE]> {
    let number = dump
        .layout
        .trailer_block(usize::from(sector))
        .map_err(|_| NfcError::InvalidArgument(format!("sector {sector} is outside the layout")))?;
    let block = dump
        .blocks
        .get(number)
        .ok_or_else(|| NfcError::InvalidArgument(format!("trailer block {number} is missing")))?;
    if !block.complete() {
        return Err(NfcError::VerificationFailed(format!(
            "sector {sector} trailer block {number} is not readable"
        )));
    }
    Ok(block.data)
}
