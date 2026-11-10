use nfc_ndef::{
    BluetoothLeRecord, DppBootstrap, LeAddressType, LeRole, NdefMessage, NdefRecord, SmartPoster,
    SmartPosterAction, TextEncoding, TextRecord, Tlv, TlvStream, Tnf, UriRecord, VCard,
    WifiAuthentication, WifiEncryption, WifiRecord, ndef_capacity,
};

fn text_message() -> NdefMessage {
    let text = TextRecord::new("Hello, world!", "en").expect("valid text record");
    NdefMessage::single(text.to_record().expect("text record")).expect("single-record message")
}

#[test]
fn empty_record_has_the_minimal_encoding() {
    let message = NdefMessage::single(NdefRecord::empty()).expect("empty record");
    assert_eq!(message.encode().expect("encode"), [0xD0, 0x00, 0x00]);
    assert_eq!(
        NdefMessage::decode(&[0xD0, 0x00, 0x00]).expect("decode"),
        message
    );
}

#[test]
fn text_record_matches_the_rtd_example() {
    let encoded = text_message().encode().expect("encode");
    assert_eq!(
        encoded,
        [
            0xD1, 0x01, 0x10, b'T', 0x02, b'e', b'n', b'H', b'e', b'l', b'l', b'o', b',', b' ',
            b'w', b'o', b'r', b'l', b'd', b'!',
        ]
    );
    assert_eq!(
        TextRecord::from_record(&NdefRecord::well_known(b"T", &encoded[4..]).expect("record"))
            .expect("text"),
        TextRecord::new("Hello, world!", "en").expect("text")
    );
}

#[test]
fn normal_record_layout_is_accepted() {
    let bytes = [0xC1, 0x01, 0x00, 0x00, 0x00, 0x03, b'T', b'a', b'b', b'c'];
    let message = NdefMessage::decode(&bytes).expect("normal layout");
    assert_eq!(message.records().len(), 1);
    assert_eq!(message.records()[0].payload(), b"abc");
    let canonical = message.encode().expect("encode");
    assert_eq!(NdefMessage::decode(&canonical).expect("decode"), message);
}

#[test]
fn id_field_round_trips() {
    let record = NdefRecord::new(Tnf::WellKnown, b"T", b"id-1", b"hello").expect("record");
    let message = NdefMessage::single(record).expect("message");
    let encoded = message.encode().expect("encode");
    assert_eq!(encoded[0] & 0x08, 0x08);
    assert_eq!(NdefMessage::decode(&encoded).expect("decode"), message);
    assert_eq!(message.records()[0].id(), b"id-1");
}

#[test]
fn long_payload_round_trips() {
    let record = NdefRecord::unknown(vec![0x5A; 300]).expect("record");
    let message = NdefMessage::single(record).expect("message");
    let encoded = message.encode().expect("encode");
    assert_eq!(&encoded[2..6], &[0x00, 0x00, 0x01, 0x2C]);
    assert_eq!(NdefMessage::decode(&encoded).expect("decode"), message);
}

#[test]
fn chunked_encoding_round_trips() {
    let message =
        NdefMessage::single(NdefRecord::well_known(b"T", vec![0x11; 600]).expect("record"))
            .expect("message");
    let encoded = message.encode_chunked(200).expect("chunked encode");
    assert_eq!(NdefMessage::decode(&encoded).expect("decode"), message);
}

#[test]
fn chunked_initial_record_may_carry_no_payload() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0xB1, 0x01, 0x00, b'T']);
    bytes.extend_from_slice(&[0x56, 0x00, 0x03, b'a', b'b', b'c']);
    let message = NdefMessage::decode(&bytes).expect("chunked message");
    assert_eq!(message.records().len(), 1);
    assert_eq!(message.records()[0].tnf(), Tnf::WellKnown);
    assert_eq!(message.records()[0].record_type(), b"T");
    assert_eq!(message.records()[0].payload(), b"abc");
}

#[test]
fn reserved_tnf_is_rejected() {
    assert!(NdefMessage::decode(&[0xD7, 0x00, 0x00]).is_err());
}

