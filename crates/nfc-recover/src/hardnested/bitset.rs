// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! 24-bit state bitsets with the bit order used by the PM3 hardnested tables.
//!
//! Bit `i` lives in word `i / 32` at mask `0x8000_0000 >> (i % 32)`, so the raw
//! table bytes can be used without a bit-order conversion.

pub const STATE_COUNT: usize = 1 << 24;
pub const WORD_COUNT: usize = STATE_COUNT / 32;

#[derive(Clone, PartialEq, Eq)]
pub struct BitSet24 {
    words: Vec<u32>,
}

impl BitSet24 {
    pub fn zeroed() -> Self {
        Self {
            words: vec![0; WORD_COUNT],
        }
    }

    pub fn all() -> Self {
        Self {
            words: vec![u32::MAX; WORD_COUNT],
        }
    }

    pub fn from_words(words: Vec<u32>) -> Self {
        debug_assert_eq!(words.len(), WORD_COUNT);
        Self { words }
    }

    pub fn words(&self) -> &[u32] {
        &self.words
    }

    pub fn words_mut(&mut self) -> &mut [u32] {
        &mut self.words
    }

    #[inline]
    pub fn set(&mut self, index: usize) {
        debug_assert!(index < STATE_COUNT);
        self.words[index >> 5] |= 0x8000_0000 >> (index & 31);
    }

    #[inline]
    pub fn get(&self, index: usize) -> bool {
        debug_assert!(index < STATE_COUNT);
        self.words[index >> 5] & (0x8000_0000 >> (index & 31)) != 0
    }

    pub fn count_ones(&self) -> u32 {
        self.words.iter().map(|word| word.count_ones()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// Intersects in place and returns the resulting population count.
    pub fn and_assign(&mut self, other: &Self) -> u32 {
        let mut count = 0;
        for (left, right) in self.words.iter_mut().zip(&other.words) {
            *left &= *right;
            count += left.count_ones();
        }
        count
    }

    /// PM3 `COUNT_BITARRAY_LOW20_AND`: a zero 16-bit chunk in `other` clears
    /// the matching chunk, but a non-zero chunk leaves `self` untouched. This
    /// is deliberately coarser than an intersection.
    pub fn low20_and_assign(&mut self, other: &Self) -> u32 {
        let mut count = 0;
        for (left, right) in self.words.iter_mut().zip(&other.words) {
            let mut value = *left;
            if right & 0xffff_0000 == 0 {
                value &= 0x0000_ffff;
            }
            if right & 0x0000_ffff == 0 {
                value &= 0xffff_0000;
            }
            *left = value;
            count += value.count_ones();
        }
        count
    }

    pub fn and_count(&self, other: &Self) -> u32 {
        self.words
            .iter()
            .zip(&other.words)
            .map(|(left, right)| (left & right).count_ones())
            .sum()
    }

    pub fn and3_count(&self, second: &Self, third: &Self) -> u32 {
        self.words
            .iter()
            .zip(&second.words)
            .zip(&third.words)
            .map(|((first, second), third)| (first & second & third).count_ones())
            .sum()
    }

    pub fn and4_count(&self, second: &Self, third: &Self, fourth: &Self) -> u32 {
        self.words
            .iter()
            .zip(&second.words)
            .zip(&third.words)
            .zip(&fourth.words)
            .map(|(((first, second), third), fourth)| {
                (first & second & third & fourth).count_ones()
            })
            .sum()
    }

    pub fn or_assign(&mut self, other: &Self) {
        for (left, right) in self.words.iter_mut().zip(&other.words) {
            *left |= *right;
        }
    }

    pub fn iter_states(&self) -> StateIter<'_> {
        StateIter {
            words: &self.words,
            word_index: 0,
            current: 0,
            base: 0,
        }
    }
}

pub struct StateIter<'a> {
    words: &'a [u32],
    word_index: usize,
    current: u32,
    base: usize,
}

impl Iterator for StateIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current == 0 {
            if self.word_index >= self.words.len() {
                return None;
            }
            self.current = self.words[self.word_index];
            self.base = self.word_index * 32;
            self.word_index += 1;
        }
        let bit = self.current.leading_zeros() as usize;
        self.current &= !(0x8000_0000u32 >> bit);
        Some(self.base + bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_order_matches_pm3_table_words() {
        let mut bits = BitSet24::zeroed();
        bits.set(0);
        bits.set(31);
        bits.set(32);
        bits.set(1 << 23);
        assert_eq!(bits.words()[0], 0x8000_0001);
        assert_eq!(bits.words()[1], 0x8000_0000);
        assert_eq!(bits.words()[(1 << 23) >> 5], 0x8000_0000);
        assert_eq!(bits.count_ones(), 4);
        assert!(bits.get(0));
        assert!(bits.get(31));
        assert!(!bits.get(33));
    }

    #[test]
    fn iterator_yields_states_in_ascending_order() {
        let mut bits = BitSet24::zeroed();
        for state in [5usize, 63, 64, 1000, STATE_COUNT - 1] {
            bits.set(state);
        }
        let states: Vec<usize> = bits.iter_states().collect();
        assert_eq!(states, vec![5, 63, 64, 1000, STATE_COUNT - 1]);
    }

    #[test]
    fn low20_and_only_clears_empty_chunks() {
        let mut left = BitSet24::all();
        let mut right = BitSet24::zeroed();
        right.set(17);
        // The first 16-bit chunk is empty, the second contains state 17.
        left.low20_and_assign(&right);
        assert_eq!(left.words()[0], 0x0000_ffff);
        assert_eq!(left.words()[1], 0);
    }

    #[test]
    fn and_and_or_update_counts() {
        let mut left = BitSet24::zeroed();
        let mut right = BitSet24::zeroed();
        for state in 0..100 {
            left.set(state);
        }
        for state in 50..150 {
            right.set(state);
        }
        assert_eq!(left.and_count(&right), 50);
        let mut union = left.clone();
        union.or_assign(&right);
        assert_eq!(union.count_ones(), 150);
        assert_eq!(left.and_assign(&right), 50);
        assert_eq!(left.count_ones(), 50);
    }
}
