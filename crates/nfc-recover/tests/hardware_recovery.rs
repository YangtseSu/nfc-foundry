//! Environment-gated recovery acquisition checks.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! NFC_FOUNDRY_HARDWARE_KEY=D3F7D3F7D3F7 \
//! cargo test -p nfc-recover --test hardware_recovery -- --ignored --nocapture
//! ```

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_core::{CardType, Key, KeyType};
use nfc_drivers::DriverRegistry;
use nfc_recover::acquire;
use nfc_transport::{Reader, Transport};

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

fn lock_hardware() -> MutexGuard<'static, ()> {
    HARDWARE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn device_connstring() -> String {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE").as_deref(),
        Ok("1"),
        "set NFC_FOUNDRY_HARDWARE=1 to run hardware tests"
    );
    env::var("NFC_FOUNDRY_DEVICE").expect("set NFC_FOUNDRY_DEVICE to an exact reader connstring")
}

fn classic_key() -> Key {
    let value = env::var("NFC_FOUNDRY_HARDWARE_KEY").unwrap_or_else(|_| "FFFFFFFFFFFF".to_owned());
    Key::from_hex(&value).expect("NFC_FOUNDRY_HARDWARE_KEY must be 12 hex digits")
}

fn target_block() -> u8 {
    env::var("NFC_FOUNDRY_HARDWARE_TARGET_BLOCK")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(0)
}

fn target_key() -> Key {
    let value =
        env::var("NFC_FOUNDRY_HARDWARE_TARGET_KEY").unwrap_or_else(|_| classic_key().to_hex());
    Key::from_hex(&value).expect("NFC_FOUNDRY_HARDWARE_TARGET_KEY must be 12 hex digits")
}

fn classic_key_type() -> KeyType {
    match env::var("NFC_FOUNDRY_HARDWARE_KEY_TYPE").as_deref() {
        Ok("B" | "b") => KeyType::B,
        _ => KeyType::A,
    }
}

