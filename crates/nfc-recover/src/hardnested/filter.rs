// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Nonce-state filtering and candidate generation from PM3
//! `client/src/cmdhfmfhard.c`.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use nfc_core::{NfcError, Result};

use super::bitset::BitSet24;
use super::nonces::{NUM_PART_SUMS, NUM_SUMS, NonceSet, SUM_PAIRS, SUMS};
use super::tables::Tables;
use crate::crypto1::filter;

pub const CHECK_1ST_BYTES: u8 = 0x01;
pub const CHECK_2ND_BYTES: u8 = 0x02;

/// Estimated bitsliced key tests per second. Used only to decide when enough
/// nonces have been collected; a wrong estimate changes the collection time,
/// not the recovered key.
const ESTIMATED_KEYS_PER_SECOND: f64 = 40_000_000.0;
const STOP_BRUTE_FORCE: f64 = 0xF00000 as f64;

struct PartSumTables {
    a0: [[BitSet24; NUM_PART_SUMS]; 2],
    a8: [[BitSet24; NUM_PART_SUMS]; 2],
}

static PART_SUMS: LazyLock<PartSumTables> = LazyLock::new(build_part_sums);

fn build_part_sums() -> PartSumTables {
    let mut a0: [[BitSet24; NUM_PART_SUMS]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|_| BitSet24::zeroed()));
    let mut a8: [[BitSet24; NUM_PART_SUMS]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|_| BitSet24::zeroed()));

    for odd_even in 0..2 {
        for state in 0..(1u32 << 20) {
            let part = (partial_sum_property(state, odd_even) / 2) as usize;
            let index = (state as usize) << 4;
            let word_index = index >> 5;
            let mask = if state & 1 == 0 {
                0xffff_0000
            } else {
                0x0000_ffff
            };
            a0[odd_even][part].words_mut()[word_index] |= mask;

            let base_word = (state as usize) >> 5;
            let bit_mask = 0x8000_0000u32 >> (state & 31);
            for high in 0..16usize {
                a8[odd_even][part].words_mut()[base_word + (high << 15)] |= bit_mask;
            }
        }
    }

    PartSumTables { a0, a8 }
}

/// PM3 `PartialSumProperty`.
pub fn partial_sum_property(state: u32, odd_even: usize) -> u32 {
    let mut sum = 0;
    for j in 0..16u32 {
        let mut st = state;
        let mut part_sum = 0u8;
        if odd_even == 1 {
            part_sum ^= filter(st);
            for i in 0..4 {
                st = (st << 1) | ((j >> (3 - i)) & 1);
                part_sum ^= filter(st);
            }
            part_sum ^= 1;
        } else {
            for i in 0..4 {
                st = (st << 1) | ((j >> (3 - i)) & 1);
                part_sum ^= filter(st);
            }
        }
        sum += u32::from(part_sum);
    }
    sum
}

/// PM3 `SumProperty` over an odd/even state pair.
pub fn sum_property(odd: u32, even: u32) -> u32 {
    let sum_odd = partial_sum_property(odd, 1);
    let sum_even = partial_sum_property(even, 0);
    sum_odd * (16 - sum_even) + (16 - sum_odd) * sum_even
}

#[derive(Default)]
pub struct StateList {
    pub odd: Vec<u32>,
    pub even: Vec<u32>,
}

impl StateList {
    pub fn product(&self) -> u64 {
        self.odd.len() as u64 * self.even.len() as u64
    }
}

pub struct Filter<'a> {
    pub tables: &'a Tables,
    pub nonces: NonceSet,
    pub all_bitflips: [BitSet24; 2],
    pub all_changed: [bool; 2],
    /// The part-sum sets narrowed by the current `all_bitflips`, as PM3
    /// mutates its global part-sum arrays in place.
    filtered_part_sums: [([BitSet24; NUM_PART_SUMS], [BitSet24; NUM_PART_SUMS]); 2],
    pub part_sum_count: [[[u32; NUM_PART_SUMS]; NUM_PART_SUMS]; 2],
    pub p_k: [f64; NUM_SUMS],
    pub stage: u8,
    pub first_byte_sum_idx: usize,
    pub best_first_bytes: [usize; 256],
    pub best_first_byte_smallest_bitarray: usize,
    pub last_update: Instant,
    pub sample_period_ms: f64,
    reduction_queue: [f64; 4],
    reduction_initialized: bool,
}

