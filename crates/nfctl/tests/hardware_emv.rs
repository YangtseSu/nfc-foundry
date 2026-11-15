//! Environment-gated EMV payment-card public-data check.
//!
//! ```sh
//! NFC_FOUNDRY_HARDWARE=1 \
//! NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
//! cargo test -p nfctl --features hardware --test hardware_emv -- --ignored
//! ```
//!
//! The test never sends GENERATE AC or a write command. It prints only the
//! application label, masked PAN, expiry, and record count.

#![cfg(feature = "hardware")]

use std::env;
use std::sync::{Arc, Mutex, MutexGuard};

use nfc_app::Service;
use nfc_core::{CancelToken, CardType};
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
    env::var("NFC_FOUNDRY_DEVICE").expect("set NFC_FOUNDRY_DEVICE to the exact reader connstring")
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
#[ignore = "requires an EMV payment card"]
fn reads_an_emv_payment_card() {
    let _guard = lock_hardware();
    let mut service = service(&conn_string());
    let card = service.card_info().expect("select the card");
    assert_eq!(
        card.card_type,
        CardType::Iso14443,
        "place an EMV payment card on the reader; selected {:?}",
        card.card_type
    );
    let result = service
        .emv_read(CancelToken::new())
        .expect("read the payment card");
    let data = &result.data;

    assert_eq!(result.card.card_type, CardType::Iso14443);
    assert!(!data.aid.is_empty(), "no payment AID");
    assert!(!data.label.is_empty(), "no application label");
    assert!(data.complete, "the read is incomplete: {:?}", data.warnings);
    assert!(
        data.warnings.is_empty(),
        "unexpected warnings: {:?}",
        data.warnings
    );
    let pan = data.pan_masked.as_deref().expect("masked PAN");
    assert!(pan.matches('*').count() >= 4, "PAN is not masked: {pan}");
    assert!(pan.len() >= 8, "PAN is too short: {pan}");
    assert!(data.expires.is_some(), "no expiry");
    assert!(!data.applications.is_empty(), "no PPSE applications");

    println!(
        "EMV read: label={} masked={} expires={} records={} atc={}",
        data.label,
        pan,
        data.expires
            .map(|value| value.to_string())
            .unwrap_or_default(),
        data.records_read,
        data.atc
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_owned())
    );
}
