//! DESFire NDEF replace and FormatPICC workflows on the scripted card.

use std::sync::Arc;

use nfc_app::Service;
use nfc_app::classic::BackupStore;
use nfc_app::desfire::{
    AuthKeyType, DesfireFormatPiccRequest, DesfireNdefFormatRequest, DesfireNdefReplaceRequest,
    DesfireVerifyKeyRequest, NDEF_APPLICATION_AID, NDEF_CAPABILITY_CONTAINER, NDEF_DATA_FILE,
};
use nfc_app::type4::Type4NdefWrite;
use nfc_core::CancelToken;
use nfc_desfire::mock::{MockDesfire, MockDesfireTransport};
use nfc_ndef::{NdefMessage, TextRecord};
use nfc_type4::NdefState;

const PICC_KEY: [u8; 8] = [0x01; 8];
const DEFAULT_AID: [u8; 3] = [0xF0, 0x0D, 0x01];

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

fn format_request() -> DesfireNdefFormatRequest {
    DesfireNdefFormatRequest::new(PICC_KEY.to_vec())
        .key_number(0)
        .advanced(true)
        .confirm(true)
}

fn replace_request() -> DesfireNdefReplaceRequest {
    DesfireNdefReplaceRequest::new(PICC_KEY.to_vec())
        .key_number(0)
        .advanced(true)
        .confirm(true)
}

fn format_picc_request() -> DesfireFormatPiccRequest {
    DesfireFormatPiccRequest::new(PICC_KEY.to_vec())
        .key_number(0)
        .advanced(true)
        .confirm(true)
}

fn message(text: &str) -> NdefMessage {
    NdefMessage::single(
        TextRecord::new(text, "en")
            .expect("text record")
            .to_record()
            .expect("record"),
    )
    .expect("message")
}

