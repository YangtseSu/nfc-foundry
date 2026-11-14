use std::collections::BTreeMap;

use nfc_classic::{BlockStatus, Dump, DumpFormat, DumpKey, DumpMetadata, Layout, exchange};
use nfc_core::{CardInfo, CardType, Key, KeyType};
use serde_json::json;

fn complete_dump(layout: Layout) -> Dump {
    let mut raw = vec![0u8; layout.dump_size()];
    raw[0..4].copy_from_slice(&[1, 2, 3, 4]);
    raw[4] = raw[0] ^ raw[1] ^ raw[2] ^ raw[3];
    for sector in 0..layout.sector_count() {
        let block = layout.trailer_block(sector).expect("trailer");
        let offset = block * 16;
        raw[offset + 6..offset + 9].copy_from_slice(&[0xFF, 0x07, 0x80]);
    }
    Dump::from_raw(&raw).expect("complete image")
}

#[test]
fn project_round_trip_preserves_partial_state() {
    let mut dump = Dump::new(Layout::Classic1K);
    dump.blocks[4].set_read([0x11; 16]);
    dump.blocks[5].known_mask = 0x000F;
    dump.blocks[5].data[0..2].copy_from_slice(&[0xAA, 0xBB]);
    dump.blocks[5].status = BlockStatus::ReadFailed;
    dump.blocks[5].error = Some("card timeout".to_owned());
    let encoded = exchange::export(&dump, DumpFormat::Project).expect("project export");
    assert!(String::from_utf8_lossy(&encoded).contains("\"knownMask\": \"000F\""));
    let loaded = exchange::load_as(&encoded, DumpFormat::Project).expect("project import");
    assert_eq!(loaded.dump, dump);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_round_trip_preserves_keys_device_and_times() {
    let mut dump = complete_dump(Layout::Classic1K);
    dump.metadata = DumpMetadata {
        device: Some("pn532_uart:/dev/ttyUSB0".to_owned()),
        started_at: Some(1_762_819_200),
        finished_at: Some(1_762_819_205),
        keys: vec![
            DumpKey {
                sector: 0,
                key_type: KeyType::A,
                key: Key::from_hex("FFFFFFFFFFFF").expect("key"),
                source: Some("read_context".to_owned()),
            },
            DumpKey {
                sector: 1,
                key_type: KeyType::B,
                key: Key::from_hex("A0A1A2A3A4A5").expect("key"),
                source: Some("recovery".to_owned()),
            },
        ],
    };
    assert_eq!(DumpFormat::Auto.resolve(&dump), DumpFormat::Raw);
    let encoded = exchange::export(&dump, DumpFormat::Project).expect("project export");
    let text = String::from_utf8_lossy(&encoded);
    assert!(text.contains("\"device\": \"pn532_uart:/dev/ttyUSB0\""));
    assert!(text.contains("\"startedAt\": 1762819200"));
    assert!(text.contains("\"source\": \"read_context\""));
    let loaded = exchange::load_as(&encoded, DumpFormat::Project).expect("project import");
    assert_eq!(loaded.dump.metadata, dump.metadata);
    assert_eq!(
        DumpFormat::Auto.resolve(&complete_dump(Layout::Classic1K)),
        DumpFormat::Raw
    );
}

#[test]
fn version_one_project_loads_with_empty_metadata() {
    let dump = complete_dump(Layout::Classic1K);
    let encoded = exchange::export(&dump, DumpFormat::Project).expect("project export");
    let text =
        String::from_utf8(encoded)
            .expect("utf-8")
            .replacen("\"version\": 2", "\"version\": 1", 1);
    let loaded = exchange::load_as(text.as_bytes(), DumpFormat::Project).expect("v1 import");
    assert_eq!(loaded.dump.metadata, DumpMetadata::default());
    assert_eq!(loaded.dump, dump);
}

#[test]
fn project_key_metadata_is_validated() {
    let dump = complete_dump(Layout::Classic1K);
    let encoded = exchange::export(&dump, DumpFormat::Project).expect("project export");
    let text = String::from_utf8(encoded).expect("utf-8");
    for key in [
        json!({"sector": 16, "type": "A", "key": "FFFFFFFFFFFF"}),
        json!({"sector": 0, "type": "C", "key": "FFFFFFFFFFFF"}),
        json!({"sector": 0, "type": "A", "key": "FFFFFFFF"}),
    ] {
        let mut value: serde_json::Value = serde_json::from_str(&text).expect("json");
        value["keys"] = json!([key]);
        let tampered = serde_json::to_string(&value).expect("json");
        assert!(
            exchange::load_as(tampered.as_bytes(), DumpFormat::Project).is_err(),
            "{tampered}"
        );
    }
    let mut value: serde_json::Value = serde_json::from_str(&text).expect("json");
    value["keys"] = json!([
        {"sector": 0, "type": "A", "key": "FFFFFFFFFFFF"},
        {"sector": 0, "type": "A", "key": "000000000000"}
    ]);
    let tampered = serde_json::to_string(&value).expect("json");
    assert!(exchange::load_as(tampered.as_bytes(), DumpFormat::Project).is_err());
}

#[test]
fn duplicate_json_keys_are_rejected() {
    let dump = complete_dump(Layout::Classic1K);
    let encoded = exchange::export(&dump, DumpFormat::Project).expect("project export");
    let text = String::from_utf8(encoded).expect("utf-8");
    let tampered = text.replacen("\"version\": 2,", "\"version\": 2,\n  \"version\": 2,", 1);
    assert!(exchange::load(tampered.as_bytes()).is_err());
}

#[test]
fn project_structural_errors_are_rejected() {
    let dump = complete_dump(Layout::Classic1K);
    let encoded = exchange::export(&dump, DumpFormat::Project).expect("project export");
    let text = String::from_utf8(encoded).expect("utf-8");

    let cases = [
        text.replacen("\"version\": 2", "\"version\": 3", 1),
        text.replacen(
            "\"layout\": \"classic_1k\"",
            "\"layout\": \"classic_mini\"",
            1,
        ),
        text.replacen("\"knownMask\": \"FFFF\"", "\"knownMask\": \"FFFFF\"", 1),
        text.replacen("\"status\": \"read\"", "\"status\": \"broken\"", 1),
    ];
    for case in cases {
        assert!(exchange::load_as(case.as_bytes(), DumpFormat::Project).is_err());
    }

    let mut incomplete = dump;
    incomplete.blocks[0].known_mask = 0;
    let encoded = exchange::export(&incomplete, DumpFormat::Project).expect("project export");
    let text = String::from_utf8(encoded).expect("utf-8");
    let tampered = text.replacen("\"complete\": false", "\"complete\": true", 1);
    assert!(exchange::load_as(tampered.as_bytes(), DumpFormat::Project).is_err());
}

#[test]
fn partial_dump_cannot_be_exported_to_external_formats() {
    let dump = Dump::new(Layout::Classic1K);
    for format in [DumpFormat::Raw, DumpFormat::Mct, DumpFormat::Proxmark3] {
        assert!(
            exchange::export(&dump, format).is_err(),
            "format {format:?}"
        );
    }
    let project = exchange::export(&dump, DumpFormat::Project).expect("project");
    let loaded = exchange::load_as(&project, DumpFormat::Project).expect("project");
    assert!(loaded.dump.raw().is_err());
}

#[test]
fn formats_are_detected_from_content() {
    let dump = complete_dump(Layout::Classic4K);
    for format in [DumpFormat::Raw, DumpFormat::Mct, DumpFormat::Proxmark3] {
        let encoded = exchange::export(&dump, format).expect("export");
        assert_eq!(
            exchange::detect_dump_format(&encoded).expect("detect"),
            format
        );
    }
    let project = exchange::export(&dump, DumpFormat::Project).expect("project");
    assert_eq!(
        exchange::detect_dump_format(&project).expect("detect"),
        DumpFormat::Project
    );
    assert!(exchange::detect_dump_format(&[0u8; 1023]).is_err());
    assert!(exchange::detect_dump_format(b"").is_err());
}

#[test]
fn mct_rejects_duplicate_missing_and_malformed_sectors() {
    let dump = complete_dump(Layout::Classic1K);
    let encoded = exchange::export(&dump, DumpFormat::Mct).expect("mct");
    let text = String::from_utf8(encoded).expect("utf-8");

    let duplicate = text.replacen("+Sector: 1\n", "+Sector: 1\n+Sector: 1\n", 1);
    assert!(exchange::load_as(duplicate.as_bytes(), DumpFormat::Mct).is_err());

    let missing = text
        .split_once("+Sector: 15")
        .expect("sector 15 header")
        .0
        .to_owned();
    assert!(exchange::load_as(missing.as_bytes(), DumpFormat::Mct).is_err());

    let short = text.replacen(&"00".repeat(16), "00", 1);
    assert!(exchange::load_as(short.as_bytes(), DumpFormat::Mct).is_err());
}

fn proxmark_document(
    uid: &str,
    block0_bcc: u8,
    sector_keys: serde_json::Value,
) -> serde_json::Value {
    let layout = Layout::Classic1K;
    let mut blocks = BTreeMap::new();
    for block in 0..layout.block_count() {
        let mut data = [0u8; 16];
        if block == 0 {
            data[0..4].copy_from_slice(&[1, 2, 3, 4]);
            data[4] = block0_bcc;
        }
        if layout.is_trailer(block).expect("trailer") {
            data[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
        }
        blocks.insert(block.to_string(), hex(&data));
    }
    json!({
        "Created": "NFC Foundry",
        "FileType": "mfc v2",
        "Card": { "UID": uid, "ATQA": "0004", "SAK": "08" },
        "blocks": blocks,
        "SectorKeys": sector_keys,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

#[test]
fn proxmark_import_validates_bcc_and_trailer_keys() {
    let document = proxmark_document("01020304", 4, json!({}));
    let encoded = serde_json::to_vec(&document).expect("json");
    let load = exchange::load_as(&encoded, DumpFormat::Proxmark3).expect("import");
    assert_eq!(load.dump.layout, Layout::Classic1K);
    assert_eq!(load.dump.card.expect("card").uid, vec![1, 2, 3, 4]);

    let bad_bcc = proxmark_document("01020304", 5, json!({}));
    let encoded = serde_json::to_vec(&bad_bcc).expect("json");
    assert!(exchange::load_as(&encoded, DumpFormat::Proxmark3).is_err());

    let conflicting_key =
        proxmark_document("01020304", 4, json!({ "0": { "KeyA": "FFFFFFFFFFFF" } }));
    let encoded = serde_json::to_vec(&conflicting_key).expect("json");
    assert!(exchange::load_as(&encoded, DumpFormat::Proxmark3).is_err());

    let matching_key = proxmark_document(
        "01020304",
        4,
        json!({ "0": {
            "KeyA": "000000000000",
            "KeyB": "000000000000",
            "AccessConditions": "FF078099"
        }}),
    );
    let encoded = serde_json::to_vec(&matching_key).expect("json");
    assert!(exchange::load_as(&encoded, DumpFormat::Proxmark3).is_err());
}

#[test]
fn proxmark_v3_warns_about_dropped_fields() {
    let mut document = proxmark_document("01020304", 4, json!({}));
    let object = document.as_object_mut().expect("object");
    object.insert("FileType".to_owned(), json!("mfc v3"));
    object
        .get_mut("Card")
        .and_then(serde_json::Value::as_object_mut)
        .expect("card")
        .insert("ATS".to_owned(), json!("06075A0001"));
    let encoded = serde_json::to_vec(&document).expect("json");
    let load = exchange::load_as(&encoded, DumpFormat::Proxmark3).expect("import");
    assert!(load.warnings.iter().any(|warning| warning.contains("ATS")));
}

#[test]
fn proxmark_export_contains_trailer_keys_and_access_text() {
    let dump = complete_dump(Layout::Classic1K);
    let encoded = exchange::export(&dump, DumpFormat::Proxmark3).expect("export");
    let document: serde_json::Value = serde_json::from_slice(&encoded).expect("json");
    assert_eq!(document["FileType"], "mfc v2");
    assert_eq!(document["blocks"]["3"], "000000000000FF078000000000000000");
    assert_eq!(document["SectorKeys"]["0"]["KeyA"], "000000000000");
    assert_eq!(
        document["SectorKeys"]["0"]["AccessConditionsText"]["block0"],
        "read A|B, write A|B, increment A|B, decrement A|B"
    );
}

#[test]
fn auto_resolves_by_completeness_and_uses_interchange_names() {
    let complete = complete_dump(Layout::Classic1K);
    let partial = Dump::new(Layout::Classic1K);
    assert_eq!(DumpFormat::Auto.resolve(&complete), DumpFormat::Raw);
    assert_eq!(DumpFormat::Auto.resolve(&partial), DumpFormat::Project);
    assert!(DumpFormat::Raw.can_represent(&complete));
    assert!(!DumpFormat::Raw.can_represent(&partial));
    assert!(!DumpFormat::Proxmark3.can_represent(&partial));
    assert!(DumpFormat::Project.can_represent(&partial));

    assert_eq!(
        exchange::detect_dump_format(&exchange::export(&complete, DumpFormat::Auto).expect("auto"))
            .expect("detect"),
        DumpFormat::Raw
    );
    assert_eq!(
        exchange::detect_dump_format(&exchange::export(&partial, DumpFormat::Auto).expect("auto"))
            .expect("detect"),
        DumpFormat::Project
    );

    assert_eq!(DumpFormat::Raw.default_file_name(), "card-dump.bin");
    assert_eq!(
        DumpFormat::Proxmark3.default_file_name(),
        "card-dump.mfc.json"
    );
    assert_eq!(DumpFormat::Mct.default_file_name(), "card-dump.mct");
    assert_eq!(
        DumpFormat::Project.default_file_name(),
        "card-dump.nfc-foundry.json"
    );
    assert_eq!(DumpFormat::Auto.default_file_name(), "card-dump.bin");
}

#[test]
fn raw_1k_and_4k_round_trips_are_exact() {
    for layout in [Layout::Classic1K, Layout::Classic4K] {
        let dump = complete_dump(layout);
        let raw = exchange::export(&dump, DumpFormat::Raw).expect("raw export");
        assert_eq!(raw.len(), layout.dump_size());
        let loaded = exchange::load(&raw).expect("raw import");
        assert_eq!(loaded.format, DumpFormat::Raw);
        assert_eq!(loaded.dump, dump);
    }
}

#[test]
fn golden_proxmark_fixture_round_trips_semantically() {
    let fixture = include_str!("fixtures/pm3-mfc-v2.json");
    let load = exchange::load(fixture.as_bytes()).expect("fixture import");
    assert_eq!(load.format, DumpFormat::Proxmark3);
    assert_eq!(load.dump.layout, Layout::Classic1K);
    let card = load.dump.card.as_ref().expect("card");
    assert_eq!(card.uid, vec![1, 2, 3, 4]);
    assert_eq!(card.atqa, [0x00, 0x04]);
    assert_eq!(card.sak, 0x08);
    assert!(load.warnings.is_empty(), "{:?}", load.warnings);

    let encoded = exchange::export(&load.dump, DumpFormat::Proxmark3).expect("re-export");
    let document: serde_json::Value = serde_json::from_slice(&encoded).expect("json");
    assert_eq!(document["Created"], "NFC Foundry");
    assert_eq!(document["FileType"], "mfc v2");
    assert_eq!(document["Card"]["UID"], "01020304");
    assert_eq!(document["blocks"].as_object().expect("blocks").len(), 64);
    assert_eq!(document["SectorKeys"]["0"]["KeyA"], "A0A1A2A3A4A5");
    assert_eq!(document["SectorKeys"]["15"]["AccessConditions"], "FF078000");
    assert!(
        document["SectorKeys"]["0"]["AccessConditionsText"]
            .as_object()
            .expect("access text")
            .contains_key("block3")
    );
    let reloaded = exchange::load(&encoded).expect("re-import");
    assert_eq!(reloaded.dump, load.dump);
    assert!(
        !String::from_utf8_lossy(&encoded)
            .to_ascii_lowercase()
            .contains("nfcx")
    );
}

#[test]
fn project_export_of_a_default_dump_keeps_unknown_bytes_unknown() {
    let dump = Dump::new(Layout::Classic1K);
    let encoded = exchange::export(&dump, DumpFormat::Project).expect("project");
    let text = String::from_utf8(encoded).expect("utf-8");
    assert!(text.contains("\"knownMask\": \"0000\""));
    let loaded = exchange::load_as(text.as_bytes(), DumpFormat::Project).expect("project");
    assert_eq!(loaded.dump.blocks[0].known_mask, 0);
}

fn sidecar_card() -> CardInfo {
    CardInfo::new(
        vec![0x01, 0x02, 0x03, 0x04],
        [0x00, 0x04],
        0x08,
        CardType::MifareClassic1K,
    )
}

#[test]
fn sidecar_round_trip_preserves_card_and_metadata() {
    let mut dump = complete_dump(Layout::Classic1K).with_card(sidecar_card());
    dump.metadata = DumpMetadata {
        device: Some("pn532_uart:/dev/ttyUSB0".to_owned()),
        started_at: Some(1_762_819_200),
        finished_at: Some(1_762_819_205),
        keys: vec![
            DumpKey {
                sector: 0,
                key_type: KeyType::A,
                key: Key::from_hex("FFFFFFFFFFFF").expect("key"),
                source: Some("read_context".to_owned()),
            },
            DumpKey {
                sector: 1,
                key_type: KeyType::B,
                key: Key::from_hex("A0A1A2A3A4A5").expect("key"),
                source: Some("recovery".to_owned()),
            },
        ],
    };
    let encoded = exchange::encode_sidecar(&dump).expect("sidecar encode");
    let document: serde_json::Value = serde_json::from_slice(&encoded).expect("json");
    assert_eq!(document["version"], 1);
    assert_eq!(document["complete"], true);
    assert_eq!(document["layout"], "classic_1k");
    assert_eq!(document["card"]["uid"], "01020304");
    assert_eq!(document["card"]["atqa"], "0004");
    assert_eq!(document["card"]["sak"], 8);
    assert_eq!(document["keys"][0]["type"], "A");
    assert_eq!(document["keys"][0]["key"], "FFFFFFFFFFFF");
    assert_eq!(document["keys"][0]["source"], "read_context");

    let loaded = exchange::parse_sidecar(&encoded).expect("sidecar parse");
    assert!(loaded.usable);
    assert!(loaded.warnings.is_empty());
    assert_eq!(loaded.layout, dump.layout);
    assert_eq!(loaded.complete, Some(true));
    assert_eq!(loaded.card, dump.card);
    assert_eq!(loaded.metadata, dump.metadata);
}

#[test]
fn sidecar_accepts_the_legacy_key_a_key_b_shape() {
    let sidecar = json!({
        "version": 1,
        "complete": true,
        "layout": "classic_1k",
        "card": {"uid": "01020304", "atqa": "0004", "sak": 8},
        "keys": [
            {"sector": 0, "keyA": "FFFFFFFFFFFF", "keyB": "A0A1A2A3A4A5"},
            {"sector": 1, "keyA": "D3F7D3F7D3F7"},
        ],
    });
    let loaded = exchange::parse_sidecar(sidecar.to_string().as_bytes()).expect("sidecar");
    assert!(loaded.usable);
    assert_eq!(loaded.metadata.keys.len(), 3);
    assert_eq!(loaded.metadata.keys[0].sector, 0);
    assert_eq!(loaded.metadata.keys[0].key_type, KeyType::A);
    assert_eq!(loaded.metadata.keys[1].key_type, KeyType::B);
    assert_eq!(loaded.metadata.keys[2].sector, 1);
    assert_eq!(loaded.metadata.keys[2].key_type, KeyType::A);
}

#[test]
fn sidecar_accepts_and_ignores_legacy_blocks() {
    let blocks = (0..Layout::Classic1K.block_count())
        .map(|block| json!({"block": block, "data": "00".repeat(16)}))
        .collect::<Vec<_>>();
    let sidecar = json!({
        "version": 1,
        "complete": true,
        "layout": "classic_1k",
        "keys": [{"sector": 0, "type": "A", "key": "FFFFFFFFFFFF"}],
        "blocks": blocks,
    });
    let loaded = exchange::parse_sidecar(sidecar.to_string().as_bytes()).expect("sidecar");
    assert!(loaded.usable);
    assert_eq!(loaded.metadata.keys.len(), 1);

    let mut short = sidecar.clone();
    short["blocks"] = json!([]);
    let error = exchange::parse_sidecar(short.to_string().as_bytes()).expect_err("block count");
    assert!(error.to_string().contains("legacy blocks"), "{error}");
}

#[test]
fn sidecar_unknown_version_warns_without_metadata() {
    let sidecar = json!({
        "version": 9,
        "complete": true,
        "layout": "classic_1k",
        "keys": [{"sector": 0, "type": "A", "key": "FFFFFFFFFFFF"}],
    });
    let loaded = exchange::parse_sidecar(sidecar.to_string().as_bytes()).expect("sidecar");
    assert!(!loaded.usable);
    assert!(loaded.metadata.keys.is_empty());
    assert_eq!(loaded.warnings.len(), 1);
    assert!(
        loaded.warnings[0].contains("version 9"),
        "{:?}",
        loaded.warnings
    );
}

#[test]
fn sidecar_rejects_invalid_keys_and_sector_bounds() {
    let duplicate = json!({
        "version": 1,
        "layout": "classic_1k",
        "keys": [
            {"sector": 0, "type": "A", "key": "FFFFFFFFFFFF"},
            {"sector": 0, "keyA": "A0A1A2A3A4A5"},
        ],
    });
    let error = exchange::parse_sidecar(duplicate.to_string().as_bytes()).expect_err("duplicate");
    assert!(error.to_string().contains("repeats"), "{error}");

    let outside = json!({
        "version": 1,
        "layout": "classic_1k",
        "keys": [{"sector": 16, "type": "A", "key": "FFFFFFFFFFFF"}],
    });
    let error = exchange::parse_sidecar(outside.to_string().as_bytes()).expect_err("bounds");
    assert!(error.to_string().contains("outside"), "{error}");

    let missing_key = json!({
        "version": 1,
        "layout": "classic_1k",
        "keys": [{"sector": 0, "type": "A"}],
    });
    let error = exchange::parse_sidecar(missing_key.to_string().as_bytes()).expect_err("key");
    assert!(error.to_string().contains("has no key"), "{error}");
}
