//! Deterministic tests for the Classic NDEF mapping.

use nfc_classic::{Dump, Layout, bcc_for_uid};
use nfc_classic_ndef::{
    ClassicNdefLayout, MAD_ACCESS_BITS, MAD1_GPB, MAD2_GPB, Mad, MadAid, MadVersion, NFC_GPB_RW,
    NFC_KEY_A, NFC_RW_ACCESS_BITS, NdefState, crc8, encode_ndef_area, format_ndef_area, read_ndef,
};
use nfc_core::{BLOCK_SIZE, CardInfo, CardType};
use nfc_ndef::{NdefMessage, TextRecord};

const UID: [u8; 4] = [0x04, 0xA2, 0x24, 0x8A];

fn blank_dump(layout: Layout) -> Dump {
    let mut dump = Dump::new(layout).with_card(CardInfo::new(
        UID.to_vec(),
        [0x00, 0x04],
        0x08,
        match layout {
            Layout::Classic1K => CardType::MifareClassic1K,
            Layout::Classic4K => CardType::MifareClassic4K,
        },
    ));
    let mut block_zero = [0u8; BLOCK_SIZE];
    block_zero[0..4].copy_from_slice(&UID);
    block_zero[4] = bcc_for_uid(&UID).expect("bcc");
    block_zero[5] = 0x08;
    block_zero[6..8].copy_from_slice(&[0x00, 0x04]);
    dump.blocks[0].set_read(block_zero);
    for sector in 0..layout.sector_count() {
        let trailer = layout.trailer_block(sector).expect("trailer");
        let mut data = [0xFF; BLOCK_SIZE];
        data[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
        data[9] = 0x69;
        dump.blocks[trailer].set_read(data);
        let first = layout.first_block(sector).expect("first");
        for block in first..trailer {
            dump.blocks[block].set_read([0u8; BLOCK_SIZE]);
        }
    }
    dump
}

fn set_trailer(dump: &mut Dump, sector: usize, key_a: [u8; 6], access: [u8; 3], gpb: u8) {
    let trailer = dump.layout.trailer_block(sector).expect("trailer");
    let mut data = [0xFF; BLOCK_SIZE];
    data[..6].copy_from_slice(&key_a);
    data[6..9].copy_from_slice(&access);
    data[9] = gpb;
    dump.blocks[trailer].set_read(data);
}

fn format_dump(dump: &mut Dump, version: MadVersion, sectors: &[u8]) {
    let mad = Mad::blank(dump.layout, version, sectors).expect("blank MAD");
    for (block, data) in mad.encode(dump.layout).expect("encode MAD") {
        dump.blocks[block].set_read(data);
    }
    set_trailer(
        dump,
        0,
        [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5],
        MAD_ACCESS_BITS,
        match version {
            MadVersion::Mad1 => MAD1_GPB,
            MadVersion::Mad2 => MAD2_GPB,
        },
    );
    if version == MadVersion::Mad2 {
        set_trailer(
            dump,
            16,
            [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5],
            MAD_ACCESS_BITS,
            0,
        );
    }
    for sector in sectors {
        set_trailer(
            dump,
            usize::from(*sector),
            NFC_KEY_A,
            NFC_RW_ACCESS_BITS,
            NFC_GPB_RW,
        );
        let first = dump
            .layout
            .first_block(usize::from(*sector))
            .expect("first");
        let trailer = dump
            .layout
            .trailer_block(usize::from(*sector))
            .expect("trailer");
        for block in first..trailer {
            dump.blocks[block].set_read([0u8; BLOCK_SIZE]);
        }
    }
    let area_len = sectors.iter().fold(0usize, |total, sector| {
        total
            + dump
                .layout
                .blocks_in_sector(usize::from(*sector))
                .expect("count")
            - 1
    }) * BLOCK_SIZE;
    let area = format_ndef_area(area_len).expect("format area");
    let mut offset = 0;
    for sector in sectors {
        let first = dump
            .layout
            .first_block(usize::from(*sector))
            .expect("first");
        let trailer = dump
            .layout
            .trailer_block(usize::from(*sector))
            .expect("trailer");
        for block in first..trailer {
            let chunk: [u8; BLOCK_SIZE] =
                area[offset..offset + BLOCK_SIZE].try_into().expect("block");
            dump.blocks[block].set_read(chunk);
            offset += BLOCK_SIZE;
        }
    }
}

fn write_area(dump: &mut Dump, area: &[u8]) {
    let layout = ClassicNdefLayout::parse(dump).expect("parse").expect("MAD");
    assert_eq!(layout.data_len(), area.len());
    for (offset, byte) in area.iter().enumerate() {
        let (block, index) = layout.location(offset).expect("location");
        let mut data = dump.blocks[block].data;
        data[index] = *byte;
        dump.blocks[block].set_read(data);
    }
}

fn message(text: &str) -> NdefMessage {
    NdefMessage::single(
        TextRecord::new(text, "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message")
}

#[test]
fn crc_matches_the_known_mad_vectors() {
    assert_eq!(crc8(b"123456789"), 0x99);
    let block1 = [
        0x14, 0x01, 0x03, 0xE1, 0x03, 0xE1, 0x03, 0xE1, 0x03, 0xE1, 0x03, 0xE1, 0x03, 0xE1, 0x03,
        0xE1,
    ];
    let block2 = [0x03u8, 0xE1].repeat(8);
    let mut input = vec![block1[1]];
    input.extend_from_slice(&block1[2..]);
    input.extend_from_slice(&block2);
    assert_eq!(crc8(&input), 0x14);
}

#[test]
fn mad1_round_trips_and_finds_every_nfc_sector() {
    let mut dump = blank_dump(Layout::Classic1K);
    let sectors: Vec<u8> = (1..=15).collect();
    format_dump(&mut dump, MadVersion::Mad1, &sectors);
    let mad = Mad::read(&dump).expect("read").expect("MAD");
    assert_eq!(mad.version, MadVersion::Mad1);
    assert_eq!(mad.ndef_sectors(), sectors);
    assert_eq!(mad.entries.len(), 15);
    let info = read_ndef(&dump).expect("NDEF");
    let layout = info.layout.expect("layout");
    assert_eq!(layout.sectors, sectors);
    assert_eq!(layout.data_len(), 45 * BLOCK_SIZE);
    assert_eq!(info.ndef.state, NdefState::Empty);
    assert_eq!(info.ndef.capacity, 715);
}

#[test]
fn mad2_round_trips_the_four_k_layout() {
    let mut dump = blank_dump(Layout::Classic4K);
    let sectors: Vec<u8> = (1..=15).chain(17..=39).collect();
    format_dump(&mut dump, MadVersion::Mad2, &sectors);
    let mad = Mad::read(&dump).expect("read").expect("MAD");
    assert_eq!(mad.version, MadVersion::Mad2);
    assert_eq!(mad.ndef_sectors(), sectors);
    assert_eq!(mad.entries.len(), 38);
    let info = read_ndef(&dump).expect("NDEF");
    let layout = info.layout.expect("layout");
    assert_eq!(layout.data_blocks.len(), 210);
    assert_eq!(layout.data_len(), 210 * BLOCK_SIZE);
    assert_eq!(info.ndef.state, NdefState::Empty);
}

#[test]
fn a_bad_crc_or_gap_is_rejected() {
    let mut dump = blank_dump(Layout::Classic1K);
    format_dump(&mut dump, MadVersion::Mad1, &[1, 2]);
    dump.blocks[1].data[0] ^= 1;
    let error = Mad::read(&dump).expect_err("CRC mismatch");
    assert!(error.to_string().contains("CRC"));

    let mut dump = blank_dump(Layout::Classic1K);
    format_dump(&mut dump, MadVersion::Mad1, &[1, 3]);
    let error = read_ndef(&dump).expect_err("gap");
    assert!(error.to_string().contains("not contiguous"), "{error}");
}

#[test]
fn reads_messages_and_writes_across_sector_boundaries() {
    let mut dump = blank_dump(Layout::Classic1K);
    format_dump(&mut dump, MadVersion::Mad1, &(1..=15).collect::<Vec<_>>());
    let layout = ClassicNdefLayout::parse(&dump)
        .expect("parse")
        .expect("layout");
    let area = layout.area(&dump).expect("area");
    let long = message(&"x".repeat(300));
    let encoded = encode_ndef_area(&layout, &area, &long).expect("encode");
    write_area(&mut dump, &encoded);
    let info = read_ndef(&dump).expect("read");
    assert_eq!(info.ndef.state, NdefState::Message(long.clone()));
    assert_eq!(
        info.ndef.message_bytes,
        long.encoded_len().expect("encoded length")
    );
    assert_eq!(info.ndef.length_field_bytes, 3);
}

#[test]
fn write_preserves_leading_proprietary_tlvs() {
    let mut dump = blank_dump(Layout::Classic1K);
    format_dump(&mut dump, MadVersion::Mad1, &[1, 2]);
    let layout = ClassicNdefLayout::parse(&dump)
        .expect("parse")
        .expect("layout");
    let mut area = layout.area(&dump).expect("area");
    area[..5].copy_from_slice(&[0xFD, 0x03, 0xAA, 0xBB, 0xCC]);
    area[5..8].copy_from_slice(&[0x03, 0x00, 0xFE]);
    write_area(&mut dump, &area);
    let info = read_ndef(&dump).expect("read");
    assert_eq!(info.ndef.tlv_offset, 5);
    let encoded = encode_ndef_area(
        &layout,
        &layout.area(&dump).expect("area"),
        &message("kept"),
    )
    .expect("encode");
    assert_eq!(&encoded[..5], &[0xFD, 0x03, 0xAA, 0xBB, 0xCC]);
    assert_eq!(encoded[5], 0x03);
}

#[test]
fn read_only_gpb_is_reported() {
    let mut dump = blank_dump(Layout::Classic1K);
    format_dump(&mut dump, MadVersion::Mad1, &[1, 2]);
    set_trailer(&mut dump, 1, NFC_KEY_A, [0x07, 0x8F, 0x0F], 0x43);
    set_trailer(&mut dump, 2, NFC_KEY_A, [0x07, 0x8F, 0x0F], 0x43);
    let info = read_ndef(&dump).expect("read");
    let layout = info.layout.expect("layout");
    assert!(layout.read_only);
    assert!(!layout.writable);
}

#[test]
fn an_incomplete_data_area_is_rejected() {
    let mut dump = blank_dump(Layout::Classic1K);
    format_dump(&mut dump, MadVersion::Mad1, &[1, 2]);
    dump.blocks[4].known_mask = 0;
    dump.blocks[4].status = nfc_classic::BlockStatus::Unread;
    let error = read_ndef(&dump).expect_err("partial");
    assert!(error.to_string().contains("not readable"), "{error}");
}

#[test]
fn mad_entry_bytes_are_application_then_cluster() {
    assert_eq!(MadAid::NFC.stored(), [0x03, 0xE1]);
    assert_eq!(MadAid::from_stored([0x03, 0xE1]), MadAid::NFC);
}

#[test]
fn format_area_is_empty_and_bounded() {
    assert_eq!(format_ndef_area(3).expect("area"), vec![0x03, 0x00, 0xFE]);
    assert_eq!(format_ndef_area(5).expect("area"), vec![3, 0, 0xFE, 0, 0]);
    assert!(format_ndef_area(2).is_err());
}
