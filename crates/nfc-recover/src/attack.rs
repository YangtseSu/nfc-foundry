use nfc_core::{Key, KeyType, Result};
use nfc_transport::Reader;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackKind {
    DarkSide,
    Nested,
    StaticNested,
    HardNested,
    Fm11rf08s,
}

#[derive(Clone, Debug, Default)]
pub struct RecoveryRequest {
    pub known_key: Option<(KeyType, Key)>,
    pub target_sector: Option<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct RecoveryResult {
    pub recovered: Vec<(u8, KeyType, Key)>,
    pub attempts: Vec<AttackKind>,
}

pub trait Attack {
    fn kind(&self) -> AttackKind;
    fn run(&mut self, reader: &mut dyn Reader, request: &RecoveryRequest)
    -> Result<RecoveryResult>;
}
