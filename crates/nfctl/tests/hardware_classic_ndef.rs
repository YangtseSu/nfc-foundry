//! Environment-gated Classic NDEF hardware checks.
//!
//! Read-only:
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware -- --ignored
//! ```
//!
//! The format/write round trip replaces every NFC sector and trailer, then
//! restores the original dump. It requires both write tokens:
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD \
//! NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=I_OWN_THIS_CARD \
//! cargo test -p nfctl --features hardware-write -- --ignored
//! ```

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_app::Service;
#[cfg(feature = "hardware-write")]
use nfc_app::classic::{
    BackupStore, ClassicNdefFormat, ClassicNdefWrite, ClassicRead, ClassicRestore,
};
use nfc_app::classic::{ClassicNdefRead, KeyStore};
use nfc_core::CardType;
use nfc_drivers::DriverRegistry;
#[cfg(feature = "hardware-write")]
use nfc_ndef::{NdefMessage, TextRecord};

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
    env::var("NFC_FOUNDRY_DEVICE").expect("set NFC_FOUNDRY_DEVICE to the exact reader connstring")
}

fn service(conn_string: &str) -> Service {
    let mut service = Service::with_transport(std::sync::Arc::new(DriverRegistry));
    service.refresh_devices().expect("enumerate readers");
    service
        .select_device(conn_string)
        .expect("select the reader");
    service
}

fn assert_classic(service: &mut Service) -> CardType {
    let card = service.card_info().expect("select the card");
    assert!(
        matches!(
            card.card_type,
            CardType::MifareClassic1K | CardType::MifareClassic4K
        ),
        "place a MIFARE Classic 1K or 4K card on the reader; selected {:?}",
        card.card_type
    );
    card.card_type
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn reads_the_classic_ndef_mapping() {
    let _guard = lock_hardware();
    let mut service = service(&conn_string());
    assert_classic(&mut service);
    let store = KeyStore::new();
    let read = service
        .classic_ndef_read(ClassicNdefRead::new(&store).include_common_keys(true))
        .expect("read the Classic NDEF mapping");
    match &read.info.layout {
        Some(layout) => println!(
            "mapping={}.{} sectors={:?} blocks={} bytes={} state={:?}",
            layout.version.major,
            layout.version.minor,
            layout.sectors,
            layout.data_blocks.len(),
            layout.data_len(),
            read.info.ndef.state
        ),
        None => println!("no MAD; state={:?}", read.info.ndef.state),
    }
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires an owned Classic card, both write tokens, and a reader"]
fn formats_writes_reads_and_restores_a_classic_card() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD to allow writes"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_NDEF_FORMAT").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=I_OWN_THIS_CARD to allow a full format"
    );
    let _guard = lock_hardware();
    let mut service = service(&conn_string());
    assert_classic(&mut service);
    let store = KeyStore::new();
    let backup = env::var("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .map(BackupStore::new)
        .unwrap_or_default();
    let original = service
        .classic_read(ClassicRead::new(&store).include_common_keys(true))
        .expect("read the original card");
    assert!(
        original.dump.complete(),
        "the format test needs a complete original read"
    );

    let formatted = service
        .classic_ndef_format(
            ClassicNdefFormat::new(&store, &backup)
                .include_common_keys(true)
                .confirm(true),
        )
        .expect("format the card");
    assert_eq!(formatted.info.ndef.state, nfc_app::ClassicNdefState::Empty);
    println!("formatted: {}", formatted.info.ndef.capacity);

    let message = NdefMessage::single(
        TextRecord::new("nfc foundry classic", "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message");
    let written = service
        .classic_ndef_write(
            ClassicNdefWrite::new(message.clone(), &store, &backup).include_common_keys(true),
        )
        .expect("write the Text record");
    assert_eq!(
        written.info.ndef.state,
        nfc_app::ClassicNdefState::Message(message)
    );
    println!("wrote {} blocks", written.written_blocks);

    let read = service
        .classic_ndef_read(ClassicNdefRead::new(&store).include_common_keys(true))
        .expect("read the NDEF message back");
    assert!(matches!(
        read.info.ndef.state,
        nfc_app::ClassicNdefState::Message(_)
    ));

    let restored = service
        .classic_restore(
            ClassicRestore::new(original.dump, &store, &backup)
                .include_common_keys(true)
                .confirmed(true),
        )
        .expect("restore the original dump");
    println!(
        "restored: {} written, {} unchanged, {} failures",
        restored.written_blocks,
        restored.unchanged_blocks,
        restored.failures.len()
    );
    assert!(
        restored.failures.is_empty(),
        "restore failures: {:?}",
        restored.failures
    );
}
