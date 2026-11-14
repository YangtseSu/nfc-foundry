//! DESFire application and file management on the scripted card.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::desfire::{
    AccessRight, AccessRights, ApplicationKeyType, AuthKeyType, CommunicationMode,
    DesfireAppCreateRequest, DesfireAppDeleteRequest, DesfireFileCreateRequest,
    DesfireFileDeleteRequest, DesfireFileInspectionRequest, DesfireFileSettingsRequest,
    DesfireFileSpec, DesfireFreeMemoryRequest, FileContents, FileType, app_create_text,
    app_delete_text, file_create_text, file_delete_text, file_settings_text, free_memory_text,
};
use nfc_core::{CancelToken, NfcError};
use nfc_desfire::mock::{MockDesfire, MockDesfireTransport};

const AID: [u8; 3] = [0xF0, 0x0D, 0x01];
const KEY: [u8; 8] = [0x00; 8];

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

fn standard_file(file_id: u8, communication: u8, access_rights: u16) -> DesfireFileSpec {
    DesfireFileSpec::Standard {
        file_id,
        iso_file_id: None,
        communication,
        access_rights,
        size: 32,
    }
}

#[test]
fn create_application_checks_memory_and_reads_the_id_back() {
    let (mut service, transport) = service(MockDesfire::new(KEY, AuthKeyType::Des));
    let result = service
        .desfire_create_application(
            DesfireAppCreateRequest::new(AID)
                .key(KEY.to_vec())
                .keys(2)
                .app_key_type(ApplicationKeyType::Aes),
        )
        .expect("create the application");

    assert_eq!(result.aid, AID);
    assert_eq!(result.keys, 2);
    assert_eq!(result.app_key_type, ApplicationKeyType::Aes);
    assert_eq!(result.free_memory_before, 8192);
    assert_eq!(result.free_memory_after, 8182);
    assert!(result.applications.contains(&AID));
    assert_eq!(command_count(&transport, 0xCA), 1);
    let text = app_create_text(&result);
    assert!(text.contains("application F0 0D 01 created"), "{text}");
    assert!(text.contains("keys: 2 · AES"), "{text}");
    assert_eq!(
        free_memory_text(Some(AID), 8182),
        "application F0 0D 01 · free 8182 bytes\n"
    );
}

#[test]
fn create_application_refuses_a_card_without_free_memory() {
    let (mut service, transport) =
        service(MockDesfire::new(KEY, AuthKeyType::Des).with_free_memory(0));
    let error = service
        .desfire_create_application(DesfireAppCreateRequest::new(AID).key(KEY.to_vec()).keys(1))
        .expect_err("no free EEPROM bytes");
    assert!(error.to_string().contains("no free EEPROM"), "{error}");
    assert_eq!(command_count(&transport, 0xCA), 0);
}

#[test]
fn create_application_rejects_a_duplicate_aid() {
    let (mut service, transport) =
        service(MockDesfire::new(KEY, AuthKeyType::Des).with_application(AID));
    let error = service
        .desfire_create_application(DesfireAppCreateRequest::new(AID).key(KEY.to_vec()))
        .expect_err("the AID already exists");
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(command_count(&transport, 0xCA), 0);
}

