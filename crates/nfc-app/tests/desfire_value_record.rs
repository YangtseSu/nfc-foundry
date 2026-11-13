//! DESFire value and record mutations on the scripted card.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::desfire::{
    AuthKeyType, CommunicationMode, DesfireRecordOperation, DesfireRecordRequest,
    DesfireValueOperation, DesfireValueRequest, FileContents, FileType, record_text, value_text,
};
use nfc_core::NfcError;
use nfc_desfire::mock::{MockDesfire, MockDesfireTransport};

const AID: [u8; 3] = [0xF0, 0x0D, 0x01];
const VALUE_FILE: u8 = 0x01;
const LINEAR_FILE: u8 = 0x02;
const CYCLIC_FILE: u8 = 0x03;
const TMAC_FILE: u8 = 0x0F;
const UID: [u8; 7] = [0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26];

fn service(tag: MockDesfire) -> (Service, Arc<MockDesfireTransport>) {
    let transport = Arc::new(MockDesfireTransport::with_tag(tag));
    let mut service = Service::with_transport(transport.clone());
    service.refresh_devices().expect("refresh devices");
    service
        .select_device("mock:desfire")
        .expect("select the mock DESFire card");
    (service, transport)
}

fn command_count(transport: &MockDesfireTransport, ins: u8) -> usize {
    transport
        .commands()
        .iter()
        .filter(|apdu| apdu.get(1) == Some(&ins))
        .count()
}

/// Value file, plain, key 0 for every right, limits -50..50, limited credit
/// and free GetValue enabled.
fn value_settings(flags: u8, access: u16) -> Vec<u8> {
    let mut settings = vec![0x02, 0x00];
    settings.extend_from_slice(&access.to_le_bytes());
    settings.extend_from_slice(&(-50i32).to_le_bytes());
    settings.extend_from_slice(&50i32.to_le_bytes());
    settings.extend_from_slice(&0i32.to_le_bytes());
    settings.push(flags);
    settings
}

fn record_settings(
    file_type: u8,
    current_records: u32,
    record_size: u32,
    max_records: u32,
) -> Vec<u8> {
    let mut settings = vec![file_type, 0x00, 0x00, 0x00];
    settings.extend_from_slice(&record_size.to_le_bytes()[..3]);
    settings.extend_from_slice(&max_records.to_le_bytes()[..3]);
    settings.extend_from_slice(&current_records.to_le_bytes()[..3]);
    settings
}

/// Transaction MAC file, plain, read free, write denied, change key 0.
fn tmac_settings() -> Vec<u8> {
    vec![0x05, 0x00, 0x0F, 0xEF, 0x02, 0x00]
}

fn tmac_file(counter: u32, value: [u8; 8]) -> Vec<u8> {
    let mut data = counter.to_le_bytes().to_vec();
    data.extend_from_slice(&value);
    data
}

/// TMI of GetValue followed by a value command for the same file.
fn value_tmi(file_id: u8, old_value: i32, instruction: u8, amount: u32) -> Vec<u8> {
    let mut tmi = vec![0x6C, file_id];
    tmi.extend_from_slice(&old_value.to_le_bytes());
    tmi.resize(16, 0);
    tmi.push(instruction);
    tmi.push(file_id);
    tmi.extend_from_slice(&amount.to_le_bytes());
    tmi.resize(32, 0);
    tmi
}

#[test]
fn credit_free_value_commits_and_verifies() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_value(
        VALUE_FILE,
        value_settings(0x03, 0xEEEE),
        20,
    );
    let (mut service, transport) = service(tag);
    let result = service
        .desfire_value_change(DesfireValueRequest::new(
            AID,
            VALUE_FILE,
            DesfireValueOperation::Credit,
            5,
        ))
        .expect("credit the value file");

    assert_eq!(result.old_value, 20);
    assert_eq!(result.new_value, 25);
    assert_eq!(result.communication, CommunicationMode::Plain);
    assert!(result.committed);
    assert!(result.transaction_mac.is_none());
    assert_eq!(command_count(&transport, 0x0C), 1);
    assert_eq!(command_count(&transport, 0xC7), 1);
    let text = value_text(&result);
    assert!(text.contains("value: 20 → 25 (credit 5)"), "{text}");
    assert!(text.contains("transaction MAC: not present"), "{text}");
}

#[test]
fn debit_below_the_lower_limit_is_refused_before_the_command() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_value(
        VALUE_FILE,
        value_settings(0x03, 0x0000),
        0,
    );
    let (mut service, transport) = service(tag);
    let error = service
        .desfire_value_change(
            DesfireValueRequest::new(AID, VALUE_FILE, DesfireValueOperation::Debit, 60)
                .key(vec![0u8; 8]),
        )
        .expect_err("the debit is below the lower limit");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0xDC), 0);
    assert_eq!(command_count(&transport, 0xC7), 0);
}

