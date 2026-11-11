//! Environment-gated PN532 UART Classic workflow checks.
//!
//! Read-only:
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
//! cargo test -p nfctl --features hardware -- --ignored
//! ```
//!
//! The temporary write/restore additionally requires the `hardware-write`
//! feature and `NFC_FOUNDRY_HARDWARE_WRITE=yes`. Set
//! `NFC_FOUNDRY_HARDWARE_KEY_2` when the card uses a second sector key.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_app::Service;
#[cfg(feature = "hardware-write")]
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicUidExecution, ClassicUidPreflight,
    ClassicWriteExecution, ClassicWritePreflight, RestoreUidState,
};
use nfc_app::classic::{
    ClassicKeyscan, ClassicRead, KeyEntry, KeyKind, KeySource, KeyStore, KeyTypes,
};
#[cfg(feature = "hardware-write")]
use nfc_classic::Layout;
#[cfg(feature = "hardware-write")]
use nfc_core::CancelToken;
use nfc_core::{CardType, Key};
use nfc_drivers::DriverRegistry;

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

fn lock_hardware() -> MutexGuard<'static, ()> {
    HARDWARE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn conn_string() -> String {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE").as_deref(),
        Ok("1"),
        "set NFC_FOUNDRY_HARDWARE=1 to run hardware tests"
    );
    env::var("NFC_FOUNDRY_DEVICE")
        .expect("set NFC_FOUNDRY_DEVICE to the exact pn532_uart:<serial-port> connstring")
}

fn key() -> Key {
    let value = env::var("NFC_FOUNDRY_HARDWARE_KEY").unwrap_or_else(|_| "FFFFFFFFFFFF".to_owned());
    Key::from_hex(&value).expect("NFC_FOUNDRY_HARDWARE_KEY must be 12 hex digits")
}

fn service(conn_string: &str) -> Service {
    let mut service = Service::with_transport(std::sync::Arc::new(DriverRegistry));
    service.refresh_devices().expect("enumerate readers");
    service
        .select_device(conn_string)
        .expect("select the reader");
    service
}

fn key_store(key: Key) -> KeyStore {
    let mut store = KeyStore::new();
    let mut keys = vec![key];
    for variable in ["NFC_FOUNDRY_HARDWARE_KEY_2", "NFC_FOUNDRY_HARDWARE_KEY_3"] {
        if let Some(extra) = env::var(variable).ok().map(|value| {
            Key::from_hex(&value).unwrap_or_else(|_| panic!("{variable} must be 12 hex digits"))
        }) && !keys.contains(&extra)
        {
            keys.push(extra);
        }
    }
    for key in keys {
        for kind in [KeyKind::A, KeyKind::B] {
            store
                .add(KeyEntry::new(None, kind, key, KeySource::Manual))
                .expect("store the key");
        }
    }
    store
}

