// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire value and record read checks.
//!
//! Creates a reserved application with one value file and one linear record
//! file, writes two records, reads both through `nfc-app`, then removes the
//! application. The test is idempotent and only touches the reserved AID.
//! Set `NFC_FOUNDRY_HARDWARE_KEEP_APP=1` to keep the fixture for manual
//! `nfctl` or GUI checks; the next run removes it before setup.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware-write \
//!   --test hardware_desfire_files -- --ignored --nocapture
//! ```

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_app::Service;
use nfc_app::desfire::{DesfireReadRequest, DesfireReadResult, FileContents};
use nfc_core::{CancelToken, NfcError, Result};
use nfc_desfire::Desfire;
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

const TEST_AID: [u8; 3] = [0xF0, 0x0D, 0x03];
const PICC_KEY: [u8; 8] = [0x00; 8];
const APP_KEY: [u8; 8] = [0x00; 8];
const VALUE_FILE: u8 = 0x01;
const RECORD_FILE: u8 = 0x02;
const RECORDS: [[u8; 8]; 2] = [
    [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
    [0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18],
];
const CYCLIC_FILE: u8 = 0x03;
const CYCLIC_RECORDS: [[u8; 4]; 2] = [[0xA1, 0xA2, 0xA3, 0xA4], [0xB1, 0xB2, 0xB3, 0xB4]];

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
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow the temporary setup"
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

fn verify(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(NfcError::VerificationFailed(message.into()))
    }
}

fn remove_test_application(desfire: &mut Desfire<'_>) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    desfire.delete_application(TEST_AID)?;
    Ok(())
}

fn setup(desfire: &mut Desfire<'_>) -> Result<()> {
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
    desfire.create_application(TEST_AID, 0x0F, 0x01, None, &[])?;

    desfire.select_application(TEST_AID)?;
    desfire.authenticate_legacy(0, &APP_KEY)?;
    desfire.create_value_file(nfc_desfire::ValueFileSpec {
        file_id: VALUE_FILE,
        iso_file_id: None,
        communication: 0,
        access_rights: 0x0000,
        lower_limit: -50,
        upper_limit: 50,
        value: 7,
        flags: 0x03,
    })?;
    desfire.create_record_file(nfc_desfire::RecordFileSpec {
        file_id: RECORD_FILE,
        iso_file_id: None,
        communication: 0,
        access_rights: 0x0000,
        record_size: 8,
        max_records: 4,
        cyclic: false,
    })?;
    desfire.create_record_file(nfc_desfire::RecordFileSpec {
        file_id: CYCLIC_FILE,
        iso_file_id: None,
        communication: 0,
        access_rights: 0x0000,
        record_size: 4,
        max_records: 2,
        cyclic: true,
    })?;
    desfire.write_record(RECORD_FILE, 0, &RECORDS[0])?;
    desfire.commit_transaction()?;
    desfire.write_record(RECORD_FILE, 0, &RECORDS[1])?;
    desfire.commit_transaction()?;
    desfire.write_record(CYCLIC_FILE, 0, &CYCLIC_RECORDS[0])?;
    desfire.commit_transaction()?;

    let settings = desfire.file_settings(RECORD_FILE)?;
    let current = match nfc_app::desfire::FileSettings::parse(&settings)?.contents {
        FileContents::Record {
            current_records, ..
        } => current_records,
        _ => {
            return Err(NfcError::VerificationFailed(
                "the reserved record file is not a record file".to_owned(),
            ));
        }
    };
    verify(
        current == RECORDS.len() as u32,
        format!("the record file has {current} current records, expected 2"),
    )?;

    let settings = desfire.file_settings(CYCLIC_FILE)?;
    let current = match nfc_app::desfire::FileSettings::parse(&settings)?.contents {
        FileContents::Record {
            current_records, ..
        } => current_records,
        _ => {
            return Err(NfcError::VerificationFailed(
                "the reserved cyclic file is not a record file".to_owned(),
            ));
        }
    };
    verify(
        current == 1,
        format!("the cyclic file has {current} current records, expected 1"),
    )
}