#[test]
fn replace_needs_advanced_and_confirmation() {
    let (mut service, transport) = service(MockDesfire::new(PICC_KEY, AuthKeyType::Des));

    let error = service
        .desfire_ndef_replace(DesfireNdefReplaceRequest::new(PICC_KEY.to_vec()))
        .expect_err("Advanced mode is required");
    assert!(error.to_string().contains("Advanced mode"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");

    let error = service
        .desfire_ndef_replace(DesfireNdefReplaceRequest::new(PICC_KEY.to_vec()).advanced(true))
        .expect_err("confirmation is required");
    assert!(error.to_string().contains("confirmation"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");
}

#[test]
fn replace_refuses_a_card_without_the_ndef_application() {
    let (mut service, transport) =
        service(MockDesfire::new(PICC_KEY, AuthKeyType::Des).with_application(DEFAULT_AID));
    let error = service
        .desfire_ndef_replace(replace_request())
        .expect_err("the NDEF application is absent");
    assert!(error.to_string().contains("not present"), "{error}");
    assert_eq!(command_count(&transport, 0xDA), 0, "no delete command");
    assert_eq!(command_count(&transport, 0xCA), 0, "no create command");
}

#[test]
fn replace_reads_the_previous_message_then_reprovisions() {
    let (mut service, transport) = service(MockDesfire::new(PICC_KEY, AuthKeyType::Des));
    service
        .desfire_ndef_format(format_request())
        .expect("initial format");
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let before = message("before replace");
    service
        .type4_ndef_write(Type4NdefWrite::new(before.clone(), &backup))
        .expect("seed the NDEF message");

    let replaced = service
        .desfire_ndef_replace(replace_request())
        .expect("replace the NDEF application");
    assert_eq!(replaced.previous.state, NdefState::Message(before));
    assert_eq!(replaced.cc.raw(), NDEF_CAPABILITY_CONTAINER.as_slice());
    assert_eq!(replaced.ndef.state, NdefState::Empty);
    assert_eq!(command_count(&transport, 0xCA), 2, "one create per format");
    assert_eq!(command_count(&transport, 0xDA), 1, "one validated delete");
    assert_eq!(command_count(&transport, 0xFC), 0, "no FormatPICC");

    let read = service.type4_read(CancelToken::new()).expect("Type 4 read");
    assert_eq!(read.ndef.state, NdefState::Empty);
    let inventory = service
        .desfire_inventory(CancelToken::new())
        .expect("inventory");
    assert!(
        inventory
            .applications
            .iter()
            .any(|application| application.aid == NDEF_APPLICATION_AID)
    );
}

#[test]
fn replace_aborts_before_delete_when_the_ndef_file_is_unreadable() {
    let (mut service, transport) = service(MockDesfire::new(PICC_KEY, AuthKeyType::Des));
    service
        .desfire_ndef_format(format_request())
        .expect("initial format");
    {
        let mut tag = transport.tag();
        let mut data = tag
            .file_data_in(NDEF_APPLICATION_AID, NDEF_DATA_FILE)
            .expect("NDEF file")
            .to_vec();
        data[0] = 0x01;
        data[1] = 0x00;
        tag.set_file_data(NDEF_APPLICATION_AID, NDEF_DATA_FILE, &data);
    }
    let error = service
        .desfire_ndef_replace(replace_request())
        .expect_err("the previous NDEF file is invalid");
    assert!(error.to_string().contains("nothing was deleted"), "{error}");
    assert_eq!(command_count(&transport, 0xDA), 0, "no delete command");
    let inventory = service
        .desfire_inventory(CancelToken::new())
        .expect("inventory");
    assert!(
        inventory
            .applications
            .iter()
            .any(|application| application.aid == NDEF_APPLICATION_AID)
    );
}

#[test]
fn replace_does_not_delete_an_application_it_did_not_create() {
    let (mut service, transport) = service(
        MockDesfire::new(PICC_KEY, AuthKeyType::Des).with_application(NDEF_APPLICATION_AID),
    );
    let error = service
        .desfire_ndef_replace(replace_request())
        .expect_err("the layout does not match AN11004");
    assert!(
        error
            .to_string()
            .contains("files that this product did not create"),
        "{error}"
    );
    assert_eq!(command_count(&transport, 0xDA), 0, "no delete command");
}

#[test]
fn replace_reports_a_failed_reprovision_after_the_delete() {
    let (mut service, _transport) =
        service(MockDesfire::new(PICC_KEY, AuthKeyType::Des).with_free_memory(300));
    service
        .desfire_ndef_format(format_request())
        .expect("initial format fits");
    let error = service
        .desfire_ndef_replace(replace_request())
        .expect_err("the second copy does not fit");
    assert!(
        error
            .to_string()
            .contains("was deleted and provisioning failed"),
        "{error}"
    );
    let inventory = service
        .desfire_inventory(CancelToken::new())
        .expect("inventory");
    assert!(
        !inventory
            .applications
            .iter()
            .any(|application| application.aid == NDEF_APPLICATION_AID)
    );
}

#[test]
fn format_refuses_an_existing_ndef_application() {
    let (mut service, transport) = service(MockDesfire::new(PICC_KEY, AuthKeyType::Des));
    service
        .desfire_ndef_format(format_request())
        .expect("first format");
    let error = service
        .desfire_ndef_format(format_request())
        .expect_err("the application already exists");
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(command_count(&transport, 0xCA), 1, "no second create");
}

#[test]
fn format_picc_needs_advanced_and_confirmation() {
    let (mut service, transport) = service(MockDesfire::new(PICC_KEY, AuthKeyType::Des));

    let error = service
        .desfire_format_picc(DesfireFormatPiccRequest::new(PICC_KEY.to_vec()))
        .expect_err("Advanced mode is required");
    assert!(error.to_string().contains("Advanced mode"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");

    let error = service
        .desfire_format_picc(DesfireFormatPiccRequest::new(PICC_KEY.to_vec()).advanced(true))
        .expect_err("confirmation is required");
    assert!(error.to_string().contains("confirmation"), "{error}");
    assert!(transport.commands().is_empty(), "no card command");
}

#[test]
fn format_picc_refuses_a_wrong_key_without_erasing() {
    let (mut service, transport) =
        service(MockDesfire::new(PICC_KEY, AuthKeyType::Des).with_application(DEFAULT_AID));
    let error = service
        .desfire_format_picc(
            DesfireFormatPiccRequest::new(vec![0xFF; 8])
                .advanced(true)
                .confirm(true),
        )
        .expect_err("the wrong PICC key is rejected");
    assert!(error.to_string().contains("authentication"), "{error}");
    assert_eq!(command_count(&transport, 0xFC), 0, "no FormatPICC");
    assert_eq!(transport.tag().application_ids(), vec![DEFAULT_AID]);
}

#[test]
fn format_picc_erases_every_application_and_resets_the_picc_key() {
    let (mut service, transport) = service(
        MockDesfire::new(PICC_KEY, AuthKeyType::Des)
            .with_application(DEFAULT_AID)
            .with_application([0xF0, 0x0D, 0x02])
            .with_max_keys(3)
            .with_key_settings(0x0B),
    );
    let result = service
        .desfire_format_picc(format_picc_request())
        .expect("format the PICC");
    assert_eq!(
        result.applications_before,
        vec![DEFAULT_AID, [0xF0, 0x0D, 0x02]]
    );
    assert!(result.applications_after.is_empty());
    assert_eq!(result.free_memory, 8192);
    assert!(result.card.contains("DESFire"), "{}", result.card);
    assert_eq!(command_count(&transport, 0xFC), 1);

    let inventory = service
        .desfire_inventory(CancelToken::new())
        .expect("inventory");
    assert!(inventory.applications.is_empty());
    let factory = service
        .desfire_verify_key(
            CancelToken::new(),
            DesfireVerifyKeyRequest::new([0x00; 3], vec![0x00; 8]),
        )
        .expect("the PICC key is back to the factory value");
    assert_eq!(factory.key_type, AuthKeyType::Des);
}
