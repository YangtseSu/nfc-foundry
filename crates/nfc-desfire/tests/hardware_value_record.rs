// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire value, record, and transaction MAC round trip.
//!
//! Creates a reserved AES application with value, linear record, cyclic
//! record, and Transaction MAC files, runs committed mutations, recomputes
//! every Transaction MAC, and deletes the application. Only the reserved AID
//! is touched.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfc-desfire --features hardware --test hardware_value_record -- --ignored
//! ```
//!
//! Set `NFC_FOUNDRY_HARDWARE_FORMAT=yes` to erase the test card first; deleted
//! applications keep their EEPROM until FormatPICC. Set
//! `NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2` to run the same round trip under EV2
//! secure messaging.
#![cfg(feature = "hardware")]

use std::env;

use nfc_core::{NfcError, Result};
use nfc_desfire::{
    AuthKeyType, CommunicationMode, Desfire, RecordFileSpec, TransactionMac,
    TransactionMacFileSpec, ValueAdjustment, ValueFileSpec, transaction_mac,
};
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

const TEST_AID: [u8; 3] = [0xF0, 0x0D, 0x0A];
const VALUE_FILE: u8 = 0x01;
const LINEAR_FILE: u8 = 0x02;
const CYCLIC_FILE: u8 = 0x03;
const ENCIPHERED_CYCLIC_FILE: u8 = 0x04;
const ENCIPHERED_VALUE_FILE: u8 = 0x05;
const TMAC_FILE: u8 = 0x0F;
const PICC_KEY: [u8; 8] = [0x00; 8];
const APP_KEY: [u8; 16] = [0x00; 16];
const TMAC_KEY: [u8; 16] = [0x11; 16];

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

fn expect_tmac(desfire: &mut Desfire<'_>, uid: [u8; 7], tmi: &[u8]) -> Result<TransactionMac> {
    let tmc = desfire
        .commit_transaction_with_mac()?
        .ok_or_else(|| NfcError::VerificationFailed("no Transaction MAC returned".to_owned()))?;
    let computed = transaction_mac(&TMAC_KEY, tmc.counter, &uid, tmi)?;
    if computed != tmc.value {
        return Err(NfcError::VerificationFailed(format!(
            "TMV mismatch: card {tmc:02X?}, computed {computed:02X?}"
        )));
    }
    let (stored, _) = desfire.read_transaction_mac_file(TMAC_FILE, CommunicationMode::Plain)?;
    if stored != tmc {
        return Err(NfcError::VerificationFailed(format!(
            "stored TMAC {stored:02X?} differs from commit {tmc:02X?}"
        )));
    }
    Ok(tmc)
}

fn check<T>(label: &str, result: Result<T>) -> Result<T> {
    result.map_err(|error| NfcError::VerificationFailed(format!("{label}: {error}")))
}

fn session_key_type() -> AuthKeyType {
    match env::var("NFC_FOUNDRY_HARDWARE_KEY_TYPE").as_deref() {
        Ok("ev2") => AuthKeyType::AesEv2,
        _ => AuthKeyType::Aes,
    }
}

