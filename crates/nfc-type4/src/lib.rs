//! NFC Forum Type 4 NDEF adapter over ISO-DEP / ISO 7816-4 APDUs.
//!
//! The adapter reads the NDEF Tag Application, parses the capability container
//! and its File Control TLVs, and reads or writes the mandatory NDEF file with
//! NLEN, capacity, and read-back checks. NDEF encoding and decoding stay in
//! `nfc-ndef`; the crate has no reader protocol mechanics.

pub mod apdu;
pub mod cc;
pub mod exchange;
pub mod file;
pub mod read;
pub mod write;

pub use cc::{CcTlv, NdefFileControl, ProprietaryFileControl, Type4CapabilityContainer};
pub use file::{NdefState, Type4Ndef};
pub use read::{Type4ReadResult, read_tag};
pub use write::{Type4ClearResult, Type4WriteResult, clear_ndef, write_ndef};
