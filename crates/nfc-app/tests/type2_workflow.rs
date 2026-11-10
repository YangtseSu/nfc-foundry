//! Type 2 workflow tests on the scripted transport.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::classic::BackupStore;
use nfc_app::type2::{
    Type2Format, Type2Lock, Type2NdefWrite, Type2PageWrite, Type2Password, Type2Restore,
};
use nfc_core::{CancelToken, CardInfo, CardType, NfcError};
use nfc_ndef::{NdefMessage, TextRecord};
use nfc_transport::DeviceInfo;
use nfc_transport::mock::{MockCall, MockFault, MockHandle, MockTransport};
use nfc_type2::{NdefState, RestoreOptions, Type2Image, Type2Product, exchange};

fn card() -> CardInfo {
    CardInfo::new(
        vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
        [0x00, 0x44],
        0x00,
        CardType::Type2,
    )
}

fn ntag215_pages() -> Vec<(u8, [u8; 4])> {
    let mut pages = Vec::new();
    for page in 0..135u8 {
        pages.push((page, [0u8; 4]));
    }
    pages[0] = (0, [0x04, 0x5B, 0x1C, 0x2A]);
    pages[1] = (1, [0x3B, 0x4C, 0x80, 0x00]);
    pages[2] = (2, [0x00, 0x00, 0x00, 0x00]);
    pages[3] = (3, [0xE1, 0x10, 0x3E, 0x00]);
    pages[4] = (4, [0x03, 0x00, 0xFE, 0x00]);
    pages[130] = (130, [0x00, 0x00, 0x00, 0xBD]);
    pages[131] = (131, [0x00, 0x00, 0x00, 0xFF]);
    pages[132] = (132, [0x00, 0x00, 0x00, 0x00]);
    pages
}

fn service() -> (Service, MockHandle) {
    let transport = MockTransport::new();
    let handle = transport.add(
        DeviceInfo::new("Mock NTAG215", "mock:ntag215"),
        Some(card()),
    );
    for (page, data) in ntag215_pages() {
        handle.set_page(page, data);
    }
    let mut service = Service::with_transport(Arc::new(transport));
    service.refresh_devices().expect("refresh");
    service
        .select_device("mock:ntag215")
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

#[test]
fn reads_a_complete_ntag215_image() {
    let (mut service, _handle) = service();
    let result = service
        .type2_read(CancelToken::new())
        .expect("read NTAG215");
    assert_eq!(result.image.layout.product, Type2Product::Ntag215);
    assert_eq!(result.image.layout.total_pages, 135);
    assert!(result.image.is_complete(), "{:?}", result.warnings);
    assert!(result.warnings.is_empty());
    assert_eq!(result.image.cc.size, 496);
}

#[test]
fn page_write_backs_up_and_reads_back() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let result = service
        .type2_write_page(Type2PageWrite::new(8, [0xA5; 4], &backup))
        .expect("write page");
    assert!(result.changed);
    assert_eq!(handle.page(8), Some([0xA5; 4]));
    let record = result.backup.expect("backup");
    let loaded = exchange::load(&std::fs::read(record.path).expect("backup file")).expect("load");
    assert!(loaded.page(8).is_some());
    assert_ne!(loaded.page(8), Some(&[0xA5; 4]));
}

#[test]
fn an_ignored_page_write_is_reported() {
    let (mut service, handle) = service();
    handle.fail_next(MockCall::WritePage, MockFault::WriteIgnored);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let result = service.type2_write_page(Type2PageWrite::new(8, [0xA5; 4], &backup));
    assert!(matches!(result, Err(NfcError::VerificationFailed(_))));
}

