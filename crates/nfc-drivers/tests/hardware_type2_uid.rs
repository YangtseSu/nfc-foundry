//! Environment-gated UID-page writability probe for a Type 2 magic clone.
//!
//! This writes the **current** page 0, 1, and 2 values back. It never changes
//! the UID; a genuine NTAG213 refuses the manufacturer pages, while a magic
//! clone accepts them. The card must remain selected with the same UID.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! NFC_FOUNDRY_HARDWARE_WRITE=yes \
//! NFC_FOUNDRY_TYPE2_UID_CONFIRM=yes \
//! cargo test -p nfc-drivers --features hardware-write \
//!   --test hardware_type2_uid -- --ignored --nocapture
//! ```

#![cfg(feature = "hardware-write")]

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_core::{PAGE_SIZE, Result};
use nfc_drivers::DriverRegistry;
use nfc_transport::{Reader, Transport};

static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

fn lock_hardware() -> MutexGuard<'static, ()> {
    HARDWARE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn conn_string() -> String {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE").as_deref(),
        Ok("1"),
        "set NFC_FOUNDRY_HARDWARE=1 to run hardware tests"
    );
    env::var("NFC_FOUNDRY_DEVICE")
        .expect("set NFC_FOUNDRY_DEVICE to the exact pn532_uart:<port> or acr122_usb:<bus>:<addr>")
}

fn open_reader() -> Box<dyn Reader> {
    let conn_string = conn_string();
    let registry = Arc::new(DriverRegistry);
    let mut reader = registry.open(&conn_string).expect("open reader");
    reader.open(&conn_string).expect("connect reader");
    reader
}

fn page_from_quad(quad: &[u8; nfc_core::PAGE_QUAD_SIZE], page: u8) -> [u8; PAGE_SIZE] {
    let index = usize::from(page) * PAGE_SIZE;
    let mut data = [0u8; PAGE_SIZE];
    data.copy_from_slice(&quad[index..index + PAGE_SIZE]);
    data
}

#[test]
#[ignore = "requires a Type 2 UID-changeable clone and both write gates"]
fn uid_pages_accept_same_value_writes() {
    assert_eq!(
        env::var("NFC_FOUNDRY_HARDWARE_WRITE").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_HARDWARE_WRITE=yes to allow a write"
    );
    assert_eq!(
        env::var("NFC_FOUNDRY_TYPE2_UID_CONFIRM").as_deref(),
        Ok("yes"),
        "set NFC_FOUNDRY_TYPE2_UID_CONFIRM=yes to allow the UID-page probe"
    );
    let _guard = lock_hardware();
    let mut reader = open_reader();
    let card = reader.card_info().expect("select the Type 2 tag");
    let uid = card.uid.clone();
    let first = reader.read_page_quad(0).expect("read pages 0..=3");
    let pages: [[u8; PAGE_SIZE]; 3] = [
        page_from_quad(&first, 0),
        page_from_quad(&first, 1),
        page_from_quad(&first, 2),
    ];

    for (page, data) in pages.into_iter().enumerate() {
        let page = page as u8;
        let written: Result<()> = reader.write_page(page, data);
        match written {
            Ok(()) => {
                let quad = reader.read_page_quad(0).expect("read back pages 0..=3");
                assert_eq!(
                    page_from_quad(&quad, page),
                    data,
                    "page {page} read back differs after a same-value write"
                );
                println!(
                    "page {page} accepted same-value write: {}",
                    data.iter()
                        .map(|byte| format!("{byte:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            Err(error) => {
                println!("page {page} refused the write: {error}");
                assert_eq!(
                    page, 0,
                    "a magic clone accepted page 0 but refused page {page}"
                );
            }
        }
    }

    let selected = reader.card_info().expect("reselect the Type 2 tag");
    assert_eq!(selected.uid, uid, "UID changed during a same-value probe");
    reader.close().expect("close reader");
}
