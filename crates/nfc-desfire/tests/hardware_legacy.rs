// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire legacy-authentication and file round trip.
//!
//! Creates a reserved temporary application and standard file, writes and
//! reads 16 bytes, then deletes the application. An interrupted earlier run is
//! cleaned up first, and only the reserved AID is ever touched.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfc-desfire --features hardware --test hardware_legacy -- --ignored
//! ```
#![cfg(feature = "hardware")]

use std::env;

use nfc_core::{NfcError, Result};
use nfc_desfire::Desfire;
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

const TEST_AID: [u8; 3] = [0xF0, 0x0D, 0x01];
const TEST_FILE: u8 = 0x01;
const PICC_KEY: [u8; 8] = [0x00; 8];
const APP_KEY: [u8; 16] = [0x00; 16];
const PAYLOAD: &[u8] = b"NFC Foundry DESFire";

fn gate() -> String {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE").as_deref(),
        Ok("1"),
        "set NFC_FOUNDRY_HARDWARE=1 to run hardware tests"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow the temporary write test"
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

fn remove_test_application(desfire: &mut Desfire<'_>) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    desfire.delete_application(TEST_AID)
}

fn run(desfire: &mut Desfire<'_>) -> Result<()> {
    let inventory = desfire.inventory()?;
    if inventory
        .applications
        .iter()
        .any(|application| application.aid == TEST_AID)
    {
        remove_test_application(desfire)?;
    }

    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    assert_eq!(
        desfire.session_key_type(),
        Some(nfc_desfire::AuthKeyType::Des)
    );
    assert_eq!(desfire.session_key().map(<[u8]>::len), Some(8));

    desfire.create_application(TEST_AID, 0x0F, 0x01, None, &[])?;

    desfire.select_application(TEST_AID)?;
    desfire.authenticate_legacy(0, &APP_KEY)?;
    assert_eq!(
        desfire.session_key_type(),
        Some(nfc_desfire::AuthKeyType::Tdes2)
    );
    assert_eq!(desfire.session_key().map(<[u8]>::len), Some(16));

    desfire.create_standard_file(TEST_FILE, None, 0, 0x0000, 32)?;
    desfire.write_data(TEST_FILE, 0, PAYLOAD)?;
    desfire.commit_transaction()?;
    let read = desfire.read_data(
        TEST_FILE,
        0,
        PAYLOAD.len() as u32,
        nfc_desfire::CommunicationMode::Plain,
    )?;
    if read != PAYLOAD {
        return Err(NfcError::VerificationFailed(format!(
            "read {read:02X?}, expected {PAYLOAD:02X?}"
        )));
    }
    Ok(())
}

#[test]
#[ignore = "requires a DESFire card and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn temporary_application_file_round_trip() -> Result<()> {
    let conn = gate();
    let mut reader = open(&conn)?;
    let mut desfire = Desfire::new(reader.as_mut());

    let result = run(&mut desfire);
    let cleanup = remove_test_application(&mut desfire);
    result?;
    cleanup?;

    let applications = desfire.application_ids()?;
    if applications.contains(&TEST_AID) {
        return Err(NfcError::VerificationFailed(format!(
            "temporary application {TEST_AID:02X?} was not removed"
        )));
    }
    Ok(())
}