fn run(desfire: &mut Desfire<'_>, key_type: AuthKeyType) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    if env::var_os("NFC_FOUNDRY_HARDWARE_FORMAT").is_some() {
        desfire.format_picc()?;
        desfire.select_application([0x00; 3])?;
        desfire.authenticate_legacy(0, &PICC_KEY)?;
        eprintln!("formatted the PICC; free memory {}", desfire.free_memory()?);
    } else if desfire.free_memory()? < 1024 {
        return Err(NfcError::Unsupported(
            "the test card is out of EEPROM; set NFC_FOUNDRY_HARDWARE_FORMAT=yes to erase it",
        ));
    }

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
    // AES application with one key; key settings 0x0F leave the application
    // manageable by its master key.
    desfire.create_application(TEST_AID, 0x0F, 0x81, None, &[])?;

    desfire.select_application(TEST_AID)?;
    desfire.authenticate(0, &APP_KEY, key_type)?;

    desfire.create_value_file(ValueFileSpec {
        file_id: VALUE_FILE,
        iso_file_id: None,
        communication: 0,
        access_rights: 0x0000,
        lower_limit: -50,
        upper_limit: 50,
        value: 20,
        flags: 0x03,
    })?;
    desfire.create_record_file(RecordFileSpec {
        file_id: LINEAR_FILE,
        iso_file_id: None,
        communication: 0,
        access_rights: 0x0000,
        record_size: 8,
        max_records: 4,
        cyclic: false,
    })?;
    desfire.create_record_file(RecordFileSpec {
        file_id: CYCLIC_FILE,
        iso_file_id: None,
        communication: 0,
        access_rights: 0x0000,
        record_size: 4,
        max_records: 2,
        cyclic: true,
    })?;
    desfire.create_value_file(ValueFileSpec {
        file_id: ENCIPHERED_VALUE_FILE,
        iso_file_id: None,
        communication: 3,
        access_rights: 0x0000,
        lower_limit: -50,
        upper_limit: 50,
        value: 10,
        flags: 0x01,
    })?;
    desfire.create_record_file(RecordFileSpec {
        file_id: ENCIPHERED_CYCLIC_FILE,
        iso_file_id: None,
        communication: 3,
        access_rights: 0x0000,
        record_size: 4,
        max_records: 2,
        cyclic: true,
    })?;
    desfire.create_transaction_mac_file(&TransactionMacFileSpec {
        file_id: TMAC_FILE,
        iso_file_id: None,
        communication: 0,
        access_rights: 0xEFF0,
        key: TMAC_KEY,
        key_version: 1,
    })?;

    let uid = desfire.card_uid()?.uid;
    let (before, _) = desfire.read_transaction_mac_file(TMAC_FILE, CommunicationMode::Plain)?;
    if before.counter != 0 {
        return Err(NfcError::VerificationFailed(format!(
            "new Transaction MAC file starts at counter {}",
            before.counter
        )));
    }

    let old = desfire
        .get_value(VALUE_FILE, CommunicationMode::Plain)?
        .value;
    if old != 20 {
        return Err(NfcError::VerificationFailed(format!(
            "initial value {old}, expected 20"
        )));
    }
    desfire.adjust_value(
        VALUE_FILE,
        ValueAdjustment::Credit,
        5,
        CommunicationMode::Plain,
    )?;
    desfire.write_record(LINEAR_FILE, 0, &[0x11; 8], CommunicationMode::Plain)?;
    desfire.write_record(CYCLIC_FILE, 0, &[0x22; 4], CommunicationMode::Plain)?;
    let tmi = desfire.transaction_mac_input().to_vec();
    let tmc = expect_tmac(desfire, uid, &tmi)?;
    if tmc.counter != 1 {
        return Err(NfcError::VerificationFailed(format!(
            "first commit counter {}, expected 1",
            tmc.counter
        )));
    }

    let value = desfire
        .get_value(VALUE_FILE, CommunicationMode::Plain)?
        .value;
    if value != 25 {
        return Err(NfcError::VerificationFailed(format!(
            "committed value {value}, expected 25"
        )));
    }
    let records = desfire.read_records(LINEAR_FILE, 0, 1, 8, CommunicationMode::Plain)?;
    if records.data != [0x11; 8] {
        return Err(NfcError::VerificationFailed(format!(
            "linear record {:02X?}",
            records.data
        )));
    }
    let cyclic = desfire.read_records(CYCLIC_FILE, 0, 1, 4, CommunicationMode::Plain)?;
    if cyclic.data != [0x22; 4] {
        return Err(NfcError::VerificationFailed(format!(
            "cyclic record {:02X?}",
            cyclic.data
        )));
    }

    // A second transaction updates a record range.
    let _ = desfire.read_records(LINEAR_FILE, 0, 1, 8, CommunicationMode::Plain)?;
    desfire.update_record(LINEAR_FILE, 0, 2, &[0xAA, 0xBB], CommunicationMode::Plain)?;
    let tmi = desfire.transaction_mac_input().to_vec();
    let tmc = expect_tmac(desfire, uid, &tmi)?;
    if tmc.counter != 2 {
        return Err(NfcError::VerificationFailed(format!(
            "second commit counter {}, expected 2",
            tmc.counter
        )));
    }
    let updated = desfire.read_records(LINEAR_FILE, 0, 1, 8, CommunicationMode::Plain)?;
    let mut expected = [0x11u8; 8];
    expected[2..4].copy_from_slice(&[0xAA, 0xBB]);
    if updated.data != expected {
        return Err(NfcError::VerificationFailed(format!(
            "updated record {:02X?}, expected {expected:02X?}",
            updated.data
        )));
    }

    // A third transaction clears the cyclic file.
    let _ = desfire.read_records(CYCLIC_FILE, 0, 1, 4, CommunicationMode::Plain)?;
    desfire.clear_record_file(CYCLIC_FILE)?;
    let tmi = desfire.transaction_mac_input().to_vec();
    let tmc = expect_tmac(desfire, uid, &tmi)?;
    if tmc.counter != 3 {
        return Err(NfcError::VerificationFailed(format!(
            "third commit counter {}, expected 3",
            tmc.counter
        )));
    }
    let cleared = desfire
        .file_settings(CYCLIC_FILE)
        .and_then(|raw| nfc_desfire::FileSettings::parse(&raw))?;
    if !matches!(
        cleared.contents,
        nfc_desfire::FileContents::Record {
            current_records: 0,
            ..
        }
    ) {
        return Err(NfcError::VerificationFailed(format!(
            "cyclic file settings after clear: {}",
            cleared.summary()
        )));
    }

    // Enciphered value and cyclic record writes use the file mode; the clear
    // uses secure-messaging MAC even though the file is enciphered.
    let value = check(
        "read enciphered value",
        desfire.get_value(ENCIPHERED_VALUE_FILE, CommunicationMode::Enciphered),
    )?
    .value;
    if value != 10 {
        return Err(NfcError::VerificationFailed(format!(
            "enciphered initial value {value}, expected 10"
        )));
    }
    check(
        "credit enciphered value",
        desfire.adjust_value(
            ENCIPHERED_VALUE_FILE,
            ValueAdjustment::Credit,
            2,
            CommunicationMode::Enciphered,
        ),
    )?;
    let tmi = desfire.transaction_mac_input().to_vec();
    let tmc = check("commit enciphered value", expect_tmac(desfire, uid, &tmi))?;
    if tmc.counter != 4 {
        return Err(NfcError::VerificationFailed(format!(
            "enciphered value commit counter {}, expected 4",
            tmc.counter
        )));
    }
    let value = check(
        "read enciphered value",
        desfire.get_value(ENCIPHERED_VALUE_FILE, CommunicationMode::Enciphered),
    )?
    .value;
    if value != 12 {
        return Err(NfcError::VerificationFailed(format!(
            "enciphered committed value {value}, expected 12"
        )));
    }

    check(
        "append enciphered record",
        desfire.write_record(
            ENCIPHERED_CYCLIC_FILE,
            0,
            &[0x44; 4],
            CommunicationMode::Enciphered,
        ),
    )?;
    let tmi = desfire.transaction_mac_input().to_vec();
    let tmc = check("commit enciphered record", expect_tmac(desfire, uid, &tmi))?;
    if tmc.counter != 5 {
        return Err(NfcError::VerificationFailed(format!(
            "enciphered record commit counter {}, expected 5",
            tmc.counter
        )));
    }
    let record = check(
        "read enciphered record",
        desfire.read_records(
            ENCIPHERED_CYCLIC_FILE,
            0,
            1,
            4,
            CommunicationMode::Enciphered,
        ),
    )?;
    if record.data != [0x44; 4] {
        return Err(NfcError::VerificationFailed(format!(
            "enciphered record {:02X?}",
            record.data
        )));
    }
    let _ = check(
        "read before enciphered clear",
        desfire.read_records(
            ENCIPHERED_CYCLIC_FILE,
            0,
            1,
            4,
            CommunicationMode::Enciphered,
        ),
    )?;
    check(
        "clear enciphered record file",
        desfire.clear_record_file(ENCIPHERED_CYCLIC_FILE),
    )?;
    let tmi = desfire.transaction_mac_input().to_vec();
    let tmc = check("commit enciphered clear", expect_tmac(desfire, uid, &tmi))?;
    if tmc.counter != 6 {
        return Err(NfcError::VerificationFailed(format!(
            "enciphered clear commit counter {}, expected 6",
            tmc.counter
        )));
    }
    let cleared = desfire
        .file_settings(ENCIPHERED_CYCLIC_FILE)
        .and_then(|raw| nfc_desfire::FileSettings::parse(&raw))?;
    if !matches!(
        cleared.contents,
        nfc_desfire::FileContents::Record {
            current_records: 0,
            ..
        }
    ) {
        return Err(NfcError::VerificationFailed(format!(
            "enciphered cyclic file settings after clear: {}",
            cleared.summary()
        )));
    }

    Ok(())
}

#[test]
#[ignore = "requires a DESFire card and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn value_record_and_transaction_mac_round_trip() -> Result<()> {
    let conn = gate();
    let mut reader = open(&conn)?;
    let mut desfire = Desfire::new(reader.as_mut());

    let result = run(&mut desfire, session_key_type());
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