#[test]
fn credit_above_the_upper_limit_is_refused_before_the_command() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_value(
        VALUE_FILE,
        value_settings(0x03, 0x0000),
        40,
    );
    let (mut service, transport) = service(tag);
    let error = service
        .desfire_value_change(
            DesfireValueRequest::new(AID, VALUE_FILE, DesfireValueOperation::Credit, 20)
                .key(vec![0u8; 8]),
        )
        .expect_err("the credit is above the upper limit");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0x0C), 0);
}

#[test]
fn limited_credit_requires_the_file_flag() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_value(
        VALUE_FILE,
        value_settings(0x02, 0x0000),
        0,
    );
    let (mut service, transport) = service(tag);
    let error = service
        .desfire_value_change(
            DesfireValueRequest::new(AID, VALUE_FILE, DesfireValueOperation::LimitedCredit, 5)
                .key(vec![0u8; 8]),
        )
        .expect_err("LimitedCredit is disabled");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0x1C), 0);
}

#[test]
fn record_append_commits_and_verifies_the_new_record() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        LINEAR_FILE,
        record_settings(0x03, 1, 8, 4),
        vec![1u8; 8],
    );
    let (mut service, transport) = service(tag);
    let payload = vec![0xAAu8; 8];
    let result = service
        .desfire_record_change(
            DesfireRecordRequest::new(AID, LINEAR_FILE, DesfireRecordOperation::Append)
                .key(vec![0u8; 8])
                .data(payload.clone()),
        )
        .expect("append a record");

    assert_eq!(result.before, vec![vec![1u8; 8]]);
    assert_eq!(result.after.len(), 2);
    assert_eq!(result.after[1], payload);
    assert_eq!(result.communication, CommunicationMode::Plain);
    assert_eq!(command_count(&transport, 0x3B), 1);
    assert_eq!(command_count(&transport, 0xC7), 1);
    let text = record_text(&result);
    assert!(text.contains("operation: append · wrote 8 bytes"), "{text}");
    assert!(text.contains("read-back: every record verified"), "{text}");
}

#[test]
fn a_full_linear_record_file_is_refused_before_the_write() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        LINEAR_FILE,
        record_settings(0x03, 2, 8, 2),
        vec![1u8; 16],
    );
    let (mut service, transport) = service(tag);
    let error = service
        .desfire_record_change(
            DesfireRecordRequest::new(AID, LINEAR_FILE, DesfireRecordOperation::Append)
                .key(vec![0u8; 8])
                .data(vec![0xAA; 8]),
        )
        .expect_err("the linear file is full");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0x3B), 0);
}

#[test]
fn cyclic_append_overwrites_the_oldest_record() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        CYCLIC_FILE,
        record_settings(0x04, 2, 4, 2),
        [vec![1u8; 4], vec![2u8; 4]].concat(),
    );
    let (mut service, _transport) = service(tag);
    let result = service
        .desfire_record_change(
            DesfireRecordRequest::new(AID, CYCLIC_FILE, DesfireRecordOperation::Append)
                .key(vec![0u8; 8])
                .data(vec![3u8; 4]),
        )
        .expect("append a cyclic record");
    assert_eq!(result.before, vec![vec![1u8; 4], vec![2u8; 4]]);
    assert_eq!(result.after, vec![vec![2u8; 4], vec![3u8; 4]]);
}

#[test]
fn record_update_replaces_a_range_of_the_newest_record() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        LINEAR_FILE,
        record_settings(0x03, 2, 8, 4),
        [vec![1u8; 8], vec![2u8; 8]].concat(),
    );
    let (mut service, transport) = service(tag);
    let result = service
        .desfire_record_change(
            DesfireRecordRequest::new(
                AID,
                LINEAR_FILE,
                DesfireRecordOperation::Update {
                    record_number: 0,
                    offset: 2,
                },
            )
            .key(vec![0u8; 8])
            .data(vec![0xAA, 0xBB, 0xCC]),
        )
        .expect("update the newest record");
    let mut expected = vec![2u8; 8];
    expected[2..5].copy_from_slice(&[0xAA, 0xBB, 0xCC]);
    assert_eq!(result.after, vec![vec![1u8; 8], expected]);
    assert_eq!(command_count(&transport, 0xDB), 1);
}

