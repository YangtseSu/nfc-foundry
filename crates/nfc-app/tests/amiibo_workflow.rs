//! Amiibo workflows on the scripted NTAG215 transport.

use std::path::Path;
use std::sync::Arc;

use nfc_amiibo::{AmiiboKeys, AmiiboPlain, AmiiboTag, bcc0, bcc1, pack, unpack};
use nfc_app::Service;
use nfc_app::amiibo::{
    AmiiboFileRequest, AmiiboReadRequest, AmiiboSourceFormat, AmiiboWriteExecution,
    AmiiboWriteRequest,
};
use nfc_app::classic::BackupStore;
use nfc_core::{CancelToken, CardInfo, CardType};
use nfc_transport::DeviceInfo;
use nfc_transport::mock::{MockHandle, MockTransport};

const SOURCE_UID: [u8; 7] = [0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
const TARGET_UID: [u8; 7] = [0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80];

fn synthetic_keys() -> AmiiboKeys {
    let mut data = vec![0u8; 80];
    for (index, value) in data.iter_mut().take(16).enumerate() {
        *value = index as u8;
    }
    data[16..30].copy_from_slice(b"unfixed infos\0");
    data[31] = 14;
    for index in 0..16 {
        data[32 + index] = 0x20 + index as u8;
    }
    for index in 0..32 {
        data[48 + index] = 0x30 + index as u8;
    }

    let mut tag = vec![0u8; 80];
    for (index, value) in tag.iter_mut().take(16).enumerate() {
        *value = 0x50 + index as u8;
    }
    tag[16..30].copy_from_slice(b"locked secret\0");
    tag[31] = 16;
    for index in 0..16 {
        tag[32 + index] = 0x60 + index as u8;
    }
    for index in 0..32 {
        tag[48 + index] = 0x70 + index as u8;
    }
    AmiiboKeys::from_parts(&data, &tag).expect("keys")
}

fn source_plain(uid: [u8; 7]) -> AmiiboPlain {
    let mut bytes = [0u8; 540];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index * 7 + 3) as u8;
    }
    bytes[..8].copy_from_slice(&[bcc1(&uid), 0x48, 0x0F, 0xE0, 0xF1, 0x10, 0xFF, 0xEE]);
    bytes[0x29..0x2B].copy_from_slice(&[0x00, 0x05]);
    bytes[0x1D4..0x1DC].copy_from_slice(&[
        uid[0],
        uid[1],
        uid[2],
        bcc0(&uid),
        uid[3],
        uid[4],
        uid[5],
        uid[6],
    ]);
    bytes[0x1DC..0x1E4].copy_from_slice(&[0x12, 0x34, 0x01, 0x02, 0x0A, 0xBC, 0x03, 0x02]);
    bytes[0x208..0x21C].copy_from_slice(&[
        0x01, 0x00, 0x0F, 0xBD, 0x00, 0x00, 0x00, 0x04, 0x5F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    AmiiboPlain::from_bytes(&bytes).expect("plain")
}

fn source_tag() -> AmiiboTag {
    pack(&synthetic_keys(), &source_plain(SOURCE_UID)).expect("source tag")
}

fn target_card() -> CardInfo {
    CardInfo::new(TARGET_UID.to_vec(), [0x00, 0x44], 0x00, CardType::Type2)
}

fn blank_ntag215(handle: &MockHandle) {
    handle.set_type2_version(Some([0x00, 0x04, 0x04, 0x02, 0x01, 0x00, 0x11, 0x03]));
    handle.set_page(
        0,
        [
            TARGET_UID[0],
            TARGET_UID[1],
            TARGET_UID[2],
            bcc0(&TARGET_UID),
        ],
    );
    handle.set_page(
        1,
        [TARGET_UID[3], TARGET_UID[4], TARGET_UID[5], TARGET_UID[6]],
    );
    handle.set_page(2, [bcc1(&TARGET_UID), 0x48, 0x00, 0x00]);
    handle.set_page(3, [0x00, 0x00, 0x00, 0x00]);
    for page in 4..=132u8 {
        handle.set_page(page, [0x00; 4]);
    }
    handle.set_page(131, [0x00, 0x00, 0x00, 0xFF]);
    handle.set_page(133, [0x00; 4]);
    handle.set_page(134, [0x00; 4]);
}

fn service() -> (Service, MockHandle) {
    let transport = MockTransport::new();
    let handle = transport.add(
        DeviceInfo::new("Mock NTAG215", "mock:ntag215"),
        Some(target_card()),
    );
    blank_ntag215(&handle);
    let mut service = Service::with_transport(Arc::new(transport));
    service.refresh_devices().expect("refresh");
    service
        .select_device("mock:ntag215")
        .expect("select mock reader");
    (service, handle)
}

fn write_source(tag: &AmiiboTag, path: &Path) {
    std::fs::write(path, tag.as_bytes()).expect("source file");
}

#[test]
fn reads_and_decrypts_a_live_amiibo_image() {
    let (mut service, handle) = service();
    let tag = source_tag();
    for page in 0..135u8 {
        handle.set_page(page, tag.page(page).expect("page"));
    }
    // An Amiibo's CC is deliberately not an NDEF E1 container.
    handle.set_page(131, [0x00, 0x00, 0x00, 0x04]);
    handle.set_page(132, [0x5F, 0x00, 0x00, 0x00]);

    let read = service
        .amiibo_read(AmiiboReadRequest::new(CancelToken::new()).keys(Some(synthetic_keys())))
        .expect("read amiibo");
    let identity = read.identity.expect("identity");
    assert_eq!(read.raw.uid().expect("uid"), SOURCE_UID);
    assert_eq!(identity.game_character_id, 0x1234);
    assert_eq!(
        read.plain.expect("plain").uid().expect("plain uid"),
        SOURCE_UID
    );
    assert!(
        read.warnings
            .iter()
            .all(|warning| !warning.contains("capability container")),
        "{:?}",
        read.warnings
    );
}

#[test]
fn a_wrong_key_file_is_reported_as_a_signature_failure() {
    let (mut service, handle) = service();
    let tag = source_tag();
    for page in 0..135u8 {
        handle.set_page(page, tag.page(page).expect("page"));
    }
    // Build a structurally valid but different key set.
    let mut data = vec![0u8; 80];
    data[16..30].copy_from_slice(b"unfixed infos\0");
    data[31] = 14;
    data[32..48].copy_from_slice(&[0xAA; 16]);
    data[48..80].copy_from_slice(&[0x55; 32]);
    let mut tag_key = vec![0u8; 80];
    tag_key[16..30].copy_from_slice(b"locked secret\0");
    tag_key[31] = 16;
    tag_key[32..48].copy_from_slice(&[0xBB; 16]);
    tag_key[48..80].copy_from_slice(&[0x66; 32]);
    let keys = AmiiboKeys::from_parts(&data, &tag_key).expect("wrong keys");
    let error = service
        .amiibo_read(AmiiboReadRequest::new(CancelToken::new()).keys(Some(keys)))
        .expect_err("wrong keys");
    assert!(error.to_string().contains("signatures"), "{error}");
}

#[test]
fn decrypt_and_encrypt_files_round_trip() {
    let directory = tempfile::tempdir().expect("temp dir");
    let raw_path = directory.path().join("source.bin");
    let plain_path = directory.path().join("plain.bin");
    let rebuilt_path = directory.path().join("rebuilt.bin");
    let source = source_tag();
    write_source(&source, &raw_path);

    let mut service = Service::default();
    let decrypted = service
        .amiibo_decrypt(AmiiboFileRequest {
            keys: synthetic_keys(),
            input: raw_path.clone(),
            output: Some(plain_path.clone()),
            cancel: CancelToken::new(),
        })
        .expect("decrypt");
    assert_eq!(decrypted.source_uid, SOURCE_UID);
    assert_eq!(decrypted.identity.game_character_id, 0x1234);
    assert_eq!(std::fs::read(&plain_path).expect("plain file").len(), 540);

    let encrypted = service
        .amiibo_encrypt(AmiiboFileRequest {
            keys: synthetic_keys(),
            input: plain_path,
            output: Some(rebuilt_path.clone()),
            cancel: CancelToken::new(),
        })
        .expect("encrypt");
    assert_eq!(encrypted.source_uid, SOURCE_UID);
    assert_eq!(
        std::fs::read(&rebuilt_path).expect("rebuilt"),
        source.as_bytes()
    );
}

fn plan(service: &mut Service, source: &Path, directory: &Path) -> nfc_app::AmiiboWritePlan {
    service
        .amiibo_write_preflight(AmiiboWriteRequest {
            keys: synthetic_keys(),
            source: source.to_path_buf(),
            format: AmiiboSourceFormat::Raw,
            auth: None,
            advanced: true,
            backup: BackupStore::new(directory),
            cancel: CancelToken::new(),
        })
        .expect("write plan")
}

#[test]
fn writes_a_resigned_amiibo_to_a_blank_ntag215() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let source_path = directory.path().join("source.bin");
    write_source(&source_tag(), &source_path);

    let plan = plan(&mut service, &source_path, directory.path());
    assert_eq!(plan.target_uid(), TARGET_UID);
    assert_eq!(plan.source_uid(), SOURCE_UID);
    assert_eq!(plan.pages().len(), 133);
    assert_eq!(plan.pages()[0], 3);
    assert_eq!(plan.pages()[1..127], (4..=129).collect::<Vec<_>>());
    assert_eq!(plan.pages()[127..], [133, 134, 2, 130, 131, 132]);
    assert!(plan.expected_page(3).expect("cc") == [0xF1, 0x10, 0xFF, 0xEE]);
    assert_eq!(
        plan.expected_page(2).expect("page 2"),
        [bcc1(&TARGET_UID), 0x48, 0x0F, 0xE0]
    );
    assert!(plan.backup_path().starts_with(directory.path()));
    assert_eq!(
        plan.backup_path()
            .extension()
            .and_then(|value| value.to_str()),
        Some("bin")
    );

    let result = service
        .amiibo_write_execute(AmiiboWriteExecution::new(plan.token()).confirm(true))
        .expect("write");
    assert_eq!(result.written_pages, 133);
    assert_eq!(result.target_uid, TARGET_UID);
    assert_eq!(result.identity.game_character_id, 0x1234);
    assert!(result.backup.path.exists());

    assert_eq!(handle.page(3), Some([0xF1, 0x10, 0xFF, 0xEE]));
    assert_eq!(handle.page(2), Some([bcc1(&TARGET_UID), 0x48, 0x0F, 0xE0]));
    assert_eq!(handle.page(131), Some([0x00, 0x00, 0x00, 0x04]));
    assert_eq!(handle.page(132), Some([0x5F, 0x00, 0x00, 0x00]));
    let expected_password = nfc_app::password_for_uid(&TARGET_UID);
    assert_eq!(
        handle.page(133),
        Some([
            expected_password[0],
            expected_password[1],
            expected_password[2],
            expected_password[3]
        ])
    );
    assert_eq!(handle.page(134), Some([0x80, 0x80, 0x00, 0x00]));

    let read = service
        .amiibo_read(AmiiboReadRequest::new(CancelToken::new()).keys(Some(synthetic_keys())))
        .expect("verify");
    let plain = read.plain.expect("plain");
    assert_eq!(plain.uid().expect("uid"), TARGET_UID);
    let expected = unpack(
        &synthetic_keys(),
        &pack(
            &synthetic_keys(),
            &source_plain(SOURCE_UID)
                .rebind_uid(&TARGET_UID)
                .expect("rebind"),
        )
        .expect("pack"),
    )
    .expect("unpack");
    assert_eq!(plain.as_bytes()[..0x208], expected.as_bytes()[..0x208]);
}

