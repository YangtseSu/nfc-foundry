// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Auto recovery against a keyed in-memory Classic card. The dictionary and
//! read-verification paths run without any Crypto1 simulation because the card
//! answers plain authentication and block reads.

use nfc_core::{BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, Result};
use nfc_recover::{AttackKind, RecoveryRequest, RecoveryStrategy, recover};
use nfc_transport::{DeviceInfo, Reader};

struct KeyedCard {
    info: DeviceInfo,
    card: CardInfo,
    keys: Vec<Option<(Key, Key)>>,
    blocks: Vec<[u8; BLOCK_SIZE]>,
    authenticated_sector: Option<usize>,
}

impl KeyedCard {
    fn classic_1k(with_keys: bool) -> Self {
        let mut keys = vec![None; 16];
        let mut blocks = vec![[0u8; BLOCK_SIZE]; 64];
        for (sector, slot) in keys.iter_mut().enumerate() {
            if with_keys {
                let key_a = Key::new([sector as u8; 6]);
                let key_b = Key::new([0xA0 ^ sector as u8; 6]);
                *slot = Some((key_a, key_b));
            }
            let first = sector * 4;
            for (offset, block) in blocks[first..first + 4].iter_mut().enumerate() {
                block[0] = (first + offset) as u8;
            }
        }
        Self {
            info: DeviceInfo::new("keyed", "keyed:0"),
            card: CardInfo::new(
                vec![0x11, 0x22, 0x33, 0x44],
                [0x00, 0x04],
                0x08,
                CardType::MifareClassic1K,
            ),
            keys,
            blocks,
            authenticated_sector: None,
        }
    }

    fn candidates(&self) -> Vec<(KeyType, Key)> {
        let mut candidates = Vec::new();
        for (key_a, key_b) in self.keys.iter().flatten() {
            candidates.push((KeyType::A, *key_a));
            candidates.push((KeyType::B, *key_b));
        }
        candidates
    }

    fn sector_of(block: usize) -> usize {
        block / 4
    }
}

impl Reader for KeyedCard {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, _conn_string: &str) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        self.authenticated_sector = None;
        Ok(self.card.clone())
    }

    fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()> {
        let sector = Self::sector_of(usize::from(block));
        let Some((key_a, key_b)) = self.keys.get(sector).copied().flatten() else {
            return Err(NfcError::AuthenticationFailed);
        };
        let expected = match key_type {
            KeyType::A => key_a,
            KeyType::B => key_b,
        };
        if key != expected {
            return Err(NfcError::AuthenticationFailed);
        }
        self.authenticated_sector = Some(sector);
        Ok(())
    }

    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        let sector = Self::sector_of(usize::from(block));
        if self.authenticated_sector != Some(sector) {
            return Err(NfcError::AuthenticationFailed);
        }
        self.blocks
            .get(usize::from(block))
            .copied()
            .ok_or(NfcError::NoCard)
    }

    fn write_block(&mut self, _block: u8, _data: [u8; BLOCK_SIZE]) -> Result<()> {
        Err(NfcError::Unsupported("the test card is read-only"))
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        Err(NfcError::Unsupported("the test card has no raw exchange"))
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Err(NfcError::Unsupported("the test card has no raw exchange"))
    }

    fn set_raw_mode(&mut self, _enabled: bool) -> Result<()> {
        Err(NfcError::Unsupported("the test card has no raw mode"))
    }
}

#[test]
fn auto_scans_every_sector_and_verifies_the_read() {
    let mut card = KeyedCard::classic_1k(true);
    let request = RecoveryRequest {
        candidates: card.candidates(),
        ..RecoveryRequest::default()
    };
    let result = recover(&mut card, RecoveryStrategy::Auto, &request).expect("auto recovery");

    assert_eq!(result.sector_keys.len(), 32);
    assert!(
        result
            .sector_keys
            .iter()
            .all(|entry| entry.method == AttackKind::Dictionary)
    );
    assert_eq!(result.attempts, vec![AttackKind::Dictionary]);
    assert_eq!(result.keys.len(), 32);
    assert!(result.note.contains("sector keys 32/32"), "{}", result.note);
    assert!(
        result.note.contains("read verification 16/16"),
        "{}",
        result.note
    );
}

#[test]
fn auto_without_candidates_reports_the_missing_darkside_path() {
    let mut card = KeyedCard::classic_1k(true);
    let request = RecoveryRequest::default();
    let error = recover(&mut card, RecoveryStrategy::Auto, &request).expect_err("no candidates");
    let message = error.to_string();
    assert!(
        message.contains("darkside produced no verified seed key"),
        "{message}"
    );
}

#[test]
fn auto_reports_partial_coverage_when_only_some_sectors_are_known() {
    let mut card = KeyedCard::classic_1k(true);
    // Keep only sector 0's keys. The remaining keys are unknown and the test
    // reader cannot run parity attacks, so auto must return the known keys with
    // an explicit partial note instead of failing.
    let candidates = vec![
        (KeyType::A, Key::new([0; 6])),
        (KeyType::B, Key::new([0xA0; 6])),
    ];
    let request = RecoveryRequest {
        candidates,
        ..RecoveryRequest::default()
    };
    let result = recover(&mut card, RecoveryStrategy::Auto, &request).expect("partial auto");
    assert_eq!(result.sector_keys.len(), 2);
    assert!(
        result.note.contains("read verification 1/16"),
        "{}",
        result.note
    );
}
