use nfc_core::{NfcError, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessCondition {
    pub c1: bool,
    pub c2: bool,
    pub c3: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessBits {
    pub groups: [AccessCondition; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyMask(u8);

impl KeyMask {
    pub const NONE: Self = Self(0);
    pub const A: Self = Self(1);
    pub const B: Self = Self(2);
    pub const ANY: Self = Self(3);

    pub const fn allows_a(self) -> bool {
        self.0 & Self::A.0 != 0
    }

    pub const fn allows_b(self) -> bool {
        self.0 & Self::B.0 != 0
    }
}

/// Decode and encode access bits. The implementation lands in phase 03.
pub fn decode_access_bits(_bytes: [u8; 3]) -> Result<AccessBits> {
    Err(NfcError::NotImplemented("Classic access-bit decode"))
}

pub fn encode_access_bits(_bits: AccessBits) -> Result<[u8; 3]> {
    Err(NfcError::NotImplemented("Classic access-bit encode"))
}
