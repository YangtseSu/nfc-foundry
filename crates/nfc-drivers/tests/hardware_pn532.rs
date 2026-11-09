//! Environment-gated PN532 UART hardware checks.
//!
//! Read-only:
//!
//! ```sh
//! NFCX_HARDWARE=1 \
//! NFCX_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfc-drivers --features hardware -- --ignored
//! ```
//!
//! The same-value write additionally requires the `hardware-write` feature and
//! `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Mutex, MutexGuard};

use nfc_core::{CardInfo, CardType, Key, KeyType, NfcError};
use nfc_drivers::pn532_uart::Pn532Uart;
use nfc_transport::{DeviceInfo, Reader};

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

fn lock_hardware() -> MutexGuard<'static, ()> {
    HARDWARE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn device_connstring() -> String {
    assert_eq!(
        env::var("NFCX_HARDWARE").as_deref(),
        Ok("1"),
        "set NFCX_HARDWARE=1 to run hardware tests"
    );
    env::var("NFCX_DEVICE")
        .expect("set NFCX_DEVICE to the exact pn532_uart:<serial-port> connstring")
}

fn open_reader(conn_string: &str) -> Pn532Uart {
    let mut reader = Pn532Uart::new(DeviceInfo::new("PN532 UART", conn_string));
    reader.open(conn_string).expect("open the PN532");
    let firmware = reader.firmware().expect("firmware after open");
    println!(
        "PN532 ic={:02X} version={}.{} support={:02X}",
        firmware.ic, firmware.version, firmware.revision, firmware.support
    );
    reader
}

fn selected_card(reader: &mut Pn532Uart) -> CardInfo {
    match reader.card_info() {
        Ok(card) => card,
        Err(NfcError::NoCard) => {
            panic!("place a MIFARE Classic or Type 2 card on the reader and rerun")
        }
        Err(error) => panic!("card selection failed: {error}"),
    }
}

fn classic_block() -> u8 {
    env::var("NFCX_HARDWARE_BLOCK")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(4)
}

fn classic_key() -> Key {
    let value = env::var("NFCX_HARDWARE_KEY").unwrap_or_else(|_| "FFFFFFFFFFFF".to_owned());
    Key::from_hex(&value).expect("NFCX_HARDWARE_KEY must be 12 hex digits")
}

fn classic_key_type() -> KeyType {
    match env::var("NFCX_HARDWARE_KEY_TYPE").as_deref() {
        Ok("B" | "b") => KeyType::B,
        _ => KeyType::A,
    }
}

fn is_ordinary_data_block(card_type: CardType, block: u8) -> bool {
    if block == 0 {
        return false;
    }
    match card_type {
        CardType::MifareClassic1K => block % 4 != 3,
        CardType::MifareClassic4K => {
            if block < 128 {
                block % 4 != 3
            } else {
                (block - 128) % 16 != 15
            }
        }
        _ => false,
    }
}

fn read_classic(reader: &mut Pn532Uart, card: &CardInfo) {
    let block = classic_block();
    reader
        .authenticate(block, classic_key_type(), classic_key())
        .expect("authenticate with NFCX_HARDWARE_KEY");
    let data = reader.read_block(block).expect("read the classic block");
    println!("classic uid={} block {block}: {data:02X?}", card.uid_hex());
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

fn read_type2(reader: &mut Pn532Uart, card: &CardInfo) {
    reader.set_raw_mode(true).expect("enter raw mode");
    let mut command = vec![0x30, 0x04];
    let crc = crc_a(&command);
    command.extend_from_slice(&crc);
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
#[ignore = "requires a PN532 and a MIFARE Classic or Type 2 card"]
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
    reader.close().expect("close the PN532");
}

#[cfg(feature = "hardware-write")]
#[test]
#[ignore = "requires an owned MIFARE Classic card and NFCX_HARDWARE_WRITE"]
fn writes_a_classic_block_with_the_same_value() {
    assert_eq!(
        env::var("NFCX_HARDWARE_WRITE").as_deref(),
        Ok("I_OWN_THIS_CARD"),
        "set NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD to allow a write"
    );
    let _guard = lock_hardware();
    let conn_string = device_connstring();
    let mut reader = open_reader(&conn_string);
    let card = selected_card(&mut reader);
    assert!(
        matches!(
            card.card_type,
            CardType::MifareClassic1K | CardType::MifareClassic4K
        ),
        "expected a MIFARE Classic card, selected {:?}",
        card.card_type
    );
    let block = classic_block();
    assert!(
        is_ordinary_data_block(card.card_type, block),
        "NFCX_HARDWARE_BLOCK={block} is block 0 or a sector trailer; choose a data block"
    );
    reader
        .authenticate(block, classic_key_type(), classic_key())
        .expect("authenticate with NFCX_HARDWARE_KEY");
    let original = reader.read_block(block).expect("read original block");
    reader
        .write_block(block, original)
        .expect("write the original value");
    let read_back = reader.read_block(block).expect("read back the block");
    assert_eq!(read_back, original, "same-value write did not verify");
    println!(
        "classic uid={} block {block} same-value write verified: {original:02X?}",
        card.uid_hex()
    );
    reader.close().expect("close the PN532");
}
