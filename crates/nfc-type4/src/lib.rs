//! ISO-DEP / NFC Forum Type 4 NDEF adapter. Implemented in a later phase.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Type4CapabilityContainer {
    pub major_version: u8,
    pub minor_version: u8,
    pub max_le: u16,
    pub max_lc: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Type4NdefFile {
    pub capacity: u16,
    pub message: nfc_ndef::NdefMessage,
}
