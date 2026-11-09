// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Encrypted nonce storage and the sum-property probability model used by the
//! hardnested attack.

use std::sync::LazyLock;

use super::bitset::BitSet24;

/// Number of possible sum property values and the values themselves.
pub const NUM_SUMS: usize = 19;
pub const SUMS: [u16; NUM_SUMS] = [
    0, 32, 56, 64, 80, 96, 104, 112, 120, 128, 136, 144, 152, 160, 176, 192, 200, 224, 256,
];
pub const NUM_PART_SUMS: usize = 9;

/// The probability that a random nonce has sum property `K`.
const P_K0: [f64; NUM_SUMS] = [
    0.0290, 0.0083, 0.0006, 0.0339, 0.0048, 0.0934, 0.0119, 0.0489, 0.0602, 0.4180, 0.0602, 0.0489,
    0.0119, 0.0934, 0.0048, 0.0339, 0.0006, 0.0083, 0.0290,
];

/// `(p, q)` part-sum pairs for every sum value. `p` applies to the odd state
/// and `q` to the even state.
pub static SUM_PAIRS: LazyLock<[Vec<(u8, u8)>; NUM_SUMS]> = LazyLock::new(|| {
    let mut pairs: [Vec<(u8, u8)>; NUM_SUMS] = std::array::from_fn(|_| Vec::new());
    for p in 0..NUM_PART_SUMS as u8 {
        for q in 0..NUM_PART_SUMS as u8 {
            let value = 2 * u32::from(p) * (16 - 2 * u32::from(q))
                + (16 - 2 * u32::from(p)) * 2 * u32::from(q);
            if let Some(index) = SUMS.iter().position(|sum| u32::from(*sum) == value) {
                pairs[index].push((p, q));
            }
        }
    }
    pairs
});

pub fn sum_index(value: u16) -> Option<usize> {
    SUMS.iter().position(|sum| *sum == value)
}

#[derive(Clone, Copy, Debug)]
pub struct NonceEntry {
    /// Encrypted tag nonce; before `pre_xor` this still contains the UID XOR.
    pub nonce_enc: u32,
    /// Encrypted parity bits, bit 3 for the first nonce byte through bit 0 for
    /// the fourth.
    pub par_enc: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Guess {
    pub sum_a8_idx: usize,
    pub prob: f64,
    pub num_states: u64,
}

impl Default for Guess {
    fn default() -> Self {
        Self {
            sum_a8_idx: 0,
            prob: 0.0,
            num_states: 0,
        }
    }
}

pub struct NonceGroup {
    pub entries: Vec<NonceEntry>,
    /// Number of distinct second nonce bytes seen for this first byte.
    pub num: u32,
    /// Sum of the second-byte parity indicators.
    pub sum: u32,
    pub bitflips: [bool; 0x400],
    pub states: [Option<BitSet24>; 2],
    pub num_states: [u32; 2],
    pub all_bitflips_dirty: [bool; 2],
    pub sum_a8_guess: [Guess; NUM_SUMS],
    pub sum_a8_guess_dirty: bool,
    pub expected_num_brute_force: f64,
}

impl Default for NonceGroup {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            num: 0,
            sum: 0,
            bitflips: [false; 0x400],
            states: [None, None],
            num_states: [0, 0],
            all_bitflips_dirty: [false, false],
            sum_a8_guess: std::array::from_fn(|index| Guess {
                sum_a8_idx: index,
                ..Guess::default()
            }),
            sum_a8_guess_dirty: false,
            expected_num_brute_force: 0.0,
        }
    }
}

impl NonceGroup {
    pub fn second_byte(&self, entry: &NonceEntry) -> u8 {
        (entry.nonce_enc >> 16) as u8
    }

    pub fn find_second_byte(&self, second: u8) -> Option<&NonceEntry> {
        self.entries
            .binary_search_by_key(&second, |entry| (entry.nonce_enc >> 16) as u8)
            .ok()
            .map(|index| &self.entries[index])
    }
}

#[derive(Default)]
pub struct NonceSet {
    pub groups: Vec<NonceGroup>,
    pub first_byte_num: usize,
    pub first_byte_sum: u32,
    /// Number of distinct nonces added, matching PM3's counter.
    pub num_acquired: usize,
}