#[test]
fn a_plaintext_source_is_re_signed_and_locked_canonically() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let source_path = directory.path().join("source.plain.bin");
    let mut bytes = *source_plain(SOURCE_UID).as_bytes();
    bytes[2] = 0x00;
    bytes[3] = 0x00;
    let plain = AmiiboPlain::from_bytes(&bytes).expect("plain source");
    std::fs::write(&source_path, plain.as_bytes()).expect("plain file");
    let plan = service
        .amiibo_write_preflight(AmiiboWriteRequest {
            keys: synthetic_keys(),
            source: source_path,
            format: AmiiboSourceFormat::Plain,
            auth: None,
            advanced: true,
            backup: BackupStore::new(directory.path()),
            cancel: CancelToken::new(),
        })
        .expect("plain write plan");
    assert_eq!(
        plan.expected_page(2).expect("page 2"),
        [bcc1(&TARGET_UID), 0x48, 0x0F, 0xE0]
    );
    service
        .amiibo_write_execute(AmiiboWriteExecution::new(plan.token()).confirm(true))
        .expect("write plain source");
    assert_eq!(handle.page(2), Some([bcc1(&TARGET_UID), 0x48, 0x0F, 0xE0]));
    assert!(handle.page(133).is_some());
}

#[test]
fn a_locked_or_completed_target_is_refused_before_a_backup() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let source_path = directory.path().join("source.bin");
    write_source(&source_tag(), &source_path);

    handle.set_page(2, [bcc1(&TARGET_UID), 0x48, 0x0F, 0xE0]);
    let error = service
        .amiibo_write_preflight(AmiiboWriteRequest {
            keys: synthetic_keys(),
            source: source_path.clone(),
            format: AmiiboSourceFormat::Raw,
            auth: None,
            advanced: true,
            backup: BackupStore::new(directory.path()),
            cancel: CancelToken::new(),
        })
        .expect_err("locked");
    assert!(error.to_string().contains("locked"), "{error}");

    blank_ntag215(&handle);
    handle.set_page(131, [0x00, 0x00, 0x00, 0x04]);
    handle.set_page(132, [0x5F, 0x00, 0x00, 0x00]);
    let error = service
        .amiibo_write_preflight(AmiiboWriteRequest {
            keys: synthetic_keys(),
            source: source_path,
            format: AmiiboSourceFormat::Raw,
            auth: None,
            advanced: true,
            backup: BackupStore::new(directory.path()),
            cancel: CancelToken::new(),
        })
        .expect_err("configuration locked");
    assert!(error.to_string().contains("locked"), "{error}");
}

