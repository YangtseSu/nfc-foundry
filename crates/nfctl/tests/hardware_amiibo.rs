//! Environment-gated Amiibo read and write checks.
//!
//! Read/decrypt:
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! NFC_FOUNDRY_AMIIBO_KEYS=/path/to/key_retail.bin \
//! cargo test -p nfctl --features hardware --test hardware_amiibo \
//!   -- --ignored reads_and_decrypts
//! ```
//!
//! Write (owned blank or magic NTAG215 only):
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_HARDWARE_AMIIBO_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! NFC_FOUNDRY_AMIIBO_KEYS=/path/to/key_retail.bin \
//! NFC_FOUNDRY_AMIIBO_SOURCE=/path/to/amiibo.bin \
//! NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-amiibo-backup \
//! cargo test -p nfctl --features hardware-write --test hardware_amiibo \
//!   -- --ignored writes_a_resigned_image
//! ```

#![cfg(feature = "hardware")]

use std::env;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use nfc_app::Service;
use nfc_app::amiibo::{AmiiboReadRequest, load_keys};
#[cfg(feature = "hardware-write")]
use nfc_app::amiibo::{AmiiboSourceFormat, AmiiboWriteExecution, AmiiboWriteRequest};
#[cfg(feature = "hardware-write")]
use nfc_app::classic::BackupStore;
use nfc_core::{CancelToken, CardType};
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
    env::var("NFC_FOUNDRY_DEVICE").expect("set NFC_FOUNDRY_DEVICE to the exact reader connstring")
}

fn key_path() -> PathBuf {
    env::var_os("NFC_FOUNDRY_AMIIBO_KEYS")
        .map(PathBuf::from)
        .expect("set NFC_FOUNDRY_AMIIBO_KEYS to the combined 160-byte key file")
}

fn service(conn_string: &str) -> Service {
    let mut service = Service::with_transport(std::sync::Arc::new(DriverRegistry));
    service.refresh_devices().expect("enumerate readers");
    service
        .select_device(conn_string)
        .expect("select the reader");
    service
}

#[cfg(feature = "hardware-write")]
fn backup_dir() -> BackupStore {
    match env::var_os("NFC_FOUNDRY_HARDWARE_BACKUP_DIR") {
        Some(path) => BackupStore::new(path),
        None => BackupStore::default(),
    }
}

#[test]
#[ignore = "requires an Amiibo tag, NFC_FOUNDRY_AMIIBO_KEYS, and NFC_FOUNDRY_HARDWARE=1"]
fn reads_and_decrypts() {
    let _guard = lock_hardware();
    let mut service = service(&conn_string());
    let card = service.card_info().expect("select the tag");
    assert_eq!(
        card.card_type,
        CardType::Type2,
        "place an Amiibo NTAG215 on the reader; selected {:?}",
        card.card_type
    );
    let keys = load_keys(&key_path()).expect("load the user's Amiibo keys");
    let read = service
        .amiibo_read(AmiiboReadRequest::new(CancelToken::new()).keys(Some(keys)))
        .expect("read and decrypt the Amiibo");
    let identity = read.identity.expect("verified Amiibo identity");
    println!(
        "{} uid={} signature=valid game=0x{:04X} character=0x{:02X} model=0x{:04X} series=0x{:02X} format={} pages={}/{}",
        read.image.layout.product.name(),
        read.image.uid_hex(),
        identity.game_character_id,
        identity.character_id,
        identity.model_number,
        identity.series,
        identity.format_version,
        read.image.known_pages(),
        read.image.layout.page_count()
    );
}

#[test]
#[ignore = "requires an owned blank/magic NTAG215, NFC_FOUNDRY_AMIIBO_WRITE=yes, and the write feature"]
#[cfg(feature = "hardware-write")]
fn writes_a_resigned_image() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_AMIIBO_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_AMIIBO_WRITE=yes to allow an Amiibo write"
    );
    let _guard = lock_hardware();
    let source = env::var_os("NFC_FOUNDRY_AMIIBO_SOURCE")
        .map(PathBuf::from)
        .expect("set NFC_FOUNDRY_AMIIBO_SOURCE to a raw or plaintext dump");
    let plain = env::var("NFC_FOUNDRY_AMIIBO_SOURCE_PLAIN").as_deref() == Ok("yes");
    let keys = load_keys(&key_path()).expect("load the user's Amiibo keys");
    let mut service = service(&conn_string());
    let plan = service
        .amiibo_write_preflight(AmiiboWriteRequest {
            keys,
            source,
            format: if plain {
                AmiiboSourceFormat::Plain
            } else {
                AmiiboSourceFormat::Raw
            },
            auth: None,
            advanced: true,
            backup: backup_dir(),
            cancel: CancelToken::new(),
        })
        .expect("build the Amiibo write plan");
    println!(
        "target uid={} source uid={} pages={} backup={}",
        plan.target_uid()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>(),
        plan.source_uid()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>(),
        plan.pages().len(),
        plan.backup_path().display()
    );
    let result = service
        .amiibo_write_execute(AmiiboWriteExecution::new(plan.token()).confirm(true))
        .expect("write and verify the Amiibo");
    println!(
        "written={} uid={} backup={}",
        result.written_pages,
        result
            .target_uid
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>(),
        result.backup.path.display()
    );
}
