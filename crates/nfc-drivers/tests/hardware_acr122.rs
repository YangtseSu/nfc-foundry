//! Environment-gated ACR122U USB/CCID hardware checks.
//!
//! Read-only:
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
//! cargo test -p nfc-drivers --features hardware -- --ignored
//! ```
//!
//! The Gen1A manufacturer-block write additionally requires the
//! `hardware-write` feature and both `NFC_FOUNDRY_HARDWARE_WRITE` and
//! `NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM` set to `I_OWN_THIS_CARD`.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_core::{CardInfo, CardType, Key, KeyType, NfcError};
use nfc_drivers::acr122_usb::Acr122Usb;
#[cfg(feature = "hardware-write")]
use nfc_transport::ManufacturerWriter;
use nfc_transport::{DeviceInfo, Reader};

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
    let conn_string = env::var("NFC_FOUNDRY_DEVICE")
        .expect("set NFC_FOUNDRY_DEVICE to the exact acr122_usb:<bus>:<address> connstring");
    assert!(
        conn_string.starts_with("acr122_usb:"),
        "NFC_FOUNDRY_DEVICE must be an acr122_usb connstring, got {conn_string}"
    );
    conn_string
}

fn open_reader(conn_string: &str) -> Acr122Usb {
    let mut reader = Acr122Usb::new(DeviceInfo::new("ACR122U USB", conn_string));
    reader.open(conn_string).expect("open the ACR122U");
    let firmware = reader.firmware().expect("firmware after open");
    println!(
        "ACR122U internal PN532 ic={:02X} version={}.{} support={:02X}",
        firmware.ic, firmware.version, firmware.revision, firmware.support
    );
    reader
}

fn selected_card(reader: &mut Acr122Usb) -> CardInfo {
    match reader.card_info() {
        Ok(card) => card,
        Err(NfcError::NoCard) => {
            panic!("place a MIFARE Classic or Type 2 card on the reader and rerun")
        }
        Err(error) => panic!("card selection failed: {error}"),
    }
}

fn classic_block() -> u8 {
    env::var("NFC_FOUNDRY_HARDWARE_BLOCK")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(4)
}

fn classic_key() -> Key {
    let value = env::var("NFC_FOUNDRY_HARDWARE_KEY").unwrap_or_else(|_| "FFFFFFFFFFFF".to_owned());
    Key::from_hex(&value).expect("NFC_FOUNDRY_HARDWARE_KEY must be 12 hex digits")
}

fn classic_key_type() -> KeyType {
    match env::var("NFC_FOUNDRY_HARDWARE_KEY_TYPE").as_deref() {
        Ok("B" | "b") => KeyType::B,
        _ => KeyType::A,
    }
}

#[cfg(feature = "hardware-write")]
fn sector_zero_key() -> Key {
    match env::var("NFC_FOUNDRY_HARDWARE_SECTOR0_KEY") {
        Ok(value) => {
            Key::from_hex(&value).expect("NFC_FOUNDRY_HARDWARE_SECTOR0_KEY must be 12 hex digits")
        }
        Err(_) => classic_key(),
    }
}

fn crc_a(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0x6363;
    for &byte in data {
        let mut mixed = byte ^ crc as u8;
        mixed ^= mixed << 4;
        crc = (crc >> 8) ^ ((mixed as u16) << 8) ^ ((mixed as u16) << 3) ^ ((mixed as u16) >> 4);
    }
    [crc as u8, (crc >> 8) as u8]
}

fn read_classic(reader: &mut Acr122Usb, card: &CardInfo) {
    let block = classic_block();
    reader
        .authenticate(block, classic_key_type(), classic_key())
        .expect("authenticate with NFC_FOUNDRY_HARDWARE_KEY");
    let data = reader.read_block(block).expect("read the classic block");
    println!("classic uid={} block {block}: {data:02X?}", card.uid_hex());
}

fn read_type2(reader: &mut Acr122Usb, card: &CardInfo) {
    reader.set_raw_mode(true).expect("enter raw mode");
    let mut command = vec![0x30, 0x04];
    command.extend_from_slice(&crc_a(&command));
    let pages = reader
        .transceive_bytes(&command)
        .expect("read Type 2 pages 4..7");
    assert!(
        pages.len() >= 16,
        "Type 2 READ returned {} bytes, expected at least 16",
        pages.len()
    );
    println!(
        "type 2 uid={} pages 4..7: {:02X?}",
        card.uid_hex(),
        &pages[..16]
    );
    reader.set_raw_mode(false).expect("leave raw mode");
}

#[test]
#[ignore = "requires an ACR122U and a MIFARE Classic or Type 2 card"]
fn reads_the_selected_card() {
    let _guard = lock_hardware();
    let conn_string = device_connstring();
    let mut reader = open_reader(&conn_string);
    let card = selected_card(&mut reader);
    match card.card_type {
        CardType::MifareClassic1K | CardType::MifareClassic4K => {
            read_classic(&mut reader, &card);
        }
        CardType::Type2 => read_type2(&mut reader, &card),
        other => panic!("expected a MIFARE Classic or Type 2 card, selected {other:?}"),
    }
    reader.close().expect("close the ACR122U");
}

