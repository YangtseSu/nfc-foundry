// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic hardnested fixtures. The nonce simulation follows PM3
//! `simulate_MFplus_RNG`; no reader is used.

use nfc_core::CancelToken;

use super::bitslice::crack_states;
use super::filter::{Filter, StateList};
use super::nonces::NonceSet;
use super::solver;
use super::tables::Tables;
use crate::crypto1::{Crypto1State, filter, odd_parity8};

const KEY: u64 = 0xA0A1_A2A3_A4A5;
const CUID: u32 = 0xFAB9_CFF4;

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    fn u32(&mut self) -> u32 {
        self.next() as u32
    }
}

/// Generates one encrypted nested nonce exactly as PM3 `simulate_MFplus_RNG`.
fn simulate_nonce(cuid: u32, key: u64, rng: &mut SplitMix64) -> (u32, u8) {
    let mut state = Crypto1State::from_key(key);
    let nt = rng.u32();
    let mut nonce_enc = 0u32;
    let mut par_enc = 0u8;
    for byte_position in (0..4).rev() {
        let plain = (nt >> (8 * byte_position)) as u8;
        let input = plain ^ ((cuid >> (8 * byte_position)) as u8);
        let keystream = state.byte(input, false);
        nonce_enc = (nonce_enc << 8) | u32::from(keystream ^ plain);
        let keystream_parity = filter(state.odd);
        par_enc = (par_enc << 1) | (keystream_parity ^ odd_parity8(plain));
    }
    (nonce_enc, par_enc)
}

fn simulated_nonces(count: usize) -> Vec<(u32, u8)> {
    let mut rng = SplitMix64(0x1234_5678_9ABC_DEF0);
    (0..count)
        .map(|_| simulate_nonce(CUID, KEY, &mut rng))
        .collect()
}

fn true_state(first_byte: u8) -> (u32, u32) {
    let mut state = Crypto1State::from_key(KEY);
    let _ = state.byte(((CUID >> 24) as u8) ^ first_byte, true);
    (state.odd & 0x00ff_ffff, state.even & 0x00ff_ffff)
}

#[test]
fn bitsliced_solver_recovers_key_from_simulated_nonces() {
    let mut nonces = NonceSet::new();
    for (nonce_enc, par_enc) in simulated_nonces(24) {
        nonces.add(nonce_enc, par_enc);
    }
    let first_byte = nonces
        .groups
        .iter()
        .position(|group| !group.entries.is_empty())
        .expect("a first-byte group") as u8;
    let (odd, even) = true_state(first_byte);

    let mut best_first_bytes: [usize; 256] = std::array::from_fn(|index| index);
    best_first_bytes.swap(0, first_byte as usize);
    nonces.pre_xor(CUID);

    let list = StateList {
        odd: vec![odd],
        even: vec![even],
    };
    let outcome = crack_states(
        CUID,
        &best_first_bytes,
        &list,
        &nonces,
        &CancelToken::new(),
        &mut |_| {},
    )
    .expect("brute force");
    assert_eq!(outcome.key, Some(KEY));
}

#[test]
#[ignore = "slow full hardnested fixture; run with --release --ignored"]
fn true_state_survives_filtering_and_solver_recovers_key() {
    let tables_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../resources/hardnested_tables")
        .canonicalize()
        .expect("workspace tables");
    let tables = Tables::load(&tables_dir).expect("tables");
    let mut filter = Filter::new(&tables);

    let mut rng = SplitMix64(0x0BAD_C0DE_1234_5678);
    let mut received = 0usize;
    while received < 4000 && !filter.nonces.all_first_bytes_seen() {
        let (nonce_enc, par_enc) = simulate_nonce(CUID, KEY, &mut rng);
        filter.add_nonce(nonce_enc, par_enc);
        received += 1;
        filter.update().expect("filter update");
    }
    assert!(filter.nonces.all_first_bytes_seen());

    // Keep collecting until the same stopping rule as real acquisition.
    while received < 8000 {
        let (nonce_enc, par_enc) = simulate_nonce(CUID, KEY, &mut rng);
        filter.add_nonce(nonce_enc, par_enc);
        received += 1;
        filter.update().expect("filter update");
        if received.is_multiple_of(8) {
            filter.sample_period_ms = 100.0;
            if filter.shrink_key_space().1 {
                break;
            }
        }
    }

    let first_byte = filter.best_first_byte();
    let (odd, even) = true_state(first_byte as u8);
    assert!(
        filter.nonces.groups[first_byte].states[1]
            .as_ref()
            .is_some_and(|bits| bits.get(odd as usize)),
        "true odd state was filtered out of its first-byte group"
    );
    assert!(
        filter.nonces.groups[first_byte].states[0]
            .as_ref()
            .is_some_and(|bits| bits.get(even as usize)),
        "true even state was filtered out of its first-byte group"
    );

    let outcome =
        solver::solve(&mut filter, CUID, &CancelToken::new(), &mut |_| {}).expect("solver");
    assert_eq!(outcome.key, Some(KEY));
}
