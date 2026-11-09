// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Crypto1 authentication and nonce acquisition over the reader parity
//! exchange.
//!
//! PM3's firmware protocol is not ported. The reader sends and receives raw
//! ISO/IEC 14443A frames with caller-supplied parity; Crypto1 state stays in
//! this crate, so nested authentication, nonce prediction, and cancellation
//! have one implementation for both supported readers.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nfc_core::{CardInfo, Key, KeyType, NfcError, Result};
use nfc_transport::{ParityFrame, Reader};

use crate::crypto1::{Crypto1State, filter, odd_parity8, prng_successor};

pub const MIFARE_AUTH_A: u8 = 0x60;
pub const MIFARE_AUTH_B: u8 = 0x61;

/// One completed MIFARE Classic authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthSession {
    pub uid: u32,
    pub tag_nonce: u32,
    pub state: Crypto1State,
}

/// A selected card kept in an authenticated session. Recovery mode stays
/// enabled until the session ends so nested authentication does not reset the
/// reader between frames.
pub struct Session<'a> {
    reader: &'a mut dyn Reader,
    recovery: bool,
    pub uid: u32,
    pub tag_nonce: u32,
    pub state: Crypto1State,
}

/// A nested authentication response before the key is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NestedNonce {
    pub tag_nonce_encrypted: u32,
    pub parity: u8,
}

/// CRC-A as used by ISO/IEC 14443A type A frames.
pub fn crc_a(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0x6363;
    for &byte in data {
        let mut mixed = byte ^ crc as u8;
        mixed ^= mixed << 4;
        crc = (crc >> 8) ^ ((mixed as u16) << 8) ^ ((mixed as u16) << 3) ^ ((mixed as u16) >> 4);
    }
    [crc as u8, (crc >> 8) as u8]
}

pub fn odd_parity(data: &[u8]) -> Vec<u8> {
    data.iter().map(|byte| odd_parity8(*byte)).collect()
}

/// Last four UID bytes in the wire order used by MIFARE authentication.
pub fn uid_tail(card: &CardInfo) -> Result<u32> {
    if card.uid.len() < 4 {
        return Err(NfcError::Unsupported(
            "MIFARE Classic authentication needs at least a four-byte UID",
        ));
    }
    let bytes = &card.uid[card.uid.len() - 4..];
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn key_to_u64(key: &Key) -> u64 {
    key.as_bytes()
        .iter()
        .fold(0u64, |value, byte| value << 8 | u64::from(*byte))
}

fn auth_command(block: u8, key_type: KeyType) -> (Vec<u8>, Vec<u8>) {
    let mut command = vec![
        match key_type {
            KeyType::A => MIFARE_AUTH_A,
            KeyType::B => MIFARE_AUTH_B,
        },
        block,
    ];
    let crc = crc_a(&command);
    command.extend_from_slice(&crc);
    let parity = odd_parity(&command);
    (command, parity)
}

fn command_with_crc(command: u8, block: u8) -> (Vec<u8>, Vec<u8>) {
    let mut data = vec![command, block];
    let crc = crc_a(&data);
    data.extend_from_slice(&crc);
    let parity = odd_parity(&data);
    (data, parity)
}

/// Reader nonce generator. It does not need to be secret, only fresh.
fn next_reader_nonce() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0x9E37_79B9);
    let counter = COUNTER.fetch_add(0x9E37_79B9, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() ^ duration.as_secs() as u32)
        .unwrap_or(0);
    let mut value = counter ^ time;
    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    value
}

/// Selects the card and returns its authentication UID tail.
pub fn select_card(reader: &mut dyn Reader) -> Result<(CardInfo, u32)> {
    let card = reader.card_info()?;
    let uid = uid_tail(&card)?;
    Ok((card, uid))
}

