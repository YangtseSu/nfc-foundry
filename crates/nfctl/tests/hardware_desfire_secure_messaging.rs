// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire EV1/EV2 secure-messaging read checks.
//!
//! Creates reserved 3K3DES and AES applications with zero test keys and one
//! plain, one MACed, and one enciphered standard file plus one MACed and one
//! enciphered value file. Reads every file through each secure channel, writes
//! a known range to the standard files through `nfc-app`, reads it back, then
//! removes both applications.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware-write \
//!   --test hardware_desfire_secure_messaging -- --ignored --nocapture
//! ```
//!
//! Deleted DESFire applications keep their EEPROM until a FormatPICC. Set
//! `NFC_FOUNDRY_HARDWARE_FORMAT=yes` to format this test card before the run;
//! that erases every application and resets the PICC to factory state.
//! Set `NFC_FOUNDRY_HARDWARE_KEEP_APP=1` to keep the fixtures for manual
//! `nfctl` or GUI checks; the next run removes them before setup.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_app::Service;
use nfc_app::desfire::{DesfireReadRequest, DesfireReadResult, DesfireWriteRequest, FileContents};
use nfc_core::{CancelToken, NfcError, Result};
use nfc_desfire::{AuthKeyType, CommunicationMode, Desfire, FileSettings, ValueFileSpec};
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

const PICC_KEY: [u8; 8] = [0x00; 8];
const TDES3_AID: [u8; 3] = [0xF0, 0x0D, 0x06];
const TDES3_KEY: [u8; 24] = [0x00; 24];
const AES_AID: [u8; 3] = [0xF0, 0x0D, 0x07];
const AES_KEY: [u8; 16] = [0x00; 16];

const PLAIN_FILE: u8 = 0x01;
const MACED_FILE: u8 = 0x02;
const FULL_FILE: u8 = 0x03;
const MACED_VALUE: u8 = 0x04;
const FULL_VALUE: u8 = 0x05;
const BACKUP_FILE: u8 = 0x06;
const DATA_SIZE: u32 = 48;
const MACED_VALUE_INIT: i32 = 21;
const FULL_VALUE_INIT: i32 = 43;

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

fn remove_application(desfire: &mut Desfire<'_>, aid: [u8; 3]) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    desfire.delete_application(aid)?;
    Ok(())
}

fn create_fixture(
    desfire: &mut Desfire<'_>,
    aid: [u8; 3],
    key_settings2: u8,
    key: &[u8],
    key_type: AuthKeyType,
) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    desfire.create_application(aid, 0x0F, key_settings2, None, &[])?;

    desfire.select_application(aid)?;
    desfire.authenticate(0, key, key_type)?;
    desfire.create_standard_file(PLAIN_FILE, None, 0x00, 0x0000, DATA_SIZE)?;
    desfire.create_standard_file(MACED_FILE, None, 0x01, 0x0000, DATA_SIZE)?;
    desfire.create_standard_file(FULL_FILE, None, 0x03, 0x0000, DATA_SIZE)?;
    desfire.create_backup_file(BACKUP_FILE, None, 0x00, 0x0000, DATA_SIZE)?;
    desfire.create_value_file(ValueFileSpec {
        file_id: MACED_VALUE,
        iso_file_id: None,
        communication: 0x01,
        access_rights: 0x0000,
        lower_limit: 0,
        upper_limit: 100,
        value: MACED_VALUE_INIT,
        flags: 0x01,
    })?;
    desfire.create_value_file(ValueFileSpec {
        file_id: FULL_VALUE,
        iso_file_id: None,
        communication: 0x03,
        access_rights: 0x0000,
        lower_limit: 0,
        upper_limit: 100,
        value: FULL_VALUE_INIT,
        flags: 0x01,
    })?;
    Ok(())
}

fn setup(desfire: &mut Desfire<'_>) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    if env::var_os("NFC_FOUNDRY_HARDWARE_FORMAT").is_some() {
        desfire.format_picc()?;
        desfire.select_application([0x00; 3])?;
        desfire.authenticate_legacy(0, &PICC_KEY)?;
        eprintln!("formatted the PICC; free memory {}", desfire.free_memory()?);
    } else {
        let free = desfire.free_memory()?;
        if free < 1024 {
            eprintln!("the test card has {free} free EEPROM bytes");
            return Err(NfcError::Unsupported(
                "the test card is out of EEPROM; set NFC_FOUNDRY_HARDWARE_FORMAT=yes to erase it",
            ));
        }
    }

    let applications = desfire.application_ids()?;
    for aid in [TDES3_AID, AES_AID] {
        if applications.contains(&aid) {
            remove_application(desfire, aid)?;
        }
    }

    create_fixture(desfire, TDES3_AID, 0x41, &TDES3_KEY, AuthKeyType::Tdes3)?;
    create_fixture(desfire, AES_AID, 0x81, &AES_KEY, AuthKeyType::Aes)
}

