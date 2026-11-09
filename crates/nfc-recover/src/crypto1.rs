// SPDX-FileCopyrightText: 2008-2014 bla <blapost@gmail.com>
// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Crypto1, the MIFARE Classic stream cipher, and the nonce/LFSR recovery
//! primitives used by the classic attacks.
//!
//! Ported from the current `RfidResearchGroup/proxmark3` `common/crapto1`
//! implementation. PM3-derived code is GPL-3.0-or-later; see `NOTICE` and
//! `THIRD_PARTY_NOTICES.md`.

use std::sync::LazyLock;

pub const LF_POLY_ODD: u32 = 0x29CE5C;
pub const LF_POLY_EVEN: u32 = 0x870804;

// PM3 indexes these as `fastfwd[isodd]`: row 0 is the even prefix, row 1 is
// the odd prefix.
const FWD_EVEN: [u32; 8] = [
    0, 0x4BC53, 0xECB1, 0x450E2, 0x25E29, 0x6E27A, 0x2B298, 0x60ECB,
];
const FWD_ODD: [u32; 8] = [
    0, 0x1D962, 0x4BC53, 0x56531, 0xECB1, 0x135D3, 0x450E2, 0x58980,
];

/// Crypto1 shift-register state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Crypto1State {
    pub odd: u32,
    pub even: u32,
}

impl Crypto1State {
    pub fn from_key(key: u64) -> Self {
        let mut state = Self::default();
        state.init(key);
        state
    }

    /// Loads the 48-bit key into the LFSR as PM3 `crypto1_init` does.
    pub fn init(&mut self, key: u64) {
        self.odd = 0;
        self.even = 0;
        let mut index = 47;
        while index > 0 {
            self.odd = self.odd << 1 | ((key >> ((index - 1) ^ 7)) & 1) as u32;
            self.even = self.even << 1 | ((key >> (index ^ 7)) & 1) as u32;
            index -= 2;
        }
    }

    /// Extracts the 48-bit key from the current cipher state.
    pub fn key(&self) -> u64 {
        let mut lfsr = 0;
        let mut index = 23;
        loop {
            lfsr = lfsr << 1 | u64::from((self.odd >> (index ^ 3)) & 1);
            lfsr = lfsr << 1 | u64::from((self.even >> (index ^ 3)) & 1);
            if index == 0 {
                break;
            }
            index -= 1;
        }
        lfsr
    }

    /// Advances the cipher by one bit and returns the keystream bit.
    pub fn bit(&mut self, input: u8, is_encrypted: bool) -> u8 {
        let ret = filter(self.odd);
        let mut feedin = u32::from(ret & u8::from(is_encrypted));
        feedin ^= u32::from(input != 0);
        feedin ^= LF_POLY_ODD & self.odd;
        feedin ^= LF_POLY_EVEN & self.even;
        self.even = self.even << 1 | parity32(feedin);

        std::mem::swap(&mut self.odd, &mut self.even);
        ret
    }

    pub fn byte(&mut self, input: u8, is_encrypted: bool) -> u8 {
        let mut ret = 0;
        for index in 0..8 {
            ret |= self.bit((input >> index) & 1, is_encrypted) << index;
        }
        ret
    }

    pub fn word(&mut self, input: u32, is_encrypted: bool) -> u32 {
        let mut ret = 0;
        for index in 0..32 {
            ret |= u32::from(self.bit(((input >> (index ^ 24)) & 1) as u8, is_encrypted))
                << (index ^ 24);
        }
        ret
    }

    /// Rolls the cipher back by one bit, used while reconstructing keys.
    pub fn rollback_bit(&mut self, input: u32, feedback: bool) -> u8 {
        self.odd &= 0x00FF_FFFF;
        std::mem::swap(&mut self.odd, &mut self.even);

        let mut out = self.even & 1;
        out ^= LF_POLY_EVEN & (self.even >> 1);
        self.even >>= 1;
        out ^= LF_POLY_ODD & self.odd;
        out ^= input & 1;
        let ret = filter(self.odd);
        out ^= u32::from(ret) & u32::from(feedback);

        self.even |= parity32(out) << 23;
        ret
    }