#[test]
fn missing_message_begin_is_rejected() {
    assert!(NdefMessage::decode(&[0x50, 0x00, 0x00]).is_err());
}

#[test]
fn missing_message_end_is_rejected() {
    assert!(NdefMessage::decode(&[0x90, 0x00, 0x00]).is_err());
}

#[test]
fn message_begin_on_later_record_is_rejected() {
    let bytes = [0x90, 0x00, 0x00, 0xD0, 0x00, 0x00];
    assert!(NdefMessage::decode(&bytes).is_err());
}

#[test]
fn trailing_bytes_after_message_end_are_reported() {
    let bytes = [0xD0, 0x00, 0x00, 0xAA];
    assert!(NdefMessage::decode(&bytes).is_err());
    let (message, consumed) = NdefMessage::decode_prefix(&bytes).expect("prefix");
    assert_eq!(consumed, 3);
    assert_eq!(
        message,
        NdefMessage::single(NdefRecord::empty()).expect("empty")
    );
}

#[test]
fn unterminated_chunk_is_rejected() {
    let bytes = [0xB1, 0x01, 0x00, b'T'];
    assert!(NdefMessage::decode(&bytes).is_err());
}

#[test]
fn new_record_inside_a_chunk_is_rejected() {
    let bytes = [0xB1, 0x01, 0x00, b'T', 0xD0, 0x00, 0x00];
    assert!(NdefMessage::decode(&bytes).is_err());
}

#[test]
fn chunk_continuation_without_initial_chunk_is_rejected() {
    assert!(NdefMessage::decode(&[0x56, 0x00, 0x00]).is_err());
}

#[test]
fn chunked_empty_record_with_payload_is_rejected() {
    let bytes = [0xB0, 0x00, 0x00, 0x56, 0x00, 0x01, 0xAA];
    assert!(NdefMessage::decode(&bytes).is_err());
}

#[test]
fn chunk_continuation_with_id_is_rejected() {
    let bytes = [0xB1, 0x01, 0x00, b'T', 0x5E, 0x00, 0x00, 0x00, 0x00];
    assert!(NdefMessage::decode(&bytes).is_err());
}

#[test]
fn empty_record_with_payload_is_rejected() {
    let bytes = [0xD0, 0x00, 0x01, 0xAA];
    assert!(NdefMessage::decode(&bytes).is_err());
}

#[test]
fn truncated_payload_is_rejected() {
    let bytes = [0xD1, 0x01, 0x04, b'T', 0xAA];
    assert!(NdefMessage::decode(&bytes).is_err());
}

#[test]
fn text_utf16_round_trips() {
    let text = TextRecord::with_encoding("Grüße", "de", TextEncoding::Utf16).expect("text");
    let record = text.to_record().expect("record");
    assert_eq!(record.payload()[0], 0x82);
    assert_eq!(&record.payload()[3..5], &[0xFE, 0xFF]);
    assert_eq!(TextRecord::from_record(&record).expect("decode"), text);
}

#[test]
fn text_rejects_empty_language() {
    let record = NdefRecord::well_known(b"T", [0x00, b'H']).expect("record");
    assert!(TextRecord::from_record(&record).is_err());
}

#[test]
fn text_rejects_odd_utf16_payload() {
    let record = NdefRecord::well_known(b"T", [0x82, b'e', b'n', 0x00]).expect("record");
    assert!(TextRecord::from_record(&record).is_err());
}

#[test]
fn uri_uses_the_longest_prefix() {
    let record = UriRecord::new("https://www.example.com/x")
        .expect("uri")
        .to_record()
        .expect("record");
    assert_eq!(record.payload()[0], 0x02);
    assert_eq!(&record.payload()[1..], b"example.com/x");
}

#[test]
fn uri_prefixes_round_trip() {
    for prefix in nfc_ndef::template::uri::URI_PREFIXES {
        let uri = format!("{prefix}example.org");
        let decoded = UriRecord::from_record(
            &UriRecord::new(&uri)
                .expect("uri")
                .to_record()
                .expect("record"),
        )
        .expect("decode");
        assert_eq!(decoded.uri, uri);
    }
}

