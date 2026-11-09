// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Native hardnested acquisition and bit-sliced solver.
//!
//! Ported from the current `RfidResearchGroup/proxmark3` client:
//! `client/src/cmdhfmfhard.c`, `client/deps/hardnested/`, and
//! `client/resources/hardnested_tables/`. The PM3 firmware command protocol is
//! not used; nonces are acquired through `nfc-transport` parity exchanges.

pub mod acquire;
pub mod bitset;
pub mod bitslice;
pub mod filter;
pub mod nonces;
pub mod solver;
pub mod tables;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use nfc_core::{CancelToken, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

pub const DEFAULT_MAX_NONCES: usize = 4000;

#[derive(Clone, Debug)]
pub struct HardnestedRequest {
    pub known_block: u8,
    pub known_key_type: KeyType,
    pub known_key: Key,
    pub target_block: u8,
    pub target_key_type: KeyType,
    pub tables_dir: Option<PathBuf>,
    /// Upper bound for collected nonce frames; zero uses the default.
    pub max_nonces: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum HardnestedEvent {
    Acquiring(acquire::AcquisitionProgress),
    Filtering(solver::SolverProgress),
}

#[derive(Clone, Debug)]
pub struct HardnestedOutcome {
    pub keys: Vec<Key>,
    pub nonces: usize,
    pub tested_states: u64,
    pub note: String,
}

/// Acquires nonces and recovers one target key. The caller must verify the
/// returned key with a fresh authentication before reporting it.
pub fn recover(
    reader: &mut dyn Reader,
    request: &HardnestedRequest,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(HardnestedEvent),
) -> Result<HardnestedOutcome> {
    let dir = tables::resolve_dir(request.tables_dir.as_deref())?;
    let tables = tables::Tables::load(&dir)?;
    recover_with_tables(reader, request, &tables, cancel, progress)
}

/// Recovers with already loaded tables. The caller can load the tables before
/// the known-key probe so the card is not left authenticated while the tables
/// are decompressed.
pub fn recover_with_tables(
    reader: &mut dyn Reader,
    request: &HardnestedRequest,
    tables: &tables::Tables,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(HardnestedEvent),
) -> Result<HardnestedOutcome> {
    let max_nonces = if request.max_nonces == 0 {
        DEFAULT_MAX_NONCES
    } else {
        request.max_nonces
    };
    let (acquisition, mut filter) =
        acquire::collect(reader, tables, request, cancel, max_nonces, &mut |event| {
            progress(HardnestedEvent::Acquiring(event))
        })?;

    if !filter.nonces.all_first_bytes_seen() {
        return Err(NfcError::VerificationFailed(format!(
            "hardnested collected only {}/256 distinct first nonce bytes in {} frames; the PRNG may be static or the acquisition cap is too low (raise --max-nonces)",
            filter.nonces.first_byte_num, acquisition.received
        )));
    }
    let outcome = solver::solve(&mut filter, acquisition.cuid, cancel, &mut |event| {
        progress(HardnestedEvent::Filtering(event))
    })?;
    let key = outcome
        .key
        .map(crate::mfkey::key_from_u64)
        .ok_or(NfcError::AuthenticationFailed)?;
    Ok(HardnestedOutcome {
        keys: vec![key],
        nonces: filter.nonces.num_acquired,
        tested_states: outcome.tested_states,
        note: format!(
            "{} nonces, {} tested state pairs, {}",
            filter.nonces.num_acquired, outcome.tested_states, outcome.note
        ),
    })
}