impl<'a> Filter<'a> {
    pub fn new(tables: &'a Tables) -> Self {
        Self {
            tables,
            nonces: NonceSet::new(),
            all_bitflips: [BitSet24::all(), BitSet24::all()],
            all_changed: [false, false],
            filtered_part_sums: std::array::from_fn(|odd_even| {
                (
                    std::array::from_fn(|part| PART_SUMS.a0[odd_even][part].clone()),
                    std::array::from_fn(|part| PART_SUMS.a8[odd_even][part].clone()),
                )
            }),
            part_sum_count: [[[0; NUM_PART_SUMS]; NUM_PART_SUMS]; 2],
            p_k: super::nonces::initial_p_k(),
            stage: CHECK_1ST_BYTES,
            first_byte_sum_idx: 0,
            best_first_bytes: std::array::from_fn(|index| index),
            best_first_byte_smallest_bitarray: 0,
            last_update: Instant::now(),
            sample_period_ms: 0.0,
            reduction_queue: [0.0; 4],
            reduction_initialized: false,
        }
    }

    pub fn add_nonce(&mut self, nonce_enc: u32, par_enc: u8) -> bool {
        self.nonces.add(nonce_enc, par_enc)
    }

    /// Runs the incremental PM3 filtering pass after new nonces arrived.
    pub fn update(&mut self) -> Result<()> {
        if self.stage & CHECK_2ND_BYTES == 0 {
            self.check_first_byte_bitflips();
            self.maybe_finish_first_byte_stage()?;
        } else {
            self.check_second_byte_bitflips();
        }
        self.update_all_bitflips();
        self.update_sum_bitarrays(0);
        self.update_sum_bitarrays(1);
        self.update_p_k();
        self.nonces.estimate_sum_a8(&self.p_k);
        Ok(())
    }

    fn maybe_finish_first_byte_stage(&mut self) -> Result<()> {
        if !self.nonces.all_first_bytes_seen() {
            return Ok(());
        }
        let Some(index) = self.nonces.first_byte_sum_index() else {
            return Err(NfcError::VerificationFailed(format!(
                "no crypto1 sum property matches the first nonce byte (sum {}, is this a genuine MIFARE Classic EV1?)",
                self.nonces.first_byte_sum
            )));
        };
        self.first_byte_sum_idx = index;
        self.apply_sum_a0();
        self.stage |= CHECK_2ND_BYTES;
        Ok(())
    }

    fn apply_sum_a0(&mut self) {
        for odd_even in 0..2 {
            let mut union = BitSet24::zeroed();
            for &(p, q) in &SUM_PAIRS[self.first_byte_sum_idx] {
                let part = if odd_even == 0 { q } else { p } as usize;
                union.or_assign(&PART_SUMS.a0[odd_even][part]);
            }
            let before = self.all_bitflips[odd_even].count_ones();
            let after = self.all_bitflips[odd_even].and_assign(&union);
            if after != before {
                self.all_changed[odd_even] = true;
            }
        }
    }

    fn check_first_byte_bitflips(&mut self) {
        let table_count = self.tables.first_byte_effective;
        for index in 0..table_count {
            let bitflip = self.tables.all_effective[index];
            let mask = (bitflip & 0xff) as usize;
            for first_byte in 0..256 {
                let (left, right) = (first_byte, first_byte ^ mask);
                if self.nonces.groups[left].entries.is_empty()
                    || self.nonces.groups[right].entries.is_empty()
                    || self.nonces.groups[left].bitflips[bitflip as usize]
                    || self.nonces.groups[left].bitflips[(bitflip ^ 0x100) as usize]
                {
                    continue;
                }
                let parity_left = self.nonces.groups[left].entries[0].par_enc >> 3;
                let parity_right = self.nonces.groups[right].entries[0].par_enc >> 3;
                let bitflips = (parity_left == parity_right) != (bitflip & 0x100 != 0);
                if bitflips {
                    self.apply_bitflip(left, bitflip);
                }
            }
        }
    }

