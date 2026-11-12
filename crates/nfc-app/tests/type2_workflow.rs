//! Type 2 workflow tests on the scripted transport.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::classic::BackupStore;
use nfc_app::type2::{
    Type2Auth, Type2Format, Type2Lock, Type2NdefWrite, Type2PageWrite, Type2Password,
    Type2ReadRequest, Type2Restore, Type2UidExecution, parse_uid_hex,
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

fn ev1_service() -> (Service, MockHandle) {
    let transport = MockTransport::new();
    let handle = transport.add(DeviceInfo::new("Mock EV1", "mock:ev1"), Some(card()));
    handle.set_type2_version(Some([0x00, 0x04, 0x03, 0x01, 0x01, 0x00, 0x0B, 0x03]));
    for page in 0..20u8 {
        handle.set_page(page, [0; 4]);
    }
    handle.set_page(0, [0x04, 0x43, 0x47, 0x88]);
    handle.set_page(1, [0x4A, 0x75, 0x22, 0x91]);
    handle.set_page(2, [0x8C, 0x48, 0x00, 0x00]);
    handle.set_page(16, [0x00, 0x00, 0x00, 0xFF]);
    handle.set_page(17, [0x00, 0x05, 0x00, 0x00]);
    let mut service = Service::with_transport(Arc::new(transport));
    service.refresh_devices().expect("refresh");
    service.select_device("mock:ev1").expect("select mock EV1");
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
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read NTAG215");
    assert_eq!(result.image.layout.product, Type2Product::Ntag215);
    assert_eq!(result.image.layout.total_pages, 135);
    assert!(result.image.is_complete(), "{:?}", result.warnings);
    assert!(result.warnings.is_empty());
    assert_eq!(result.image.cc.size, 496);
}

fn magic_ntag213_pages() -> Vec<[u8; 4]> {
    let mut pages = vec![[0u8; 4]; 45];
    pages[0] = [0x04, 0x00, 0xE3, 0x6F];
    pages[1] = [0xC2, 0x00, 0x00, 0x02];
    pages[2] = [0xC0, 0x08, 0x00, 0x00];
    pages[3] = [0xE1, 0x10, 0x12, 0x00];
    pages[4] = [0x03, 0x00, 0xFE, 0x00];
    pages[40] = [0x00, 0x00, 0x00, 0xBD];
    pages[41] = [0x00, 0x00, 0x00, 0xFF];
    pages
}

fn magic_service() -> (Service, MockHandle) {
    let transport = MockTransport::new();
    let card = CardInfo::new(
        vec![0x04, 0x00, 0xE3, 0xC2, 0x00, 0x00, 0x02],
        [0x00, 0x44],
        0x00,
        CardType::Type2,
    );
    let handle = transport.add(
        DeviceInfo::new("Mock magic NTAG213", "mock:magic"),
        Some(card),
    );
    for (page, data) in magic_ntag213_pages().into_iter().enumerate() {
        handle.set_page(page as u8, data);
    }
    handle.set_type2_version(Some([0x00, 0x04, 0x04, 0x02, 0x01, 0x00, 0x0F, 0x03]));
    handle.set_type2_uid_writable(true);
    let mut service = Service::with_transport(Arc::new(transport));
    service.refresh_devices().expect("refresh");
    service
        .select_device("mock:magic")
        .expect("select mock magic tag");
    (service, handle)
}

#[test]
fn type2_uid_changes_and_restores() {
    let (mut service, handle) = magic_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let target = parse_uid_hex("0400E3C2000003").expect("target UID");
    let plan = service
        .type2_uid_preflight(&target, CancelToken::new())
        .expect("UID plan");
    assert_eq!(plan.new_pages[0], [0x04, 0x00, 0xE3, 0x6F]);
    assert_eq!(plan.new_pages[1], [0xC2, 0x00, 0x00, 0x03]);
    assert_eq!(plan.new_pages[2], [0xC1, 0x08, 0x00, 0x00]);
    let changed = service
        .type2_uid_execute(Type2UidExecution::new(plan, &backup).confirm(true))
        .expect("change UID");
    assert_eq!(changed.card_after.uid, target);
    assert_eq!(handle.page(1), Some([0xC2, 0x00, 0x00, 0x03]));
    assert_eq!(handle.page(2), Some([0xC1, 0x08, 0x00, 0x00]));

    let original = parse_uid_hex("0400E3C2000002").expect("original UID");
    let plan = service
        .type2_uid_preflight(&original, CancelToken::new())
        .expect("restore plan");
    let restored = service
        .type2_uid_execute(Type2UidExecution::new(plan, &backup).confirm(true))
        .expect("restore UID");
    assert_eq!(restored.card_after.uid, original);
    assert_eq!(handle.page(1), Some([0xC2, 0x00, 0x00, 0x02]));
    assert_eq!(handle.page(2), Some([0xC0, 0x08, 0x00, 0x00]));
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
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read NDEF");
    assert_eq!(read.ndef.state, NdefState::Message(message("hello")));

    let unconfirmed = service.type2_ndef_format(Type2Format::new(&backup));
    assert!(matches!(unconfirmed, Err(NfcError::InvalidArgument(_))));
    let formatted = service
        .type2_ndef_format(Type2Format::new(&backup).confirm(true))
        .expect("format");
    assert!(formatted.summary.written_pages > 0);
    let read = service
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read NDEF");
    assert_eq!(read.ndef.state, NdefState::Empty);
}

#[test]
fn ev1_format_requires_the_otp_gate_and_writes_the_cc() {
    let (mut service, handle) = ev1_service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let refused = service.type2_ndef_format(Type2Format::new(&backup).confirm(true));
    assert!(matches!(refused, Err(NfcError::Unsupported(_))));
    assert_eq!(handle.page(3), Some([0; 4]));

    let formatted = service
        .type2_ndef_format(Type2Format::new(&backup).confirm(true).allow_otp_cc(true))
        .expect("format blank EV1");
    assert_eq!(handle.page(3), Some([0xE1, 0x10, 0x06, 0x00]));
    assert_eq!(formatted.image.cc.size, 48);
    assert_eq!(formatted.image.page(4), Some(&[0x03, 0x00, 0xFE, 0x00]));
    let read = service
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
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
    let read = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read");
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
    let image = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read")
        .image;
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
    let source = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("read")
        .image;
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
    for (page, data) in ntag215_pages()
        .into_iter()
        .filter(|(page, _)| *page < 4 || *page >= 128)
    {
        handle.set_page(page, data);
    }
    let result = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("partial read");
    assert_eq!(result.image.known_pages(), 11);
    assert!(!result.warnings.is_empty());
    let error = service
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()))
        .expect_err("NDEF area is incomplete");
    assert!(matches!(error, NfcError::InvalidArgument(_)));
}