#[test]
#[ignore = "requires a PN532 and a MIFARE Classic card"]
fn dumps_and_scans_a_classic_card() {
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let card = service.card_info().expect("select the card");
    assert!(
        matches!(
            card.card_type,
            CardType::MifareClassic1K | CardType::MifareClassic4K
        ),
        "place a MIFARE Classic 1K or 4K card on the reader; selected {:?}",
        card.card_type
    );
    let key = key();
    let store = key_store(key);

    let read = service
        .classic_read(ClassicRead::new(&store).include_common_keys(false))
        .expect("read the Classic card");
    let readability = read.dump.readability();
    assert!(
        readability.readable_sectors > 0,
        "no sector authenticated with NFC_FOUNDRY_HARDWARE_KEY"
    );
    println!(
        "{} uid={} readable={}/{} sectors",
        read.dump.layout.name(),
        read.dump
            .card
            .as_ref()
            .map_or_else(String::new, |card| card.uid_hex()),
        readability.readable_sectors,
        readability.total_sectors
    );

    let scan = service
        .classic_keyscan(
            ClassicKeyscan::new(&store, Vec::new())
                .include_common_keys(false)
                .key_types(KeyTypes::a_only()),
        )
        .expect("scan Key A per sector");
    assert!(
        scan.verified > 0,
        "the configured key did not verify on any sector"
    );
    println!(
        "keyscan attempts={} verified={}",
        scan.attempts, scan.verified
    );
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires a MIFARE Classic card and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn writes_and_restores_one_data_block() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow a write"
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let key = key();
    let store = key_store(key);
    let block = env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(4);
    let card = service.card_info().expect("select the Classic card");
    let layout = match card.card_type {
        nfc_core::CardType::MifareClassic1K => Layout::Classic1K,
        nfc_core::CardType::MifareClassic4K => Layout::Classic4K,
        other => panic!("expected a MIFARE Classic card, selected {other:?}"),
    };
    assert!(
        block != 0 && !layout.is_trailer(block).expect("block in layout"),
        "NFC_FOUNDRY_HARDWARE_BLOCK={block} must be an ordinary data block"
    );
    let directory = env::var("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .map(BackupStore::new)
        .unwrap_or_default();
    let cancel = CancelToken::new();
    let original = service
        .classic_read_block(block, &store, false, cancel.clone())
        .expect("read the original block")
        .data;
    let mut changed = original;
    changed[0] ^= 0xFF;

    let first = service
        .classic_write_block(
            ClassicBlockWrite::new(block, changed, &store, &directory).include_common_keys(false),
        )
        .expect("write the changed block");
    assert!(first.changed);
    println!("wrote block {block}, backup {:?}", first.backup);

    let second = service
        .classic_write_block(
            ClassicBlockWrite::new(block, original, &store, &directory).include_common_keys(false),
        )
        .expect("restore the original block");
    assert!(second.changed);
    println!("restored block {block}, backup {:?}", second.backup);

    let read_back = service
        .classic_read_block(block, &store, false, CancelToken::new())
        .expect("verify the original block")
        .data;
    assert_eq!(read_back, original);
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires an owned magic Classic 1K and NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes"]
fn changes_and_restores_the_card_uid() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow a write"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_UID_CONFIRM").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes to allow a UID change"
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let key = key();
    let store = key_store(key);
    let directory = env::var("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .map(BackupStore::new)
        .unwrap_or_default();
    let card = service.card_info().expect("select the magic Classic card");
    assert_eq!(
        card.card_type,
        CardType::MifareClassic1K,
        "the UID path requires a MIFARE Classic 1K card"
    );
    assert_eq!(card.uid.len(), 4, "the UID path requires a four-byte UID");
    let original = service
        .classic_read_block(0, &store, false, CancelToken::new())
        .expect("read manufacturer block 0")
        .data;
    let original_uid: [u8; 4] = original[0..4].try_into().expect("four-byte UID");
    let target = env::var("NFC_FOUNDRY_HARDWARE_UID")
        .ok()
        .map(|value| nfc_app::classic::parse_uid(&value).expect("valid NFC_FOUNDRY_HARDWARE_UID"))
        .unwrap_or_else(|| {
            let mut uid = original_uid;
            uid[3] ^= 0x01;
            uid
        });
    assert_ne!(target, original_uid, "choose a different target UID");
    println!(
        "UID write: {} -> {} · original block 0 {:02X?}",
        nfc_app::classic::format_uid(&original_uid),
        nfc_app::classic::format_uid(&target),
        original
    );

    let plan = service
        .classic_uid_preflight(
            ClassicUidPreflight::new(target, &store)
                .include_common_keys(false)
                .advanced(true),
        )
        .expect("UID preflight");
    println!(
        "plan backends {:?} · old {:02X?} · new {:02X?}",
        plan.backends, plan.old_block0, plan.new_block0
    );
    let result = service
        .classic_uid_execute(
            ClassicUidExecution::new(plan.token, &store, &directory)
                .include_common_keys(false)
                .advanced(true)
                .confirmed(true),
        )
        .expect("UID write");
    assert!(result.changed);
    assert_eq!(result.card_after.uid, target.to_vec());
    assert_eq!(result.new_block0[0..4], target);
    println!(
        "UID write verified: uid={} backend={:?} backup={:?}",
        result.card_after.uid_hex(),
        result.backend,
        result.backup.as_ref().map(|record| &record.backup.path)
    );

    let plan = service
        .classic_uid_preflight(
            ClassicUidPreflight::new(original_uid, &store)
                .include_common_keys(false)
                .advanced(true),
        )
        .expect("restore preflight");
    let restored = service
        .classic_uid_execute(
            ClassicUidExecution::new(plan.token, &store, &directory)
                .include_common_keys(false)
                .advanced(true)
                .confirmed(true),
        )
        .expect("restore the original UID");
    assert!(restored.changed);
    assert_eq!(restored.card_after.uid, original_uid.to_vec());
    assert_eq!(restored.new_block0, original);
    println!(
        "UID restore verified: uid={} block0={:02X?}",
        restored.card_after.uid_hex(),
        restored.new_block0
    );
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires an owned magic Classic 1K and NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes"]
fn restores_a_dump_and_changes_the_uid_last() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow a write"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_UID_CONFIRM").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes to allow a UID change"
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let key = key();
    let store = key_store(key);
    let directory = env::var("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .map(BackupStore::new)
        .unwrap_or_default();
    let card = service.card_info().expect("select the magic Classic card");
    assert_eq!(
        card.card_type,
        CardType::MifareClassic1K,
        "restore with UID requires a MIFARE Classic 1K card"
    );
    assert_eq!(
        card.uid.len(),
        4,
        "restore with UID requires a four-byte UID"
    );
    let original = service
        .classic_read(ClassicRead::new(&store).include_common_keys(false))
        .expect("read the full dump")
        .dump;
    if !original.complete() {
        let unknown: Vec<usize> = original
            .blocks
            .iter()
            .filter(|block| !block.complete())
            .map(|block| block.number)
            .collect();
        panic!("the restore source must be a full dump; unknown blocks {unknown:?}");
    }
    let original_uid: [u8; 4] = original.blocks[0].data[0..4]
        .try_into()
        .expect("four-byte UID");
    let data_block = 4usize;
    let mut source = original.clone();
    let mut changed_data = original.blocks[data_block].data;
    changed_data[0] ^= 0xFF;
    source.blocks[data_block].set_read(changed_data);
    let target = env::var("NFC_FOUNDRY_HARDWARE_UID")
        .ok()
        .map(|value| nfc_app::classic::parse_uid(&value).expect("valid NFC_FOUNDRY_HARDWARE_UID"))
        .unwrap_or_else(|| {
            let mut uid = original_uid;
            uid[3] ^= 0x01;
            uid
        });
    assert_ne!(target, original_uid, "choose a different target UID");
    println!(
        "restore with UID: {} -> {} · {} blocks",
        nfc_app::classic::format_uid(&original_uid),
        nfc_app::classic::format_uid(&target),
        source.blocks.len()
    );

    let plan = service
        .classic_write_preflight(
            ClassicWritePreflight::new(source.clone(), &store)
                .include_common_keys(false)
                .uid_target(target)
                .advanced(true),
        )
        .expect("restore with UID preflight");
    let uid_plan = plan.uid.as_ref().expect("UID plan");
    println!(
        "plan backends {:?} · old {:02X?} · new {:02X?}",
        uid_plan.backends, uid_plan.old_block0, uid_plan.new_block0
    );
    let result = service
        .classic_write_execute(
            ClassicWriteExecution::new(plan.token, source.clone(), &store, &directory)
                .include_common_keys(false)
                .advanced(true)
                .confirmed(true),
        )
        .expect("restore with UID");
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    let uid = result.uid.expect("UID result");
    assert_eq!(uid.state, RestoreUidState::Applied);
    assert_eq!(
        uid.card_after.as_ref().expect("card after").uid,
        target.to_vec()
    );
    println!(
        "restore with UID verified: uid={} backend={:?} backup={:?}",
        uid.card_after.as_ref().expect("card after").uid_hex(),
        uid.backend,
        uid.backup.as_ref().map(|record| &record.backup.path)
    );

    let read_back = service
        .classic_read(ClassicRead::new(&store).include_common_keys(false))
        .expect("full read after restore")
        .dump;
    assert_eq!(read_back.card.as_ref().expect("card").uid, target.to_vec());
    for (before, after) in source.blocks.iter().zip(&read_back.blocks) {
        if before.number == 0 {
            continue;
        }
        assert_eq!(after.data, before.data, "block {}", before.number);
    }
    assert_eq!(&read_back.blocks[0].data[0..4], target.as_slice());
    assert_eq!(
        read_back.blocks[0].data[4],
        nfc_classic::bcc_for_uid(&target).expect("target BCC")
    );

    let mut return_source = read_back;
    return_source.blocks[data_block].set_read(original.blocks[data_block].data);
    let plan = service
        .classic_write_preflight(
            ClassicWritePreflight::new(return_source.clone(), &store)
                .include_common_keys(false)
                .uid_target(original_uid)
                .advanced(true),
        )
        .expect("return preflight");
    let returned = service
        .classic_write_execute(
            ClassicWriteExecution::new(plan.token, return_source, &store, &directory)
                .include_common_keys(false)
                .advanced(true)
                .confirmed(true),
        )
        .expect("restore the original dump and UID");
    assert!(returned.failures.is_empty(), "{:?}", returned.failures);
    let uid = returned.uid.expect("return UID result");
    assert_eq!(uid.state, RestoreUidState::Applied);
    assert_eq!(
        uid.card_after.as_ref().expect("card after").uid,
        original_uid.to_vec()
    );
    assert_eq!(uid.new_block0, original.blocks[0].data);

    let final_read = service
        .classic_read(ClassicRead::new(&store).include_common_keys(false))
        .expect("full read after return")
        .dump;
    assert_eq!(
        final_read.card.as_ref().expect("card").uid,
        original_uid.to_vec()
    );
    for (before, after) in original.blocks.iter().zip(&final_read.blocks) {
        assert_eq!(after.data, before.data, "block {}", before.number);
    }
    println!(
        "original dump and UID restored: {}",
        final_read.card.as_ref().expect("card").uid_hex()
    );
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires a MIFARE Classic card and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn writes_and_restores_a_sector_trailer() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow a write"
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let key = key();
    let store = key_store(key);
    let directory = env::var("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .map(BackupStore::new)
        .unwrap_or_default();
    let block = env::var("NFC_FOUNDRY_HARDWARE_TRAILER")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(7);
    let card = service.card_info().expect("select the Classic card");
    let layout = match card.card_type {
        nfc_core::CardType::MifareClassic1K => Layout::Classic1K,
        nfc_core::CardType::MifareClassic4K => Layout::Classic4K,
        other => panic!("expected a MIFARE Classic card, selected {other:?}"),
    };
    assert!(
        layout
            .is_trailer(block)
            .expect("block is within the layout"),
        "NFC_FOUNDRY_HARDWARE_TRAILER={block} must be a sector trailer"
    );
    let original = service
        .classic_read_block(block, &store, false, CancelToken::new())
        .expect("read the original trailer")
        .data;
    let mut changed = original;
    changed[9] ^= 0xFF;

    let first = service
        .classic_write_block(
            ClassicBlockWrite::new(block, changed, &store, &directory)
                .include_common_keys(false)
                .confirm_trailer(true),
        )
        .expect("write the changed trailer");
    assert!(first.changed);
    println!("wrote trailer block {block}, backup {:?}", first.backup);
    let read_back = service
        .classic_read_block(block, &store, false, CancelToken::new())
        .expect("verify the changed trailer")
        .data;
    assert_eq!(read_back, changed);

    let second = service
        .classic_write_block(
            ClassicBlockWrite::new(block, original, &store, &directory)
                .include_common_keys(false)
                .confirm_trailer(true),
        )
        .expect("restore the original trailer");
    assert!(second.changed);
    println!("restored trailer block {block}, backup {:?}", second.backup);
    let read_back = service
        .classic_read_block(block, &store, false, CancelToken::new())
        .expect("verify the original trailer")
        .data;
    assert_eq!(read_back, original);
}