    fn check_second_byte_bitflips(&mut self) {
        for index in self.tables.first_byte_effective..self.tables.effective_count() {
            let bitflip = self.tables.all_effective[index];
            let flip = (bitflip & 0xff) as u8;
            for first_byte in 0..256 {
                if self.nonces.groups[first_byte].bitflips[bitflip as usize] {
                    continue;
                }
                let group = &self.nonces.groups[first_byte];
                for second in 0..=255u8 {
                    let (Some(first), Some(second_entry)) = (
                        group.find_second_byte(second),
                        group.find_second_byte(second ^ flip),
                    ) else {
                        continue;
                    };
                    let parity_first = (first.par_enc >> 2) & 1;
                    let parity_second = (second_entry.par_enc >> 2) & 1;
                    if (parity_first == parity_second) != (bitflip & 0x100 != 0) {
                        self.apply_bitflip(first_byte, bitflip);
                        break;
                    }
                }
            }
        }
    }

    fn apply_bitflip(&mut self, first_byte: usize, bitflip: u16) {
        self.nonces.groups[first_byte].bitflips[bitflip as usize] = true;
        for odd_even in 0..2 {
            if let Some(table) = self.tables.table(odd_even, bitflip) {
                let group = &mut self.nonces.groups[first_byte];
                let Some(states) = group.states[odd_even].as_mut() else {
                    continue;
                };
                let before = group.num_states[odd_even];
                let after = states.and_assign(&table.bits);
                group.num_states[odd_even] = after;
                if after != before {
                    group.all_bitflips_dirty[odd_even] = true;
                }
            }
        }
    }

    fn update_all_bitflips(&mut self) {
        if self.stage & CHECK_2ND_BYTES == 0 {
            return;
        }
        for odd_even in 0..2 {
            let mut changed = false;
            for first_byte in 0..256 {
                if !self.nonces.groups[first_byte].all_bitflips_dirty[odd_even] {
                    continue;
                }
                if let Some(states) = &self.nonces.groups[first_byte].states[odd_even] {
                    let before = self.all_bitflips[odd_even].count_ones();
                    let after = self.all_bitflips[odd_even].low20_and_assign(states);
                    if after != before {
                        changed = true;
                    }
                }
                self.nonces.groups[first_byte].all_bitflips_dirty[odd_even] = false;
            }
            if changed {
                self.all_changed[odd_even] = true;
            }
        }
    }

    fn update_sum_bitarrays(&mut self, odd_even: usize) {
        if !self.all_changed[odd_even] {
            return;
        }
        for part in 0..NUM_PART_SUMS {
            self.filtered_part_sums[odd_even].0[part].and_assign(&self.all_bitflips[odd_even]);
            self.filtered_part_sums[odd_even].1[part].and_assign(&self.all_bitflips[odd_even]);
        }

        for first_byte in 0..256 {
            if let Some(states) = &self.nonces.groups[first_byte].states[odd_even] {
                self.nonces.groups[first_byte].num_states[odd_even] =
                    states.and_count(&self.all_bitflips[odd_even]);
            }
        }
        for p in 0..NUM_PART_SUMS {
            for q in 0..NUM_PART_SUMS {
                self.part_sum_count[odd_even][p][q] += self.filtered_part_sums[odd_even].0[p]
                    .and_count(&self.filtered_part_sums[odd_even].1[q]);
            }
        }
        self.all_changed[odd_even] = false;
    }

    fn update_p_k(&mut self) {
        if self.stage & CHECK_2ND_BYTES == 0 {
            return;
        }
        let sum_a0 = SUMS[self.first_byte_sum_idx];
        let mut total = 0u64;
        let mut counts = [0u64; NUM_SUMS];
        for (index, count) in counts.iter_mut().enumerate() {
            *count = self.estimated_num_states_coarse(sum_a0, SUMS[index]);
            total += *count;
        }
        if total == 0 {
            return;
        }
        for (index, count) in counts.iter().enumerate() {
            self.p_k[index] = *count as f64 / total as f64;
        }
    }

    pub fn estimated_num_states_coarse(&self, sum_a0: u16, sum_a8: u16) -> u64 {
        let Some(a0_index) = super::nonces::sum_index(sum_a0) else {
            return 0;
        };
        let Some(a8_index) = super::nonces::sum_index(sum_a8) else {
            return 0;
        };
        let mut total = 0u64;
        for &(p, q) in &SUM_PAIRS[a0_index] {
            for &(r, s) in &SUM_PAIRS[a8_index] {
                total += u64::from(self.part_sum_count[1][p as usize][r as usize])
                    * u64::from(self.part_sum_count[0][q as usize][s as usize]);
            }
        }
        total
    }

