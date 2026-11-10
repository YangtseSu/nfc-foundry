//! Environment-gated Type 4 NDEF workflow checks.
//!
//! Read-only:
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware --test hardware_type4 -- --ignored
//! ```
//!
//! The write round trip additionally requires the `hardware-write` feature and
//! `NFC_FOUNDRY_HARDWARE_WRITE=yes`. The original NDEF message is restored
//! at the end of the test.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_app::Service;
#[cfg(feature = "hardware-write")]
use nfc_app::classic::BackupStore;
#[cfg(feature = "hardware-write")]
use nfc_app::type4::{Type4Clear, Type4NdefWrite};
use nfc_core::{CancelToken, CardType};
use nfc_drivers::DriverRegistry;
#[cfg(feature = "hardware-write")]
use nfc_ndef::{NdefMessage, TextRecord};
#[cfg(feature = "hardware-write")]
use nfc_type4::NdefState;

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
    let mut service = Service::with_transport(Arc::new(DriverRegistry));
    service.refresh_devices().expect("enumerate readers");
    service
        .select_device(conn_string)
        .expect("select the reader");
    service
}

#[test]
#[ignore = "requires a Type 4 / ISO-DEP tag"]
fn reads_and_reports_cc_and_ndef_state() {
    let _guard = lock_hardware();
    let conn_string = conn_string();
    let mut service = service(&conn_string);
    let card = service.card_info().expect("select the tag");
    assert_eq!(
        card.card_type,
        CardType::Iso14443,
        "place an NFC Forum Type 4 tag on the reader; selected {:?}",
        card.card_type
    );
    let read = service
        .type4_read(CancelToken::new())
        .expect("read the Type 4 tag");
    println!(
        "uid={} cc={} bytes version={}.{:X} MLe={} MLc={} file={:04X} max={} access={:02X}/{:02X} state={:?}",
        card.uid_hex(),
        read.cc.cclen,
        read.cc.major_version,
        read.cc.minor_version,
        read.cc.max_le,
        read.cc.max_lc,
        read.ndef.file_id,
        read.ndef.max_size,
        read.ndef.read_access,
        read.ndef.write_access,
        read.ndef.state,
    );
    println!(
        "writable={} reason={:?}",
        read.writable(),
        read.read_only_reason()
    );
    for tlv in read.cc.tlvs() {
        println!("tlv: {tlv:?}");
    }
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires a Type 4 tag and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
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
    assert_eq!(card.card_type, CardType::Iso14443);
    let original = service
        .type4_read(CancelToken::new())
        .expect("read the original NDEF state");
    assert!(
        original.writable(),
        "the Type 4 NDEF file is not writable: {:?}",
        original.read_only_reason()
    );

    let backup_dir = env::var_os("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("nfc-foundry-hardware"));
    let backup = BackupStore::new(&backup_dir);
    let message = NdefMessage::single(
        TextRecord::new("NFC Foundry Type 4 hardware check", "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message");

    service
        .type4_ndef_write(Type4NdefWrite::new(message.clone(), &backup))
        .expect("write NDEF");
    let read = service
        .type4_read(CancelToken::new())
        .expect("read the written NDEF");
    assert_eq!(read.ndef.state, NdefState::Message(message.clone()));
    println!("wrote and verified {} record(s)", message.records().len());

    match &original.ndef.state {
        NdefState::Message(message) => {
            service
                .type4_ndef_write(Type4NdefWrite::new(message.clone(), &backup))
                .expect("restore the original NDEF message");
        }
        NdefState::Empty => {
            service
                .type4_clear(Type4Clear::new(&backup).confirm(true))
                .expect("restore the empty NDEF state");
        }
    }
    let restored = service
        .type4_read(CancelToken::new())
        .expect("read the restored NDEF state");
    assert_eq!(restored.ndef.state, original.ndef.state);
    println!(
        "restored original state; backups in {}",
        backup_dir.display()
    );
}
