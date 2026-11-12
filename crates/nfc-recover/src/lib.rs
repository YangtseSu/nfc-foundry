//! Native MIFARE Classic key recovery.
//!
//! The behavioral source is the current RfidResearchGroup/proxmark3 tree.
//! PM3 firmware commands are not ported; nonce acquisition is implemented over
//! `nfc-transport`.

pub mod acquire;
pub mod attack;
pub mod auto;
pub mod classify;
pub mod crypto1;
pub mod darkside;
pub mod hardnested;
pub mod mfkey;
pub mod nested;
pub mod staticnested;

pub use attack::{
    AttackKind, ProgressSink, RecoveredKey, RecoveryProgress, RecoveryRequest, RecoveryResult,
    recover,
};
pub use auto::{CardProfile, PrngProfile, RecoveryStrategy};