    pub fn estimated_num_states(&self, first_byte: usize, sum_a0: u16, sum_a8: u16) -> u64 {
        let Some(a0_index) = super::nonces::sum_index(sum_a0) else {
            return 0;
        };
        let Some(a8_index) = super::nonces::sum_index(sum_a8) else {
            return 0;
        };
        let mut total = 0u64;
        for &(p, q) in &SUM_PAIRS[a0_index] {
            for &(r, s) in &SUM_PAIRS[a8_index] {
                total += u64::from(
                    self.estimated_num_states_part_sum(first_byte, p as usize, r as usize, 1),
                ) * u64::from(
                    self.estimated_num_states_part_sum(first_byte, q as usize, s as usize, 0),
                );
            }
        }
        total
    }

    fn estimated_num_states_part_sum(
        &self,
        first_byte: usize,
        part_a0: usize,
        part_a8: usize,
        odd_even: usize,
    ) -> u32 {
        let group = &self.nonces.groups[first_byte];
        let a0 = &self.filtered_part_sums[odd_even].0[part_a0];
        let a8 = &self.filtered_part_sums[odd_even].1[part_a8];
        if odd_even == 1 {
            count_and_optional3(a0, a8, group.states[odd_even].as_ref())
        } else {
            count_and_optional4(
                a0,
                a8,
                group.states[odd_even].as_ref(),
                self.nonces.groups[first_byte ^ 0x80].states[odd_even].as_ref(),
            )
        }
    }

    fn update_reduction_rate(&mut self, last: f64) -> f64 {
        if !self.reduction_initialized {
            self.reduction_queue = [(1u64 << 48) as f64; 4];
            self.reduction_initialized = true;
        } else {
            self.reduction_queue.rotate_left(1);
        }
        self.reduction_queue[3] = last;
        let avg_x = 1.5;
        let avg_y = self.reduction_queue.iter().sum::<f64>() / 4.0;
        let mut dev_xy = 0.0;
        let mut dev_x2 = 0.0;
        for (index, value) in self.reduction_queue.iter().enumerate() {
            let x = index as f64;
            dev_xy += (x - avg_x) * (value - avg_y);
            dev_x2 += (x - avg_x) * (x - avg_x);
        }
        -dev_xy / dev_x2
    }

    pub fn check_smallest_bitflip_bitarrays(&mut self) -> f64 {
        let mut smallest = (1u64 << 48) as f64;
        let mut best = 0;
        for (first_byte, group) in self.nonces.groups.iter().enumerate().take(256) {
            if group.states[0].is_none() || group.states[1].is_none() {
                continue;
            }
            let product = u64::from(group.num_states[0]) * u64::from(group.num_states[1]);
            if (product as f64) < smallest {
                smallest = product as f64;
                best = first_byte;
            }
        }
        self.best_first_byte_smallest_bitarray = best;
        smallest / 2.0
    }

    pub fn sort_best_first_bytes(&mut self) -> f64 {
        let sum_a0 = SUMS[self.first_byte_sum_idx];
        for first_byte in 0..256 {
            self.best_first_bytes[first_byte] = first_byte;
            let guesses = self.nonces.groups[first_byte].sum_a8_guess;
            let mut prob_all_failed = 1.0;
            let mut expected = 0.0;
            for (guess_index, guess) in guesses.iter().enumerate() {
                let num_states = self.estimated_num_states_coarse(sum_a0, SUMS[guess.sum_a8_idx]);
                self.nonces.groups[first_byte].sum_a8_guess[guess_index].num_states = num_states;
                expected += guess.prob * num_states as f64 / 2.0;
                prob_all_failed -= guess.prob;
                expected += prob_all_failed * num_states as f64 / 2.0;
            }
            self.nonces.groups[first_byte].expected_num_brute_force = expected;
        }
        let mut order: Vec<usize> = (0..256).collect();
        order.sort_by(|left, right| {
            self.nonces.groups[*left]
                .expected_num_brute_force
                .total_cmp(&self.nonces.groups[*right].expected_num_brute_force)
        });
        self.best_first_bytes.copy_from_slice(&order);

        let first_byte = self.best_first_bytes[0];
        let guesses = self.nonces.groups[first_byte].sum_a8_guess;
        for (guess_index, guess) in guesses.iter().enumerate() {
            if guess.prob <= 0.05 {
                break;
            }
            let num_states = self.estimated_num_states(first_byte, sum_a0, SUMS[guess.sum_a8_idx]);
            self.nonces.groups[first_byte].sum_a8_guess[guess_index].num_states = num_states;
        }
        self.nonces.update_expected_brute_force(first_byte);

        let mut best = 0;
        let mut least = (1u64 << 48) as f64;
        for index in 0..10 {
            let first_byte = self.best_first_bytes[index];
            if self.nonces.groups[first_byte].expected_num_brute_force < least {
                least = self.nonces.groups[first_byte].expected_num_brute_force;
                best = index;
            }
        }
        if best != 0 {
            self.best_first_bytes.swap(0, best);
        }
        self.nonces.groups[self.best_first_bytes[0]].expected_num_brute_force
    }