#[test]
fn uri_reserved_identifier_code_is_rejected() {
    let record = NdefRecord::well_known(b"U", [0x24, b'x']).expect("record");
    assert!(UriRecord::from_record(&record).is_err());
}

#[test]
fn uri_empty_payload_is_rejected() {
    let record = NdefRecord::well_known(b"U", [0x00]).expect("record");
    assert!(UriRecord::from_record(&record).is_err());
}

#[test]
fn smart_poster_round_trips() {
    let mut poster = SmartPoster::new("https://example.com/")
        .expect("poster")
        .with_title(TextRecord::new("Example", "en").expect("title"))
        .with_title(TextRecord::new("Beispiel", "de").expect("title"))
        .with_action(SmartPosterAction::Save);
    poster
        .additional
        .push(NdefRecord::external("example.com:extra", b"data").expect("external record"));
    let record = poster.to_record().expect("record");
    assert_eq!(SmartPoster::from_record(&record).expect("decode"), poster);
}

#[test]
fn smart_poster_requires_a_uri() {
    let nested = NdefMessage::single(
        TextRecord::new("Only text", "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message");
    let record = NdefRecord::well_known(b"Sp", nested.encode().expect("encode")).expect("record");
    assert!(SmartPoster::from_record(&record).is_err());
}

#[test]
fn vcard_round_trips() {
    let card = VCard {
        formatted_name: "Doe; John, Jr.".to_owned(),
        organization: Some("Example Org".to_owned()),
        phone: Some("+123456789".to_owned()),
        email: Some("john@example.com".to_owned()),
        url: Some("https://example.com".to_owned()),
    };
    let record = card.to_record().expect("record");
    assert!(record.payload().windows(2).any(|window| window == b"\r\n"));
    assert_eq!(VCard::from_record(&record).expect("decode"), card);
}

#[test]
fn vcard_encodes_newlines_once() {
    let card = VCard {
        formatted_name: "Doe\r\nJohn".to_owned(),
        ..VCard::default()
    };
    let record = card.to_record().expect("record");
    let text = std::str::from_utf8(record.payload()).expect("utf-8");
    assert!(text.contains("FN:Doe\\nJohn\r\n"));
    let decoded = VCard::from_record(&record).expect("decode");
    assert_eq!(decoded.formatted_name, "Doe\nJohn");
}

#[test]
fn vcard_rejects_missing_version() {
    let payload = b"BEGIN:VCARD\r\nFN:John\r\nEND:VCARD\r\n";
    let record = NdefRecord::media(b"text/vcard", &payload[..]).expect("record");
    assert!(VCard::from_record(&record).is_err());
}

#[test]
fn vcard_rejects_wrong_version() {
    let payload = b"BEGIN:VCARD\r\nVERSION:4.0\r\nFN:John\r\nEND:VCARD\r\n";
    let record = NdefRecord::media(b"text/vcard", &payload[..]).expect("record");
    assert!(VCard::from_record(&record).is_err());
}

#[test]
fn wifi_open_network_round_trips() {
    let wifi = WifiRecord::new(
        b"Guest".to_vec(),
        WifiAuthentication::OPEN,
        WifiEncryption::NONE,
    )
    .expect("wifi");
    let record = wifi.to_record().expect("record");
    assert_eq!(&record.payload()[..2], &[0x10, 0x0E]);
    assert_eq!(WifiRecord::from_record(&record).expect("decode"), wifi);
}

#[test]
fn wifi_personal_network_round_trips() {
    let wifi = WifiRecord::new(
        b"Home".to_vec(),
        WifiAuthentication::WPA2_PERSONAL,
        WifiEncryption::AES,
    )
    .expect("wifi")
    .with_network_key(b"password123".to_vec())
    .expect("key")
    .with_mac_address([1, 2, 3, 4, 5, 6])
    .expect("mac");
    let record = wifi.to_record().expect("record");
    assert_eq!(WifiRecord::from_record(&record).expect("decode"), wifi);
}

#[test]
fn wifi_unknown_attributes_are_preserved() {
    let mut wifi = WifiRecord::new(
        b"Home".to_vec(),
        WifiAuthentication::WPA2_PERSONAL,
        WifiEncryption::AES,
    )
    .expect("wifi")
    .with_network_key(b"password123".to_vec())
    .expect("key");
    wifi.credential_attributes.push((0x1061, vec![0x01]));
    wifi.outer_attributes.push((0x1049, vec![0x00, 0x37, 0x2A]));
    let record = wifi.to_record().expect("record");
    assert_eq!(WifiRecord::from_record(&record).expect("decode"), wifi);
}

#[test]
fn wifi_rejects_open_network_with_key() {
    let mut wifi = WifiRecord::new(
        b"Guest".to_vec(),
        WifiAuthentication::OPEN,
        WifiEncryption::NONE,
    )
    .expect("wifi");
    wifi.network_key = Some(b"unexpected".to_vec());
    assert!(wifi.validate().is_err());
}

#[test]
fn wifi_rejects_payload_without_credential() {
    let payload = [0x10, 0x45, 0x00, 0x01, b'X'];
    let record = NdefRecord::media(b"application/vnd.wfa.wsc", payload).expect("record");
    assert!(WifiRecord::from_record(&record).is_err());
}

#[test]
fn bluetooth_le_round_trips() {
    let ble = BluetoothLeRecord {
        address: Some(([1, 2, 3, 4, 5, 6], LeAddressType::Random)),
        role: Some(LeRole::PeripheralPreferred),
        name: Some("Tag".to_owned()),
        appearance: Some(0x0080),
        flags: Some(0x06),
        extra: vec![(0x03, vec![0x0F, 0x18])],
    };
    let record = ble.to_record().expect("record");
    assert_eq!(
        BluetoothLeRecord::from_record(&record).expect("decode"),
        ble
    );
}

#[test]
fn bluetooth_le_address_is_little_endian_on_the_wire() {
    let ble = BluetoothLeRecord {
        address: Some(([1, 2, 3, 4, 5, 6], LeAddressType::Public)),
        ..BluetoothLeRecord::default()
    };
    let record = ble.to_record().expect("record");
    let payload = record.payload();
    assert_eq!(&payload[..2], &[0x08, 0x1B]);
    assert_eq!(&payload[2..8], &[6, 5, 4, 3, 2, 1]);
    assert_eq!(payload[8], 0);
}

#[test]
fn bluetooth_le_rejects_bad_address_length() {
    let record =
        NdefRecord::media(b"application/vnd.bluetooth.le.oob", [0x02, 0x1B, 0x00]).expect("record");
    assert!(BluetoothLeRecord::from_record(&record).is_err());
}

#[test]
fn bluetooth_le_rejects_bad_role_length() {
    let record = NdefRecord::media(
        b"application/vnd.bluetooth.le.oob",
        [0x03, 0x1C, 0x00, 0x00],
    )
    .expect("record");
    assert!(BluetoothLeRecord::from_record(&record).is_err());
}

#[test]
fn dpp_bootstrap_round_trips_in_both_carriers() {
    let uri = "DPP:C:81/1,115/36;K:MDkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDIgADM2206avxHJaHXgLMkq/24e0rsrfMP9K1Tm8gx+ovP0I=;;";
    let bootstrap = DppBootstrap::new(uri).expect("bootstrap");
    let uri_record = bootstrap.to_uri_record().expect("uri record");
    assert_eq!(
        DppBootstrap::from_uri_record(&uri_record).expect("decode"),
        bootstrap
    );
    let media_record = bootstrap.to_media_record().expect("media record");
    assert_eq!(media_record.payload()[0], 0);
    assert_eq!(
        DppBootstrap::from_media_record(&media_record).expect("decode"),
        bootstrap
    );
}

#[test]
fn dpp_bootstrap_requires_a_public_key() {
    assert!(DppBootstrap::new("DPP:C:81/1;;").is_err());
    assert!(DppBootstrap::new("DPP:C:81/1;K:abc;").is_err());
    assert!(DppBootstrap::new("DPP:C:81/1;K:abc;;\n").is_err());
}

#[test]
fn tlv_short_and_long_lengths_round_trip() {
    for length in [0usize, 1, 254, 255, 256, 65_535] {
        let tlv = Tlv::Proprietary(vec![0xAA; length]);
        let encoded = tlv.encode().expect("encode");
        if length < 255 {
            assert_eq!(encoded[1], length as u8);
        } else {
            assert_eq!(encoded[1], 0xFF);
            assert_eq!(&encoded[2..4], &(length as u16).to_be_bytes());
        }
        let (decoded, consumed) = Tlv::parse_one(&encoded).expect("decode");
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded, tlv);
    }
}