    pub fn rollback_byte(&mut self, input: u8, feedback: bool) -> u8 {
        let mut ret = 0;
        for index in (0..8).rev() {
            ret |= self.rollback_bit(u32::from((input >> index) & 1), feedback) << index;
        }
        ret
    }

    pub fn rollback_word(&mut self, input: u32, feedback: bool) -> u32 {
        let mut ret = 0;
        for index in (0..32).rev() {
            let input_bit = (input >> (index ^ 24)) & 1;
            ret |= u32::from(self.rollback_bit(input_bit, feedback)) << (index ^ 24);
        }
        ret
    }
}

/// XOR/parity of all set bits. PM3's `evenparity32` compiles to
/// `__builtin_parity`, which returns one for an odd number of set bits.
#[inline]
pub fn parity32(value: u32) -> u32 {
    value.count_ones() & 1
}

/// Odd parity bit for one byte: one when the byte has an even number of set
/// bits, so the byte plus parity always contains an odd number of ones.
#[inline]
pub fn odd_parity8(value: u8) -> u8 {
    !(value.count_ones() as u8 & 1) & 1
}

/// Crypto1 filter function over the odd LFSR state.
#[inline]
pub fn filter(x: u32) -> u8 {
    let mut f = 0xF22C0u32 >> (x & 0xF) & 16;
    f |= 0x6C9C0u32 >> ((x >> 4) & 0xF) & 8;
    f |= 0x3C8B0u32 >> ((x >> 8) & 0xF) & 4;
    f |= 0x1E458u32 >> ((x >> 12) & 0xF) & 2;
    f |= 0x0D938u32 >> ((x >> 16) & 0xF) & 1;
    ((0xEC57E80Au32 >> f) & 1) as u8
}

/// Advances the MIFARE Classic PRNG by `steps`.
pub fn prng_successor(mut x: u32, steps: u32) -> u32 {
    swap_endian(&mut x);
    for _ in 0..steps {
        x = x >> 1 | (x >> 16 ^ x >> 18 ^ x >> 19 ^ x >> 21) << 31;
    }
    swap_endian(&mut x);
    x
}

fn swap_endian(x: &mut u32) {
    *x = x.swap_bytes();
}

static DISTANCE: LazyLock<Box<[u16; 65536]>> = LazyLock::new(|| {
    let mut table = Box::new([0u16; 65536]);
    let mut x = 1u16;
    for index in 1..=u16::MAX {
        table[((x & 0xFF) << 8 | x >> 8) as usize] = index;
        x = x >> 1 | (x ^ x >> 2 ^ x >> 3 ^ x >> 5) << 15;
    }
    table
});

/// PRNG distance from `from` to `to`, or zero when they are equal.
pub fn nonce_distance(from: u32, to: u32) -> u16 {
    let from_index = u32::from(DISTANCE[(from >> 16) as usize]);
    let to_index = u32::from(DISTANCE[(to >> 16) as usize]);
    ((u32::from(u16::MAX) + to_index - from_index) % u32::from(u16::MAX)) as u16
}

/// Whether a nonce is consistent with the weak 16-bit MIFARE PRNG.
pub fn validate_prng_nonce(nonce: u32) -> bool {
    let mut x = (nonce >> 16) as u16;
    x = (x & 0xFF) << 8 | x >> 8;
    for _ in 0..16 {
        x = x >> 1 | (x ^ x >> 2 ^ x >> 3 ^ x >> 5) << 15;
    }
    x = (x & 0xFF) << 8 | x >> 8;
    u32::from(x) == (nonce & 0xFFFF)
}