    /// PM3 `shrink_key_space`: returns the estimated remaining key space and
    /// whether collection can stop.
    pub fn shrink_key_space(&mut self) -> (f64, bool) {
        let brute_forces1 = self.check_smallest_bitflip_bitarrays();
        let mut brute_forces2 = (1u64 << 47) as f64;
        if self.stage & CHECK_2ND_BYTES != 0 {
            brute_forces2 = self.sort_best_first_bytes();
        }
        let brute_forces = brute_forces1.min(brute_forces2);
        let reduction_rate = self.update_reduction_rate(brute_forces);
        let sample_period = if self.sample_period_ms > 0.0 {
            self.sample_period_ms
        } else {
            100.0
        };
        let stop = self.stage & CHECK_2ND_BYTES != 0
            && reduction_rate >= 0.0
            && (reduction_rate < ESTIMATED_KEYS_PER_SECOND * sample_period / 1000.0
                || brute_forces < STOP_BRUTE_FORCE);
        (brute_forces, stop)
    }

    pub fn best_first_byte(&self) -> usize {
        self.best_first_bytes[0]
    }

    /// PM3 `add_bitflip_candidates`.
    pub fn bitflip_candidates(&self, first_byte: usize) -> StateList {
        let mut list = StateList::default();
        for odd_even in 0..2 {
            if let Some(states) = &self.nonces.groups[first_byte].states[odd_even] {
                list_for_state(odd_even, first_byte, states, &mut list, self);
            }
        }
        list
    }

    /// PM3 `generate_candidates`.
    pub fn generate_candidates(&self, sum_a0_idx: usize, sum_a8_idx: usize) -> Vec<StateList> {
        let first_byte = self.best_first_byte();
        let mut odd_lists: HashMap<(usize, usize), Vec<u32>> = HashMap::new();
        let mut even_lists: HashMap<(usize, usize), Vec<u32>> = HashMap::new();
        let mut candidates = Vec::new();

        for &(p, q) in &SUM_PAIRS[sum_a0_idx] {
            for &(r, s) in &SUM_PAIRS[sum_a8_idx] {
                let odd = odd_lists
                    .entry((p as usize, r as usize))
                    .or_insert_with(|| self.matching_states(first_byte, p as usize, r as usize, 1));
                if odd.is_empty() {
                    continue;
                }
                let even = even_lists
                    .entry((q as usize, s as usize))
                    .or_insert_with(|| self.matching_states(first_byte, q as usize, s as usize, 0));
                if even.is_empty() {
                    continue;
                }
                candidates.push(StateList {
                    odd: odd.clone(),
                    even: even.clone(),
                });
            }
        }
        candidates
    }

    fn matching_states(
        &self,
        first_byte: usize,
        part_a0: usize,
        part_a8: usize,
        odd_even: usize,
    ) -> Vec<u32> {
        let a0 = &self.filtered_part_sums[odd_even].0[part_a0];
        let a8 = &self.filtered_part_sums[odd_even].1[part_a8];
        let bitflips = self.nonces.groups[first_byte].states[odd_even]
            .as_ref()
            .expect("candidate first byte has nonces");
        let mut bits = BitSet24::zeroed();
        let words = bits.words_mut();
        for (index, word) in words.iter_mut().enumerate() {
            *word = a0.words()[index] & a8.words()[index] & bitflips.words()[index];
        }
        let mut states = Vec::new();
        for state in bits.iter_states() {
            if self.all_bitflips_match(first_byte, state, odd_even) {
                states.push(state as u32);
            }
        }
        states
    }

