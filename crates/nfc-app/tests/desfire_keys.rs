//! DESFire key management on the scripted card.

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use nfc_app::Service;
use nfc_app::desfire::{
    AuthKeyType, DesfireKeyChangeRequest, DesfireKeySettingsRequest, key_change_text,
    key_settings_text,
};
use nfc_core::NfcError;
use nfc_desfire::mock::{MockDesfire, MockDesfireTransport};

const AID: [u8; 3] = [0xF0, 0x0D, 0x0A];
const AES_KEY: [u8; 16] = [0x00; 16];

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

fn export_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "nfc-foundry-key-{label}-{}.json",
        std::process::id()
    ))
}

#[test]
fn read_key_settings_and_key_versions() {
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(3)
            .with_key(0, AES_KEY.to_vec(), 0)
            .with_key(1, [0x11; 16].to_vec(), 4)
            .with_key(2, [0x22; 16].to_vec(), 7),
    );
    let result = service
        .desfire_key_settings(
            DesfireKeySettingsRequest::new(AID)
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes),
        )
        .expect("read key settings");

    assert_eq!(result.aid, AID);
    assert_eq!(result.before.key_settings, 0x0F);
    assert_eq!(result.after, result.before);
    assert!(!result.changed);
    assert_eq!(result.key_versions, vec![Some(0), Some(4), Some(7)]);
    assert_eq!(command_count(&transport, 0x45), 1);
    assert_eq!(command_count(&transport, 0x54), 0);
    let text = key_settings_text(&result);
    assert!(text.contains("application F00D0A"), "{text}");
    assert!(text.contains("key versions: 0=0 · 1=4 · 2=7"), "{text}");
    assert!(!text.contains("00 00 00 00"), "no key material in the text");
}

#[test]
fn change_key_settings_needs_advanced_and_confirmation() {
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(1)
            .with_key(0, AES_KEY.to_vec(), 0),
    );
    let error = service
        .desfire_key_settings(
            DesfireKeySettingsRequest::new(AID)
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .new_key_settings(0x0E),
        )
        .expect_err("Advanced mode is required");
    assert!(error.to_string().contains("Advanced mode"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");

    let error = service
        .desfire_key_settings(
            DesfireKeySettingsRequest::new(AID)
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .new_key_settings(0x0E)
                .advanced(true),
        )
        .expect_err("confirmation is required");
    assert!(error.to_string().contains("confirmation"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");
}

#[test]
fn change_key_settings_reads_the_new_byte_back() {
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(1)
            .with_key(0, AES_KEY.to_vec(), 0),
    );
    let result = service
        .desfire_key_settings(
            DesfireKeySettingsRequest::new(AID)
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .new_key_settings(0x0E)
                .advanced(true)
                .confirm(true),
        )
        .expect("change key settings");

    assert!(result.changed);
    assert_eq!(result.before.key_settings, 0x0F);
    assert_eq!(result.after.key_settings, 0x0E);
    assert_eq!(transport.tag().key_settings(), 0x0E);
    assert_eq!(command_count(&transport, 0x54), 1);
    assert_eq!(command_count(&transport, 0x45), 2);
    let text = key_settings_text(&result);
    assert!(text.contains("settings changed: 0F -> 0E"), "{text}");
}

#[test]
fn change_key_settings_refuses_a_frozen_configuration() {
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(1)
            .with_key(0, AES_KEY.to_vec(), 0)
            .with_key_settings(0x07),
    );
    let error = service
        .desfire_key_settings(
            DesfireKeySettingsRequest::new(AID)
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .new_key_settings(0x0E)
                .advanced(true)
                .confirm(true),
        )
        .expect_err("the configuration bit is clear");
    assert!(error.to_string().contains("configuration"), "{error}");
    assert_eq!(command_count(&transport, 0x54), 0);
}

#[test]
fn change_key_reauthenticates_and_reads_the_version_back() {
    let path = export_path("aes");
    let _ = std::fs::remove_file(&path);
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(2)
            .with_key(0, AES_KEY.to_vec(), 0)
            .with_key(1, [0x11; 16].to_vec(), 1),
    );
    let new_key = [0x22u8; 16];
    let result = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new(AID, 1, new_key.to_vec())
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .auth_key_number(0)
                .current_key(vec![0x11; 16])
                .new_key_version(3)
                .export_path(&path)
                .advanced(true)
                .confirm(true),
        )
        .expect("change key 1");

    assert_eq!(result.key_number, 1);
    assert_eq!(result.key_version_before, Some(1));
    assert_eq!(result.verified_key_version, Some(3));
    assert_eq!(result.export_path.as_deref(), Some(path.as_path()));
    assert_eq!(transport.tag().key_value(1), Some(&new_key[..]));
    assert_eq!(transport.tag().key_version_of(1), Some(3));
    assert_eq!(command_count(&transport, 0xC4), 1);

    let export = std::fs::read_to_string(&path).expect("export file");
    assert!(export.contains("nfc-foundry/desfire-key/1"), "{export}");
    assert!(
        export.contains("22222222222222222222222222222222"),
        "{export}"
    );
    assert!(export.contains("\"key_version\": 3"), "{export}");
    let mode = std::fs::metadata(&path)
        .expect("export metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the export file is private");
    let _ = std::fs::remove_file(&path);

    let text = key_change_text(&result);
    assert!(text.contains("key 1 · AES · key version 1 -> 3"), "{text}");
    assert!(text.contains("re-authenticated with the new key"), "{text}");
    assert!(
        !text.contains("22222222"),
        "the result text never prints the key"
    );
}