impl NonceSet {
    pub fn new() -> Self {
        Self {
            groups: (0..256).map(|_| NonceGroup::default()).collect(),
            first_byte_num: 0,
            first_byte_sum: 0,
            num_acquired: 0,
        }
    }

    /// Adds one encrypted nonce. Returns false when the second-byte slot was
    /// already occupied, matching PM3's `add_nonce`.
    pub fn add(&mut self, nonce_enc: u32, par_enc: u8) -> bool {
        let first_byte = (nonce_enc >> 24) as usize;
        let second_byte = ((nonce_enc >> 16) & 0xff) as u8;
        let group = &mut self.groups[first_byte];
        match group
            .entries
            .binary_search_by_key(&second_byte, |entry| (entry.nonce_enc >> 16) as u8)
        {
            Ok(_) => return false,
            Err(position) => group
                .entries
                .insert(position, NonceEntry { nonce_enc, par_enc }),
        }

        if group.entries.len() == 1 {
            self.first_byte_num += 1;
            self.first_byte_sum += super::super::crypto1::parity32(
                (nonce_enc & 0xff00_0000) | u32::from(par_enc & 0x08),
            );
            group.states = [Some(BitSet24::all()), Some(BitSet24::all())];
            group.num_states = [1 << 24, 1 << 24];
        }

        group.num += 1;
        group.sum +=
            super::super::crypto1::parity32((nonce_enc & 0x00ff_0000) | u32::from(par_enc & 0x04));
        group.sum_a8_guess_dirty = true;
        self.num_acquired += 1;
        true
    }

    pub fn all_first_bytes_seen(&self) -> bool {
        self.first_byte_num == 256
    }

    pub fn first_byte_sum_index(&self) -> Option<usize> {
        sum_index(self.first_byte_sum as u16)
    }

    /// Removes the UID XOR from every nonce and parity bit before brute force.
    pub fn pre_xor(&mut self, cuid: u32) {
        for group in &mut self.groups {
            for entry in &mut group.entries {
                entry.nonce_enc ^= cuid;
                entry.par_enc ^= super::super::crypto1::odd_parity8((cuid & 0xff) as u8);
                entry.par_enc ^=
                    super::super::crypto1::odd_parity8(((cuid >> 8) & 0xff) as u8) << 1;
                entry.par_enc ^=
                    super::super::crypto1::odd_parity8(((cuid >> 16) & 0xff) as u8) << 2;
                entry.par_enc ^=
                    super::super::crypto1::odd_parity8(((cuid >> 24) & 0xff) as u8) << 3;
            }
        }
    }

    /// Recomputes the Sum(a8) probabilities once all first bytes are present.
    pub fn estimate_sum_a8(&mut self, p_k: &[f64; NUM_SUMS]) {
        if !self.all_first_bytes_seen() {
            return;
        }
        for group in &mut self.groups {
            if !group.sum_a8_guess_dirty {
                continue;
            }
            if group.entries.is_empty() {
                continue;
            }
            for guess in &mut group.sum_a8_guess {
                guess.prob = sum_probability(guess.sum_a8_idx, group.num, group.sum, p_k);
            }
            group
                .sum_a8_guess
                .sort_by(|left, right| right.prob.total_cmp(&left.prob));
            group.sum_a8_guess_dirty = false;
        }
    }

    /// PM3 `update_expected_brute_force` for one first byte.
    pub fn update_expected_brute_force(&mut self, first_byte: usize) {
        let group = &mut self.groups[first_byte];
        let total_prob: f64 = group.sum_a8_guess.iter().map(|guess| guess.prob).sum();
        if total_prob > 0.0 {
            for guess in &mut group.sum_a8_guess {
                guess.prob /= total_prob;
            }
        }
        let mut prob_all_failed = 1.0;
        let mut expected = 0.0;
        for guess in &group.sum_a8_guess {
            expected += guess.prob * guess.num_states as f64 / 2.0;
            prob_all_failed -= guess.prob;
            expected += prob_all_failed * guess.num_states as f64 / 2.0;
        }
        group.expected_num_brute_force = expected;
    }
}

