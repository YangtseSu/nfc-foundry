// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Environment-gated DESFire 3K3DES and AES authentication checks.
//!
//! Creates reserved 3K3DES and AES applications with known zero test keys,
//! authenticates through each family, checks the session keys, then removes
//! both applications. Reads under these families need secure messaging
//! (phase 26.5) and are not part of this test.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware-write \
//!   --test hardware_desfire_auth -- --ignored --nocapture
//! ```
//!
//! Deleted DESFire applications keep their EEPROM until a FormatPICC. Set
//! `NFC_FOUNDRY_HARDWARE_FORMAT=yes` to format this test card before the run;
//! that erases every application and resets the PICC to factory state.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_core::{NfcError, Result};
use nfc_desfire::{AuthKeyType, Desfire};
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

const PICC_KEY: [u8; 8] = [0x00; 8];
const TDES3_AID: [u8; 3] = [0xF0, 0x0D, 0x04];
const TDES3_KEY: [u8; 24] = [0x00; 24];
const AES_AID: [u8; 3] = [0xF0, 0x0D, 0x05];
const AES_KEY: [u8; 16] = [0x00; 16];

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

fn setup(desfire: &mut Desfire<'_>) -> Result<()> {
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    if env::var_os("NFC_FOUNDRY_HARDWARE_FORMAT").is_some() {
        desfire.format_picc()?;
        desfire.select_application([0x00; 3])?;
        desfire.authenticate_legacy(0, &PICC_KEY)?;
        eprintln!("formatted the PICC; free memory {}", desfire.free_memory()?);
    } else if desfire.free_memory()? < 512 {
        return Err(NfcError::Unsupported(
            "the test card is out of EEPROM; set NFC_FOUNDRY_HARDWARE_FORMAT=yes to erase it",
        ));
    }

    let inventory = desfire.inventory()?;
    for aid in [TDES3_AID, AES_AID] {
        if inventory
            .applications
            .iter()
            .any(|application| application.aid == aid)
        {
            remove_application(desfire, aid)?;
        }
    }

    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    desfire.create_application(TDES3_AID, 0x0F, 0x41, None, &[])?;
    desfire.select_application(TDES3_AID)?;
    desfire.authenticate(0, &TDES3_KEY, AuthKeyType::Tdes3)?;

    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(0, &PICC_KEY)?;
    desfire.create_application(AES_AID, 0x0F, 0x81, None, &[])?;
    desfire.select_application(AES_AID)?;
    desfire.authenticate(0, &AES_KEY, AuthKeyType::Aes)?;
    Ok(())
}

fn session_checks(desfire: &mut Desfire<'_>) -> Result<()> {
    desfire.select_application(TDES3_AID)?;
    desfire.authenticate(0, &TDES3_KEY, AuthKeyType::Tdes3)?;
    verify(
        desfire.session_key_type() == Some(AuthKeyType::Tdes3),
        "3K3DES session type",
    )?;
    verify(
        desfire.session_key().map(<[u8]>::len) == Some(24),
        "3K3DES session key length",
    )?;
    verify(
        desfire.session_mac_key().map(<[u8]>::len) == Some(24),
        "3K3DES session MAC length",
    )?;

    desfire.select_application(AES_AID)?;
    desfire.authenticate(0, &AES_KEY, AuthKeyType::Aes)?;
    verify(
        desfire.session_key_type() == Some(AuthKeyType::Aes),
        "AES session type",
    )?;
    verify(
        desfire.session_key().map(<[u8]>::len) == Some(16),
        "AES session key length",
    )?;
    verify(
        desfire.session_mac_key().map(<[u8]>::len) == Some(16),
        "AES session MAC length",
    )?;

    desfire.select_application(AES_AID)?;
    desfire.authenticate(0, &AES_KEY, AuthKeyType::AesEv2)?;
    verify(
        desfire.session_key_type() == Some(AuthKeyType::AesEv2),
        "EV2 session type",
    )?;
    let enc = desfire.session_key().map(<[u8]>::to_vec).expect("EV2 ENC");
    let mac = desfire
        .session_mac_key()
        .map(<[u8]>::to_vec)
        .expect("EV2 MAC");
    verify(enc.len() == 16 && mac.len() == 16, "EV2 key lengths")?;
    verify(enc != mac, "EV2 ENC and MAC keys must differ")?;
    verify(desfire.session_key_version() == Some(0), "EV2 key version")?;
    Ok(())
}

fn failures(desfire: &mut Desfire<'_>) -> Result<()> {
    desfire.select_application(AES_AID)?;
    verify(
        desfire
            .authenticate(0, &[0x11; 16], AuthKeyType::Aes)
            .is_err(),
        "a wrong AES key must fail",
    )?;
    verify(
        desfire.authenticate(1, &AES_KEY, AuthKeyType::Aes).is_err(),
        "an unknown key number must fail",
    )?;
    verify(
        desfire
            .authenticate(0, &AES_KEY, AuthKeyType::Tdes3)
            .is_err(),
        "a mismatched key family must fail",
    )?;
    verify(
        desfire
            .authenticate(0, &AES_KEY, AuthKeyType::Tdes2)
            .is_err(),
        "a legacy family against an AES key must fail",
    )?;
    Ok(())
}

#[test]
#[ignore = "requires a DESFire EV3 and NFC_FOUNDRY_HARDWARE_WRITE=yes"]
fn authenticates_with_3k3des_aes_and_ev2_keys() -> Result<()> {
    let _guard = lock_hardware();
    let conn = gate();

    let setup_error = {
        let mut reader = open(&conn)?;
        let mut desfire = Desfire::new(reader.as_mut());
        setup(&mut desfire).err()
    };

    let check_result = if let Some(error) = setup_error {
        Err(error)
    } else {
        let mut reader = open(&conn)?;
        let mut desfire = Desfire::new(reader.as_mut());
        session_checks(&mut desfire).and_then(|()| failures(&mut desfire))
    };

    {
        let mut reader = open(&conn)?;
        let mut desfire = Desfire::new(reader.as_mut());
        for aid in [TDES3_AID, AES_AID] {
            let applications = desfire.application_ids()?;
            if applications.contains(&aid) {
                remove_application(&mut desfire, aid)?;
            }
        }
        let applications = desfire.application_ids()?;
        verify(
            !applications.contains(&TDES3_AID) && !applications.contains(&AES_AID),
            "the reserved authentication applications are still present",
        )?;
    }
    check_result
}