#[test]
fn record_update_outside_the_record_is_refused() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        LINEAR_FILE,
        record_settings(0x03, 1, 8, 4),
        vec![1u8; 8],
    );
    let (mut service, transport) = service(tag);
    let error = service
        .desfire_record_change(
            DesfireRecordRequest::new(
                AID,
                LINEAR_FILE,
                DesfireRecordOperation::Update {
                    record_number: 0,
                    offset: 6,
                },
            )
            .key(vec![0u8; 8])
            .data(vec![0xAA, 0xBB, 0xCC]),
        )
        .expect_err("the update crosses the record boundary");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0xDB), 0);
}

#[test]
fn record_clear_commits_and_reads_back_empty() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        LINEAR_FILE,
        record_settings(0x03, 2, 8, 4),
        vec![1u8; 16],
    );
    let (mut service, transport) = service(tag);
    let result = service
        .desfire_record_change(
            DesfireRecordRequest::new(AID, LINEAR_FILE, DesfireRecordOperation::Clear)
                .key(vec![0u8; 8]),
        )
        .expect("clear the record file");
    assert!(result.after.is_empty());
    assert_eq!(
        result.settings_after.contents,
        FileContents::Record {
            record_size: 8,
            max_records: 4,
            current_records: 0,
        }
    );
    assert_eq!(command_count(&transport, 0xEB), 1);
    assert_eq!(command_count(&transport, 0xC7), 1);
}

#[test]
fn transaction_mac_is_verified_and_cross_checked() {
    let tmac_key = [0x11u8; 16];
    let tmi = value_tmi(VALUE_FILE, 20, 0x0C, 5);
    let tmv = nfc_desfire::transaction_mac(&tmac_key, 1, &UID, &tmi).expect("TMV");
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des)
        .with_value(VALUE_FILE, value_settings(0x03, 0x0000), 20)
        .with_file(TMAC_FILE, tmac_settings(), tmac_file(0, [0u8; 8]))
        .with_commit_mac(1, tmv);
    let (mut service, _transport) = service(tag);
    let result = service
        .desfire_value_change(
            DesfireValueRequest::new(AID, VALUE_FILE, DesfireValueOperation::Credit, 5)
                .key(vec![0u8; 8])
                .tmac_key(tmac_key),
        )
        .expect("credit with a verified Transaction MAC");

    let tmac = result.transaction_mac.expect("Transaction MAC");
    assert!(tmac.verified, "{}", tmac.detail);
    assert_eq!(tmac.counter, 1);
    assert_eq!(tmac.value, tmv);
    assert_eq!(
        result.settings.file_type,
        FileType::Value,
        "settings stay value"
    );
}

#[test]
fn transaction_mac_with_a_wrong_key_is_not_accepted() {
    let tmac_key = [0x11u8; 16];
    let tmi = value_tmi(VALUE_FILE, 20, 0x0C, 5);
    let tmv = nfc_desfire::transaction_mac(&tmac_key, 1, &UID, &tmi).expect("TMV");
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des)
        .with_value(VALUE_FILE, value_settings(0x03, 0x0000), 20)
        .with_file(TMAC_FILE, tmac_settings(), tmac_file(0, [0u8; 8]))
        .with_commit_mac(1, tmv);
    let (mut service, _transport) = service(tag);
    let error = service
        .desfire_value_change(
            DesfireValueRequest::new(AID, VALUE_FILE, DesfireValueOperation::Credit, 5)
                .key(vec![0u8; 8])
                .tmac_key([0x22u8; 16]),
        )
        .expect_err("the Transaction MAC key is wrong");
    assert!(matches!(error, NfcError::VerificationFailed(_)), "{error}");
}

#[test]
fn transaction_mac_without_the_key_is_reported_unverified() {
    let tmi = value_tmi(VALUE_FILE, 20, 0x0C, 5);
    let tmv = nfc_desfire::transaction_mac(&[0x11u8; 16], 1, &UID, &tmi).expect("TMV");
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des)
        .with_value(VALUE_FILE, value_settings(0x03, 0x0000), 20)
        .with_file(TMAC_FILE, tmac_settings(), tmac_file(0, [0u8; 8]))
        .with_commit_mac(1, tmv);
    let (mut service, _transport) = service(tag);
    let result = service
        .desfire_value_change(
            DesfireValueRequest::new(AID, VALUE_FILE, DesfireValueOperation::Credit, 5)
                .key(vec![0u8; 8]),
        )
        .expect("credit without the Transaction MAC key");
    let tmac = result.transaction_mac.as_ref().expect("Transaction MAC");
    assert!(!tmac.verified);
    assert!(tmac.detail.contains("no AppTransactionMACKey"));
    let text = value_text(&result);
    assert!(text.contains("UNVERIFIED"), "{text}");
}