fn settings(
    desfire: &mut Desfire<'_>,
    file_id: u8,
    expected: CommunicationMode,
) -> Result<FileSettings> {
    let raw = desfire.file_settings(file_id)?;
    let settings = FileSettings::parse(&raw)?;
    verify(
        settings.communication == expected,
        format!(
            "file {file_id:02X} is {}, expected {}",
            settings.communication.name(),
            expected.name()
        ),
    )?;
    Ok(settings)
}

fn direct_reads(
    desfire: &mut Desfire<'_>,
    aid: [u8; 3],
    key: &[u8],
    key_type: AuthKeyType,
) -> Result<()> {
    desfire.select_application(aid)?;
    desfire.authenticate(0, key, key_type)?;

    for (file_id, mode) in [
        (PLAIN_FILE, CommunicationMode::Plain),
        (MACED_FILE, CommunicationMode::Maced),
        (FULL_FILE, CommunicationMode::Enciphered),
        (BACKUP_FILE, CommunicationMode::Plain),
    ] {
        let settings = settings(desfire, file_id, mode)?;
        let data = desfire
            .read_data(file_id, 0, DATA_SIZE, settings.communication)?
            .data;
        verify(
            data == vec![0u8; DATA_SIZE as usize],
            format!(
                "{} read of file {file_id:02X} returned {} bytes",
                key_type.name(),
                data.len()
            ),
        )?;
    }

    for (file_id, mode, expected) in [
        (MACED_VALUE, CommunicationMode::Maced, MACED_VALUE_INIT),
        (FULL_VALUE, CommunicationMode::Enciphered, FULL_VALUE_INIT),
    ] {
        let settings = settings(desfire, file_id, mode)?;
        let value = desfire.get_value(file_id, settings.communication)?.value;
        verify(
            value == expected,
            format!(
                "{} value file {file_id:02X} returned {value}",
                key_type.name()
            ),
        )?;
    }
    Ok(())
}

const WRITE_OFFSET: u32 = 8;
const WRITE_PAYLOAD: [u8; 16] = [0x5A; 16];

fn service_writes(
    service: &mut Service,
    aid: [u8; 3],
    key: &[u8],
    key_type: AuthKeyType,
) -> Result<()> {
    for (file_id, mode, backup) in [
        (PLAIN_FILE, CommunicationMode::Plain, false),
        (MACED_FILE, CommunicationMode::Maced, false),
        (FULL_FILE, CommunicationMode::Enciphered, false),
        (BACKUP_FILE, CommunicationMode::Plain, true),
    ] {
        let backup_path = std::env::temp_dir().join(format!(
            "nfc-foundry-desfire-write-{}-{file_id:02X}.json",
            std::process::id()
        ));
        let request = DesfireWriteRequest::new(aid, file_id, WRITE_PAYLOAD.to_vec())
            .offset(WRITE_OFFSET)
            .key(key.to_vec())
            .key_type(key_type)
            .backup_path(&backup_path);
        let write = service.desfire_write(CancelToken::new(), request)?;
        let backup_path = write.backup_path.clone().ok_or_else(|| {
            NfcError::VerificationFailed(format!(
                "{} write of file {file_id:02X} has no backup path",
                key_type.name()
            ))
        })?;
        verify(
            backup_path.exists(),
            format!(
                "{} write backup {} is missing",
                key_type.name(),
                backup_path.display()
            ),
        )?;
        verify(
            write.communication == mode,
            format!(
                "{} write of file {file_id:02X} used {}, expected {}",
                key_type.name(),
                write.communication.name(),
                mode.name()
            ),
        )?;
        verify(
            write.offset == WRITE_OFFSET && write.written == WRITE_PAYLOAD.len(),
            format!(
                "{} write of file {file_id:02X} covered the wrong range",
                key_type.name()
            ),
        )?;
        verify(
            write.committed == backup,
            format!(
                "{} file {file_id:02X} commit state is {}, expected {backup}",
                key_type.name(),
                write.committed
            ),
        )?;
        verify(
            write.read_back == WRITE_PAYLOAD,
            format!(
                "{} write of file {file_id:02X} did not read back",
                key_type.name()
            ),
        )?;
        verify(
            write.backup.len() == DATA_SIZE as usize,
            "the pre-write backup did not contain the whole file",
        )?;
        println!(
            "{} file {file_id:02X} {} write: {}",
            key_type.name(),
            mode.name(),
            write.integrity.label()
        );

        let read = service.desfire_read(
            CancelToken::new(),
            DesfireReadRequest::new(aid, file_id)
                .key(key.to_vec())
                .key_type(key_type),
        )?;
        let DesfireReadResult::Data(read) = read else {
            return Err(NfcError::VerificationFailed(
                "a standard file did not use ReadData".to_owned(),
            ));
        };
        let start = WRITE_OFFSET as usize;
        let end = start + WRITE_PAYLOAD.len();
        verify(
            read.data[start..end] == WRITE_PAYLOAD,
            format!(
                "{} file {file_id:02X} did not keep the written range",
                key_type.name()
            ),
        )?;
    }
    Ok(())
}