/// Recover every cipher state compatible with 32 bits of keystream.
///
/// `input` is the value fed into the cipher when the keystream was generated.
pub fn lfsr_recovery32(keystream: u32, input: u32) -> Vec<Crypto1State> {
    let mut odd_ks = 0u32;
    let mut index = 31i32;
    while index >= 0 {
        odd_ks = odd_ks << 1 | ((keystream >> (((index as u32) ^ 24) & 31)) & 1);
        index -= 2;
    }
    let mut even_ks = 0u32;
    let mut index = 30i32;
    while index >= 0 {
        even_ks = even_ks << 1 | ((keystream >> (((index as u32) ^ 24) & 31)) & 1);
        index -= 2;
    }

    let odd_bit = (odd_ks & 1) as u8;
    let even_bit = (even_ks & 1) as u8;
    let mut odd = Vec::with_capacity(1 << 20);
    let mut even = Vec::with_capacity(1 << 20);
    for candidate in (0..=0x000F_FFFFu32).rev() {
        let value = filter(candidate);
        if value == odd_bit {
            odd.push(candidate);
        }
        if value == even_bit {
            even.push(candidate);
        }
    }

    for _ in 0..4 {
        odd_ks >>= 1;
        even_ks >>= 1;
        odd = extend_table_simple(&odd, (odd_ks & 1) as u8);
        even = extend_table_simple(&even, (even_ks & 1) as u8);
    }

    let swapped = (input >> 16 & 0xFF) | (input << 16) | (input & 0xFF00);
    recover(odd, even, odd_ks, even_ks, 11, swapped << 1)
}

fn recover(
    odd: Vec<u32>,
    even: Vec<u32>,
    mut odd_ks: u32,
    mut even_ks: u32,
    mut remaining: i32,
    mut input: u32,
) -> Vec<Crypto1State> {
    if remaining == -1 {
        let mut states = Vec::new();
        for mut even_entry in even {
            even_entry =
                even_entry << 1 ^ parity32(even_entry & LF_POLY_EVEN) ^ u32::from(input & 4 != 0);
            for &odd_entry in &odd {
                states.push(Crypto1State {
                    even: odd_entry,
                    odd: even_entry ^ parity32(odd_entry & LF_POLY_ODD),
                });
            }
        }
        return states;
    }

    let mut odd_table = odd;
    let mut even_table = even;
    let mut index = 0;
    while index < 4 {
        if remaining == 0 {
            remaining -= 1;
            break;
        }
        remaining -= 1;
        odd_ks >>= 1;
        even_ks >>= 1;
        input >>= 2;
        odd_table = extend_table(
            &odd_table,
            (odd_ks & 1) as u8,
            LF_POLY_EVEN << 1 | 1,
            LF_POLY_ODD << 1,
            0,
        );
        if odd_table.is_empty() {
            return Vec::new();
        }
        even_table = extend_table(
            &even_table,
            (even_ks & 1) as u8,
            LF_POLY_ODD,
            LF_POLY_EVEN << 1 | 1,
            input & 3,
        );
        if even_table.is_empty() {
            return Vec::new();
        }
        index += 1;
    }

    let mut odd_buckets: Vec<Vec<u32>> = (0..256).map(|_| Vec::new()).collect();
    let mut even_buckets: Vec<Vec<u32>> = (0..256).map(|_| Vec::new()).collect();
    for entry in odd_table {
        odd_buckets[(entry >> 24) as usize].push(entry);
    }
    for entry in even_table {
        even_buckets[(entry >> 24) as usize].push(entry);
    }

    let mut states = Vec::new();
    for (odd_bucket, even_bucket) in odd_buckets.into_iter().zip(even_buckets) {
        if odd_bucket.is_empty() || even_bucket.is_empty() {
            continue;
        }
        states.extend(recover(
            odd_bucket,
            even_bucket,
            odd_ks,
            even_ks,
            remaining,
            input,
        ));
    }
    states
}

fn update_contribution(item: &mut u32, mask1: u32, mask2: u32) {
    let mut contribution = *item >> 25;
    contribution = contribution << 1 | parity32(*item & mask1);
    contribution = contribution << 1 | parity32(*item & mask2);
    *item = contribution << 24 | (*item & 0x00FF_FFFF);
}

fn extend_table(table: &[u32], keystream_bit: u8, mask1: u32, mask2: u32, input: u32) -> Vec<u32> {
    let mut output = Vec::with_capacity(table.len() * 2);
    let input = input << 24;
    for &entry in table {
        let entry = entry << 1;
        let entry_filter = filter(entry);
        if entry_filter ^ filter(entry | 1) != 0 {
            let mut value = entry | u32::from(entry_filter ^ keystream_bit);
            update_contribution(&mut value, mask1, mask2);
            value ^= input;
            output.push(value);
        } else if entry_filter == keystream_bit {
            let mut first = entry | 1;
            update_contribution(&mut first, mask1, mask2);
            output.push(first ^ input);
            let mut second = entry;
            update_contribution(&mut second, mask1, mask2);
            output.push(second ^ input);
        }
    }
    output
}