#[test]
fn null_and_terminator_tlvs_round_trip() {
    assert_eq!(Tlv::Null.encode().expect("encode"), [0x00]);
    assert_eq!(Tlv::Terminator.encode().expect("encode"), [0xFE]);
}

#[test]
fn unknown_tlv_is_preserved_exactly() {
    let bytes = [0x42, 0x03, 0xAA, 0xBB, 0xCC, 0xFE, 0x00, 0x00];
    let stream = TlvStream::parse(&bytes).expect("parse");
    assert_eq!(stream.tlvs().len(), 2);
    assert_eq!(stream.padding(), [0x00, 0x00]);
    assert_eq!(stream.encode().expect("encode"), bytes);
}

#[test]
fn lock_and_memory_control_tlvs_round_trip() {
    let tlvs = vec![
        Tlv::LockControl(vec![0x04, 0x03, 0x00]),
        Tlv::MemoryControl(vec![0x05, 0x00, 0x03]),
        Tlv::Terminator,
    ];
    let stream = TlvStream::new(tlvs.clone(), Vec::new()).expect("stream");
    let encoded = stream.encode().expect("encode");
    assert_eq!(TlvStream::parse(&encoded).expect("parse"), stream);
}

#[test]
fn tlv_truncated_length_is_rejected() {
    assert!(TlvStream::parse(&[0x03, 0xFF, 0x00]).is_err());
}

