use nfc_ndef::template::uri::URI_PREFIXES;
use nfc_ndef::{
    BluetoothLeRecord, LeAddressType, LeRole, NdefMessage, NdefRecord, TextRecord, Tlv, TlvStream,
    Tnf, UriRecord, WifiAuthentication, WifiEncryption, WifiRecord,
};
use proptest::prelude::*;

fn record_strategy() -> impl Strategy<Value = NdefRecord> {
    (
        0u8..5,
        prop::collection::vec(any::<u8>(), 0..20),
        prop::collection::vec(any::<u8>(), 0..10),
        prop::collection::vec(any::<u8>(), 0..300),
    )
        .prop_map(|(kind, record_type, id, payload)| {
            let record_type = if record_type.is_empty() {
                vec![b'X']
            } else {
                record_type
            };
            match kind {
                0 => NdefRecord::new(Tnf::WellKnown, record_type, id, payload),
                1 => NdefRecord::new(Tnf::Media, record_type, id, payload),
                2 => NdefRecord::new(Tnf::AbsoluteUri, record_type, id, payload),
                3 => NdefRecord::new(Tnf::External, record_type, id, payload),
                _ => NdefRecord::new(Tnf::Unknown, Vec::new(), id, payload),
            }
            .expect("generated record is valid")
        })
}

fn message_strategy() -> impl Strategy<Value = NdefMessage> {
    prop::collection::vec(record_strategy(), 1..5)
        .prop_map(|records| NdefMessage::new(records).expect("generated message is valid"))
}

fn tlv_strategy() -> impl Strategy<Value = Tlv> {
    prop_oneof![
        Just(Tlv::Null),
        prop::collection::vec(any::<u8>(), 3..=3).prop_map(Tlv::LockControl),
        prop::collection::vec(any::<u8>(), 3..=3).prop_map(Tlv::MemoryControl),
        prop::collection::vec(any::<u8>(), 0..300).prop_map(Tlv::Proprietary),
        (0x04u8..=0xFCu8, prop::collection::vec(any::<u8>(), 0..300))
            .prop_map(|(tag, value)| Tlv::Unknown { tag, value }),
    ]
}

proptest! {
    #[test]
    fn message_round_trips_through_the_codec(message in message_strategy()) {
        let encoded = message.encode().expect("encode generated message");
        let decoded = NdefMessage::decode(&encoded).expect("decode generated message");
        prop_assert_eq!(decoded, message);
    }

    #[test]
    fn chunked_encoding_round_trips(
        message in message_strategy(),
        chunk in 1usize..64,
    ) {
        let encoded = message.encode_chunked(chunk).expect("chunked encode");
        let decoded = NdefMessage::decode(&encoded).expect("decode chunked message");
        prop_assert_eq!(decoded, message);
    }

    #[test]
    fn arbitrary_bytes_decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(message) = NdefMessage::decode(&bytes) {
            let encoded = message.encode().expect("re-encode decoded message");
            let decoded = NdefMessage::decode(&encoded).expect("decode re-encoded message");
            prop_assert_eq!(decoded, message);
        }
    }

    #[test]
    fn text_records_round_trip(
        text in ".*",
        language in prop_oneof![Just("en"), Just("de"), Just("en-US"), Just("zh-Hant")],
    ) {
        let record = TextRecord::new(text, language).expect("text");
        let encoded = record.to_record().expect("record");
        let decoded = TextRecord::from_record(&encoded).expect("decode");
        prop_assert_eq!(decoded, record);
    }

    #[test]
    fn uri_records_round_trip(
        prefix in 0usize..URI_PREFIXES.len(),
        suffix in "[a-z0-9./_-]{1,40}",
    ) {
        let uri = format!("{}{suffix}", URI_PREFIXES[prefix]);
        let record = UriRecord::new(uri).expect("uri");
        let encoded = record.to_record().expect("record");
        let decoded = UriRecord::from_record(&encoded).expect("decode");
        prop_assert_eq!(decoded, record);
    }

    #[test]
    fn tlv_streams_round_trip(
        tlvs in prop::collection::vec(tlv_strategy(), 0..6),
        terminator in any::<bool>(),
        padding in prop::collection::vec(any::<u8>(), 0..8),
    ) {
        let mut tlvs = tlvs;
        let padding = if terminator {
            tlvs.push(Tlv::Terminator);
            padding
        } else {
            Vec::new()
        };
        let stream = TlvStream::new(tlvs, padding).expect("stream");
        let encoded = stream.encode().expect("encode");
        let decoded = TlvStream::parse(&encoded).expect("parse");
        prop_assert_eq!(decoded, stream);
    }

    #[test]
    fn arbitrary_bytes_tlv_parse_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(stream) = TlvStream::parse(&bytes)
            && let Ok(encoded) = stream.encode()
        {
            let decoded = TlvStream::parse(&encoded).expect("parse re-encoded stream");
            prop_assert_eq!(decoded, stream);
        }
    }

    #[test]
    fn unknown_tlvs_are_preserved(
        tag in 0x04u8..=0xFCu8,
        value in prop::collection::vec(any::<u8>(), 0..300),
    ) {
        let tlv = Tlv::Unknown { tag, value };
        let stream = TlvStream::new(
            vec![Tlv::Null, tlv.clone(), Tlv::Terminator],
            vec![0x00, 0xFF],
        )
        .expect("stream");
        let encoded = stream.encode().expect("encode");
        let decoded = TlvStream::parse(&encoded).expect("parse");
        prop_assert_eq!(decoded.tlvs(), stream.tlvs());
        prop_assert_eq!(decoded.padding(), stream.padding());
    }

    #[test]
    fn ndef_tlvs_round_trip(message in message_strategy()) {
        let tlv = Tlv::ndef(&message).expect("ndef tlv");
        let encoded = tlv.encode().expect("encode");
        let (decoded, _) = Tlv::parse_one(&encoded).expect("parse");
        let decoded_message = decoded.as_ndef_message().expect("message").expect("present");
        prop_assert_eq!(decoded_message, message);
    }

    #[test]
    fn wifi_credentials_round_trip(
        ssid in prop::collection::vec(any::<u8>(), 1..32),
        key in "[A-Za-z0-9]{8,63}",
    ) {
        let record = WifiRecord::new(
            ssid,
            WifiAuthentication::WPA2_PERSONAL,
            WifiEncryption::AES,
        )
        .expect("wifi")
        .with_network_key(key.into_bytes())
        .expect("network key");
        let encoded = record.to_record().expect("record");
        let decoded = WifiRecord::from_record(&encoded).expect("decode");
        prop_assert_eq!(decoded, record);
    }

    #[test]
    fn bluetooth_le_records_round_trip(
        address in any::<[u8; 6]>(),
        random in any::<bool>(),
        role in 0u8..4,
        name in "[ -~]{0,32}",
        appearance in any::<u16>(),
        flags in any::<u8>(),
    ) {
        let role = match role {
            0 => LeRole::PeripheralOnly,
            1 => LeRole::CentralOnly,
            2 => LeRole::PeripheralPreferred,
            _ => LeRole::CentralPreferred,
        };
        let address_type = if random {
            LeAddressType::Random
        } else {
            LeAddressType::Public
        };
        let record = BluetoothLeRecord {
            address: Some((address, address_type)),
            role: Some(role),
            name: Some(name),
            appearance: Some(appearance),
            flags: Some(flags),
            extra: Vec::new(),
        };
        let encoded = record.to_record().expect("record");
        let decoded = BluetoothLeRecord::from_record(&encoded).expect("decode");
        prop_assert_eq!(decoded, record);
    }
}