#[test]
fn change_key_refuses_an_unrecorded_new_key_before_any_command() {
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(1)
            .with_key(0, AES_KEY.to_vec(), 0),
    );
    let error = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new(AID, 0, vec![0x33; 16])
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .advanced(true)
                .confirm(true),
        )
        .expect_err("an unrecorded key is refused");
    assert!(error.to_string().contains("cannot be recovered"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");
}

#[test]
fn change_key_needs_the_target_current_key() {
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(2)
            .with_key(0, AES_KEY.to_vec(), 0)
            .with_key(1, [0x11; 16].to_vec(), 1),
    );
    let error = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new(AID, 1, vec![0x22; 16])
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .auth_key_number(0)
                .acknowledge_unrecorded(true)
                .advanced(true)
                .confirm(true),
        )
        .expect_err("the target key value is required");
    assert!(error.to_string().contains("current value"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");
}

#[test]
fn change_key_refuses_a_mismatched_family() {
    let (mut service, transport) = service(
        MockDesfire::new(AES_KEY, AuthKeyType::Aes)
            .with_application(AID)
            .with_max_keys(1)
            .with_key(0, AES_KEY.to_vec(), 0),
    );
    let error = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new(AID, 0, vec![0x33; 24])
                .key(AES_KEY.to_vec())
                .key_type(AuthKeyType::Aes)
                .acknowledge_unrecorded(true)
                .advanced(true)
                .confirm(true),
        )
        .expect_err("24-byte 3K3DES key under an AES session");
    assert!(error.to_string().contains("16-byte AES"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");
}

#[test]
fn change_key_without_a_key_fails_as_authentication_required() {
    let mut service = Service::default();
    let error = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new(AID, 0, vec![0x33; 16])
                .advanced(true)
                .confirm(true),
        )
        .expect_err("the authenticating key is required");
    assert!(
        matches!(error, NfcError::AuthenticationRequired(_)),
        "{error}"
    );
}

#[test]
fn change_des_key_and_read_its_embedded_version_back() {
    let path = export_path("des");
    let _ = std::fs::remove_file(&path);
    let (mut service, transport) = service(
        MockDesfire::new([0u8; 8], AuthKeyType::Des)
            .with_application(AID)
            .with_max_keys(1)
            .with_key_version(0),
    );
    let result = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new(AID, 0, vec![0x33; 8])
                .key(vec![0x00; 8])
                .key_type(AuthKeyType::Des)
                .new_key_version(2)
                .export_path(&path)
                .advanced(true)
                .confirm(true),
        )
        .expect("change the DES key");

    assert_eq!(result.key_version_before, Some(0));
    assert_eq!(result.verified_key_version, Some(2));
    assert_eq!(transport.tag().key_version_of(0), Some(2));
    let stored = transport.tag().key_value(0).expect("stored key").to_vec();
    assert_eq!(stored.len(), 16, "a DES key is stored as a 2K3DES key");
    // Every key byte carries one version bit in its LSB, LSB first from the
    // end. Version 2 has bit 1 set, so the second byte from the end has LSB 1.
    assert_eq!(stored[15] & 1, 0);
    assert_eq!(stored[14] & 1, 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn picc_master_key_change_needs_advanced_confirmation_and_a_recorded_key() {
    let path = export_path("picc");
    let _ = std::fs::remove_file(&path);
    let (mut service, transport) = service(
        MockDesfire::new([0u8; 8], AuthKeyType::Des)
            .with_max_keys(1)
            .with_key_version(0),
    );
    let error = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new([0x00; 3], 0, vec![0x44; 8])
                .key(vec![0x00; 8])
                .key_type(AuthKeyType::Des)
                .advanced(true),
        )
        .expect_err("confirmation is required");
    assert!(error.to_string().contains("confirmation"), "{error}");
    assert!(transport.commands().is_empty());

    let result = service
        .desfire_change_key(
            DesfireKeyChangeRequest::new([0x00; 3], 0, vec![0x44; 8])
                .key(vec![0x00; 8])
                .key_type(AuthKeyType::Des)
                .new_key_version(0)
                .export_path(&path)
                .advanced(true)
                .confirm(true),
        )
        .expect("change the PICC master key");
    assert_eq!(result.aid, [0x00; 3]);
    assert_eq!(result.verified_key_version, Some(0));
    assert_eq!(transport.tag().key_value(0).map(<[u8]>::len), Some(16));
    let _ = std::fs::remove_file(&path);
}