fn parse_nonce(response: &ParityFrame) -> Result<u32> {
    if response.data.len() != 4 || response.bits != 32 {
        return Err(NfcError::Io(format!(
            "MIFARE authentication returned {} bytes and {} bits instead of a 32-bit nonce",
            response.data.len(),
            response.bits
        )));
    }
    Ok(u32::from_be_bytes([
        response.data[0],
        response.data[1],
        response.data[2],
        response.data[3],
    ]))
}

impl<'a> Session<'a> {
    /// Selects the card, enters recovery mode, and completes Crypto1
    /// authentication with the given key.
    pub fn authenticate(
        reader: &'a mut dyn Reader,
        block: u8,
        key_type: KeyType,
        key: Key,
        reader_nonce: Option<u32>,
    ) -> Result<Self> {
        let (_, uid) = select_card(reader)?;
        reader.set_recovery_mode(true)?;
        let mut session = Self {
            reader,
            recovery: true,
            uid,
            tag_nonce: 0,
            state: Crypto1State::default(),
        };
        match session.authenticate_selected(block, key_type, key, reader_nonce) {
            Ok(()) => Ok(session),
            Err(error) => {
                let _ = session.leave();
                Err(error)
            }
        }
    }

    fn authenticate_selected(
        &mut self,
        block: u8,
        key_type: KeyType,
        key: Key,
        reader_nonce: Option<u32>,
    ) -> Result<()> {
        let (command, parity) = auth_command(block, key_type);
        let response = self
            .reader
            .transceive_parity_expect(&command, &parity, 36)?;
        let tag_nonce = parse_nonce(&response)?;
        let reader_nonce = reader_nonce.unwrap_or_else(next_reader_nonce);
        let mut state = Crypto1State::from_key(key_to_u64(&key));
        state.word(tag_nonce ^ self.uid, false);

        let mut response = [0u8; 8];
        let mut parity = [0u8; 8];
        let nonce_bytes = reader_nonce.to_be_bytes();
        for (index, byte) in nonce_bytes.iter().enumerate() {
            response[index] = state.byte(*byte, false) ^ *byte;
            parity[index] = (filter(state.odd) ^ odd_parity8(*byte)) & 1;
        }

        let mut plain_reader_response = prng_successor(tag_nonce, 32);
        for index in 4..8 {
            plain_reader_response = prng_successor(plain_reader_response, 8);
            let byte = plain_reader_response as u8;
            response[index] = state.byte(0, false) ^ byte;
            parity[index] = (filter(state.odd) ^ odd_parity8(byte)) & 1;
        }

        let answer = self
            .reader
            .transceive_parity_expect(&response, &parity, 36)?;
        if answer.data.len() != 4 || answer.bits != 32 {
            return Err(NfcError::AuthenticationFailed);
        }
        let tag_answer = u32::from_be_bytes([
            answer.data[0],
            answer.data[1],
            answer.data[2],
            answer.data[3],
        ]);
        let expected = prng_successor(plain_reader_response, 32) ^ state.word(0, false);
        if tag_answer != expected {
            return Err(NfcError::AuthenticationFailed);
        }

        self.tag_nonce = tag_nonce;
        self.state = state;
        Ok(())
    }

    /// Sends a nested authentication command under the active session and
    /// returns the encrypted tag nonce.
    pub fn nested_nonce(&mut self, block: u8, key_type: KeyType) -> Result<NestedNonce> {
        let (plain, _) = command_with_crc(
            match key_type {
                KeyType::A => MIFARE_AUTH_A,
                KeyType::B => MIFARE_AUTH_B,
            },
            block,
        );
        let mut encrypted = vec![0u8; plain.len()];
        let mut parity = vec![0u8; plain.len()];
        for (index, byte) in plain.iter().enumerate() {
            encrypted[index] = self.state.byte(0, false) ^ *byte;
            parity[index] = (filter(self.state.odd) ^ odd_parity8(*byte)) & 1;
        }
        let response = self
            .reader
            .transceive_parity_expect(&encrypted, &parity, 36)?;
        if response.data.len() != 4 || response.bits != 32 {
            return Err(NfcError::AuthenticationFailed);
        }
        let mut parity_byte = 0u8;
        for (index, bit) in response.parity.iter().take(4).enumerate() {
            parity_byte |= (bit & 1) << (7 - index);
        }
        Ok(NestedNonce {
            tag_nonce_encrypted: u32::from_be_bytes([
                response.data[0],
                response.data[1],
                response.data[2],
                response.data[3],
            ]),
            parity: parity_byte,
        })
    }