fn extend_table_simple(table: &[u32], keystream_bit: u8) -> Vec<u32> {
    let mut output = Vec::with_capacity(table.len() * 2);
    for &entry in table {
        let entry = entry << 1;
        let entry_filter = filter(entry);
        if entry_filter ^ filter(entry | 1) != 0 {
            output.push(entry | u32::from(entry_filter ^ keystream_bit));
        } else if entry_filter == keystream_bit {
            output.push(entry | 1);
            output.push(entry);
        }
    }
    output
}

const S1: [u32; 19] = [
    0x62141, 0x310A0, 0x18850, 0x0C428, 0x06214, 0x0310A, 0x85E30, 0xC69AD, 0x634D6, 0xB5CDE,
    0xDE8DA, 0x6F46D, 0xB3C83, 0x59E41, 0xA8995, 0xD027F, 0x6813F, 0x3409F, 0x9E6FA,
];
const S2: [u32; 19] = [
    0x3A557B00, 0x5D2ABD80, 0x2E955EC0, 0x174AAF60, 0x0BA557B0, 0x05D2ABD8, 0x0449DE68, 0x048464B0,
    0x42423258, 0x278192A8, 0x156042D0, 0x0AB02168, 0x43F89B30, 0x61FC4D98, 0x765EAD48, 0x7D8FDD20,
    0x7EC7EE90, 0x7F63F748, 0x79117020,
];
const T1: [u32; 32] = [
    0x4F37D, 0x279BE, 0x97A6A, 0x4BD35, 0x25E9A, 0x12F4D, 0x097A6, 0x80D66, 0xC4006, 0x62003,
    0xB56B4, 0x5AB5A, 0xA9318, 0xD0F39, 0x6879C, 0xB057B, 0x582BD, 0x2C15E, 0x160AF, 0x8F6E2,
    0xC3DC4, 0xE5857, 0x72C2B, 0x39615, 0x98DBF, 0xC806A, 0xE0680, 0x70340, 0x381A0, 0x98665,
    0x4C332, 0xA272C,
];
const T2: [u32; 32] = [
    0x3C88B810, 0x5E445C08, 0x2982A580, 0x14C152C0, 0x4A60A960, 0x253054B0, 0x52982A58, 0x2FEC9EA8,
    0x1156C4D0, 0x08AB6268, 0x42F53AB0, 0x217A9D58, 0x161DC528, 0x0DAE6910, 0x46D73488, 0x25CB11C0,
    0x52E588E0, 0x6972C470, 0x34B96238, 0x5CFC3A98, 0x28DE96C8, 0x12CFC0E0, 0x4967E070, 0x64B3F038,
    0x74F97398, 0x7CDC3248, 0x38CE92A0, 0x1C674950, 0x0E33A4A8, 0x01B959D0, 0x40DCACE8, 0x26CEDDF0,
];
const C1: [u32; 3] = [0x846B5, 0x4235A, 0x211AD];
const C2: [u32; 3] = [0x1A822E0, 0x21A822E0, 0x21A822E0];

