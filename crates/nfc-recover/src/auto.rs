// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Card/PRNG profile and recovery strategy selection.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrngProfile {
    #[default]
    Unknown,
    Weak,
    Hard,
    StaticNonce,
    StaticEncrypted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardProfile {
    pub prng: PrngProfile,
    pub has_known_key: bool,
    pub darkside_vulnerable: bool,
    pub ev1_signature: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStrategy {
    Auto,
    Dictionary,
    DarkSide,
    Nested,
    StaticNested,
    HardNested,
    Fm11rf08s,
}
