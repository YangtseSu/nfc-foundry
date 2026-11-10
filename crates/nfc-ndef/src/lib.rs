//! Hardware-free NDEF core: messages, records, TLVs, templates, validation.
//!
//! Family adapters (`nfc-type2`, `nfc-classic-ndef`, `nfc-type4`) map card
//! memory to this model. This crate has no reader or card dependency.

pub mod message;
pub mod record;
pub mod template;
pub mod tlv;

pub use message::NdefMessage;
pub use record::{NdefRecord, Tnf};
pub use template::{
    BluetoothLeRecord, DppBootstrap, LeAddressType, LeRole, SmartPoster, SmartPosterAction,
    TextEncoding, TextRecord, UriRecord, VCard, WifiAuthentication, WifiEncryption, WifiRecord,
};
pub use tlv::{Tlv, TlvStream, ndef_capacity};