#[test]
fn advanced_pages_require_confirmation() {
    let (mut service, _handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let result = service
        .type2_write_page(Type2PageWrite::new(3, [0xE1, 0x10, 0x3E, 0x00], &backup).advanced(true));
    assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
    let result = service
        .type2_write_page(
            Type2PageWrite::new(3, [0xE1, 0x10, 0x3E, 0x00], &backup)
                .advanced(true)
                .confirm(true),
        )
        .expect("confirmed CC write");
    assert!(!result.changed);
}

#[test]
fn ndef_write_read_and_format_round_trip() {
    let (mut service, _handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let written = service
        .type2_ndef_write(Type2NdefWrite::new(message("hello"), &backup))
        .expect("write NDEF");
    assert!(written.summary.written_pages > 0);
    let read = service
        .type2_ndef_read(CancelToken::new())
        .expect("read NDEF");
    assert_eq!(read.ndef.state, NdefState::Message(message("hello")));

    let unconfirmed = service.type2_ndef_format(Type2Format::new(&backup));
    assert!(matches!(unconfirmed, Err(NfcError::InvalidArgument(_))));
    let formatted = service
        .type2_ndef_format(Type2Format::new(&backup).confirm(true))
        .expect("format");
    assert!(formatted.summary.written_pages > 0);
    let read = service
        .type2_ndef_read(CancelToken::new())
        .expect("read NDEF");
    assert_eq!(read.ndef.state, NdefState::Empty);
}

#[test]
fn ndef_write_preserves_unknown_tlvs() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    handle.set_page(4, [0xFD, 0x02, 0xAA, 0xBB]);
    handle.set_page(5, [0x03, 0x00, 0xFE, 0x00]);
    service
        .type2_ndef_write(Type2NdefWrite::new(message("kept"), &backup))
        .expect("write NDEF");
    let read = service.type2_read(CancelToken::new()).expect("read");
    let area = read.image.ndef_area().expect("area");
    assert!(
        area.windows(4)
            .any(|window| window == [0xFD, 0x02, 0xAA, 0xBB]),
        "proprietary TLV was lost"
    );
}

#[test]
fn locks_are_one_way_and_confirmed() {
    let (mut service, _handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let unconfirmed = service.type2_lock(Type2Lock::new(vec![4], &backup));
    assert!(matches!(unconfirmed, Err(NfcError::InvalidArgument(_))));
    let result = service
        .type2_lock(Type2Lock::new(vec![4], &backup).confirm(true))
        .expect("lock page 4");
    assert!(result.update.static_changed);
    let image = service.type2_read(CancelToken::new()).expect("read").image;
    assert_eq!(image.page(2).expect("page 2")[2] & 0x10, 0x10);
    let denied = service.type2_write_page(Type2PageWrite::new(4, [0; 4], &backup));
    assert!(matches!(denied, Err(NfcError::Unsupported(_))));
}

#[test]
fn password_configuration_requires_confirmation_and_verifies() {
    let (mut service, handle) = service();
    handle.set_password([1, 2, 3, 4], [0xAA, 0x55]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let unconfirmed = service.type2_password(Type2Password::new(
        0x20,
        0x00,
        [1, 2, 3, 4],
        [0xAA, 0x55],
        &backup,
    ));
    assert!(matches!(unconfirmed, Err(NfcError::InvalidArgument(_))));
    let result = service
        .type2_password(
            Type2Password::new(0x20, 0x00, [1, 2, 3, 4], [0xAA, 0x55], &backup).confirm(true),
        )
        .expect("password configuration");
    assert_eq!(result.image.page(131).expect("cfg0")[3], 0x20);
}

#[test]
fn restore_requires_confirmation_and_checks_the_layout() {
    let (mut service, _handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let source = service.type2_read(CancelToken::new()).expect("read").image;
    let unconfirmed = service.type2_restore(Type2Restore::new(source.clone(), &backup));
    assert!(matches!(unconfirmed, Err(NfcError::InvalidArgument(_))));
    let result = service
        .type2_restore(Type2Restore::new(source.clone(), &backup).confirm(true))
        .expect("restore");
    assert_eq!(result.summary.written_pages, 0);
    assert_eq!(result.summary.unchanged_pages, 126);

    let mut wrong = Type2Image::new(
        source.layout,
        nfc_type2::CapabilityContainer::parse(&[0xE1, 0x10, 0x12, 0x00]).expect("CC"),
        source.uid.clone(),
    );
    wrong.set_page(3, [0xE1, 0x10, 0x12, 0x00]).expect("page 3");
    let error = service
        .type2_restore(
            Type2Restore::new(wrong, &backup)
                .options(RestoreOptions {
                    restore_user_memory: true,
                    ..RestoreOptions::default()
                })
                .confirm(true),
        )
        .expect_err("different CC must fail");
    assert!(matches!(error, NfcError::InvalidArgument(_)));
}

#[test]
fn partial_reads_stay_explicit() {
    let (mut service, handle) = service();
    handle.clear_pages();
    for (page, data) in ntag215_pages().into_iter().take(4) {
        handle.set_page(page, data);
    }
    let result = service
        .type2_read(CancelToken::new())
        .expect("partial read");
    assert_eq!(result.image.known_pages(), 4);
    assert!(!result.warnings.is_empty());
    let error = service
        .type2_ndef_read(CancelToken::new())
        .expect_err("NDEF area is incomplete");
    assert!(matches!(error, NfcError::InvalidArgument(_)));
}

#[test]
fn dump_and_restore_round_trip_through_json() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let record = service
        .type2_dump(&backup, CancelToken::new())
        .expect("dump");
    let source = exchange::load(&std::fs::read(&record.path).expect("dump file")).expect("load");
    handle.set_page(8, [0x77; 4]);
    service
        .type2_restore(Type2Restore::new(source, &backup).confirm(true))
        .expect("restore");
    assert_eq!(handle.page(8), Some([0; 4]));
}