#[test]
fn advanced_confirmation_and_single_use_tokens_are_enforced() {
    let (mut service, _handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let source_path = directory.path().join("source.bin");
    write_source(&source_tag(), &source_path);

    let error = service
        .amiibo_write_preflight(AmiiboWriteRequest {
            keys: synthetic_keys(),
            source: source_path.clone(),
            format: AmiiboSourceFormat::Raw,
            auth: None,
            advanced: false,
            backup: BackupStore::new(directory.path()),
            cancel: CancelToken::new(),
        })
        .expect_err("advanced");
    assert!(error.to_string().contains("Advanced"), "{error}");

    let plan = plan(&mut service, &source_path, directory.path());
    let token = plan.token();
    let error = service
        .amiibo_write_execute(AmiiboWriteExecution::new(token).confirm(false))
        .expect_err("confirmation");
    assert!(error.to_string().contains("confirmation"), "{error}");
    let error = service
        .amiibo_write_execute(AmiiboWriteExecution::new(token).confirm(true))
        .expect_err("reuse");
    assert!(error.to_string().contains("already used"), "{error}");
}

#[test]
fn a_changed_target_image_invalidates_the_plan() {
    let (mut service, handle) = service();
    let directory = tempfile::tempdir().expect("temp dir");
    let source_path = directory.path().join("source.bin");
    write_source(&source_tag(), &source_path);
    let plan = plan(&mut service, &source_path, directory.path());

    handle.set_page(64, [0xDE, 0xAD, 0xBE, 0xEF]);
    let error = service
        .amiibo_write_execute(AmiiboWriteExecution::new(plan.token()).confirm(true))
        .expect_err("changed image");
    assert!(
        error.to_string().contains("changed after the write plan"),
        "{error}"
    );
}

#[test]
fn a_partial_live_read_is_refused() {
    let (mut service, handle) = service();
    let tag = source_tag();
    for page in 0..135u8 {
        handle.set_page(page, tag.page(page).expect("page"));
    }
    // Remove one page so the read stays partial.
    handle.clear_pages();
    for page in 0..135u8 {
        if page != 64 {
            handle.set_page(page, tag.page(page).expect("page"));
        }
    }
    let error = service
        .amiibo_read(AmiiboReadRequest::new(CancelToken::new()))
        .expect_err("partial");
    assert!(error.to_string().contains("partial"), "{error}");
}