#[test]
fn create_all_six_file_types_and_read_each_back() {
    const AES_KEY: [u8; 16] = [0; 16];
    let (mut service, transport) =
        service(MockDesfire::new(AES_KEY, AuthKeyType::Aes).with_application(AID));
    let specs = vec![
        standard_file(0x01, 0, 0),
        DesfireFileSpec::Backup {
            file_id: 0x02,
            iso_file_id: None,
            communication: 0,
            access_rights: 0,
            size: 24,
        },
        DesfireFileSpec::Value {
            file_id: 0x03,
            iso_file_id: None,
            communication: 0,
            access_rights: 0,
            lower_limit: -50,
            upper_limit: 50,
            value: 7,
            flags: 0x03,
        },
        DesfireFileSpec::LinearRecord {
            file_id: 0x04,
            iso_file_id: None,
            communication: 0,
            access_rights: 0,
            record_size: 8,
            max_records: 4,
        },
        DesfireFileSpec::CyclicRecord {
            file_id: 0x05,
            iso_file_id: None,
            communication: 0,
            access_rights: 0,
            record_size: 4,
            max_records: 2,
        },
        DesfireFileSpec::TransactionMac {
            file_id: 0x06,
            iso_file_id: None,
            communication: 0,
            access_rights: 0xEFF0,
            key: [0x11; 16],
            key_version: 3,
        },
    ];

    for spec in specs {
        let expected = spec.file_type();
        let expected_rights = spec.access_rights();
        let result = service
            .desfire_create_file(
                DesfireFileCreateRequest::new(AID, spec)
                    .key(AES_KEY.to_vec())
                    .key_type(AuthKeyType::Aes),
            )
            .expect("create the file");
        assert_eq!(result.settings.file_type, expected);
        assert_eq!(result.settings.access.raw, expected_rights);
        assert_eq!(result.aid, AID);
        let text = file_create_text(&result);
        assert!(
            text.contains(&format!("file {:02X} created", result.file_id)),
            "{text}"
        );
    }

    assert_eq!(transport.tag().file_ids_in(AID), [1, 2, 3, 4, 5, 6]);
    assert_eq!(
        command_count(&transport, 0xCE),
        1,
        "one Transaction MAC create"
    );

    let tag = transport.tag();
    let raw = tag
        .file_settings_in(AID, 0x06)
        .expect("Transaction MAC settings");
    let settings = nfc_desfire::FileSettings::parse(raw).expect("Transaction MAC settings");
    assert_eq!(settings.file_type, FileType::TransactionMac);
    match settings.contents {
        FileContents::TransactionMac { key_version, .. } => {
            assert_eq!(key_version, 3, "the Transaction MAC key version reads back");
        }
        _ => panic!("a Transaction MAC file decoded as another type"),
    }
}

#[test]
fn create_file_refuses_content_beyond_free_memory() {
    let (mut service, transport) = service(
        MockDesfire::new(KEY, AuthKeyType::Des)
            .with_application(AID)
            .with_free_memory(8),
    );
    let error = service
        .desfire_create_file(
            DesfireFileCreateRequest::new(AID, standard_file(0x01, 0, 0)).key(KEY.to_vec()),
        )
        .expect_err("32 content bytes do not fit");
    assert!(
        error.to_string().contains("needs 32 content bytes"),
        "{error}"
    );
    assert_eq!(command_count(&transport, 0xCD), 0);
    assert!(transport.tag().file_ids_in(AID).is_empty());
}

#[test]
fn change_settings_applies_and_reads_back() {
    const AES_KEY: [u8; 16] = [0; 16];
    let (mut service, transport) =
        service(MockDesfire::new(AES_KEY, AuthKeyType::Aes).with_file_in(
            AID,
            0x01,
            vec![0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00],
            vec![0u8; 32],
        ));
    let rights = AccessRights::from_parts(
        AccessRight::Free,
        AccessRight::Key(0),
        AccessRight::Denied,
        AccessRight::Key(0),
    );
    let result = service
        .desfire_change_file_settings(
            DesfireFileSettingsRequest::new(AID, 0x01)
                .communication(CommunicationMode::Enciphered)
                .access_rights(rights.raw)
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .advanced(true)
                .confirm(true),
        )
        .expect("change settings");
    assert!(result.changed);
    assert_eq!(result.before.communication, CommunicationMode::Plain);
    assert_eq!(result.after.communication, CommunicationMode::Enciphered);
    assert_eq!(result.after.access.raw, rights.raw);
    assert_eq!(command_count(&transport, 0x5F), 1);
    let text = file_settings_text(&result);
    assert!(text.contains("change: applied"), "{text}");
}