fn open_reader() -> Box<dyn Reader> {
    let conn_string = device_connstring();
    let registry = Arc::new(DriverRegistry);
    let mut reader = registry.open(&conn_string).expect("open reader");
    reader.open(&conn_string).expect("connect reader");
    reader
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn software_authentication_matches_the_card() {
    let _guard = lock_hardware();
    let mut reader = open_reader();
    assert!(reader.capabilities().recovery_parity);
    let card = reader.card_info().expect("select card");
    assert!(
        matches!(
            card.card_type,
            CardType::MifareClassic1K | CardType::MifareClassic4K
        ),
        "place a MIFARE Classic card on the reader"
    );
    let block = env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(4);
    let mut session = acquire::Session::authenticate(
        &mut *reader,
        block,
        classic_key_type(),
        classic_key(),
        Some(0x1234_5678),
    )
    .expect("software Crypto1 authentication");
    println!(
        "software auth uid={:08X} nt={:08X} state={:08X}/{:08X}",
        session.uid, session.tag_nonce, session.state.odd, session.state.even
    );

    let nested = session.nested_nonce(&mut *reader, block, classic_key_type());
    match nested {
        Ok(nonce) => println!("nested nonce {nonce:?}"),
        Err(error) => println!("nested nonce unavailable: {error}"),
    }
    session.leave(&mut *reader).expect("leave recovery mode");
    reader.close().expect("close reader");
}

#[test]
#[ignore = "requires an FM11RF08S card and a known key"]
fn staticnested_candidates_contain_the_known_key() {
    let _guard = lock_hardware();
    let mut reader = open_reader();
    let card = reader.card_info().expect("select card");
    assert_eq!(
        card.card_type,
        CardType::MifareClassic1K,
        "place an FM11RF08S (MIFARE Classic 1K) card on the reader"
    );
    let target_block = target_block().max(4);
    let key = classic_key();
    let sample =
        nfc_recover::staticnested::collect_sample_backdoor(&mut *reader, target_block, KeyType::A)
            .expect("collect a backdoor static encrypted nonce");
    let actual_nt = acquire::decrypt_nested_nonce(sample.uid, sample.nt_enc, key);
    assert_eq!(
        sample.nt, actual_nt,
        "the backdoor clear nonce does not match the known-key decryption"
    );
    let candidates = nfc_recover::staticnested::candidates(&sample);
    println!(
        "static sample uid={:08X} nt={:08X} nt_enc={:08X} par_err={:02X} candidates={}",
        sample.uid,
        sample.nt,
        sample.nt_enc,
        sample.par_err,
        candidates.len()
    );
    assert!(
        candidates.contains(&key),
        "the known key is missing from the staticnested candidate set"
    );
    reader.close().expect("close reader");
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn nested_calibration_reports_distance() {
    let _guard = lock_hardware();
    let mut reader = open_reader();
    let block = env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(4);
    let key = classic_key();
    let key_type = classic_key_type();
    for round in 0..10 {
        let mut session = acquire::Session::authenticate(&mut *reader, block, key_type, key, None)
            .expect("first authentication");
        let first = session
            .nested_nonce(&mut *reader, block, key_type)
            .expect("first nested authentication");
        let first_plain =
            acquire::decrypt_nested_nonce(session.uid, first.tag_nonce_encrypted, key);
        let first_distance = nfc_recover::crypto1::nonce_distance(session.tag_nonce, first_plain);
        let first_tag_nonce = session.tag_nonce;
        session.leave(&mut *reader).expect("leave recovery mode");
        let mut delayed = acquire::Session::authenticate(&mut *reader, block, key_type, key, None)
            .expect("delayed authentication");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = delayed
            .nested_nonce(&mut *reader, block, key_type)
            .expect("delayed nested authentication");
        let second_plain =
            acquire::decrypt_nested_nonce(delayed.uid, second.tag_nonce_encrypted, key);
        let second_distance = nfc_recover::crypto1::nonce_distance(delayed.tag_nonce, second_plain);
        println!(
            "round {round}: nt1={:08X} d1={first_distance} delayed_nt={:08X} d2={second_distance} enc1={:08X} enc2={:08X} p1={:08X} p2={:08X}",
            first_tag_nonce,
            delayed.tag_nonce,
            first.tag_nonce_encrypted,
            second.tag_nonce_encrypted,
            first_plain,
            second_plain
        );
        delayed.leave(&mut *reader).expect("leave recovery mode");
    }
    reader.close().expect("close reader");
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn nested_recovers_a_known_test_key() {
    let _guard = lock_hardware();
    use nfc_core::CancelToken;
    use nfc_recover::nested::{NestedRequest, recover_nested, verify_key};

    let mut reader = open_reader();
    let known_key = classic_key();
    let request = NestedRequest {
        known_block: env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(4),
        known_key_type: classic_key_type(),
        known_key,
        target_block: target_block(),
        target_key_type: classic_key_type(),
    };
    let outcome =
        recover_nested(&mut *reader, &request, &CancelToken::new()).expect("nested recovery");
    println!(
        "nested candidates={:?} distance={}..={} samples={}",
        outcome
            .keys
            .iter()
            .map(|key| key.to_hex())
            .collect::<Vec<_>>(),
        outcome.distance_min,
        outcome.distance_max,
        outcome.samples
    );
    assert!(!outcome.keys.is_empty(), "no candidate key recovered");
    let verified = outcome.keys.iter().find(|key| {
        verify_key(
            &mut *reader,
            request.target_block,
            request.target_key_type,
            **key,
        )
        .is_ok()
    });
    assert!(verified.is_some(), "no candidate authenticated");
    println!(
        "verified key {}",
        verified.map(|key| key.to_hex()).unwrap_or_default()
    );
    reader.close().expect("close reader");
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn hardnested_recovers_a_known_test_key() {
    let _guard = lock_hardware();
    use nfc_core::CancelToken;
    use nfc_recover::hardnested::{HardnestedRequest, recover};
    use nfc_recover::nested::verify_key;

    let mut reader = open_reader();
    let request = HardnestedRequest {
        known_block: env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(4),
        known_key_type: classic_key_type(),
        known_key: classic_key(),
        target_block: target_block(),
        target_key_type: classic_key_type(),
        tables_dir: env::var_os("NFC_FOUNDRY_HARDNESTED_TABLES").map(std::path::PathBuf::from),
        max_nonces: env::var("NFC_FOUNDRY_HARDNESTED_MAX_NONCES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(4000),
    };
    let outcome = recover(&mut *reader, &request, &CancelToken::new(), &mut |_| {})
        .expect("hardnested recovery");
    println!(
        "hardnested nonces={} tested={} keys={:?}",
        outcome.nonces,
        outcome.tested_states,
        outcome
            .keys
            .iter()
            .map(|key| key.to_hex())
            .collect::<Vec<_>>()
    );
    assert!(!outcome.keys.is_empty(), "no key recovered");
    let verified = outcome.keys.iter().find(|key| {
        verify_key(
            &mut *reader,
            request.target_block,
            request.target_key_type,
            **key,
        )
        .is_ok()
    });
    assert!(verified.is_some(), "no candidate authenticated");
    println!(
        "verified key {}",
        verified.map(|key| key.to_hex()).unwrap_or_default()
    );
    reader.close().expect("close reader");
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn nested_target_distance_probe() {
    let _guard = lock_hardware();
    let mut reader = open_reader();
    let known_key = classic_key();
    let target_key = target_key();
    let known_block = env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(4);
    for round in 0..6 {
        let mut session = acquire::Session::authenticate(
            &mut *reader,
            known_block,
            classic_key_type(),
            known_key,
            None,
        )
        .expect("known auth");
        let nested = session
            .nested_nonce(&mut *reader, target_block(), classic_key_type())
            .expect("nested target");
        let plain =
            acquire::decrypt_nested_nonce(session.uid, nested.tag_nonce_encrypted, target_key);
        let distance = nfc_recover::crypto1::nonce_distance(session.tag_nonce, plain);
        println!(
            "round {round}: nt1={:08X} enc={:08X} plain={:08X} distance={distance} parity={:02X} matches={}",
            session.tag_nonce,
            nested.tag_nonce_encrypted,
            plain,
            nested.parity,
            nfc_recover::nested::parity_matches(plain, nested.tag_nonce_encrypted, nested.parity)
        );
        session.leave(&mut *reader).expect("leave recovery mode");
    }
    reader.close().expect("close reader");
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn darkside_collects_repeated_nonces() {
    let _guard = lock_hardware();
    use nfc_core::CancelToken;
    use nfc_recover::darkside::{collect, solve};
    use nfc_recover::nested::verify_key;

    let mut reader = open_reader();
    let cancel = CancelToken::new();
    let sample = collect(&mut *reader, 0, KeyType::A, &cancel, 200).expect("darkside collection");
    match sample {
        Some(sample) => {
            let keys = solve(&sample);
            println!(
                "darkside nt={:08X} keys={:?}",
                sample.tag_nonce,
                keys.iter().map(|key| key.to_hex()).collect::<Vec<_>>()
            );
            let verified = keys
                .into_iter()
                .find(|key| verify_key(&mut *reader, 0, KeyType::A, *key).is_ok());
            assert!(verified.is_some(), "no darkside candidate authenticated");
            println!(
                "darkside verified {}",
                verified.map(|key| key.to_hex()).unwrap_or_default()
            );
        }
        None => println!("darkside found no repeated-nonce sample"),
    }
    reader.close().expect("close reader");
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn prng_classification_probes_the_card() {
    use nfc_recover::classify::classify;

    let _guard = lock_hardware();
    let mut reader = open_reader();
    let block = env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(4);
    let profile = classify(&mut *reader, block, classic_key_type(), None).expect("classify card");
    println!("profile {:?}", profile);
    assert!(!matches!(profile.prng, nfc_recover::PrngProfile::Hard));
    reader.close().expect("close reader");
}