/// Recover every cipher state compatible with 64 bits of keystream.
pub fn lfsr_recovery64(ks2: u32, ks3: u32) -> Vec<Crypto1State> {
    let mut odd_ks = [0u8; 32];
    let mut even_ks = [0u8; 32];
    let mut high = [0u8; 32];
    let mut low = 0u32;
    let mut win = 0u32;
    let mut states = Vec::new();

    let mut index = 30i32;
    while index >= 0 {
        odd_ks[(index >> 1) as usize] = ((ks2 >> (((index as u32) ^ 24) & 31)) & 1) as u8;
        odd_ks[16 + (index >> 1) as usize] = ((ks3 >> (((index as u32) ^ 24) & 31)) & 1) as u8;
        index -= 2;
    }
    let mut index = 31i32;
    while index >= 0 {
        even_ks[(index >> 1) as usize] = ((ks2 >> (((index as u32) ^ 24) & 31)) & 1) as u8;
        even_ks[16 + (index >> 1) as usize] = ((ks3 >> (((index as u32) ^ 24) & 31)) & 1) as u8;
        index -= 2;
    }

    for candidate in (0..=0x000F_FFFFu32).rev() {
        if filter(candidate) != odd_ks[0] {
            continue;
        }
        let mut table = vec![candidate];
        let mut j = 1;
        while j < 29 && !table.is_empty() {
            table = extend_table_simple(&table, odd_ks[j]);
            j += 1;
        }
        if table.is_empty() {
            continue;
        }

        for s1 in S1 {
            low = low << 1 | parity32(candidate & s1);
        }
        for (j, t1) in T1.iter().enumerate() {
            high[j] = parity32(candidate & t1) as u8;
        }

        'tail: for tail in table.into_iter().rev() {
            let mut tail = tail;
            for j in 0..3 {
                tail = tail << 1 | parity32((candidate & C1[j]) ^ (tail & C2[j]));
                if filter(tail) != odd_ks[29 + j] {
                    continue 'tail;
                }
            }
            for s2 in S2 {
                win = win << 1 | parity32(tail & s2);
            }
            win ^= low;
            for (j, t2) in T2.iter().enumerate() {
                win = win << 1 ^ u32::from(high[j]) ^ parity32(tail & t2);
                if filter(win) != even_ks[j] {
                    continue 'tail;
                }
            }
            tail = tail << 1 | parity32(LF_POLY_EVEN & tail);
            states.push(Crypto1State {
                odd: tail ^ parity32(LF_POLY_ODD & win),
                even: win,
            });
        }
    }
    states
}

fn lfsr_prefix_ks(keystream: &[u8; 8], odd: bool) -> Vec<u32> {
    let forward = if odd { &FWD_ODD } else { &FWD_EVEN };
    let offset = u32::from(odd);
    let mut candidates = Vec::new();
    for candidate in 0..1 << 21 {
        let mut good = true;
        for index in 0..8 {
            let entry = candidate ^ forward[index];
            good &= u32::from((keystream[index] >> offset) & 1) == u32::from(filter(entry >> 1));
            good &= u32::from((keystream[index] >> (offset + 2)) & 1) == u32::from(filter(entry));
            if !good {
                break;
            }
        }
        if good {
            candidates.push(candidate);
        }
    }
    candidates
}

fn check_pfx_parity(
    prefix: u32,
    reader_response: u32,
    parities: &[[u8; 8]; 8],
    odd: u32,
    even: u32,
    no_parity: bool,
) -> Option<Crypto1State> {
    let mut good = 1u32;
    let mut state = Crypto1State { odd, even };
    for (index, parity_row) in parities.iter().enumerate() {
        if good == 0 {
            break;
        }
        state.odd = odd ^ FWD_ODD[index];
        state.even = even ^ FWD_EVEN[index];

        state.rollback_bit(0, false);
        state.rollback_bit(0, false);

        let ks3 = u32::from(state.rollback_bit(0, false));
        let ks2 = state.rollback_word(0, false);
        let ks1 = state.rollback_word(prefix | (index as u32) << 5, true);

        if no_parity {
            break;
        }

        let nr = ks1 ^ (prefix | (index as u32) << 5);
        let rr = ks2 ^ reader_response;
        good &= parity32(nr & 0x0000_00FF) ^ u32::from(parity_row[3]) ^ ((ks2 >> 24) & 1);
        good &= parity32(rr & 0xFF00_0000) ^ u32::from(parity_row[4]) ^ ((ks2 >> 16) & 1);
        good &= parity32(rr & 0x00FF_0000) ^ u32::from(parity_row[5]) ^ ((ks2 >> 8) & 1);
        good &= parity32(rr & 0x0000_FF00) ^ u32::from(parity_row[6]) ^ (ks2 & 1);
        good &= parity32(rr & 0x0000_00FF) ^ u32::from(parity_row[7]) ^ ks3;
    }
    (good != 0).then_some(state)
}

