// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire key management round trip.
//!
//! Creates reserved AES and DES applications, changes a different key and the
//! authenticated key, re-authenticates with every new key, reads the key
//! version back, changes KeySettings, and deletes both applications. The PICC
//! master key is never changed.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfc-desfire --features hardware --test hardware_keys -- --ignored
//! ```
//!
//! Set `NFC_FOUNDRY_HARDWARE_FORMAT=yes` to erase the test card first. Set
//! `NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2` to use EV2 secure messaging.
#![cfg(feature = "hardware")]

use std::env;

use nfc_core::{NfcError, Result};
use nfc_desfire::{AuthKeyType, Desfire, KeyChange};
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

const AES_AID: [u8; 3] = [0xF0, 0x0D, 0x0B];
const DES_AID: [u8; 3] = [0xF0, 0x0D, 0x0C];
const PICC_KEY: [u8; 8] = [0x00; 8];
const APP_KEY: [u8; 16] = [0x00; 16];

fn gate() -> String {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE").as_deref(),
        Ok("1"),
        "set NFC_FOUNDRY_HARDWARE=1 to run hardware tests"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow the temporary key test"
    );
    env::var("NFC_FOUNDRY_DEVICE")
        .expect("set NFC_FOUNDRY_DEVICE to the exact pn532_uart:<port> connstring")
}

fn open(conn: &str) -> Result<Box<dyn Reader>> {
    let mut reader = DriverRegistry.open(conn)?;
    reader.open(conn)?;
    reader.card_info()?;
    Ok(reader)
}

fn session_key_type() -> AuthKeyType {
    match env::var("NFC_FOUNDRY_HARDWARE_KEY_TYPE").as_deref() {
        Ok("ev2") => AuthKeyType::AesEv2,
        _ => AuthKeyType::Aes,
    }
}

fn picc_authenticate(desfire: &mut Desfire<'_>) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)
}

fn remove_application(desfire: &mut Desfire<'_>, aid: [u8; 3]) -> Result<()> {
    picc_authenticate(desfire)?;
    if desfire.application_ids()?.contains(&aid) {
        desfire.delete_application(aid)?;
    }
    Ok(())
}

fn check<T>(label: &str, result: Result<T>) -> Result<T> {
    result.map_err(|error| NfcError::VerificationFailed(format!("{label}: {error}")))
}

fn expect_version(desfire: &mut Desfire<'_>, key_no: u8, expected: u8) -> Result<()> {
    let version = check(
        &format!("GetKeyVersion for key {key_no}"),
        desfire.key_version(key_no),
    )?;
    if version != Some(expected) {
        return Err(NfcError::VerificationFailed(format!(
            "key {key_no} version {version:?}, expected {expected}"
        )));
    }
    Ok(())
}

fn run_aes(desfire: &mut Desfire<'_>, key_type: AuthKeyType) -> Result<()> {
    picc_authenticate(desfire)?;
    desfire.create_application(AES_AID, 0x0F, 0x82, None, &[])?;

    desfire.select_application(AES_AID)?;
    desfire.authenticate(0, &APP_KEY, key_type)?;
    let settings = check("GetKeySettings", desfire.key_settings())?;
    if settings.max_keys != Some(2)
        || settings.key_type != Some(nfc_desfire::ApplicationKeyType::Aes)
    {
        return Err(NfcError::VerificationFailed(format!(
            "AES application settings: {settings:?}"
        )));
    }

    // Case 1: a different key, with its stored value. The factory key
    // settings allow only the application master key to change keys, so the
    // session stays on key 0.
    let first = [0x22u8; 16];
    check(
        "change key 1",
        desfire.change_key(KeyChange {
            key_number: 1,
            new_key: first.to_vec(),
            new_key_version: 4,
            current_key: Some(APP_KEY.to_vec()),
        }),
    )?;
    check(
        "re-authenticate key 1",
        desfire.authenticate(1, &first, key_type),
    )?;
    expect_version(desfire, 1, 4)?;

    // Case 2: the authenticated key itself.
    check(
        "re-authenticate key 0",
        desfire.authenticate(0, &APP_KEY, key_type),
    )?;
    let second = [0x33u8; 16];
    check(
        "change the authenticated key",
        desfire.change_key(KeyChange {
            key_number: 0,
            new_key: second.to_vec(),
            new_key_version: 2,
            current_key: None,
        }),
    )?;
    if desfire.authenticated_key_number().is_some() {
        return Err(NfcError::VerificationFailed(
            "changing the authenticated key must end the session".to_owned(),
        ));
    }
    check(
        "re-authenticate the changed key",
        desfire.authenticate(0, &second, key_type),
    )?;
    expect_version(desfire, 0, 2)?;

    let third = [0x55u8; 16];
    check(
        "change the authenticated key again",
        desfire.change_key(KeyChange {
            key_number: 0,
            new_key: third.to_vec(),
            new_key_version: 5,
            current_key: None,
        }),
    )?;
    check(
        "re-authenticate the twice-changed key",
        desfire.authenticate(0, &third, key_type),
    )?;
    expect_version(desfire, 0, 5)?;

    let before = check("read KeySettings", desfire.key_settings())?;
    if !before.configuration_changeable() {
        return Err(NfcError::VerificationFailed(
            "the application configuration is frozen".to_owned(),
        ));
    }
    // Toggle the free file-list bit; the configuration-changeable bit stays
    // set so the change can be restored.
    let toggled = before.key_settings ^ 0x02;
    check("change KeySettings", desfire.change_key_settings(toggled))?;
    let changed = check("read changed KeySettings", desfire.key_settings())?;
    if changed.key_settings != toggled {
        return Err(NfcError::VerificationFailed(format!(
            "KeySettings {:02X}, expected {toggled:02X}",
            changed.key_settings
        )));
    }
    check(
        "restore KeySettings",
        desfire.change_key_settings(before.key_settings),
    )?;
    Ok(())
}

