//! Native MIFARE Classic key recovery.
//!
//! The behavioral source is the current RfidResearchGroup/proxmark3 tree.
//! PM3 firmware commands are not ported; nonce acquisition is implemented over
//! `nfc-transport`.

pub mod attack;
pub mod auto;
pub mod crypto1;

pub use attack::{Attack, AttackKind, RecoveryRequest, RecoveryResult};
pub use auto::{CardProfile, PrngProfile, RecoveryStrategy};