/// Recover candidate states from eight NACK observations that share one tag
/// nonce. `reader_nonce` must have its low five bits clear.
pub fn lfsr_common_prefix(
    prefix: u32,
    reader_response: u32,
    keystream: &[u8; 8],
    parities: &[[u8; 8]; 8],
    no_parity: bool,
) -> Vec<Crypto1State> {
    let mut odd = lfsr_prefix_ks(keystream, true);
    let mut even = lfsr_prefix_ks(keystream, false);
    let mut states = Vec::new();

    let mut odd_index = 0;
    while odd_index < odd.len() {
        let mut even_index = 0;
        while even_index < even.len() {
            for top in 0..64u32 {
                odd[odd_index] = odd[odd_index].wrapping_add(1 << 21);
                even[even_index] =
                    even[even_index].wrapping_add((u32::from(top & 7 == 0) + 1) << 21);
                if let Some(state) = check_pfx_parity(
                    prefix,
                    reader_response,
                    parities,
                    odd[odd_index],
                    even[even_index],
                    no_parity,
                ) {
                    states.push(state);
                }
            }
            even_index += 1;
        }
        odd_index += 1;
    }
    states
}

/// Recover a key from one darkside observation.
///
/// The parity and keystream words use the PM3 byte order: the first observed
/// NACK is the least significant byte.
pub fn nonce2key(
    uid: u32,
    nt: u32,
    mut reader_nonce: u32,
    reader_response: u32,
    parity_info: u64,
    keystream_info: u64,
) -> Vec<u64> {
    reader_nonce &= 0xFFFF_FF1F;
    let keystream = keystream_bytes(keystream_info);
    let parities = parity_rows(parity_info);
    let states = lfsr_common_prefix(
        reader_nonce,
        reader_response,
        &keystream,
        &parities,
        parity_info == 0,
    );
    let mut keys = Vec::with_capacity(states.len());
    for mut state in states {
        state.rollback_word(uid ^ nt, false);
        keys.push(state.key());
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Parses the 64-bit parity layout used by PM3's `mfkey` tools into eight rows.
pub fn parity_rows(parity_info: u64) -> [[u8; 8]; 8] {
    let mut rows = [[0u8; 8]; 8];
    for position in 0..8 {
        let byte = (parity_info >> (position * 8)) as u8;
        for (index, bit) in rows[7 - position].iter_mut().enumerate() {
            *bit = (byte >> index) & 1;
        }
    }
    rows
}

/// Parses the 64-bit nibble keystream layout used by PM3's `nonce2key`.
pub fn keystream_bytes(keystream_info: u64) -> [u8; 8] {
    let mut bytes = [0u8; 8];
    for position in 0..8 {
        bytes[7 - position] = (keystream_info >> (position * 8)) as u8 & 0x0F;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prng_successor_matches_known_pm3_values() {
        assert_eq!(prng_successor(0x0120_0145, 160), 0x7EEF_3586);
        assert_eq!(prng_successor(0x0120_0145, 16), 0x0145_C976);
    }

    #[test]
    fn nonce_distance_round_trips() {
        let start = 0x0120_0145u32;
        let later = prng_successor(start, 1234);
        assert_eq!(nonce_distance(start, later), 1234);
        assert_eq!(nonce_distance(0x0120_0145, 0x0120_0145), 0);
    }

    #[test]
    fn weak_prng_detection_matches_pm3() {
        assert!(validate_prng_nonce(0x0120_0145));
        assert!(validate_prng_nonce(prng_successor(0x0120_0145, 123)));
        assert!(!validate_prng_nonce(0xDEAD_BEEF));
    }

    #[test]
    fn crypto1_state_and_key_round_trip() {
        for key in [
            0u64,
            1,
            0xFFFF_FFFF_FFFF,
            0xA0A1_A2A3_A4A5,
            0x091E_639C_B715,
        ] {
            assert_eq!(Crypto1State::from_key(key).key(), key);
        }
    }

    #[test]
    fn even_parity_is_xor_of_bits() {
        assert_eq!(parity32(0), 0);
        assert_eq!(parity32(1), 1);
        assert_eq!(parity32(3), 0);
        assert_eq!(odd_parity8(0), 1);
        assert_eq!(odd_parity8(1), 0);
        assert_eq!(odd_parity8(0xFF), 1);
    }
}
