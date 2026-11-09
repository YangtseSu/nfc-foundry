#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffState {
    Equal,
    Changed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockDiff {
    pub block: usize,
    pub sector: usize,
    pub state: DiffState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DumpDiff {
    pub changed_blocks: usize,
    pub equal_blocks: usize,
    pub unknown_blocks: usize,
    pub blocks: Vec<BlockDiff>,
}

/// Compare two dumps. The implementation lands in phase 03.
pub fn diff(_before: &super::Dump, _after: &super::Dump) -> nfc_core::Result<DumpDiff> {
    Err(nfc_core::NfcError::NotImplemented("Classic dump diff"))
}
