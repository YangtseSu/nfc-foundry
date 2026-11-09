// SPDX-FileCopyrightText: 2015-2016 Aram Verstegen
// SPDX-FileCopyrightText: 2016, 2017 piwi
// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bit-sliced Crypto1 brute force for hardnested candidates.
//!
//! Ported from PM3 `client/deps/hardnested/hardnested_bf_core.c`. The PM3
//! kernel is compiled once per SIMD width; this port uses 64 lanes, the same
//! layout as PM3's NOSIMD build.
//!
//! The bitsliced approach derives from aczid's MIT-licensed
//! `crypto1_bs` (Copyright (c) 2015-2016 Aram Verstegen). The MIT permission
//! notice is reproduced in `THIRD_PARTY_NOTICES.md`; this combined work is
//! distributed under GPL-3.0-or-later.

use nfc_core::{CancelToken, Result};

use super::filter::StateList;
use super::nonces::NonceSet;
use crate::crypto1::{Crypto1State, filter, parity32};

const KEYSTREAM_SIZE: usize = 24;
const STATE_SIZE: usize = 48;
const LANES: usize = 64;

#[derive(Clone, Copy)]
struct TestNonce {
    nonce_enc: u32,
    par_enc: u8,
    second: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BruteForceProgress {
    pub tested_states: u64,
    pub total_states: u64,
}

pub struct BruteForceOutcome {
    pub key: Option<u64>,
    pub tested_states: u64,
}

/// Reorders the tested nonces so the first four share as many trailing bits of
/// the second byte as possible, reducing repeated state work.
fn prepare_test_nonces(nonces: &NonceSet, first_byte: usize) -> Vec<TestNonce> {
    let entries = &nonces.groups[first_byte].entries;
    let mut test: Vec<TestNonce> = entries
        .iter()
        .map(|entry| TestNonce {
            nonce_enc: entry.nonce_enc,
            par_enc: entry.par_enc,
            second: (entry.nonce_enc >> 16) as u8,
        })
        .collect();
    let count = test.len();
    if count < 2 {
        return test;
    }

    let mut best = [0usize; 4];
    let mut best_sum = -1i32;
    if count <= 16 {
        for n1 in 0..count {
            for n2 in 0..count {
                if n2 == n1 {
                    continue;
                }
                for n3 in 0..count {
                    if (n3 == n2 || n3 == n1) && count >= 3 {
                        continue;
                    }
                    for n4 in 0..count {
                        if (n4 == n3 || n4 == n2 || n4 == n1) && count >= 4 {
                            continue;
                        }
                        let mut sum = 0i32;
                        if count > 1 {
                            sum += i32::from(test[n1].second ^ test[n2].second).trailing_zeros()
                                as i32;
                        }
                        if count > 2 {
                            sum += i32::from(test[n2].second ^ test[n3].second).trailing_zeros()
                                as i32;
                        }
                        if count > 3 {
                            sum += i32::from(test[n3].second ^ test[n4].second).trailing_zeros()
                                as i32;
                        }
                        if sum > best_sum {
                            best_sum = sum;
                            best = [n1, n2, n3, n4];
                        }
                    }
                }
            }
        }
    } else {
        best = [0, 1, 2, 3];
    }

    let original = test.clone();
    for index in 0..4.min(count) {
        test[index] = original[best[index]];
    }
    test
}

fn f20a(a: u64, b: u64, c: u64, d: u64) -> u64 {
    ((a | b) ^ (a & d)) ^ (c & ((a ^ b) | d))
}

fn f20b(a: u64, b: u64, c: u64, d: u64) -> u64 {
    ((a & b) | c) ^ ((a ^ b) & (c | d))
}

fn f20c(a: u64, b: u64, c: u64, d: u64, e: u64) -> u64 {
    (a | ((b | e) & (d ^ e))) ^ ((a ^ (b & d)) & ((c ^ d) | (b & e)))
}

fn vector(bit: bool) -> u64 {
    if bit { u64::MAX } else { 0 }
}

fn bitsliced_nonces(test: &[TestNonce]) -> Vec<[u64; KEYSTREAM_SIZE]> {
    let mut result = vec![[0u64; KEYSTREAM_SIZE]; test.len()];
    for (index, nonce) in test.iter().enumerate() {
        for (ks_idx, slot) in result[index].iter_mut().enumerate() {
            let bit_position = KEYSTREAM_SIZE - 1 - ks_idx;
            let byte_shift = 16 - 8 * (bit_position / 8);
            let bit = (nonce.nonce_enc >> byte_shift) >> (bit_position % 8) & 1;
            *slot = vector(bit != 0);
        }
    }
    result
}

fn bitsliced_parity(test: &[TestNonce]) -> Vec<[u64; 4]> {
    let mut result = vec![[0u64; 4]; test.len()];
    for (index, nonce) in test.iter().enumerate() {
        for (bit_index, slot) in result[index].iter_mut().enumerate() {
            *slot = vector((nonce.par_enc >> (3 - bit_index)) & 1 != 0);
        }
    }
    result
}

/// PM3 `crack_states_bitsliced` over one statelist.
pub fn crack_states(
    cuid: u32,
    best_first_bytes: &[usize; 256],
    list: &StateList,
    nonces: &NonceSet,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(BruteForceProgress),
) -> Result<BruteForceOutcome> {
    if list.odd.is_empty() || list.even.is_empty() {
        return Ok(BruteForceOutcome {
            key: None,
            tested_states: 0,
        });
    }

    let first_byte = best_first_bytes[0];
    let test = prepare_test_nonces(nonces, first_byte);
    if test.is_empty() {
        return Ok(BruteForceOutcome {
            key: None,
            tested_states: 0,
        });
    }
    let encrypted_nonces = bitsliced_nonces(&test);
    let encrypted_parity = bitsliced_parity(&test);
    let nonce_count = test.len();

    let mut tested_states = 0u64;
    let total_states = list.product();

    let block_count = list.even.len().div_ceil(LANES);
    let mut even_blocks: Vec<[u64; STATE_SIZE / 2]> = Vec::with_capacity(block_count);
    let mut even_feedback: Vec<u64> = Vec::with_capacity(block_count);
    let mut block_sizes: Vec<usize> = Vec::with_capacity(block_count);

    for block in 0..block_count {
        let start = block * LANES;
        let end = (start + LANES).min(list.even.len());
        let size = end - start;
        block_sizes.push(size);
        let mut lanes = [0u64; STATE_SIZE / 2];
        for lane in 0..LANES {
            let state = list.even[if lane < size { start + lane } else { end - 1 }];
            for (bit_index, slot) in lanes.iter_mut().enumerate() {
                if (state >> bit_index) & 1 != 0 {
                    *slot |= 1u64 << lane;
                }
            }
        }
        even_feedback.push(lanes[23] ^ lanes[18] ^ lanes[17] ^ lanes[16] ^ lanes[11] ^ lanes[2]);
        even_blocks.push(lanes);
    }

    for &odd_state in &list.odd {
        cancel.check()?;
        let mut states = [0u64; KEYSTREAM_SIZE + STATE_SIZE];
        let odd_feedback = vector(parity32(odd_state & 0x29ce5c) != 0);

        let mut bits = odd_state;
        for state_index in (0..STATE_SIZE).step_by(2) {
            states[KEYSTREAM_SIZE + state_index] = vector(bits & 1 != 0);
            bits >>= 1;
        }

        let mut f20b_2 = [0u64; 16];
        let mut f20b_3 = [0u64; 8];
        f20b_2[0] = f20b(
            states[KEYSTREAM_SIZE + 22],
            states[KEYSTREAM_SIZE + 20],
            states[KEYSTREAM_SIZE + 18],
            states[KEYSTREAM_SIZE + 16],
        );
        f20b_3[0] = f20b(
            states[KEYSTREAM_SIZE + 6],
            states[KEYSTREAM_SIZE + 4],
            states[KEYSTREAM_SIZE + 2],
            states[KEYSTREAM_SIZE],
        );
        let mut ksb = [0u64; 8];
        ksb[0] = f20c(
            f20a(
                states[KEYSTREAM_SIZE + 38],
                states[KEYSTREAM_SIZE + 36],
                states[KEYSTREAM_SIZE + 34],
                states[KEYSTREAM_SIZE + 32],
            ),
            f20b(
                states[KEYSTREAM_SIZE + 30],
                states[KEYSTREAM_SIZE + 28],
                states[KEYSTREAM_SIZE + 26],
                states[KEYSTREAM_SIZE + 24],
            ),
            f20b_2[0],
            f20a(
                states[KEYSTREAM_SIZE + 14],
                states[KEYSTREAM_SIZE + 12],
                states[KEYSTREAM_SIZE + 10],
                states[KEYSTREAM_SIZE + 8],
            ),
            f20b_3[0],
        );

        for (block_index, even_lanes) in even_blocks.iter().enumerate() {
            for (lane_index, lane) in even_lanes.iter().enumerate() {
                states[KEYSTREAM_SIZE + 1 + 2 * lane_index] = *lane;
            }

            let mut fbb = [0u64; 8];
            fbb[0] = odd_feedback ^ even_feedback[block_index];
            let mut results = u64::MAX;
            let mut par = [0u64; 8];
            let mut next_common_bits = 0usize;

            let mut tests_passed = true;
            for test_index in 0..nonce_count {
                let common_bits = next_common_bits;
                next_common_bits = if test_index + 1 < nonce_count {
                    (test[test_index].second ^ test[test_index + 1].second).trailing_zeros()
                        as usize
                } else {
                    0
                };
                let mut parity_bit_index = 1usize;
                let mut fb_bits = fbb[common_bits];
                let mut ks_bits = ksb[common_bits];
                let mut parity_bits = par[common_bits];
                let mut state_index = KEYSTREAM_SIZE - common_bits;

                let first_ks = KEYSTREAM_SIZE - 1 - common_bits;
                for ks_idx in (0..=first_ks).rev() {
                    let encrypted = encrypted_nonces[test_index][ks_idx];
                    let decrypted = encrypted ^ ks_bits;
                    parity_bits ^= decrypted;
                    state_index -= 1;
                    states[state_index] = fb_bits ^ decrypted;

                    let f20a_2 = f20a(
                        states[state_index + 14],
                        states[state_index + 12],
                        states[state_index + 10],
                        states[state_index + 8],
                    );
                    let f20b_3_current = f20b(
                        states[state_index + 6],
                        states[state_index + 4],
                        states[state_index + 2],
                        states[state_index],
                    );
                    let (f20a_1, f20b_1, f20b_2_current) = if ks_idx > KEYSTREAM_SIZE - 8 {
                        let f20a_1 = f20a(
                            states[state_index + 38],
                            states[state_index + 36],
                            states[state_index + 34],
                            states[state_index + 32],
                        );
                        let f20b_1 = f20b(
                            states[state_index + 30],
                            states[state_index + 28],
                            states[state_index + 26],
                            states[state_index + 24],
                        );
                        let f20b_2_current = f20b(
                            states[state_index + 22],
                            states[state_index + 20],
                            states[state_index + 18],
                            states[state_index + 16],
                        );
                        f20b_2[KEYSTREAM_SIZE - ks_idx] = f20b_2_current;
                        f20b_3[KEYSTREAM_SIZE - ks_idx] = f20b_3_current;
                        (f20a_1, f20b_1, f20b_2_current)
                    } else if ks_idx > KEYSTREAM_SIZE - 16 {
                        let f20a_1 = f20a(
                            states[state_index + 38],
                            states[state_index + 36],
                            states[state_index + 34],
                            states[state_index + 32],
                        );
                        let f20b_1 = f20b_2[KEYSTREAM_SIZE - ks_idx - 8];
                        let f20b_2_current = f20b(
                            states[state_index + 22],
                            states[state_index + 20],
                            states[state_index + 18],
                            states[state_index + 16],
                        );
                        f20b_2[KEYSTREAM_SIZE - ks_idx] = f20b_2_current;
                        (f20a_1, f20b_1, f20b_2_current)
                    } else if ks_idx > KEYSTREAM_SIZE - 24 {
                        let f20a_1 = f20a(
                            states[state_index + 38],
                            states[state_index + 36],
                            states[state_index + 34],
                            states[state_index + 32],
                        );
                        let f20b_1 = f20b_2[KEYSTREAM_SIZE - ks_idx - 8];
                        let f20b_2_current = f20b_3[KEYSTREAM_SIZE - ks_idx - 16];
                        (f20a_1, f20b_1, f20b_2_current)
                    } else {
                        let f20a_1 = f20a(
                            states[state_index + 38],
                            states[state_index + 36],
                            states[state_index + 34],
                            states[state_index + 32],
                        );
                        let f20b_1 = f20b(
                            states[state_index + 30],
                            states[state_index + 28],
                            states[state_index + 26],
                            states[state_index + 24],
                        );
                        let f20b_2_current = f20b(
                            states[state_index + 22],
                            states[state_index + 20],
                            states[state_index + 18],
                            states[state_index + 16],
                        );
                        (f20a_1, f20b_1, f20b_2_current)
                    };
                    ks_bits = f20c(f20a_1, f20b_1, f20b_2_current, f20a_2, f20b_3_current);

                    if ks_idx & 0x07 == 0 {
                        let encrypted_parity = encrypted_parity[test_index][parity_bit_index];
                        parity_bit_index += 1;
                        let decrypted_parity = encrypted_parity ^ ks_bits;
                        results &= !parity_bits ^ decrypted_parity;
                        if results == 0 {
                            tests_passed = false;
                            break;
                        }
                        parity_bits = 0;
                    }

                    if ks_idx != 0 {
                        fb_bits = states[state_index + 47]
                            ^ states[state_index + 42]
                            ^ states[state_index + 38]
                            ^ states[state_index + 37]
                            ^ states[state_index + 35]
                            ^ states[state_index + 33]
                            ^ states[state_index + 32]
                            ^ states[state_index + 30]
                            ^ states[state_index + 28]
                            ^ states[state_index + 23]
                            ^ states[state_index + 22]
                            ^ states[state_index + 20]
                            ^ states[state_index + 18]
                            ^ states[state_index + 12]
                            ^ states[state_index + 8]
                            ^ states[state_index + 6]
                            ^ states[state_index + 5]
                            ^ states[state_index + 4];
                    }

                    let bit = KEYSTREAM_SIZE - ks_idx;
                    if bit <= next_common_bits.min(7) {
                        fbb[bit] = fb_bits;
                        ksb[bit] = ks_bits;
                        par[bit] = parity_bits;
                    }
                }
            }

            if tests_passed {
                let block_start = block_index * LANES;
                let block_size = block_sizes[block_index];
                let mut results_word = results;
                for lane in 0..block_size {
                    if results_word & 1 != 0 {
                        let even_state = list.even[block_start + lane];
                        if verify_key(cuid, best_first_bytes, odd_state, even_state, nonces) {
                            let mut state = Crypto1State {
                                odd: odd_state,
                                even: even_state,
                            };
                            state.rollback_byte(
                                ((cuid >> 24) as u8) ^ best_first_bytes[0] as u8,
                                true,
                            );
                            return Ok(BruteForceOutcome {
                                key: Some(state.key()),
                                tested_states,
                            });
                        }
                    }
                    results_word >>= 1;
                }
            }

            tested_states += block_sizes[block_index] as u64;
            progress(BruteForceProgress {
                tested_states,
                total_states,
            });
        }
    }

    Ok(BruteForceOutcome {
        key: None,
        tested_states,
    })
}

/// PM3 `verify_key`: checks every nonce outside the brute-forced first-byte
/// group against the candidate state.
fn verify_key(
    cuid: u32,
    best_first_bytes: &[usize; 256],
    odd: u32,
    even: u32,
    nonces: &NonceSet,
) -> bool {
    for test_index in 1..256 {
        let first_byte = best_first_bytes[test_index];
        for entry in &nonces.groups[first_byte].entries {
            let mut state = Crypto1State { odd, even };
            state.rollback_byte(((cuid >> 24) as u8) ^ best_first_bytes[0] as u8, true);
            for byte_position in (0..4).rev() {
                let encrypted_parity = (entry.par_enc >> byte_position) & 1;
                let encrypted_byte = (entry.nonce_enc >> (8 * byte_position)) as u8;
                let keystream = state.byte(encrypted_byte, true);
                let decrypted = keystream ^ encrypted_byte;
                let keystream_parity = filter(state.odd);
                let expected = keystream_parity ^ parity32(u32::from(decrypted)) as u8;
                if encrypted_parity != expected {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_subfunctions_are_sixteen_bit_truth_tables() {
        for a in [0u64, u64::MAX] {
            for b in [0u64, u64::MAX] {
                for c in [0u64, u64::MAX] {
                    for d in [0u64, u64::MAX] {
                        let expected_a = ((a | b) ^ (a & d)) ^ (c & ((a ^ b) | d));
                        assert_eq!(f20a(a, b, c, d), expected_a);
                        let expected_b = ((a & b) | c) ^ ((a ^ b) & (c | d));
                        assert_eq!(f20b(a, b, c, d), expected_b);
                    }
                }
            }
        }
    }

    #[test]
    fn bitsliced_nonce_bit_order_matches_pm3() {
        // A nonce whose second byte is 0x01 and remaining bytes zero: the
        // first encrypted bit tested is bit 0 of the second byte.
        let test = [TestNonce {
            nonce_enc: 0x0001_0000,
            par_enc: 0,
            second: 0x01,
        }];
        let sliced = bitsliced_nonces(&test);
        assert_eq!(sliced[0][23], u64::MAX);
        for value in sliced[0].iter().take(23) {
            assert_eq!(*value, 0);
        }
    }
}
