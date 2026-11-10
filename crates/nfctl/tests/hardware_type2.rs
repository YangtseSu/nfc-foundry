//! Environment-gated Type 2 / NTAG workflow checks.
//!
//! Read-only:
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
//! cargo test -p nfctl --features hardware -- --ignored
//! ```
//!
//! The NDEF round trip additionally requires the `hardware-write` feature and
//! `NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD`. The original user memory is restored
//! from a project backup at the end of the test.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_app::Service;
#[cfg(feature = "hardware-write")]
use nfc_app::classic::BackupStore;
use nfc_app::type2::Type2ReadResult;
#[cfg(feature = "hardware-write")]
use nfc_app::type2::{Type2Format, Type2NdefWrite, Type2Password, Type2Restore};
use nfc_core::{CancelToken, CardType};
use nfc_drivers::DriverRegistry;
#[cfg(feature = "hardware-write")]
use nfc_ndef::{NdefMessage, TextRecord};
#[cfg(feature = "hardware-write")]
use nfc_type2::{NdefState, RestoreOptions, exchange};

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
        .expect("set NFC_FOUNDRY_DEVICE to the exact pn532_uart:<port> or acr122_usb:<bus>:<addr>")
}

fn service(conn_string: &str) -> Service {
    let mut service = Service::with_transport(std::sync::Arc::new(DriverRegistry));
    service.refresh_devices().expect("enumerate readers");
    service
        .select_device(conn_string)
        .expect("select the reader");
    service
}

#[test]
#[ignore = "requires a Type 2 / NTAG tag"]
fn reads_and_reports_ndef_state() {
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let card = service.card_info().expect("select the tag");
    assert_eq!(
        card.card_type,
        CardType::Type2,
        "place an NFC Forum Type 2 tag on the reader; selected {:?}",
        card.card_type
    );
    let read: Type2ReadResult = service
        .type2_read(CancelToken::new())
        .expect("read the Type 2 tag");
    println!(
        "{} uid={} pages={}/{} warnings={}",
        read.image.layout.product.name(),
        read.image.uid_hex(),
        read.image.known_pages(),
        read.image.layout.page_count(),
        read.warnings.len()
    );
    for warning in &read.warnings {
        println!("warning: {warning}");
    }
    let ndef = service
        .type2_ndef_read(CancelToken::new())
        .expect("read NDEF");
    println!(
        "ndef state={:?} capacity={} bytes",
        ndef.ndef.state, ndef.ndef.capacity
    );
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires an owned Type 2 tag and NFC_FOUNDRY_HARDWARE_WRITE"]
fn ndef_write_read_and_restore_round_trip() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD to allow writes"
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let card = service.card_info().expect("select the tag");
    assert_eq!(card.card_type, CardType::Type2);
    let original = service
        .type2_read(CancelToken::new())
        .expect("read the original image");
    assert!(
        original.image.is_complete(),
        "the original image must be complete before a write"
    );

    let backup_dir = env::var_os("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("nfc-foundry-hardware"));
    let backup = BackupStore::new(&backup_dir);
    let message = NdefMessage::single(
        TextRecord::new("NFC Foundry hardware check", "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message");

    service
        .type2_ndef_format(Type2Format::new(&backup).confirm(true))
        .expect("format NDEF");
    service
        .type2_ndef_write(Type2NdefWrite::new(message.clone(), &backup))
        .expect("write NDEF");
    let read = service
        .type2_ndef_read(CancelToken::new())
        .expect("read NDEF");
    assert_eq!(read.ndef.state, NdefState::Message(message));
    println!(
        "NDEF round trip: {} bytes, {} pages",
        read.ndef.message_bytes,
        read.image.known_pages()
    );

    let dump = exchange::export(&original.image).expect("encode original");
    let path = backup_dir.join("original-type2.json");
    std::fs::write(&path, dump).expect("save original image");
    service
        .type2_restore(
            Type2Restore::new(original.image.clone(), &backup)
                .options(RestoreOptions {
                    restore_user_memory: true,
                    ..RestoreOptions::default()
                })
                .confirm(true),
        )
        .expect("restore original user memory");
    println!("restored original image from {}", path.display());
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires an owned NTAG with a known password"]
fn password_configuration_writes_and_verifies() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD to allow configuration writes"
    );
    let password = parse_password(
        &env::var("NFC_FOUNDRY_TYPE2_PASSWORD")
            .expect("set NFC_FOUNDRY_TYPE2_PASSWORD to 8 hex digits"),
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let backup_dir = env::var_os("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("nfc-foundry-hardware"));
    let backup = BackupStore::new(&backup_dir);
    service
        .type2_password(
            Type2Password::new(0x20, 0x00, password, [0xAA, 0x55], &backup).confirm(true),
        )
        .expect("write password configuration");
    println!("password configuration written and PWD_AUTH verified");
}

#[cfg(feature = "hardware-write")]
fn parse_password(value: &str) -> [u8; 4] {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    assert_eq!(
        compact.len(),
        8,
        "NFC_FOUNDRY_TYPE2_PASSWORD must be 8 hex digits"
    );
    let bytes = compact.as_bytes();
    let mut password = [0u8; 4];
    for (index, byte) in password.iter_mut().enumerate() {
        let digit = |character: u8| match character {
            b'0'..=b'9' => character - b'0',
            b'a'..=b'f' => character - b'a' + 10,
            b'A'..=b'F' => character - b'A' + 10,
            _ => panic!("NFC_FOUNDRY_TYPE2_PASSWORD contains a non-hexadecimal character"),
        };
        *byte = (digit(bytes[index * 2]) << 4) | digit(bytes[index * 2 + 1]);
    }
    password
}
