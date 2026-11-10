//! Type 4 workflow tests on the scripted transport.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::classic::BackupStore;
use nfc_app::type4::{Type4Clear, Type4NdefWrite};
use nfc_core::{CancelToken, CardInfo, CardType, NfcError};
use nfc_ndef::{NdefMessage, TextRecord};
use nfc_transport::DeviceInfo;
use nfc_transport::mock::{MockCall, MockFault, MockHandle, MockTransport, MockType4Tag};
use nfc_type4::NdefState;

fn card() -> CardInfo {
    CardInfo::new(
        vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        [0x00, 0x44],
        0x20,
        CardType::Iso14443,
    )
}

fn service() -> (Service, MockHandle) {
    let transport = MockTransport::new();
    let handle = transport.add(DeviceInfo::new("Mock Type 4", "mock:type4"), Some(card()));
    handle.set_type4_tag(MockType4Tag::ntag424());
    let mut service = Service::with_transport(Arc::new(transport));
    service.refresh_devices().expect("refresh");
    service
        .select_device("mock:type4")
        .expect("select mock reader");
    (service, handle)
}

fn message(text: &str) -> NdefMessage {
    NdefMessage::new(vec![
        TextRecord::new(text, "en")
            .expect("text")
            .to_record()
            .expect("record"),
    ])
    .expect("message")
}

fn backup_files(directory: &std::path::Path) -> usize {
    std::fs::read_dir(directory)
        .expect("read backup directory")
        .count()
}

#[test]
fn reads_the_type4_cc_and_ndef_file() {
    let (mut service, _handle) = service();
    let read = service.type4_read(CancelToken::new()).expect("read");
    assert_eq!(read.cc.cclen, 23);
    assert_eq!(read.cc.max_le, 0x0100);
    assert_eq!(read.ndef.max_size, 0x0100);
    assert_eq!(read.ndef.state, NdefState::Empty);
    assert!(read.writable());
}

#[test]
fn write_backs_up_before_mutating_and_reads_back() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let result = service
        .type4_ndef_write(Type4NdefWrite::new(message("hello"), &backup))
        .expect("write");
    assert_eq!(result.written, message("hello").encoded_len().expect("len"));
    assert_eq!(result.ndef.message(), Some(&message("hello")));
    assert_eq!(backup_files(directory.path()), 1);

    let ndef = handle.type4_ndef().expect("NDEF file");
    let nlen = usize::from(u16::from_be_bytes([ndef[0], ndef[1]]));
    assert_eq!(nlen, message("hello").encoded_len().expect("len"));
    assert_eq!(
        &ndef[2..2 + nlen],
        message("hello").encode().expect("encode")
    );

    let reread = service.type4_read(CancelToken::new()).expect("reread");
    assert_eq!(reread.ndef.message(), Some(&message("hello")));
}

#[test]
fn clear_requires_confirmation() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    service
        .type4_ndef_write(Type4NdefWrite::new(message("hello"), &backup))
        .expect("write");

    assert!(matches!(
        service.type4_clear(Type4Clear::new(&backup)),
        Err(NfcError::InvalidArgument(_))
    ));
    assert_eq!(
        handle.type4_ndef().expect("NDEF")[0..2],
        (message("hello").encoded_len().expect("len") as u16).to_be_bytes()
    );

    let result = service
        .type4_clear(Type4Clear::new(&backup).confirm(true))
        .expect("clear");
    assert_eq!(result.ndef.state, NdefState::Empty);
    assert_eq!(handle.type4_ndef().expect("NDEF")[0..2], [0x00, 0x00]);
}

#[test]
fn read_only_files_are_refused_without_a_backup() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let cc = vec![
        0x00, 0x0F, 0x20, 0x00, 0x3A, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00, 0xFF,
    ];
    handle.set_type4_tag(MockType4Tag::new(cc, vec![0u8; 256]).with_write_access(0xFF));

    assert!(matches!(
        service.type4_ndef_write(Type4NdefWrite::new(message("hello"), &backup)),
        Err(NfcError::Unsupported(_))
    ));
    assert_eq!(backup_files(directory.path()), 0);
}

#[test]
fn an_ignored_write_is_detected_after_a_backup() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    // Ignore NLEN=0 and the message update; the final NLEN is written but the
    // message bytes do not match.
    handle.fail_next(MockCall::ApduWrite, MockFault::WriteIgnored);
    handle.fail_next(MockCall::ApduWrite, MockFault::WriteIgnored);
    assert!(matches!(
        service.type4_ndef_write(Type4NdefWrite::new(message("hello"), &backup)),
        Err(NfcError::VerificationFailed(_))
    ));
    assert_eq!(backup_files(directory.path()), 1);
}

#[test]
fn dump_saves_a_project_backup() {
    let (mut service, _handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let record = service
        .type4_dump(&backup, CancelToken::new())
        .expect("dump");
    let bytes = std::fs::read(&record.path).expect("backup file");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(value["format"], "nfc-foundry-type4");
    assert_eq!(value["ndef"], "0000");
}

#[test]
fn cancellation_is_checked_before_io() {
    let (mut service, _handle) = service();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        service.type4_read(cancel),
        Err(NfcError::Cancelled)
    ));
}
