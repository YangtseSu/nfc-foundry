/// NDEF TLV types used by Type 2 and Classic mappings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tlv {
    Null,
    LockControl(Vec<u8>),
    MemoryControl(Vec<u8>),
    Proprietary { type_: u8, data: Vec<u8> },
    Terminator,
}