#[test]
fn tlv_truncated_value_is_rejected() {
    assert!(TlvStream::parse(&[0x42, 0x04, 0x01]).is_err());
}

#[test]
fn tlv_0xff_type_is_rejected() {
    assert!(TlvStream::parse(&[0xFF, 0x00]).is_err());
}

#[test]
fn control_tlv_lengths_are_checked() {
    assert!(TlvStream::parse(&[0x01, 0x02, 0x00, 0x00]).is_err());
    assert!(TlvStream::parse(&[0x02, 0x04, 0x00, 0x00, 0x00, 0x00]).is_err());
}

#[test]
fn ndef_tlv_round_trips_through_the_message_codec() {
    let tlv = Tlv::ndef(&text_message()).expect("ndef tlv");
    let encoded = tlv.encode().expect("encode");
    let (decoded, _) = Tlv::parse_one(&encoded).expect("decode");
    assert_eq!(
        decoded
            .as_ndef_message()
            .expect("message")
            .expect("present"),
        text_message()
    );
}

#[test]
fn ndef_capacity_uses_the_shortest_tlv_form() {
    assert_eq!(ndef_capacity(0), 0);
    assert_eq!(ndef_capacity(1), 0);
    assert_eq!(ndef_capacity(2), 0);
    assert_eq!(ndef_capacity(3), 0);
    assert_eq!(ndef_capacity(4), 1);
    assert_eq!(ndef_capacity(257), 254);
    assert_eq!(ndef_capacity(258), 254);
    assert_eq!(ndef_capacity(259), 254);
    assert_eq!(ndef_capacity(260), 255);
    assert_eq!(ndef_capacity(65_540), 65_535);
    assert_eq!(ndef_capacity(100_000), 65_535);
}

#[test]
fn message_capacity_check_reports_the_shortfall() {
    let message = text_message();
    let needed = message.encoded_len().expect("length");
    message.fits_in(needed).expect("fits exactly");
    let error = message.fits_in(needed - 1).expect_err("must not fit");
    assert!(error.to_string().contains("only"));
}
