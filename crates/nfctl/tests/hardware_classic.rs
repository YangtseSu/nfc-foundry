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
//! feature and `NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD`.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_app::Service;
#[cfg(feature = "hardware-write")]
use nfc_app::classic::{BackupStore, ClassicBlockWrite};
use nfc_app::classic::{
    ClassicKeyscan, ClassicRead, KeyEntry, KeyKind, KeySource, KeyStore, KeyTypes,
};
#[cfg(feature = "hardware-write")]
use nfc_classic::Layout;
#[cfg(feature = "hardware-write")]
use nfc_core::CancelToken;
use nfc_core::{CardType, Key, KeyType};
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
        .expect("select the PN532 reader");
    service
}

fn key_type() -> KeyType {
    match env::var("NFC_FOUNDRY_HARDWARE_KEY_TYPE").as_deref() {
        Ok("B" | "b") => KeyType::B,
        _ => KeyType::A,
    }
}

fn key_store(key: Key) -> KeyStore {
    let mut store = KeyStore::new();
    let kind = match key_type() {
        KeyType::A => KeyKind::A,
        KeyType::B => KeyKind::B,
    };
    store
        .add(KeyEntry::new(None, kind, key, KeySource::Manual))
        .expect("store the key");
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
#[ignore = "requires an owned MIFARE Classic card and NFC_FOUNDRY_HARDWARE_WRITE"]
fn writes_and_restores_one_data_block() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD to allow a write"
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
        .expect("read the original block");
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
        .expect("verify the original block");
    assert_eq!(read_back, original);
}
