//! MIFARE Classic NDEF adapter: MAD1/MAD2, NFC sector mapping, TLV framing.

pub mod layout;
pub mod mad;

pub use layout::{
    ClassicNdef, ClassicNdefInfo, ClassicNdefLayout, MappingVersion, NFC_GPB_RO, NFC_GPB_RW,
    NFC_KEY_A, NFC_RO_ACCESS_BITS, NFC_RW_ACCESS_BITS, NdefState, encode_ndef_area,
    format_ndef_area, length_field_bytes, read_ndef, read_ndef_with_layout,
};
pub use mad::{
    MAD_ACCESS_BITS, MAD_KEY_A, MAD1_GPB, MAD2_GPB, Mad, MadAid, MadVersion, NFC_APPLICATION,
    NFC_CLUSTER, crc8,
};