#[test]
fn dump_and_restore_round_trip_through_json() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let record = service
        .type2_dump(&backup, Type2ReadRequest::new(CancelToken::new()))
        .expect("dump");
    let source = exchange::load(&std::fs::read(&record.path).expect("dump file")).expect("load");
    handle.set_page(8, [0x77; 4]);
    service
        .type2_restore(Type2Restore::new(source, &backup).confirm(true))
        .expect("restore");
    assert_eq!(handle.page(8), Some([0; 4]));
}

fn protect_ntag215(handle: &MockHandle, password: [u8; 4], pack: [u8; 2]) {
    handle.set_page(131, [0x00, 0x00, 0x00, 0x20]);
    handle.set_page(132, [0x80, 0x00, 0x00, 0x00]);
    handle.set_password(password, pack);
}

#[test]
fn protected_access_uses_one_session_per_operation() {
    let (mut service, handle) = service();
    let password = [0xDE, 0xAD, 0xBE, 0xEF];
    let pack = [0xCA, 0xFE];
    protect_ntag215(&handle, password, pack);

    let error = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect_err("protected read needs credentials");
    assert!(matches!(error, NfcError::AuthenticationRequired(_)));
    assert_eq!(handle.password_auths(), 0);

    let auth = Type2Auth::new(password, pack);
    let read = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()).auth(Some(auth)))
        .expect("authenticated read");
    assert!(read.image.is_complete());
    assert_eq!(handle.password_auths(), 1);

    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let auth = Type2Auth::new(password, pack);
    service
        .type2_ndef_write(Type2NdefWrite::new(message("protected"), &backup).auth(Some(auth)))
        .expect("authenticated NDEF write");
    assert_eq!(
        handle.password_auths(),
        2,
        "the NDEF write operation must authenticate once"
    );
    let auth = Type2Auth::new(password, pack);
    let ndef = service
        .type2_ndef_read(Type2ReadRequest::new(CancelToken::new()).auth(Some(auth)))
        .expect("authenticated NDEF read");
    assert_eq!(ndef.ndef.state, NdefState::Message(message("protected")));

    let bad = Type2Auth::new(password, [0x00, 0x00]);
    let error = service
        .type2_write_page(Type2PageWrite::new(32, [0xAA; 4], &backup).auth(Some(bad)))
        .expect_err("wrong PACK");
    assert!(matches!(error, NfcError::VerificationFailed(_)));
    assert!(!error.to_string().contains("CAFE"));
    assert_eq!(handle.page(32), Some([0; 4]));
}

#[test]
fn password_change_and_clear_restore_authenticated_access() {
    let (mut service, handle) = service();
    let old = Type2Auth::new([0xDE, 0xAD, 0xBE, 0xEF], [0xCA, 0xFE]);
    protect_ntag215(&handle, [0xDE, 0xAD, 0xBE, 0xEF], [0xCA, 0xFE]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let result = service
        .type2_password(
            Type2Password::new(0x20, 0x80, [1, 2, 3, 4], [0x12, 0x34], &backup)
                .current(Some(old))
                .confirm(true),
        )
        .expect("change password");
    assert_eq!(result.image.page(131).expect("cfg0")[3], 0x20);

    let read = service
        .type2_read(
            Type2ReadRequest::new(CancelToken::new())
                .auth(Some(Type2Auth::new([1, 2, 3, 4], [0x12, 0x34]))),
        )
        .expect("new credentials");
    assert!(read.image.is_complete());

    for entry in std::fs::read_dir(directory.path()).expect("backup dir") {
        let bytes = std::fs::read(entry.expect("entry").path()).expect("backup file");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("DEADBEEF"), "backup leaked the old password");
        assert!(!text.contains("CAFE"), "backup leaked the old PACK");
        assert!(!text.contains("01020304"), "backup leaked the new password");
        assert!(!text.contains("1234"), "backup leaked the new PACK");
    }

    let current = Type2Auth::new([1, 2, 3, 4], [0x12, 0x34]);
    service
        .type2_password(
            Type2Password::new(0xFF, 0x00, [1, 2, 3, 4], [0x12, 0x34], &backup)
                .current(Some(current))
                .confirm(true),
        )
        .expect("clear protection");
    let read = service
        .type2_read(Type2ReadRequest::new(CancelToken::new()))
        .expect("unprotected read");
    assert!(read.image.is_complete());
    assert_eq!(read.image.page(131).expect("cfg0")[3], 0xFF);
    assert_eq!(read.image.page(132).expect("cfg1")[0], 0x00);
}
