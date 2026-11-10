//! Property tests for malformed Type 4 containers and NDEF files.

use nfc_type4::Type4CapabilityContainer;
use nfc_type4::cc::NdefFileControl;
use nfc_type4::file::Type4Ndef;
use proptest::prelude::*;

proptest! {
    // Arbitrary capability-container bytes must produce a value or a typed
    // error, never a panic or a slice out of bounds.
    #[test]
    fn capability_container_parse_is_total(bytes in prop::collection::vec(any::<u8>(), 0..=512)) {
        if let Ok(cc) = Type4CapabilityContainer::parse(&bytes) {
            prop_assert_eq!(cc.raw(), bytes.as_slice());
            prop_assert!(cc.ndef_file().is_ok());
        }
    }

    // Arbitrary NDEF file bytes must not panic; a success never reports more
    // file bytes than were supplied.
    #[test]
    fn ndef_file_parse_is_total(
        bytes in prop::collection::vec(any::<u8>(), 0..=512),
        max_size in 5u16..=0x0200,
    ) {
        let control = NdefFileControl {
            file_id: 0xE104,
            max_size,
            read_access: 0,
            write_access: 0,
        };
        if let Ok(ndef) = Type4Ndef::from_control(control, bytes.clone()) {
            prop_assert!(ndef.file_bytes().len() <= bytes.len());
            prop_assert!(ndef.message_bytes <= control.capacity());
        }
    }
}
