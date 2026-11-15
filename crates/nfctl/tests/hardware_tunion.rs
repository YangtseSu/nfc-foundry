//! Environment-gated China T-Union read-only check.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware --test hardware_tunion -- --ignored
//! ```
//!
//! The test never writes to the card. It prints counts only; the card number,
//! terminal IDs, and balance never appear in the test output.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_app::Service;
use nfc_core::CancelToken;
use nfc_drivers::DriverRegistry;

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
        .expect("set NFC_FOUNDRY_DEVICE to the exact pn532_uart:<serial-port> connstring")
}

fn service(conn_string: &str) -> Service {
    let mut service = Service::with_transport(Arc::new(DriverRegistry));
    service.refresh_devices().expect("enumerate readers");
    service
        .select_device(conn_string)
        .expect("select the reader");
    service
}

#[test]
#[ignore = "requires a China T-Union card"]
fn reads_a_t_union_card() {
    let _guard = lock_hardware();
    let mut service = service(&conn_string());
    let result = service
        .tunion_read(CancelToken::new())
        .expect("T-Union read");

    assert_eq!(result.card.card_type, nfc_core::CardType::Iso14443);
    assert_eq!(result.data.aid, nfc_pboc::tunion::TUNION_AID);
    assert!(result.data.complete, "warnings: {:?}", result.data.warnings);
    assert!(!result.data.transactions.is_empty());
    assert!(!result.data.travels.is_empty());
    let masked = result.data.masked_card_number();
    assert!(masked.matches('*').count() >= 4);
    assert_ne!(masked, result.data.card_number);

    println!(
        "T-Union read: balance present, {} transactions, {} travel records",
        result.data.transactions.len(),
        result.data.travels.len()
    );
}