    fn bitflips_match(&self, first_byte: usize, state: usize, odd_even: usize) -> bool {
        match &self.nonces.groups[first_byte].states[odd_even] {
            Some(bits) => bits.get(state),
            None => true,
        }
    }

    /// PM3 `all_bitflips_match`: checks the invariant of all other first-byte
    /// values for a candidate state.
    fn all_bitflips_match(&self, first_byte: usize, state: usize, odd_even: usize) -> bool {
        let masks: [[u32; 8]; 2] = [
            [
                0x00ff_fff0,
                0x00ff_fff8,
                0x00ff_fff8,
                0x00ff_fffc,
                0x00ff_fffc,
                0x00ff_fffe,
                0x00ff_fffe,
                0x00ff_ffff,
            ],
            [
                0x00ff_fff0,
                0x00ff_fff0,
                0x00ff_fff8,
                0x00ff_fff8,
                0x00ff_fffc,
                0x00ff_fffc,
                0x00ff_fffe,
                0x00ff_fffe,
            ],
        ];
        let state = state as u32;
        for index in 1..256u32 {
            let bytes_diff = (index as u8).reverse_bits();
            let byte2 = (first_byte as u8 ^ bytes_diff) as usize;
            let num_common = bytes_diff.trailing_zeros() as usize;
            let mask = masks[odd_even][num_common];
            let mut found = false;
            let remaining = !mask & 0xff;
            for remaining_bits in 0..=remaining {
                let candidate = (state & mask) | remaining_bits;
                if remaining_bits_match(num_common, bytes_diff, state, candidate, odd_even)
                    && self.bitflips_match(byte2, candidate as usize, odd_even)
                {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        true
    }
}

fn list_for_state(
    odd_even: usize,
    first_byte: usize,
    bits: &BitSet24,
    list: &mut StateList,
    filter: &Filter<'_>,
) {
    let target = if odd_even == 1 {
        &mut list.odd
    } else {
        &mut list.even
    };
    for state in bits.iter_states() {
        if filter.all_bitflips_match(first_byte, state, odd_even) {
            target.push(state as u32);
        }
    }
}

fn count_and_optional3(a: &BitSet24, b: &BitSet24, c: Option<&BitSet24>) -> u32 {
    match c {
        Some(c) => a.and3_count(b, c),
        None => a.and_count(b),
    }
}

fn count_and_optional4(
    a: &BitSet24,
    b: &BitSet24,
    c: Option<&BitSet24>,
    d: Option<&BitSet24>,
) -> u32 {
    let mut count = 0;
    for (index, word) in a.words().iter().enumerate() {
        let mut value = *word & b.words()[index];
        if let Some(c) = c {
            value &= c.words()[index];
        }
        if let Some(d) = d {
            value &= d.words()[index];
        }
        count += value.count_ones();
    }
    count
}

fn invariant_holds(byte_diff: u8, state1: u32, state2: u32, bit: u8, state_bit: u8) -> bool {
    let j_1_bit_mask = 0x01u8 << (bit - 1);
    let bit_diff = byte_diff & j_1_bit_mask;
    let filter_diff = filter(state1 >> (4 - state_bit)) ^ filter(state2 >> (4 - state_bit));
    let mask_y12_y13 = 0xc0u32 >> state_bit;
    let state_bits_diff = (state1 ^ state2) & mask_y12_y13;
    let all_diff = (bit_diff as u32 ^ state_bits_diff ^ u32::from(filter_diff)).count_ones() & 1;
    all_diff == 0
}

fn invalid_state(byte_diff: u8, state1: u32, state2: u32, bit: u8, state_bit: u8) -> bool {
    let j_bit_mask = 0x01u8 << bit;
    let bit_diff = byte_diff & j_bit_mask;
    let mask_y13_y16 = 0x48u32 >> state_bit;
    let state_bits_diff = (state1 ^ state2) & mask_y13_y16;
    (u32::from(bit_diff) ^ state_bits_diff).count_ones() & 1 != 0
}

fn remaining_bits_match(
    num_common_bits: usize,
    byte_diff: u8,
    state1: u32,
    state2: u32,
    odd_even: usize,
) -> bool {
    if odd_even == 1 {
        match num_common_bits {
            0 => {
                if !invariant_holds(byte_diff, state1, state2, 1, 0) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 1, 0) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 3, 1) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 3, 1) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 5, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 5, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 7, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 7, 3) {
                    return false;
                }
            }
            1 => {
                if invalid_state(byte_diff, state1, state2, 1, 0) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 3, 1) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 3, 1) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 5, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 5, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 7, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 7, 3) {
                    return false;
                }
            }
            2 => {
                if !invariant_holds(byte_diff, state1, state2, 3, 1) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 3, 1) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 5, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 5, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 7, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 7, 3) {
                    return false;
                }
            }
            3 => {
                if invalid_state(byte_diff, state1, state2, 3, 1) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 5, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 5, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 7, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 7, 3) {
                    return false;
                }
            }
            4 => {
                if !invariant_holds(byte_diff, state1, state2, 5, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 5, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 7, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 7, 3) {
                    return false;
                }
            }
            5 => {
                if invalid_state(byte_diff, state1, state2, 5, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 7, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 7, 3) {
                    return false;
                }
            }
            6 => {
                if !invariant_holds(byte_diff, state1, state2, 7, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 7, 3) {
                    return false;
                }
            }
            7 if invalid_state(byte_diff, state1, state2, 7, 3) => return false,
            _ => {}
        }
    } else {
        match num_common_bits {
            0 => {
                if invalid_state(byte_diff, state1, state2, 0, 0) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 2, 1) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 2, 1) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 4, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 4, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 6, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 6, 3) {
                    return false;
                }
            }
            1 => {
                if !invariant_holds(byte_diff, state1, state2, 2, 1) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 2, 1) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 4, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 4, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 6, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 6, 3) {
                    return false;
                }
            }
            2 => {
                if invalid_state(byte_diff, state1, state2, 2, 1) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 4, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 4, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 6, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 6, 3) {
                    return false;
                }
            }
            3 => {
                if !invariant_holds(byte_diff, state1, state2, 4, 2) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 4, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 6, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 6, 3) {
                    return false;
                }
            }
            4 => {
                if invalid_state(byte_diff, state1, state2, 4, 2) {
                    return false;
                }
                if !invariant_holds(byte_diff, state1, state2, 6, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 6, 3) {
                    return false;
                }
            }
            5 => {
                if !invariant_holds(byte_diff, state1, state2, 6, 3) {
                    return true;
                }
                if invalid_state(byte_diff, state1, state2, 6, 3) {
                    return false;
                }
            }
            6 if invalid_state(byte_diff, state1, state2, 6, 3) => return false,
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_sum_property_is_in_range() {
        for state in [0, 1, 0x0029_ce5c, 0x000f_ffff, 0x0001_2345] {
            let even = partial_sum_property(state, 0);
            let odd = partial_sum_property(state, 1);
            assert!(even <= 16);
            assert!(odd <= 16);
            assert!(SUMS.contains(&(sum_property(odd, even) as u16)));
        }
    }

    #[test]
    fn part_sum_sets_are_disjoint_and_cover_the_state_space() {
        for sums in &PART_SUMS.a0 {
            for part in sums {
                assert!(!part.is_empty());
            }
        }
        assert_eq!(
            PART_SUMS.a0[0][0].count_ones()
                + PART_SUMS.a0[0][1].count_ones()
                + PART_SUMS.a0[0][2].count_ones()
                + PART_SUMS.a0[0][3].count_ones()
                + PART_SUMS.a0[0][4].count_ones()
                + PART_SUMS.a0[0][5].count_ones()
                + PART_SUMS.a0[0][6].count_ones()
                + PART_SUMS.a0[0][7].count_ones()
                + PART_SUMS.a0[0][8].count_ones(),
            1 << 24
        );
    }

    #[test]
    fn invariant_helpers_reject_nothing_for_equal_states() {
        // Equal states have no bit difference, so the "invalid" check must
        // never fire and the invariant must hold.
        for bit in 0..8 {
            assert!(!invalid_state(0, 0x123456, 0x123456, bit, 0));
        }
    }
}
