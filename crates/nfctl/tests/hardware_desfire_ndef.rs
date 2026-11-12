// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire EV3 NDEF provisioning round trip.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware-write \
//!   --test hardware_desfire_ndef -- --ignored
//! ```
//!
//! The test refuses to start when an NDEF application already exists, then
//! formats the owned factory-blank card, writes and restores one Text record,
//! deletes the NDEF application, and leaves the card without that application.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_app::Service;
use nfc_app::classic::BackupStore;
use nfc_app::desfire::{
    DesfireNdefDeleteRequest, DesfireNdefFormatRequest, NDEF_APPLICATION_AID,
    NDEF_CAPABILITY_CONTAINER,
};
use nfc_app::type4::{Type4Clear, Type4NdefWrite};
use nfc_core::{CancelToken, CardType, NfcError, Result};
use nfc_drivers::DriverRegistry;
use nfc_ndef::{NdefMessage, TextRecord};
use nfc_type4::NdefState;

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

const PICC_KEY: [u8; 8] = [0x00; 8];
const WRONG_KEY: [u8; 8] = [0x11; 8];

fn lock_hardware() -> MutexGuard<'static, ()> {
    HARDWARE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn gate() -> String {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE").as_deref(),
        Ok("1"),
        "set NFC_FOUNDRY_HARDWARE=1 to run hardware tests"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow NDEF provisioning"
    );
    env::var("NFC_FOUNDRY_DEVICE")
        .expect("set NFC_FOUNDRY_DEVICE to the exact pn532_uart:<port> connstring")
}

fn service(conn_string: &str) -> Service {
    let mut service = Service::with_transport(std::sync::Arc::new(DriverRegistry));
    service.refresh_devices().expect("enumerate readers");
    service
        .select_device(conn_string)
        .expect("select the reader");
    service
}

fn verify(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(NfcError::VerificationFailed(message.into()))
    }
}

fn format_request(key: [u8; 8], advanced: bool, confirm: bool) -> DesfireNdefFormatRequest {
    DesfireNdefFormatRequest::new(key.to_vec())
        .key_number(0)
        .advanced(advanced)
        .confirm(confirm)
}

fn delete_request(key: [u8; 8]) -> DesfireNdefDeleteRequest {
    DesfireNdefDeleteRequest::new(key.to_vec())
        .key_number(0)
        .advanced(true)
        .confirm(true)
}

fn run(service: &mut Service, backup: &BackupStore) -> Result<()> {
    let card = service.card_info()?;
    verify(
        card.card_type == CardType::Iso14443,
        format!(
            "place the DESFire EV3 on the reader; selected {:?}",
            card.card_type
        ),
    )?;
    let before = service.desfire_inventory(CancelToken::new())?;
    verify(
        !before
            .applications
            .iter()
            .any(|application| application.aid == NDEF_APPLICATION_AID),
        "an NDEF application already exists; delete it before this test",
    )?;

    let gate_error = service
        .desfire_ndef_format(format_request(PICC_KEY, false, true))
        .err()
        .ok_or_else(|| {
            NfcError::VerificationFailed("formatting without Advanced mode succeeded".to_owned())
        })?;
    verify(
        gate_error.to_string().contains("Advanced"),
        format!("unexpected Advanced-mode error: {gate_error}"),
    )?;
    let gate_error = service
        .desfire_ndef_format(format_request(PICC_KEY, true, false))
        .err()
        .ok_or_else(|| {
            NfcError::VerificationFailed("formatting without confirmation succeeded".to_owned())
        })?;
    verify(
        gate_error.to_string().contains("confirmation"),
        format!("unexpected confirmation error: {gate_error}"),
    )?;

    let wrong = service.desfire_ndef_format(format_request(WRONG_KEY, true, true));
    verify(wrong.is_err(), "a wrong PICC key must not format the card")?;
    let after_wrong = service.desfire_inventory(CancelToken::new())?;
    verify(
        !after_wrong
            .applications
            .iter()
            .any(|application| application.aid == NDEF_APPLICATION_AID),
        "a wrong PICC key created the NDEF application",
    )?;

    let format = service.desfire_ndef_format(format_request(PICC_KEY, true, true))?;
    verify(
        format.cc.raw() == NDEF_CAPABILITY_CONTAINER.as_slice(),
        format!("CC read back as {:02X?}", format.cc.raw()),
    )?;
    verify(
        format.ndef.state == NdefState::Empty,
        "the NDEF file is not empty after formatting",
    )?;
    println!(
        "formatted aid={:02X?} cc={} state={:?}",
        NDEF_APPLICATION_AID,
        format.cc.raw().len(),
        format.ndef.state
    );

    let read = service.type4_read(CancelToken::new())?;
    verify(read.writable(), "the formatted NDEF file is not writable")?;
    verify(
        read.cc.raw() == NDEF_CAPABILITY_CONTAINER.as_slice(),
        "the Type 4 read did not return the written CC",
    )?;
    verify(
        read.ndef.state == NdefState::Empty,
        "the Type 4 read did not return an empty NDEF file",
    )?;

    let message =
        NdefMessage::single(TextRecord::new("NFC Foundry DESFire NDEF check", "en")?.to_record()?)?;
    let written = service.type4_ndef_write(Type4NdefWrite::new(message.clone(), backup))?;
    verify(
        written.ndef.state == NdefState::Message(message.clone()),
        "the NDEF write did not read back the message",
    )?;
    let after_write = service.type4_read(CancelToken::new())?;
    verify(
        after_write.ndef.state == NdefState::Message(message),
        "the Type 4 read did not return the written message",
    )?;

    service.type4_clear(Type4Clear::new(backup).confirm(true))?;
    let cleared = service.type4_read(CancelToken::new())?;
    verify(
        cleared.ndef.state == NdefState::Empty,
        "the NDEF file was not restored to empty",
    )?;

    let deleted = service.desfire_ndef_delete(delete_request(PICC_KEY))?;
    verify(
        !deleted.applications.contains(&NDEF_APPLICATION_AID),
        "the NDEF application is still listed after deletion",
    )?;
    let type4 = service.type4_read(CancelToken::new());
    verify(
        type4.is_err(),
        "the Type 4 NDEF application still selects after deletion",
    )?;
    let after = service.desfire_inventory(CancelToken::new())?;
    verify(
        after.applications == before.applications,
        format!(
            "application inventory changed: before {:?}, after {:?}",
            before.applications, after.applications
        ),
    )?;
    Ok(())
}

#[test]
#[ignore = "requires a factory-blank DESFire EV3 and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn formats_writes_restores_and_deletes_the_ndef_application() -> Result<()> {
    let _guard = lock_hardware();
    let conn_string = gate();
    let mut service = service(&conn_string);
    let backup_dir = env::var_os("NFC_FOUNDRY_HARDWARE_BACKUP_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("nfc-foundry-hardware"));
    let backup = BackupStore::new(&backup_dir);

    let result = run(&mut service, &backup);
    if result.is_err() {
        let inventory = service.desfire_inventory(CancelToken::new());
        if inventory.is_ok_and(|inventory| {
            inventory
                .applications
                .iter()
                .any(|application| application.aid == NDEF_APPLICATION_AID)
        }) {
            eprintln!("cleaning up the NDEF application after a failed run");
            let _ = service.desfire_ndef_delete(delete_request(PICC_KEY));
        }
    }
    result
}
