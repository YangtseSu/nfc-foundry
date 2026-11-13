// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire application and file management checks.
//!
//! Creates a reserved AES application, creates all six file types through
//! `nfc-app`, changes one Standard file's communication mode and access
//! rights, reads every setting back, deletes every file, and removes the
//! application. Each deletion writes a manifest to a temporary directory.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware-write \
//!   --test hardware_desfire_management -- --ignored --nocapture
//! ```
//!
//! Set `NFC_FOUNDRY_HARDWARE_FORMAT=yes` to format the test card first;
//! deleted applications keep their EEPROM until FormatPICC.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_app::Service;
use nfc_app::desfire::{
    AccessRight, AccessRights, ApplicationKeyType, AuthKeyType, CommunicationMode,
    DesfireAppCreateRequest, DesfireAppDeleteRequest, DesfireFileCreateRequest,
    DesfireFileDeleteRequest, DesfireFileSettingsRequest, DesfireFileSpec, FileContents, FileType,
    app_create_text, app_delete_text, file_create_text, file_delete_text, file_settings_text,
};
use nfc_core::{CancelToken, NfcError, Result};
use nfc_desfire::Desfire;
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

const TEST_AID: [u8; 3] = [0xF0, 0x0D, 0x08];
const PICC_KEY: [u8; 8] = [0x00; 8];
const APP_KEY: [u8; 16] = [0x00; 16];
const TMAC_KEY: [u8; 16] = [0x11; 16];

const STANDARD_FILE: u8 = 0x01;
const BACKUP_FILE: u8 = 0x02;
const VALUE_FILE: u8 = 0x03;
const LINEAR_FILE: u8 = 0x04;
const CYCLIC_FILE: u8 = 0x05;
const TMAC_FILE: u8 = 0x06;

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
    env::var("NFC_FOUNDRY_DEVICE").expect("set NFC_FOUNDRY_DEVICE to the exact reader connstring")
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

fn prepare(conn: &str) -> Result<()> {
    let mut reader = open(conn)?;
    let mut desfire = Desfire::new(reader.as_mut());
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    if env::var_os("NFC_FOUNDRY_HARDWARE_FORMAT").is_some() {
        desfire.format_picc()?;
        desfire.select_application([0x00; 3])?;
        desfire.authenticate_legacy(0, &PICC_KEY)?;
        eprintln!("formatted the PICC; free memory {}", desfire.free_memory()?);
    }
    let applications = desfire.application_ids()?;
    if applications.contains(&TEST_AID) {
        desfire.delete_application(TEST_AID)?;
    }
    let free = desfire.free_memory()?;
    eprintln!("free memory before setup: {free}");
    if free < 1024 {
        return Err(NfcError::Unsupported(
            "the test card is out of EEPROM; set NFC_FOUNDRY_HARDWARE_FORMAT=yes to erase it",
        ));
    }
    Ok(())
}

fn service(conn: &str) -> Result<Service> {
    let mut service = Service::with_transport(Arc::new(DriverRegistry));
    service.refresh_devices()?;
    service.select_device(conn)?;
    Ok(service)
}

fn file_specs() -> Vec<DesfireFileSpec> {
    vec![
        DesfireFileSpec::Standard {
            file_id: STANDARD_FILE,
            iso_file_id: None,
            communication: 0,
            access_rights: 0x0000,
            size: 16,
        },
        DesfireFileSpec::Backup {
            file_id: BACKUP_FILE,
            iso_file_id: None,
            communication: 0,
            access_rights: 0x0000,
            size: 16,
        },
        DesfireFileSpec::Value {
            file_id: VALUE_FILE,
            iso_file_id: None,
            communication: 0,
            access_rights: 0x0000,
            lower_limit: -50,
            upper_limit: 50,
            value: 7,
            flags: 0x03,
        },
        DesfireFileSpec::LinearRecord {
            file_id: LINEAR_FILE,
            iso_file_id: None,
            communication: 0,
            access_rights: 0x0000,
            record_size: 4,
            max_records: 4,
        },
        DesfireFileSpec::CyclicRecord {
            file_id: CYCLIC_FILE,
            iso_file_id: None,
            communication: 0,
            access_rights: 0x0000,
            record_size: 4,
            max_records: 2,
        },
        DesfireFileSpec::TransactionMac {
            file_id: TMAC_FILE,
            iso_file_id: None,
            communication: 0,
            // Transaction MAC files refuse direct writes and the
            // CommitReaderID path; read is free, change is key 0.
            access_rights: 0xEFF0,
            key: TMAC_KEY,
            key_version: 1,
        },
    ]
}

fn create_and_check(service: &mut Service, spec: DesfireFileSpec) -> Result<()> {
    let file_id = spec.file_id();
    let expected_type = spec.file_type();
    let result = service
        .desfire_create_file(
            DesfireFileCreateRequest::new(TEST_AID, spec)
                .key(APP_KEY.to_vec())
                .key_type(AuthKeyType::Aes),
        )
        .map_err(|error| {
            NfcError::VerificationFailed(format!("create file {file_id:02X}: {error}"))
        })?;
    verify(
        result.settings.file_type == expected_type,
        format!(
            "file {file_id:02X} read back as {}, expected {}",
            result.settings.file_type.name(),
            expected_type.name()
        ),
    )?;
    verify(
        result.required > 0 && result.free_memory_after < result.free_memory_before,
        format!("file {file_id:02X} did not consume free memory"),
    )?;
    println!("{}", file_create_text(&result).trim_end());
    Ok(())
}

