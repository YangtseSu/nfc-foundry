//! Property tests for MAD and Classic NDEF area round trips.

use nfc_classic::{Dump, Layout, bcc_for_uid};
use nfc_classic_ndef::{
    ClassicNdefLayout, MAD_ACCESS_BITS, MAD1_GPB, MAD2_GPB, Mad, MadVersion, NFC_GPB_RW, NFC_KEY_A,
    NFC_RW_ACCESS_BITS, NdefState, encode_ndef_area, format_ndef_area, read_ndef,
};
use nfc_core::{BLOCK_SIZE, CardInfo, CardType};
use nfc_ndef::{NdefMessage, TextRecord};
use proptest::prelude::*;

const UID: [u8; 4] = [0x04, 0xA2, 0x24, 0x8A];

fn base_dump(layout: Layout) -> Dump {
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
    }
    dump
}

fn apply_mad(dump: &mut Dump, version: MadVersion, sectors: &[u8]) {
    let mad = Mad::blank(dump.layout, version, sectors).expect("MAD");
    for (block, data) in mad.encode(dump.layout).expect("encode") {
        dump.blocks[block].set_read(data);
    }
    let trailer = dump.layout.trailer_block(0).expect("trailer");
    let mut data = [0xFF; BLOCK_SIZE];
    data[..6].copy_from_slice(&[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);
    data[6..9].copy_from_slice(&MAD_ACCESS_BITS);
    data[9] = match version {
        MadVersion::Mad1 => MAD1_GPB,
        MadVersion::Mad2 => MAD2_GPB,
    };
    dump.blocks[trailer].set_read(data);
    if version == MadVersion::Mad2 {
        let trailer = dump.layout.trailer_block(16).expect("trailer");
        let mut data = [0xFF; BLOCK_SIZE];
        data[..6].copy_from_slice(&[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);
        data[6..9].copy_from_slice(&MAD_ACCESS_BITS);
        data[9] = 0;
        dump.blocks[trailer].set_read(data);
    }
    for sector in sectors {
        let sector = usize::from(*sector);
        let trailer = dump.layout.trailer_block(sector).expect("trailer");
        let mut data = [0xFF; BLOCK_SIZE];
        data[..6].copy_from_slice(&NFC_KEY_A);
        data[6..9].copy_from_slice(&NFC_RW_ACCESS_BITS);
        data[9] = NFC_GPB_RW;
        dump.blocks[trailer].set_read(data);
        let first = dump.layout.first_block(sector).expect("first");
        for block in &mut dump.blocks[first..trailer] {
            block.set_read([0u8; BLOCK_SIZE]);
        }
    }
}

fn area_len(layout: Layout, sectors: &[u8]) -> usize {
    sectors.iter().fold(0usize, |total, sector| {
        total
            + layout
                .blocks_in_sector(usize::from(*sector))
                .expect("count")
            - 1
    }) * BLOCK_SIZE
}

fn write_area(dump: &mut Dump, layout: &ClassicNdefLayout, area: &[u8]) {
    for (offset, byte) in area.iter().enumerate() {
        let (block, index) = layout.location(offset).expect("location");
        let mut data = dump.blocks[block].data;
        data[index] = *byte;
        dump.blocks[block].set_read(data);
    }
}

fn contiguous(count: usize, selector: usize, width: usize) -> (usize, usize) {
    let start = selector % (count + 1);
    let len = width % (count - start + 1);
    (start, len)
}

proptest! {
    #[test]
    fn mad1_round_trips_arbitrary_contiguous_sectors(selector in 0usize..1000, width in 0usize..1000) {
        let (start, len) = contiguous(15, selector, width);
        let sectors: Vec<u8> = ((start + 1)..=(start + len)).map(|sector| sector as u8).collect();
        let mut dump = base_dump(Layout::Classic1K);
        apply_mad(&mut dump, MadVersion::Mad1, &sectors);
        let mad = Mad::read(&dump).expect("read").expect("MAD");
        prop_assert_eq!(mad.ndef_sectors(), sectors);
        prop_assert_eq!(mad.version, MadVersion::Mad1);
    }

    #[test]
    fn mad2_round_trips_arbitrary_contiguous_sectors(selector in 0usize..1000, width in 0usize..1000) {
        let available: Vec<u8> = (1..=15).chain(17..=39).collect();
        let (start, len) = contiguous(available.len(), selector, width);
        let sectors: Vec<u8> = available[start..start + len].to_vec();
        let mut dump = base_dump(Layout::Classic4K);
        apply_mad(&mut dump, MadVersion::Mad2, &sectors);
        let mad = Mad::read(&dump).expect("read").expect("MAD");
        prop_assert_eq!(mad.ndef_sectors(), sectors);
        prop_assert_eq!(mad.version, MadVersion::Mad2);
    }

    #[test]
    fn ndef_text_round_trips_across_the_1k_area(text in "[ -~]{0,180}") {
        let sectors: Vec<u8> = (1..=15).collect();
        let mut dump = base_dump(Layout::Classic1K);
        apply_mad(&mut dump, MadVersion::Mad1, &sectors);
        let area = format_ndef_area(area_len(Layout::Classic1K, &sectors)).expect("area");
        let layout = ClassicNdefLayout::parse(&dump)
            .expect("parse")
            .expect("layout");
        write_area(&mut dump, &layout, &area);
        let current = layout.area(&dump).expect("area");
        let message = NdefMessage::single(
            TextRecord::new(text.clone(), "en")
                .expect("text")
                .to_record()
                .expect("record"),
        )
        .expect("message");
        let encoded = encode_ndef_area(&layout, &current, &message).expect("encode");
        write_area(&mut dump, &layout, &encoded);
        let info = read_ndef(&dump).expect("read");
        prop_assert_eq!(info.ndef.state, NdefState::Message(message));
    }
}
