use nfc_classic::{
    AccessBits, AccessCondition, BlockStatus, Dump, DumpFormat, Layout, ValueBlock, exchange,
};
use proptest::prelude::*;

fn layout_and_raw() -> impl Strategy<Value = (Layout, Vec<u8>)> {
    (0usize..2).prop_flat_map(|index| {
        let layout = Layout::ALL[index];
        (
            Just(layout),
            prop::collection::vec(any::<u8>(), layout.dump_size()),
        )
    })
}

fn patched_trailers(layout: Layout, mut raw: Vec<u8>) -> Vec<u8> {
    for sector in 0..layout.sector_count() {
        let block = layout.trailer_block(sector).expect("trailer");
        let offset = block * 16;
        raw[offset + 6..offset + 9].copy_from_slice(&[0xFF, 0x07, 0x80]);
    }
    raw
}

proptest! {
    #[test]
    fn access_decode_then_encode_preserves_valid_bytes(bytes in any::<[u8; 3]>()) {
        if let Ok(bits) = AccessBits::decode(bytes) {
            prop_assert_eq!(bits.encode(), bytes);
        }
    }

    #[test]
    fn access_encode_then_decode_preserves_groups(codes in prop::collection::vec(0u8..8, 4)) {
        let bits = AccessBits {
            groups: std::array::from_fn(|index| {
                let code = codes[index];
                AccessCondition::new(code & 4 != 0, code & 2 != 0, code & 1 != 0)
            }),
        };
        let decoded = AccessBits::decode(bits.encode()).expect("encoded access bits");
        prop_assert_eq!(decoded, bits);
    }

    #[test]
    fn value_blocks_round_trip(value in any::<i32>(), address in any::<u8>()) {
        let block = ValueBlock { value, address };
        prop_assert_eq!(
            nfc_classic::value::decode(nfc_classic::value::encode(block))
                .expect("canonical value block"),
            block
        );
    }

    #[test]
    fn raw_images_round_trip_exactly((layout, raw) in layout_and_raw()) {
        let dump = Dump::from_raw(&raw).expect("raw image");
        prop_assert!(dump.complete());
        prop_assert_eq!(dump.raw().expect("raw"), raw);
        let readability = dump.readability();
        prop_assert_eq!(readability.readable_blocks, layout.block_count());
        prop_assert_eq!(readability.readable_sectors, layout.sector_count());
    }

    #[test]
    fn project_round_trip_preserves_masks_and_statuses(
        raw in prop::collection::vec(any::<u8>(), Layout::Classic1K.dump_size()),
        known in prop::collection::vec(any::<bool>(), Layout::Classic1K.block_count()),
    ) {
        let mut dump = Dump::from_raw(&raw).expect("raw image");
        for (block, known) in dump.blocks.iter_mut().zip(known) {
            if known {
                block.known_mask = u16::MAX;
                block.status = BlockStatus::Read;
            } else {
                block.known_mask = 0;
                block.status = BlockStatus::Unread;
            }
        }
        let encoded = exchange::export(&dump, DumpFormat::Project).expect("project export");
        let loaded = exchange::load_as(&encoded, DumpFormat::Project).expect("project import");
        prop_assert_eq!(loaded.dump, dump);
    }

    #[test]
    fn complete_images_round_trip_through_external_formats((layout, raw) in layout_and_raw()) {
        let raw = patched_trailers(layout, raw);
        let dump = Dump::from_raw(&raw).expect("raw image");
        for format in [DumpFormat::Raw, DumpFormat::Mct, DumpFormat::Proxmark3] {
            let encoded = exchange::export(&dump, format).expect("external export");
            let loaded = exchange::load_as(&encoded, format).expect("external import");
            prop_assert_eq!(loaded.dump.raw().expect("raw"), raw.clone());
        }
    }
}

#[test]
fn geometry_walks_every_block_in_order() {
    for layout in Layout::ALL {
        let mut next_block = 0;
        for sector in 0..layout.sector_count() {
            let first = layout.first_block(sector).expect("first block");
            let count = layout.blocks_in_sector(sector).expect("block count");
            assert_eq!(first, next_block);
            for local in 0..count {
                let block = first + local;
                assert_eq!(
                    layout.sector_for_block(block).expect("sector"),
                    sector,
                    "block {block}"
                );
                let expected_group = if local + 1 == count {
                    3
                } else if count == 16 {
                    local / 5
                } else {
                    local
                };
                assert_eq!(
                    layout.access_group_for_block(block).expect("group"),
                    expected_group,
                    "block {block}"
                );
            }
            assert_eq!(
                layout.trailer_block(sector).expect("trailer"),
                first + count - 1
            );
            next_block += count;
        }
        assert_eq!(next_block, layout.block_count());
    }
}
