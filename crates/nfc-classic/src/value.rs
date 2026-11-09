use nfc_core::{NfcError, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ValueBlock {
    pub value: i32,
    pub address: u8,
}

/// Decode/encode value blocks. The implementation lands in phase 03.
pub fn decode(_data: [u8; nfc_core::BLOCK_SIZE]) -> Result<ValueBlock> {
    Err(NfcError::NotImplemented("Classic value-block decode"))
}

pub fn encode(_value: ValueBlock) -> Result<[u8; nfc_core::BLOCK_SIZE]> {
    Err(NfcError::NotImplemented("Classic value-block encode"))
}
