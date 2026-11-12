//! DESFire verification workflows on the scripted card.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::desfire::{
    AuthKeyType, DesfireUidRequest, DesfireVerifyKeyRequest, IntegrityCheck, card_uid_text,
    key_verification_text,
};
use nfc_core::{CancelToken, NfcError};
use nfc_desfire::mock::{MockDesfire, MockDesfireTransport};

const AID: [u8; 3] = [0xF0, 0x0D, 0x01];

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

#[test]
fn verify_key_reports_the_legacy_session_and_key_version() {
    let (mut service, transport) =
        service(MockDesfire::new([0u8; 8], AuthKeyType::Des).with_key_version(0x03));
    let result = service
        .desfire_verify_key(
            CancelToken::new(),
            DesfireVerifyKeyRequest::new(AID, vec![0u8; 8]),
        )
        .expect("verify the legacy key");

    assert_eq!(result.aid, AID);
    assert_eq!(result.key_number, 0);
    assert_eq!(result.key_type, AuthKeyType::Des);
    assert!(!result.secure_messaging);
    assert_eq!(result.key_version, Some(3));
    let text = key_verification_text(&result);
    assert!(text.contains("session: established"), "{text}");
    assert!(text.contains("legacy D40 channel"), "{text}");
    assert!(text.contains("key version: 3"), "{text}");

    assert_eq!(command_count(&transport, 0x5A), 1, "select the application");
    assert_eq!(command_count(&transport, 0x0A), 1, "authenticate once");
    assert_eq!(command_count(&transport, 0x64), 1, "read the key version");
    for file_command in [0xBD, 0x6C, 0xBB, 0xF5] {
        assert_eq!(
            command_count(&transport, file_command),
            0,
            "verification must not read file data"
        );
    }
}

#[test]
fn verify_key_reports_aes_secure_messaging_and_verifies_the_response_mac() {
    let (mut service, _transport) =
        service(MockDesfire::new([0u8; 16], AuthKeyType::Aes).with_key_version(0x02));
    let result = service
        .desfire_verify_key(
            CancelToken::new(),
            DesfireVerifyKeyRequest::new(AID, vec![0u8; 16]).key_type(AuthKeyType::Aes),
        )
        .expect("verify the AES key");

    assert_eq!(result.key_type, AuthKeyType::Aes);
    assert!(result.secure_messaging);
    assert_eq!(result.key_version, Some(2));
    let text = key_verification_text(&result);
    assert!(text.contains("CMAC secure messaging"), "{text}");
    assert!(text.contains("key version: 2"), "{text}");
}

#[test]
fn wrong_key_fails_once_without_retry() {
    let (mut service, transport) = service(MockDesfire::new([0u8; 8], AuthKeyType::Des));
    let error = service
        .desfire_verify_key(
            CancelToken::new(),
            DesfireVerifyKeyRequest::new(AID, vec![0xFF; 8]),
        )
        .expect_err("the key is wrong");
    assert!(matches!(error, NfcError::VerificationFailed(_)), "{error}");
    assert_eq!(command_count(&transport, 0x5A), 1);
    assert_eq!(command_count(&transport, 0x0A), 1, "exactly one attempt");
    assert_eq!(command_count(&transport, 0xAF), 1, "one final challenge");
    assert_eq!(command_count(&transport, 0x64), 0);
}

#[test]
fn uid_read_reports_the_verified_ev1_crc() {
    let uid = [0x04, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01];
    let (mut service, transport) =
        service(MockDesfire::new([0u8; 16], AuthKeyType::Aes).with_uid(uid));
    let result = service
        .desfire_card_uid(
            CancelToken::new(),
            DesfireUidRequest::new(AID, vec![0u8; 16]).key_type(AuthKeyType::Aes),
        )
        .expect("read the authenticated UID");

    assert_eq!(result.uid, uid);
    assert_eq!(result.key_type, AuthKeyType::Aes);
    assert_eq!(result.integrity, IntegrityCheck::Ev1Crc32);
    let text = card_uid_text(&result);
    assert!(text.contains("uid: 04 AA BB CC DD EE 01"), "{text}");
    assert!(text.contains("response: EV1 CRC32 verified"), "{text}");
    assert_eq!(command_count(&transport, 0x51), 1);
}

#[test]
fn uid_read_rejects_a_corrupt_ciphertext() {
    let (mut service, _transport) =
        service(MockDesfire::new([0u8; 16], AuthKeyType::Aes).with_corrupt_uid());
    let error = service
        .desfire_card_uid(
            CancelToken::new(),
            DesfireUidRequest::new(AID, vec![0u8; 16]).key_type(AuthKeyType::Aes),
        )
        .expect_err("the UID ciphertext is wrong");
    assert!(matches!(error, NfcError::VerificationFailed(_)), "{error}");
    assert!(
        error.to_string().contains("enciphered response"),
        "the failure must name the enciphered response check: {error}"
    );
}

#[test]
fn verification_rejects_a_bad_key_length_before_reader_io() {
    let (mut service, transport) = service(MockDesfire::default());
    let error = service
        .desfire_verify_key(
            CancelToken::new(),
            DesfireVerifyKeyRequest::new(AID, vec![0u8; 7]),
        )
        .expect_err("a 7-byte key is invalid");
    assert!(error.to_string().contains("8-byte DES"), "{error}");
    assert!(
        transport.commands().is_empty(),
        "no reader command may run for invalid input"
    );
}
