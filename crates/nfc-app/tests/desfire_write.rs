//! DESFire write workflows on the scripted card.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::desfire::{AuthKeyType, CommunicationMode, DesfireWriteRequest, write_text};
use nfc_core::{CancelToken, NfcError};
use nfc_desfire::mock::{MockDesfire, MockDesfireTransport};

const AID: [u8; 3] = [0xF0, 0x0D, 0x01];
const FILE: u8 = 0x01;

/// Standard file, 32 bytes, read and write free.
const STANDARD_FREE: [u8; 7] = [0x00, 0x00, 0xEE, 0xEE, 0x20, 0x00, 0x00];
/// Standard file, 32 bytes, read and write with key 0.
const STANDARD_KEYED: [u8; 7] = [0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00];
/// Standard file, 32 bytes, read free, write and read/write denied.
const STANDARD_DENIED_WRITE: [u8; 7] = [0x00, 0x00, 0xFE, 0xEF, 0x20, 0x00, 0x00];
/// Standard file, 32 bytes, write free, read and read/write denied.
const STANDARD_WRITE_ONLY: [u8; 7] = [0x00, 0x00, 0xFE, 0xFE, 0x20, 0x00, 0x00];
/// Backup file, 32 bytes, read and write free.
const BACKUP_FREE: [u8; 7] = [0x01, 0x00, 0xEE, 0xEE, 0x20, 0x00, 0x00];

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

fn free_file(settings: [u8; 7]) -> MockDesfire {
    MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(FILE, settings.to_vec(), vec![0u8; 32])
}

#[test]
fn write_free_standard_file_exports_and_verifies() {
    let (mut service, transport) = service(free_file(STANDARD_FREE));
    let payload = [0x11u8; 8];
    let result = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, payload.to_vec()).offset(4),
        )
        .expect("write the free file");

    assert_eq!(result.communication, CommunicationMode::Plain);
    assert_eq!(result.key_type, None);
    assert_eq!(result.offset, 4);
    assert_eq!(result.written, payload.len());
    assert_eq!(result.backup.len(), 32);
    assert!(!result.committed, "Standard files have no transaction");
    assert_eq!(result.read_back, payload);
    let text = write_text(&result);
    assert!(text.contains("written: 4..12 (8 bytes)"), "{text}");
    assert!(text.contains("commit: not needed"), "{text}");

    assert_eq!(command_count(&transport, 0x3D), 1, "one WriteData");
    assert_eq!(command_count(&transport, 0xC7), 0, "no commit");
    assert_eq!(
        transport.tag().file_data(FILE).expect("file data")[4..12],
        payload
    );
}

#[test]
fn write_keyed_file_authenticates_before_writing() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        FILE,
        STANDARD_KEYED.to_vec(),
        vec![0u8; 32],
    );
    let (mut service, transport) = service(tag);
    let result = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, vec![0xA5; 4]).key(vec![0u8; 8]),
        )
        .expect("write the keyed file");

    assert_eq!(result.key_type, Some(AuthKeyType::Des));
    assert_eq!(result.communication, CommunicationMode::Plain);
    assert_eq!(command_count(&transport, 0x0A), 1);
    assert_eq!(command_count(&transport, 0x3D), 1);
}

#[test]
fn wrong_key_fails_before_the_write() {
    let tag = MockDesfire::new([0u8; 8], AuthKeyType::Des).with_file(
        FILE,
        STANDARD_KEYED.to_vec(),
        vec![0u8; 32],
    );
    let (mut service, transport) = service(tag);
    let error = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, vec![0xA5; 4]).key(vec![0xFF; 8]),
        )
        .expect_err("the key is wrong");
    assert!(matches!(error, NfcError::VerificationFailed(_)), "{error}");
    assert_eq!(command_count(&transport, 0x3D), 0);
}

#[test]
fn denied_write_is_refused_before_any_command() {
    let (mut service, transport) = service(free_file(STANDARD_DENIED_WRITE));
    let error = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, vec![0xA5; 4]),
        )
        .expect_err("write is denied");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0x3D), 0);
}

#[test]
fn out_of_range_write_is_refused_before_the_backup() {
    let (mut service, transport) = service(free_file(STANDARD_FREE));
    let error = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, vec![0xA5; 8]).offset(28),
        )
        .expect_err("the range exceeds the file");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0xBD), 0, "no backup read");
    assert_eq!(command_count(&transport, 0x3D), 0);
}

#[test]
fn write_only_file_is_refused_because_it_cannot_be_verified() {
    let (mut service, transport) = service(free_file(STANDARD_WRITE_ONLY));
    let error = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, vec![0xA5; 4]),
        )
        .expect_err("the file cannot be read back");
    assert!(matches!(error, NfcError::InvalidArgument(_)), "{error}");
    assert_eq!(command_count(&transport, 0x3D), 0);
}

#[test]
fn backup_file_commits_then_reads_back() {
    let (mut service, transport) = service(free_file(BACKUP_FREE));
    let payload = [0x5Au8; 6];
    let result = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, payload.to_vec()).offset(10),
        )
        .expect("write the backup file");

    assert!(result.committed);
    assert_eq!(result.read_back, payload);
    assert_eq!(command_count(&transport, 0xC7), 1);
    assert_eq!(
        command_count(&transport, 0xBD),
        2,
        "backup and post-commit read"
    );
}

#[test]
fn failed_commit_is_reported_and_never_reported_as_committed() {
    let (mut service, transport) = service(free_file(BACKUP_FREE).with_commit_failure());
    let error = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, vec![0x5A; 6]),
        )
        .expect_err("the card refuses the commit");
    assert!(matches!(error, NfcError::VerificationFailed(_)), "{error}");
    assert_eq!(command_count(&transport, 0xC7), 1);
}

#[test]
fn read_back_mismatch_after_commit_is_reported() {
    let (mut service, transport) = service(free_file(BACKUP_FREE).with_corrupt_readback());
    let error = service
        .desfire_write(
            CancelToken::new(),
            DesfireWriteRequest::new(AID, FILE, vec![0x5A; 6]),
        )
        .expect_err("the read-back differs");
    assert!(matches!(error, NfcError::VerificationFailed(_)), "{error}");
    assert_eq!(command_count(&transport, 0x3D), 1);
    assert_eq!(command_count(&transport, 0xC7), 1, "the commit ran");
    assert_eq!(
        command_count(&transport, 0xA7),
        0,
        "a committed write cannot abort"
    );
}