fn reads(service: &mut Service) -> Result<()> {
    let value = service.desfire_read(
        CancelToken::new(),
        DesfireReadRequest::new(TEST_AID, VALUE_FILE).key(APP_KEY.to_vec()),
    )?;
    match value {
        DesfireReadResult::Value(read) => {
            println!("value file: {}", read.settings.summary());
            verify(read.value == 7, format!("value {} != 7", read.value))?;
            match read.settings.contents {
                FileContents::Value {
                    lower_limit,
                    upper_limit,
                    limited_credit,
                    free_get_value,
                    ..
                } => {
                    verify(lower_limit == -50, "lower limit")?;
                    verify(upper_limit == 50, "upper limit")?;
                    verify(limited_credit, "limited credit flag")?;
                    verify(free_get_value, "free GetValue flag")?;
                }
                _ => {
                    return Err(NfcError::VerificationFailed(
                        "value read returned non-value settings".to_owned(),
                    ));
                }
            }
        }
        _ => {
            return Err(NfcError::VerificationFailed(
                "a value file did not use GetValue".to_owned(),
            ));
        }
    }

    let records = service.desfire_read(
        CancelToken::new(),
        DesfireReadRequest::new(TEST_AID, RECORD_FILE)
            .key(APP_KEY.to_vec())
            .start_record(0)
            .record_count(Some(2)),
    )?;
    match records {
        DesfireReadResult::Records(read) => {
            println!("record file: {}", read.settings.summary());
            verify(read.record_size == 8, "record size")?;
            verify(read.records.len() == 2, "record count")?;
            verify(read.first_record == 0, "first full record index")?;
            verify(
                read.records == RECORDS.iter().map(Vec::from).collect::<Vec<_>>(),
                "record payloads",
            )?;
        }
        _ => {
            return Err(NfcError::VerificationFailed(
                "a record file did not use ReadRecords".to_owned(),
            ));
        }
    }

    let too_many = service.desfire_read(
        CancelToken::new(),
        DesfireReadRequest::new(TEST_AID, RECORD_FILE)
            .key(APP_KEY.to_vec())
            .record_count(Some(3)),
    );
    verify(
        too_many.is_err(),
        "a record read beyond the current count must fail",
    )?;

    let single = service.desfire_read(
        CancelToken::new(),
        DesfireReadRequest::new(TEST_AID, RECORD_FILE)
            .key(APP_KEY.to_vec())
            .start_record(1)
            .record_count(Some(1)),
    )?;
    match single {
        DesfireReadResult::Records(read) => {
            verify(read.start_record == 1, "record offset")?;
            verify(read.first_record == 0, "single record index")?;
            verify(
                read.records == vec![RECORDS[0].to_vec()],
                format!("single record was {:02X?}", read.records),
            )?;
        }
        _ => {
            return Err(NfcError::VerificationFailed(
                "a single record read returned the wrong type".to_owned(),
            ));
        }
    }

    let latest = service.desfire_read(
        CancelToken::new(),
        DesfireReadRequest::new(TEST_AID, RECORD_FILE)
            .key(APP_KEY.to_vec())
            .start_record(0)
            .record_count(Some(1)),
    )?;
    match latest {
        DesfireReadResult::Records(read) => {
            verify(read.first_record == 1, "latest record index")?;
            verify(read.records == vec![RECORDS[1].to_vec()], "latest record")?;
        }
        _ => {
            return Err(NfcError::VerificationFailed(
                "a latest record read returned the wrong type".to_owned(),
            ));
        }
    }

    let cyclic = service.desfire_read(
        CancelToken::new(),
        DesfireReadRequest::new(TEST_AID, CYCLIC_FILE)
            .key(APP_KEY.to_vec())
            .record_count(Some(1)),
    )?;
    match cyclic {
        DesfireReadResult::Records(read) => {
            verify(read.record_size == 4, "cyclic record size")?;
            verify(read.first_record == 0, "cyclic first record")?;
            verify(
                read.records == vec![CYCLIC_RECORDS[0].to_vec()],
                format!("cyclic records were {:02X?}", read.records),
            )?;
        }
        _ => {
            return Err(NfcError::VerificationFailed(
                "a cyclic file did not use ReadRecords".to_owned(),
            ));
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a DESFire EV3 and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn reads_value_and_record_files_on_a_reserved_application() -> Result<()> {
    let _guard = lock_hardware();
    let conn = gate();

    let setup_error = {
        let mut reader = open(&conn)?;
        let mut desfire = Desfire::new(reader.as_mut());
        setup(&mut desfire).err()
    };

    let result = if let Some(error) = setup_error {
        Err(error)
    } else {
        let mut service = Service::with_transport(Arc::new(DriverRegistry));
        service.refresh_devices()?;
        service.select_device(&conn)?;
        let result = reads(&mut service);
        drop(service);
        result
    };

    if env::var_os("NFC_FOUNDRY_HARDWARE_KEEP_APP").is_some() {
        eprintln!("keeping the reserved test application for manual checks");
    } else {
        let mut reader = open(&conn)?;
        let mut desfire = Desfire::new(reader.as_mut());
        let applications = desfire.application_ids()?;
        if applications.contains(&TEST_AID) {
            let cleanup = remove_test_application(&mut desfire);
            if let Err(error) = &cleanup {
                eprintln!("cleanup failed: {error}");
            }
            cleanup?;
        }
        let applications = desfire.application_ids()?;
        verify(
            !applications.contains(&TEST_AID),
            "the reserved test application is still present",
        )?;
    }
    result
}