#[test]
fn change_settings_needs_advanced_and_confirmation() {
    let (mut service, transport) = service(MockDesfire::new(KEY, AuthKeyType::Des).with_file_in(
        AID,
        0x01,
        vec![0x00, 0x00, 0xEE, 0xEE, 0x20, 0x00, 0x00],
        vec![0u8; 32],
    ));
    let error = service
        .desfire_change_file_settings(
            DesfireFileSettingsRequest::new(AID, 0x01).communication(CommunicationMode::Enciphered),
        )
        .expect_err("Advanced mode is required");
    assert!(error.to_string().contains("Advanced mode"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");

    let error = service
        .desfire_change_file_settings(
            DesfireFileSettingsRequest::new(AID, 0x01)
                .communication(CommunicationMode::Enciphered)
                .advanced(true),
        )
        .expect_err("confirmation is required");
    assert!(error.to_string().contains("confirmation"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");
}

#[test]
fn inspect_file_reads_settings_without_changing_them() {
    let (mut service, transport) = service(MockDesfire::new(KEY, AuthKeyType::Des).with_file_in(
        AID,
        0x01,
        vec![0x00, 0x00, 0xEE, 0xEE, 0x20, 0x00, 0x00],
        vec![0u8; 32],
    ));
    let inspection = service
        .desfire_inspect_file(DesfireFileInspectionRequest::new(AID, 0x01))
        .expect("inspect settings");
    assert_eq!(inspection.file_id, 0x01);
    assert_eq!(inspection.settings.file_type, FileType::Standard);
    assert_eq!(inspection.raw_settings.len(), 7);
    assert_eq!(inspection.settings.access.raw, 0xEEEE);
    assert_eq!(command_count(&transport, 0x5F), 0, "no settings command");
}

#[test]
fn change_settings_rejects_a_non_data_communication_change() {
    let (mut service, _transport) = service(MockDesfire::new(KEY, AuthKeyType::Des).with_file_in(
        AID,
        0x02,
        vec![
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x07, 0x00,
            0x00, 0x00, 0x00,
        ],
        Vec::new(),
    ));
    let error = service
        .desfire_change_file_settings(
            DesfireFileSettingsRequest::new(AID, 0x02)
                .communication(CommunicationMode::Maced)
                .key(KEY.to_vec())
                .advanced(true)
                .confirm(true),
        )
        .expect_err("value files keep their mode");
    assert!(
        error.to_string().contains("only Standard and Backup"),
        "{error}"
    );
}

#[test]
fn file_delete_exports_a_manifest_and_verifies_absence() {
    let directory = tempfile::tempdir().expect("manifest directory");
    let manifest_path = directory.path().join("file.json");
    let (mut service, transport) = service(MockDesfire::new(KEY, AuthKeyType::Des).with_file_in(
        AID,
        0x01,
        vec![0x00, 0x00, 0xEE, 0xEE, 0x20, 0x00, 0x00],
        vec![0u8; 32],
    ));
    let result = service
        .desfire_delete_file(
            DesfireFileDeleteRequest::new(AID, 0x01)
                .key(KEY.to_vec())
                .advanced(true)
                .confirm(true)
                .manifest_path(&manifest_path),
        )
        .expect("delete the file");

    assert_eq!(result.files, Vec::<u8>::new());
    assert_eq!(result.manifest_path, manifest_path);
    let manifest = std::fs::read_to_string(&manifest_path).expect("manifest file");
    assert!(manifest.contains("\"schema\": \"nfc-foundry/desfire-manifest/1\""));
    assert!(manifest.contains("\"file_id\": \"01\""));
    assert!(manifest.contains("\"raw_settings\": \"00 00 EE EE 20 00 00\""));
    assert!(manifest.contains("cannot restore"));
    assert_eq!(command_count(&transport, 0xDF), 1);
    let text = file_delete_text(&result);
    assert!(text.contains("file 01 deleted"), "{text}");
    assert!(text.contains("manifest:"), "{text}");
}

#[test]
fn file_delete_requires_advanced_and_confirmation_before_reader_io() {
    let mut service = Service::default();
    let error = service
        .desfire_delete_file(DesfireFileDeleteRequest::new(AID, 0x01).key(KEY.to_vec()))
        .expect_err("Advanced mode is required");
    assert!(error.to_string().contains("Advanced mode"), "{error}");
    let error = service
        .desfire_delete_file(
            DesfireFileDeleteRequest::new(AID, 0x01)
                .key(KEY.to_vec())
                .advanced(true),
        )
        .expect_err("confirmation is required");
    assert!(error.to_string().contains("confirmation"), "{error}");
}

#[test]
fn app_delete_exports_a_manifest_and_reports_the_remaining_apps() {
    let directory = tempfile::tempdir().expect("manifest directory");
    let manifest_path = directory.path().join("app.json");
    let (mut service, transport) = service(
        MockDesfire::new(KEY, AuthKeyType::Des)
            .with_file_in(
                AID,
                0x01,
                vec![0x00, 0x00, 0xEE, 0xEE, 0x20, 0x00, 0x00],
                vec![0u8; 32],
            )
            .with_application([0xF0, 0x0D, 0x02]),
    );
    let result = service
        .desfire_delete_application(
            DesfireAppDeleteRequest::new(AID)
                .key(KEY.to_vec())
                .advanced(true)
                .confirm(true)
                .manifest_path(&manifest_path),
        )
        .expect("delete the application");

    assert_eq!(result.applications, [[0xF0, 0x0D, 0x02]]);
    let manifest = std::fs::read_to_string(&manifest_path).expect("manifest file");
    assert!(manifest.contains("\"aid\": \"F00D01\""));
    assert!(manifest.contains("\"file_id\": \"01\""));
    assert!(manifest.contains("not file contents"));
    assert_eq!(command_count(&transport, 0xDA), 1);
    assert_eq!(transport.tag().application_ids(), [[0xF0, 0x0D, 0x02]]);
    let text = app_delete_text(&result);
    assert!(text.contains("not a byte-for-byte restore"), "{text}");
}

#[test]
fn app_delete_refuses_an_unknown_application_without_deleting() {
    let directory = tempfile::tempdir().expect("manifest directory");
    let manifest_path = directory.path().join("app.json");
    let (mut service, transport) = service(MockDesfire::new(KEY, AuthKeyType::Des));
    let error = service
        .desfire_delete_application(
            DesfireAppDeleteRequest::new(AID)
                .key(KEY.to_vec())
                .advanced(true)
                .confirm(true)
                .manifest_path(&manifest_path),
        )
        .expect_err("the application is absent");
    assert!(error.to_string().contains("is not present"), "{error}");
    assert_eq!(command_count(&transport, 0xDA), 0);
    assert!(!manifest_path.exists(), "no manifest for a refused delete");
}

#[test]
fn free_memory_reads_the_picc_or_an_application() {
    let (mut service, _transport) =
        service(MockDesfire::new(KEY, AuthKeyType::Des).with_free_memory(4096));
    assert_eq!(
        service
            .desfire_free_memory(CancelToken::new(), DesfireFreeMemoryRequest::default())
            .expect("PICC free memory"),
        4096
    );
    assert_eq!(
        service
            .desfire_free_memory(
                CancelToken::new(),
                DesfireFreeMemoryRequest::new(Some(AID)).key(KEY.to_vec()),
            )
            .expect("application free memory"),
        4096
    );
    assert_eq!(free_memory_text(None, 4096), "PICC · free 4096 bytes\n");
}

#[test]
fn delete_without_a_key_is_still_refused_when_the_card_demands_auth() {
    let directory = tempfile::tempdir().expect("manifest directory");
    let manifest_path = directory.path().join("app.json");
    let (mut service, _transport) =
        service(MockDesfire::new(KEY, AuthKeyType::Des).with_application(AID));
    let error = service
        .desfire_delete_application(
            DesfireAppDeleteRequest::new(AID)
                .advanced(true)
                .confirm(true)
                .manifest_path(&manifest_path),
        )
        .expect_err("the mock card requires an authenticated delete");
    assert!(
        matches!(
            error,
            NfcError::AuthenticationRequired(_) | NfcError::VerificationFailed(_)
        ),
        "{error}"
    );
}