fn run_des(desfire: &mut Desfire<'_>) -> Result<()> {
    picc_authenticate(desfire)?;
    // Two DES keys: KeySettings2 = key family 00, count 2.
    desfire.create_application(DES_AID, 0x0F, 0x02, None, &[])?;

    desfire.select_application(DES_AID)?;
    desfire.authenticate_legacy(0, &[0x00; 8])?;

    let first = [0x44u8; 8];
    check(
        "change DES key 1",
        desfire.change_key(KeyChange {
            key_number: 1,
            new_key: first.to_vec(),
            new_key_version: 3,
            current_key: Some(vec![0x00; 8]),
        }),
    )?;
    check(
        "re-authenticate DES key 1",
        desfire.authenticate_legacy(1, &first),
    )?;
    expect_version(desfire, 1, 3)?;

    check(
        "re-authenticate DES key 0",
        desfire.authenticate_legacy(0, &[0x00; 8]),
    )?;
    let second = [0x66u8; 8];
    check(
        "change the authenticated DES key",
        desfire.change_key(KeyChange {
            key_number: 0,
            new_key: second.to_vec(),
            new_key_version: 1,
            current_key: None,
        }),
    )?;
    check(
        "re-authenticate the changed DES key",
        desfire.authenticate_legacy(0, &second),
    )?;
    expect_version(desfire, 0, 1)?;

    let third = [0x77u8; 8];
    check(
        "change the authenticated DES key again",
        desfire.change_key(KeyChange {
            key_number: 0,
            new_key: third.to_vec(),
            new_key_version: 2,
            current_key: None,
        }),
    )?;
    check(
        "re-authenticate the twice-changed DES key",
        desfire.authenticate_legacy(0, &third),
    )?;
    expect_version(desfire, 0, 2)?;
    Ok(())
}

#[test]
#[ignore = "requires a DESFire card and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn key_management_round_trip() -> Result<()> {
    let conn = gate();
    let mut reader = open(&conn)?;
    let mut desfire = Desfire::new(reader.as_mut());

    picc_authenticate(&mut desfire)?;
    if env::var_os("NFC_FOUNDRY_HARDWARE_FORMAT").is_some() {
        desfire.format_picc()?;
        eprintln!("formatted the PICC");
    }
    remove_application(&mut desfire, AES_AID)?;
    remove_application(&mut desfire, DES_AID)?;

    let aes = run_aes(&mut desfire, session_key_type());
    let aes_cleanup = remove_application(&mut desfire, AES_AID);
    aes?;
    aes_cleanup?;

    let des = run_des(&mut desfire);
    let des_cleanup = remove_application(&mut desfire, DES_AID);
    des?;
    des_cleanup?;

    let applications = desfire.application_ids()?;
    if applications.contains(&AES_AID) || applications.contains(&DES_AID) {
        return Err(NfcError::VerificationFailed(
            "a temporary key-management application was not removed".to_owned(),
        ));
    }
    Ok(())
}