fn change_standard_settings(service: &mut Service) -> Result<()> {
    let rights = AccessRights::from_parts(
        AccessRight::Free,
        AccessRight::Key(0),
        AccessRight::Denied,
        AccessRight::Key(0),
    );
    let result = service.desfire_change_file_settings(
        DesfireFileSettingsRequest::new(TEST_AID, STANDARD_FILE)
            .communication(CommunicationMode::Enciphered)
            .access_rights(rights.raw)
            .key(APP_KEY.to_vec())
            .key_type(AuthKeyType::Aes),
    )?;
    verify(
        result.changed,
        "the settings change was reported as unnecessary",
    )?;
    verify(
        result.after.communication == CommunicationMode::Enciphered,
        "the Standard file mode did not read back as enciphered",
    )?;
    verify(
        result.after.access.raw == rights.raw,
        "the Standard file access rights did not read back",
    )?;
    verify(
        result.after.file_type == FileType::Standard,
        "the Standard file changed type",
    )?;
    println!("{}", file_settings_text(&result).trim_end());
    Ok(())
}

fn check_contents(service: &mut Service) -> Result<()> {
    let inventory = service.desfire_inventory(CancelToken::new())?;
    let application = inventory
        .applications
        .iter()
        .find(|application| application.aid == TEST_AID)
        .ok_or_else(|| {
            NfcError::VerificationFailed("the reserved application is missing".to_owned())
        })?;
    let mut types = Vec::new();
    for file in &application.files {
        let settings = file.decoded()?;
        types.push((file.file_id, settings.file_type));
        match settings.contents {
            FileContents::Value {
                lower_limit,
                upper_limit,
                ..
            } => verify(
                lower_limit == -50 && upper_limit == 50,
                "the value file settings did not read back",
            )?,
            FileContents::Record {
                record_size,
                max_records,
                current_records,
            } => verify(
                record_size == 4 && current_records == 0 && (max_records == 4 || max_records == 2),
                "the record file settings did not read back",
            )?,
            FileContents::TransactionMac {
                key_version,
                counter_limit,
                ..
            } => verify(
                key_version == 1 && counter_limit.is_none(),
                "the Transaction MAC settings did not read back",
            )?,
            FileContents::Data { size } => verify(size == 16, "the data file size changed")?,
        }
    }
    types.sort_by_key(|(file_id, _)| *file_id);
    verify(
        types
            == [
                (STANDARD_FILE, FileType::Standard),
                (BACKUP_FILE, FileType::Backup),
                (VALUE_FILE, FileType::Value),
                (LINEAR_FILE, FileType::LinearRecord),
                (CYCLIC_FILE, FileType::CyclicRecord),
                (TMAC_FILE, FileType::TransactionMac),
            ],
        format!("the six file types did not read back: {types:?}"),
    )?;
    Ok(())
}

fn delete_files(service: &mut Service, directory: &std::path::Path) -> Result<()> {
    for file_id in [
        STANDARD_FILE,
        BACKUP_FILE,
        VALUE_FILE,
        LINEAR_FILE,
        CYCLIC_FILE,
        TMAC_FILE,
    ] {
        let manifest = directory.join(format!("file-{file_id:02X}.json"));
        let result = service.desfire_delete_file(
            DesfireFileDeleteRequest::new(TEST_AID, file_id)
                .key(APP_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .advanced(true)
                .confirm(true)
                .manifest_path(&manifest),
        )?;
        verify(
            !result.files.contains(&file_id),
            format!("file {file_id:02X} is still present after deletion"),
        )?;
        verify(
            manifest.exists(),
            format!("file {file_id:02X} has no manifest"),
        )?;
        println!("{}", file_delete_text(&result).trim_end());
    }
    Ok(())
}

#[test]
#[ignore = "requires a DESFire EV3 and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn creates_changes_and_deletes_every_file_type() -> Result<()> {
    let _guard = lock_hardware();
    let conn = gate();
    prepare(&conn)?;

    let directory = tempfile::tempdir().map_err(|error| {
        NfcError::Io(format!("could not create the manifest directory: {error}"))
    })?;

    let mut service = service(&conn)?;
    let created = service.desfire_create_application(
        DesfireAppCreateRequest::new(TEST_AID)
            .key(PICC_KEY.to_vec())
            .keys(2)
            .app_key_type(ApplicationKeyType::Aes),
    )?;
    verify(
        created.applications.contains(&TEST_AID),
        "the application is missing after creation",
    )?;
    println!("{}", app_create_text(&created).trim_end());

    for spec in file_specs() {
        create_and_check(&mut service, spec)?;
    }
    change_standard_settings(&mut service)?;
    check_contents(&mut service)?;

    delete_files(&mut service, directory.path())?;
    let manifest = directory.path().join("application.json");
    let deleted = service.desfire_delete_application(
        DesfireAppDeleteRequest::new(TEST_AID)
            .key(PICC_KEY.to_vec())
            .advanced(true)
            .confirm(true)
            .manifest_path(&manifest),
    )?;
    verify(
        !deleted.applications.contains(&TEST_AID),
        "the application is still present after deletion",
    )?;
    verify(manifest.exists(), "the application has no manifest")?;
    println!("{}", app_delete_text(&deleted).trim_end());

    let inventory = service.desfire_inventory(CancelToken::new())?;
    verify(
        !inventory
            .applications
            .iter()
            .any(|application| application.aid == TEST_AID),
        "the reserved application is still listed",
    )?;
    println!("application management passed");
    Ok(())
}
