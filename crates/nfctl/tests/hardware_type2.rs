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
//! `NFC_FOUNDRY_HARDWARE_WRITE=yes`. The original user memory is restored
//! from a project backup at the end of the test.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_app::Service;
#[cfg(feature = "hardware-write")]
use nfc_app::classic::BackupStore;
#[cfg(feature = "hardware-write")]
use nfc_app::type2::{Type2Auth, Type2Format, Type2NdefWrite, Type2Password, Type2Restore};
use nfc_app::type2::{Type2ReadRequest, Type2ReadResult};
use nfc_core::{CancelToken, CardType};
use nfc_drivers::DriverRegistry;
#[cfg(feature = "hardware-write")]
use nfc_ndef::{NdefMessage, TextRecord};
#[cfg(feature = "hardware-write")]
#[cfg(feature = "hardware-write")]
use nfc_type2::{Configuration, NdefState, RestoreOptions, Type2Image, exchange};

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
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
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
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read NDEF");
    println!(
        "ndef state={:?} capacity={} bytes",
        ndef.ndef.state, ndef.ndef.capacity
    );
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires a Type 2 tag and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn ndef_write_read_and_restore_round_trip() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow writes"
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let card = service.card_info().expect("select the tag");
    assert_eq!(card.card_type, CardType::Type2);
    let original = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
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
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
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
#[ignore = "requires an owned NTAG21x tag and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn password_authentication_round_trip() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow configuration writes"
    );
    let password = parse_hex::<4>(
        "NFC_FOUNDRY_TYPE2_PASSWORD",
        &env::var("NFC_FOUNDRY_TYPE2_PASSWORD")
            .expect("set NFC_FOUNDRY_TYPE2_PASSWORD to 8 hex digits"),
    );
    let pack = parse_hex::<2>(
        "NFC_FOUNDRY_TYPE2_PACK",
        &env::var("NFC_FOUNDRY_TYPE2_PACK").expect("set NFC_FOUNDRY_TYPE2_PACK to 4 hex digits"),
    );
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let backup_dir = env::var_os("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("nfc-foundry-hardware"));
    let backup = BackupStore::new(&backup_dir);

    let original = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read the original image");
    assert!(
        original.image.is_complete(),
        "the original image must be complete before a protection test"
    );
    let config = Configuration::from_image(&original.image).expect("configuration");
    assert_eq!(
        config.auth0, 0xFF,
        "the dedicated tag must start unprotected; restore it or use another tag"
    );
    let original_ndef = service
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read the original NDEF");
    let original_state = original_ndef.ndef.state.clone();
    println!(
        "original: {} pages, NDEF {:?} ({} bytes)",
        original.image.known_pages(),
        original_state,
        original_ndef.ndef.message_bytes
    );

    let source = original.image.clone();
    service
        .type2_password(Type2Password::new(0x20, 0x80, password, pack, &backup).confirm(true))
        .expect("set AUTH0=0x20 with PROT");

    let auth = Type2Auth::new(password, pack);
    let wrong = Type2Auth::new(password, [0x00, 0x00]);
    assert!(
        service
            .type2_read(Type2ReadRequest::new(CancelToken::new()).auth(Some(wrong)))
            .is_err(),
        "a wrong PACK must be refused"
    );
    let protected = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()).auth(Some(auth)))
        .expect("authenticated read");
    assert!(protected.image.is_complete());
    println!(
        "AUTH0=0x20: authenticated read {}/{} pages",
        protected.image.known_pages(),
        protected.image.layout.page_count()
    );

    service
        .type2_ndef_format(Type2Format::new(&backup).confirm(true).auth(Some(auth)))
        .expect("authenticated format");
    let test_message = NdefMessage::single(
        TextRecord::new("NFC Foundry authenticated access", "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message");
    service
        .type2_ndef_write(Type2NdefWrite::new(test_message.clone(), &backup).auth(Some(auth)))
        .expect("authenticated NDEF write");
    let rewritten = service
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()).auth(Some(auth)))
        .expect("authenticated NDEF read");
    assert_eq!(rewritten.ndef.state, NdefState::Message(test_message));
    println!("authenticated NDEF write and read verified");

    service
        .type2_password(
            Type2Password::new(0xFF, 0x00, password, pack, &backup)
                .current(Some(auth))
                .confirm(true),
        )
        .expect("clear protection");
    let cleared = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("unprotected read");
    assert!(cleared.image.is_complete());
    assert_eq!(
        Configuration::from_image(&cleared.image)
            .expect("configuration")
            .auth0,
        0xFF
    );

    service
        .type2_restore(
            Type2Restore::new(source.clone(), &backup)
                .options(RestoreOptions {
                    restore_user_memory: true,
                    ..RestoreOptions::default()
                })
                .confirm(true),
        )
        .expect("restore the original user memory");
    let final_read = service
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("final NDEF read");
    assert_eq!(
        final_read.ndef.state, original_state,
        "the original NDEF state must be restored"
    );
    let mut check_pages = vec![2, 3];
    if let Some(page) = source.layout.dynamic_lock {
        check_pages.push(page);
    }
    if let Some(config) = source.layout.config {
        check_pages.extend([config.cfg0, config.cfg1]);
    }
    assert_same_pages(&source, &final_read.image, &check_pages);
    println!(
        "restored configuration and locks: AUTH0={:02X} ACCESS={:02X}",
        Configuration::from_image(&final_read.image)
            .expect("configuration")
            .auth0,
        Configuration::from_image(&final_read.image)
            .expect("configuration")
            .access
    );
}

#[cfg(feature = "hardware-write")]
fn assert_same_pages(source: &Type2Image, live: &Type2Image, pages: &[u8]) {
    for page in pages {
        assert_eq!(
            live.page(*page),
            source.page(*page),
            "page {page} differs from the original image"
        );
    }
}

#[cfg(feature = "hardware-write")]
fn parse_hex<const N: usize>(name: &str, value: &str) -> [u8; N] {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    assert_eq!(
        compact.len(),
        N * 2,
        "{name} must contain {} hexadecimal digits",
        N * 2
    );
    let bytes = compact.as_bytes();
    let mut output = [0u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        let digit = |character: u8| match character {
            b'0'..=b'9' => character - b'0',
            b'a'..=b'f' => character - b'a' + 10,
            b'A'..=b'F' => character - b'A' + 10,
            _ => panic!("{name} contains a non-hexadecimal character"),
        };
        *byte = (digit(bytes[index * 2]) << 4) | digit(bytes[index * 2 + 1]);
    }
    output
}
