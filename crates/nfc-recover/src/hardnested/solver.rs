// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardnested solver orchestration: candidate generation, brute force, and key
//! selection.

use nfc_core::{CancelToken, Result};

use super::bitslice::{BruteForceProgress, crack_states};
use super::filter::Filter;
use super::nonces::NUM_SUMS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolverPhase {
    Filtering,
    BruteForce,
}

#[derive(Clone, Copy, Debug)]
pub struct SolverProgress {
    pub phase: SolverPhase,
    pub tested_states: u64,
    pub total_states: u64,
    pub expected_states: f64,
}

pub struct SolverOutcome {
    pub key: Option<u64>,
    pub tested_states: u64,
    pub expected_states: f64,
    pub note: String,
}

/// Recovers one key from an already filtered nonce set. The caller is
/// responsible for `pre_xor` and for authenticating any returned key.
pub fn solve(
    filter: &mut Filter<'_>,
    cuid: u32,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(SolverProgress),
) -> Result<SolverOutcome> {
    cancel.check()?;
    if filter.stage & super::filter::CHECK_2ND_BYTES == 0 {
        return Err(nfc_core::NfcError::VerificationFailed(
            "hardnested needs at least 256 distinct first nonce bytes before solving".to_owned(),
        ));
    }
    filter.nonces.pre_xor(cuid);

    let bitflip_brute_force = filter.check_smallest_bitflip_bitarrays();
    let sum_a8_brute_force = filter.sort_best_first_bytes();
    progress(SolverProgress {
        phase: SolverPhase::Filtering,
        tested_states: 0,
        total_states: 0,
        expected_states: bitflip_brute_force.min(sum_a8_brute_force),
    });

    let mut total_tested = 0u64;
    if bitflip_brute_force < sum_a8_brute_force {
        filter.best_first_bytes[0] = filter.best_first_byte_smallest_bitarray;
        let expected = bitflip_brute_force;
        let list = filter.bitflip_candidates(filter.best_first_bytes[0]);
        let outcome = crack_states(
            cuid,
            &filter.best_first_bytes,
            &list,
            &filter.nonces,
            cancel,
            &mut |state| report_brute_force(progress, state, expected),
        )?;
        total_tested += outcome.tested_states;
        return Ok(SolverOutcome {
            key: outcome.key,
            tested_states: total_tested,
            expected_states: expected,
            note: "used all bitflip properties without a Sum(a8) estimate".to_owned(),
        });
    }

    let first_byte = filter.best_first_byte();
    let expected = sum_a8_brute_force;
    for guess_index in 0..NUM_SUMS {
        cancel.check()?;
        let sum_a8_idx = filter.nonces.groups[first_byte].sum_a8_guess[guess_index].sum_a8_idx;
        let sum_a8 = super::nonces::SUMS[sum_a8_idx];
        let candidates = filter.generate_candidates(filter.first_byte_sum_idx, sum_a8_idx);
        for list in &candidates {
            cancel.check()?;
            let outcome = crack_states(
                cuid,
                &filter.best_first_bytes,
                list,
                &filter.nonces,
                cancel,
                &mut |state| report_brute_force(progress, state, expected),
            )?;
            total_tested += outcome.tested_states;
            if let Some(key) = outcome.key {
                return Ok(SolverOutcome {
                    key: Some(key),
                    tested_states: total_tested,
                    expected_states: expected,
                    note: format!("Sum(a8) = {sum_a8}, guess {}/{NUM_SUMS}", guess_index + 1),
                });
            }
        }
    }

    Ok(SolverOutcome {
        key: None,
        tested_states: total_tested,
        expected_states: expected,
        note: "no key candidate passed nonce verification".to_owned(),
    })
}

fn report_brute_force(
    progress: &mut dyn FnMut(SolverProgress),
    state: BruteForceProgress,
    expected_states: f64,
) {
    if state.tested_states.is_multiple_of(0x100000) || state.tested_states == state.total_states {
        progress(SolverProgress {
            phase: SolverPhase::BruteForce,
            tested_states: state.tested_states,
            total_states: state.total_states,
            expected_states,
        });
    }
}