fn service_reads(
    service: &mut Service,
    aid: [u8; 3],
    key: &[u8],
    key_type: AuthKeyType,
) -> Result<()> {
    for (file_id, mode, expected) in [
        (PLAIN_FILE, CommunicationMode::Plain, DATA_SIZE),
        (MACED_FILE, CommunicationMode::Maced, DATA_SIZE),
        (FULL_FILE, CommunicationMode::Enciphered, DATA_SIZE),
        (BACKUP_FILE, CommunicationMode::Plain, DATA_SIZE),
    ] {
        let request = DesfireReadRequest::new(aid, file_id)
            .key(key.to_vec())
            .key_type(key_type);
        let read = service.desfire_read(CancelToken::new(), request)?;
        match read {
            DesfireReadResult::Data(read) => {
                verify(
                    read.settings.communication == mode,
                    format!("service read of file {file_id:02X} decoded the wrong mode"),
                )?;
                verify(
                    read.data == vec![0u8; expected as usize],
                    format!(
                        "service read of file {file_id:02X} returned {} bytes",
                        read.data.len()
                    ),
                )?;
            }
            _ => {
                return Err(NfcError::VerificationFailed(
                    "a standard file did not use ReadData".to_owned(),
                ));
            }
        }
    }

    for (file_id, expected) in [
        (MACED_VALUE, MACED_VALUE_INIT),
        (FULL_VALUE, FULL_VALUE_INIT),
    ] {
        let request = DesfireReadRequest::new(aid, file_id)
            .key(key.to_vec())
            .key_type(key_type);
        let read = service.desfire_read(CancelToken::new(), request)?;
        match read {
            DesfireReadResult::Value(read) => {
                verify(
                    read.value == expected,
                    format!("service value file {file_id:02X} returned {}", read.value),
                )?;
                verify(
                    matches!(read.settings.contents, FileContents::Value { .. }),
                    "a value file did not decode as a value file",
                )?;
            }
            _ => {
                return Err(NfcError::VerificationFailed(
                    "a value file did not use GetValue".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a DESFire EV3 and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn reads_maced_and_enciphered_files_over_ev1_and_ev2() -> Result<()> {
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
        let mut outcome = Ok(());
        for (aid, key, key_type) in [
            (TDES3_AID, &TDES3_KEY[..], AuthKeyType::Tdes3),
            (AES_AID, &AES_KEY[..], AuthKeyType::Aes),
            (AES_AID, &AES_KEY[..], AuthKeyType::AesEv2),
        ] {
            {
                let mut reader = open(&conn)?;
                let mut desfire = Desfire::new(reader.as_mut());
                if let Err(error) = direct_reads(&mut desfire, aid, key, key_type) {
                    outcome = Err(error);
                    break;
                }
            }

            let mut service = Service::with_transport(Arc::new(DriverRegistry));
            service.refresh_devices()?;
            service.select_device(&conn)?;
            let service_result = service_reads(&mut service, aid, key, key_type);
            drop(service);
            if let Err(error) = service_result {
                outcome = Err(error);
                break;
            }
            eprintln!("{} secure-messaging reads passed", key_type.name());
        }
        if outcome.is_ok() {
            for (aid, key, key_type) in [
                (TDES3_AID, &TDES3_KEY[..], AuthKeyType::Tdes3),
                (AES_AID, &AES_KEY[..], AuthKeyType::Aes),
                (AES_AID, &AES_KEY[..], AuthKeyType::AesEv2),
            ] {
                let mut service = Service::with_transport(Arc::new(DriverRegistry));
                service.refresh_devices()?;
                service.select_device(&conn)?;
                let service_result = service_writes(&mut service, aid, key, key_type);
                drop(service);
                if let Err(error) = service_result {
                    outcome = Err(error);
                    break;
                }
                eprintln!("{} secure-messaging writes passed", key_type.name());
            }
        }
        outcome
    };

    if env::var_os("NFC_FOUNDRY_HARDWARE_KEEP_APP").is_some() {
        eprintln!("keeping the reserved test applications for manual checks");
    } else {
        let mut reader = open(&conn)?;
        let mut desfire = Desfire::new(reader.as_mut());
        let applications = desfire.application_ids()?;
        for aid in [TDES3_AID, AES_AID] {
            if applications.contains(&aid) {
                remove_application(&mut desfire, aid)?;
            }
        }
        let applications = desfire.application_ids()?;
        verify(
            !applications.contains(&TDES3_AID) && !applications.contains(&AES_AID),
            "the reserved secure-messaging applications are still present",
        )?;
        eprintln!("applications: none");
    }
    result
}