    pub fn leave(&mut self) -> Result<()> {
        if self.recovery {
            self.recovery = false;
            self.reader.set_recovery_mode(false)?;
        }
        Ok(())
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if self.recovery {
            let _ = self.reader.set_recovery_mode(false);
        }
    }
}

/// Leaves recovery mode on every exit path.
struct RecoveryModeGuard<'a> {
    reader: &'a mut dyn Reader,
    active: bool,
}

impl<'a> RecoveryModeGuard<'a> {
    fn enter(reader: &'a mut dyn Reader) -> Result<Self> {
        reader.set_recovery_mode(true)?;
        Ok(Self {
            reader,
            active: true,
        })
    }

    fn leave(&mut self) -> Result<()> {
        if self.active {
            self.active = false;
            self.reader.set_recovery_mode(false)?;
        }
        Ok(())
    }
}

impl Drop for RecoveryModeGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.reader.set_recovery_mode(false);
        }
    }
}

/// Selects a card and performs one partial authentication to read the tag
/// nonce without completing the exchange. Returns `None` when the card does
/// not answer the authentication frame.
pub fn probe_tag_nonce(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
) -> Result<Option<u32>> {
    reader.set_recovery_mode(false)?;
    select_card(reader)?;
    let mut guard = RecoveryModeGuard::enter(reader)?;
    let (command, parity) = auth_command(block, key_type);
    let result = match guard.reader.transceive_parity_expect(&command, &parity, 36) {
        Ok(response) => parse_nonce(&response).map(Some),
        Err(NfcError::Timeout(_) | NfcError::NoCard) => Ok(None),
        Err(error) => Err(error),
    };
    guard.leave()?;
    result
}

/// Performs a full Crypto1 authentication after selecting the card.
pub fn authenticate(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
    key: Key,
) -> Result<AuthSession> {
    authenticate_with_nonce(reader, block, key_type, key, None)
}

/// Performs a full Crypto1 authentication with an explicit reader nonce and
/// leaves recovery mode before returning.
pub fn authenticate_with_nonce(
    reader: &mut dyn Reader,
    block: u8,
    key_type: KeyType,
    key: Key,
    reader_nonce: Option<u32>,
) -> Result<AuthSession> {
    let mut session = Session::authenticate(reader, block, key_type, key, reader_nonce)?;
    let result = AuthSession {
        uid: session.uid,
        tag_nonce: session.tag_nonce,
        state: session.state,
    };
    session.leave()?;
    Ok(result)
}

/// Decrypts a nested tag nonce when the key and nested key type are known.
pub fn decrypt_nested_nonce(uid: u32, encrypted: u32, key: Key) -> u32 {
    let mut nested = Crypto1State::from_key(key_to_u64(&key));
    nested.word(encrypted ^ uid, true) ^ encrypted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_a_matches_iso14443a_vectors() {
        assert_eq!(crc_a(&[0x60, 0x04]), [0xD1, 0x3D]);
        assert_eq!(crc_a(&[0x26]), [0xCA, 0x15]);
    }

    #[test]
    fn uid_tail_uses_the_last_four_bytes() {
        let card = CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            [0x00, 0x44],
            0x00,
            nfc_core::CardType::Type2,
        );
        assert_eq!(uid_tail(&card).expect("uid"), 0x3344_5566);
    }

    #[test]
    fn key_to_u64_is_big_endian() {
        let key = Key::from_hex("A0A1A2A3A4A5").expect("key");
        assert_eq!(key_to_u64(&key), 0xA0A1_A2A3_A4A5);
    }
}