pub fn initial_p_k() -> [f64; NUM_SUMS] {
    P_K0
}

fn p_hypergeometric(sum_index: usize, n: u32, k: u32) -> f64 {
    const N: i32 = 256;
    let k_big = i32::from(SUMS[sum_index]);
    let n = n as i32;
    let k = k as i32;

    if n - k > N - k_big || k > k_big {
        return 0.0;
    }
    if k == 0 {
        let mut log_result = 0.0;
        for i in (N - k_big - n + 1)..=(N - k_big) {
            log_result += (i as f64).ln();
        }
        for i in (N - n + 1)..=N {
            log_result -= (i as f64).ln();
        }
        return log_result.exp();
    }

    if n - k == N - k_big {
        let mut log_result = 0.0;
        for i in (k + 1)..=n {
            log_result += (i as f64).ln();
        }
        for i in (k_big + 1)..=N {
            log_result -= (i as f64).ln();
        }
        return log_result.exp();
    }

    p_hypergeometric(sum_index, n as u32, (k - 1) as u32)
        * f64::from(k_big - k + 1)
        * f64::from(n - k + 1)
        / (f64::from(k) * f64::from(N - k_big - n + k))
}

fn sum_probability(sum_index: usize, n: u32, k: u32, p_k: &[f64; NUM_SUMS]) -> f64 {
    if k > u32::from(SUMS[sum_index]) {
        return 0.0;
    }
    let p_t_is_k_when_s_is_k = p_hypergeometric(sum_index, n, k);
    let p_s_is_k = p_k[sum_index];
    let mut p_t_is_k = 0.0;
    for (index, probability) in p_k.iter().enumerate() {
        p_t_is_k += probability * p_hypergeometric(index, n, k);
    }
    if p_t_is_k == 0.0 {
        0.0
    } else {
        p_t_is_k_when_s_is_k * p_s_is_k / p_t_is_k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_pair_table_covers_every_property() {
        for (index, pairs) in SUM_PAIRS.iter().enumerate() {
            assert!(
                !pairs.is_empty(),
                "sum {} has no part-sum pairs",
                SUMS[index]
            );
            for &(p, q) in pairs {
                let value = 2 * u32::from(p) * (16 - 2 * u32::from(q))
                    + (16 - 2 * u32::from(p)) * 2 * u32::from(q);
                assert_eq!(value, u32::from(SUMS[index]));
            }
        }
        assert_eq!(SUM_PAIRS[9].len(), 17);
    }

    #[test]
    fn add_nonce_keeps_distinct_second_bytes_and_counts() {
        let mut nonces = NonceSet::new();
        assert!(nonces.add(0x1122_3344, 0b1101));
        assert!(!nonces.add(0x1122_3344, 0b1101));
        assert!(nonces.add(0x11aa_3344, 0b1101));
        let group = &nonces.groups[0x11];
        assert_eq!(group.entries.len(), 2);
        assert_eq!(group.num, 2);
        assert_eq!(nonces.num_acquired, 2);
        assert_eq!(nonces.first_byte_num, 1);
        assert!(group.states[0].is_some());
    }

    #[test]
    fn pre_xor_removes_uid_and_parity() {
        let mut nonces = NonceSet::new();
        nonces.add(0x1122_3344, 0b1101);
        nonces.pre_xor(0x1122_3344);
        let entry = &nonces.groups[0x11].entries[0];
        assert_eq!(entry.nonce_enc, 0);
        let uid_parity = super::super::super::crypto1::odd_parity8(0x44)
            | super::super::super::crypto1::odd_parity8(0x33) << 1
            | super::super::super::crypto1::odd_parity8(0x22) << 2
            | super::super::super::crypto1::odd_parity8(0x11) << 3;
        assert_eq!(entry.par_enc, 0b1101 ^ uid_parity);
    }

    #[test]
    fn hypergeometric_is_a_probability() {
        for n in [1u32, 8, 64, 256] {
            for k in 0..=n.min(SUMS[9] as u32) {
                let value = p_hypergeometric(9, n, k);
                assert!(
                    (-1e-9..=1.0 + 1e-9).contains(&value),
                    "n={n} k={k} value={value}"
                );
            }
        }
    }
}