#[test]
#[ignore = "requires an ACR122U and an ISO/IEC 14443A card"]
fn raw_bits_receive_atqa() {
    let _guard = lock_hardware();
    let conn_string = device_connstring();
    let mut reader = open_reader(&conn_string);
    let _card = selected_card(&mut reader);
    reader.cycle_rf_field().expect("cycle the RF field");
    reader.set_raw_mode(true).expect("enter raw mode");

    let (atqa, bits) = reader
        .transceive_bits(&[0x26], 7)
        .expect("send the seven-bit REQA");
    assert!(
        bits >= 16,
        "REQA response has {bits} bits, expected at least 16"
    );
    assert!(
        atqa.len() >= 2,
        "REQA response has {} bytes, expected at least 2",
        atqa.len()
    );
    assert!(
        atqa[..2].iter().any(|byte| *byte != 0),
        "REQA response ATQA is zero"
    );
    println!("raw REQA ATQA: {:02X?} ({bits} bits)", &atqa[..2]);
    reader.set_raw_mode(false).expect("leave raw mode");
    reader.close().expect("close the ACR122U");
}

#[cfg(feature = "hardware-write")]
fn planned_uid(original: &[u8; 16]) -> [u8; 4] {
    match env::var("NFC_FOUNDRY_TEST_UID") {
        Ok(value) => {
            let compact: String = value
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .collect();
            assert_eq!(
                compact.len(),
                8,
                "NFC_FOUNDRY_TEST_UID must contain eight hexadecimal digits"
            );
            let mut uid = [0u8; 4];
            for (index, slot) in uid.iter_mut().enumerate() {
                *slot = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16)
                    .expect("NFC_FOUNDRY_TEST_UID must be hexadecimal");
            }
            uid
        }
        Err(_) => [original[0], original[1], original[2], original[3]],
    }
}

#[cfg(feature = "hardware-write")]
fn manufacturer_block_with_uid(original: &[u8; 16], uid: [u8; 4]) -> [u8; 16] {
    let mut block = *original;
    block[0..4].copy_from_slice(&uid);
    block[4] = uid[0] ^ uid[1] ^ uid[2] ^ uid[3];
    block
}

#[cfg(feature = "hardware-write")]
fn read_manufacturer_block(reader: &mut Acr122Usb) -> ([u8; 16], CardInfo) {
    let card = selected_card(reader);
    reader
        .authenticate(0, classic_key_type(), sector_zero_key())
        .expect("authenticate sector 0 with NFC_FOUNDRY_HARDWARE_SECTOR0_KEY");
    let block = reader.read_block(0).expect("read manufacturer block 0");
    (block, card)
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires an owned ACR122U Gen1A magic card and explicit confirmation"]
fn writes_and_restores_a_gen1a_manufacturer_block() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=I_OWN_THIS_CARD to allow a write"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM=I_OWN_THIS_CARD to allow a Gen1A UID write"
    );
    let _guard = lock_hardware();
    let conn_string = device_connstring();
    let mut reader = open_reader(&conn_string);

    let (original, card) = read_manufacturer_block(&mut reader);
    assert_eq!(
        card.card_type,
        CardType::MifareClassic1K,
        "the Gen1A path requires a MIFARE Classic 1K card"
    );
    assert_eq!(card.uid.len(), 4, "the Gen1A path requires a four-byte UID");

    let uid = planned_uid(&original);
    let planned = manufacturer_block_with_uid(&original, uid);
    let changing_uid = planned != original;
    println!(
        "Gen1A original uid={} block0={original:02X?}",
        card.uid_hex()
    );

    reader
        .write_manufacturer_block(planned)
        .expect("Gen1A manufacturer block write");
    let (read_back, new_card) = read_manufacturer_block(&mut reader);
    assert_eq!(read_back, planned, "manufacturer block 0 did not verify");
    assert_eq!(
        new_card.uid,
        planned[0..4].to_vec(),
        "card selection did not return the planned UID"
    );
    println!(
        "Gen1A write verified: uid={} block0={read_back:02X?}",
        new_card.uid_hex()
    );

    if changing_uid {
        reader
            .write_manufacturer_block(original)
            .expect("restore the original manufacturer block");
        let (restored, restored_card) = read_manufacturer_block(&mut reader);
        assert_eq!(restored, original, "the original block 0 was not restored");
        assert_eq!(
            restored_card.uid, card.uid,
            "the original UID was not restored"
        );
        println!(
            "Gen1A restore verified: uid={} block0={restored:02X?}",
            restored_card.uid_hex()
        );
    }
    reader.close().expect("close the ACR122U");
}

#[test]
#[ignore = "requires a reader and a MIFARE Classic card"]
fn recovery_parity_probe_reads_a_tag_nonce() {
    let _guard = lock_hardware();
    let conn_string = device_connstring();
    let mut reader = open_reader(&conn_string);
    let card = selected_card(&mut reader);
    assert!(
        matches!(
            card.card_type,
            CardType::MifareClassic1K | CardType::MifareClassic4K
        ),
        "place a MIFARE Classic card on the reader"
    );
    assert!(
        reader.recovery_capabilities().parity_exchange,
        "the reader did not advertise parity exchange"
    );
    reader.set_recovery_mode(true).expect("enter recovery mode");
    let mut command = vec![0x60, classic_block()];
    let crc = crc_a(&command);
    command.extend_from_slice(&crc);
    let parity: Vec<u8> = command
        .iter()
        .map(|byte| !(byte.count_ones() as u8 & 1) & 1)
        .collect();
    let response = reader
        .transceive_parity(&command, &parity)
        .expect("partial authentication");
    println!(
        "parity probe card={} response bits={} data={:02X?} parity={:?}",
        card.uid_hex(),
        response.bits,
        response.data,
        response.parity
    );
    assert_eq!(response.data.len(), 4, "partial auth must return one nonce");
    assert_eq!(response.bits, 32);
    reader
        .set_recovery_mode(false)
        .expect("leave recovery mode");
    reader.close().expect("close the reader");
}
