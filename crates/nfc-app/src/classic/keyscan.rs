//! Key scan: authenticate candidate keys per sector without writing.

use nfc_core::{CancelToken, Key, KeyType, NfcError, Result};

use crate::operation::Operation;

use super::keys::{KeyEntry, KeyKind, KeySource, KeyStore, common_keys};
use super::read::layout_for_card;

/// Which key types the scan attempts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyTypes {
    pub a: bool,
    pub b: bool,
}

impl KeyTypes {
    pub const ALL: Self = Self { a: true, b: true };

    pub const fn a_only() -> Self {
        Self { a: true, b: false }
    }

    pub const fn b_only() -> Self {
        Self { a: false, b: true }
    }

    pub const fn new(a: bool, b: bool) -> Self {
        Self { a, b }
    }

    fn enabled(self) -> Vec<KeyType> {
        let mut types = Vec::new();
        if self.a {
            types.push(KeyType::A);
        }
        if self.b {
            types.push(KeyType::B);
        }
        types
    }
}

impl Default for KeyTypes {
    fn default() -> Self {
        Self::ALL
    }
}

/// Key scan request. Authentication is the only card operation.
pub struct ClassicKeyscan<'a> {
    pub keys: &'a KeyStore,
    pub candidates: Vec<Key>,
    pub include_common_keys: bool,
    pub key_types: KeyTypes,
    pub cancel: CancelToken,
}

impl<'a> ClassicKeyscan<'a> {
    pub fn new(keys: &'a KeyStore, candidates: Vec<Key>) -> Self {
        Self {
            keys,
            candidates,
            include_common_keys: true,
            key_types: KeyTypes::ALL,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn key_types(mut self, key_types: KeyTypes) -> Self {
        self.key_types = key_types;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Keys found for one sector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SectorKeys {
    pub sector: usize,
    pub key_a: Option<Key>,
    pub key_b: Option<Key>,
}

/// Scan outcome. `store` contains only the keys discovered by this scan.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicKeyscanResult {
    pub sectors: Vec<SectorKeys>,
    pub store: KeyStore,
    pub attempts: usize,
    pub verified: usize,
}

/// Tries every candidate key as Key A and Key B per sector. A successful
/// authentication is verified by reading the sector trailer.
pub fn keyscan(
    operation: &mut Operation<'_>,
    request: ClassicKeyscan<'_>,
) -> Result<ClassicKeyscanResult> {
    let layout = layout_for_card(operation.card())?;
    let types = request.key_types.enabled();
    if types.is_empty() {
        return Err(NfcError::InvalidArgument(
            "select at least one key type to scan".to_owned(),
        ));
    }
    let mut candidates = request.candidates;
    for key in request.keys.all_keys() {
        push_unique(&mut candidates, key);
    }
    if request.include_common_keys {
        for key in common_keys() {
            push_unique(&mut candidates, key);
        }
    }
    if candidates.is_empty() {
        return Err(NfcError::InvalidArgument(
            "the key scan has no candidate keys".to_owned(),
        ));
    }

    let mut sectors = Vec::with_capacity(layout.sector_count());
    let mut store = KeyStore::new();
    let mut attempts = 0;
    let mut verified = 0;
    for sector in 0..layout.sector_count() {
        request.cancel.check()?;
        let first = layout
            .first_block(sector)
            .expect("sector is within the layout");
        let trailer = layout
            .trailer_block(sector)
            .expect("sector is within the layout");
        let mut found = SectorKeys {
            sector,
            key_a: None,
            key_b: None,
        };
        for key_type in types.iter().copied() {
            for key in candidates.iter().copied() {
                request.cancel.check()?;
                attempts += 1;
                match operation.authenticate(first as u8, key_type, key) {
                    Ok(()) => match operation.read_block(trailer as u8) {
                        Ok(_) => {
                            verified += 1;
                            match key_type {
                                KeyType::A => found.key_a = Some(key),
                                KeyType::B => found.key_b = Some(key),
                            }
                            store.add(KeyEntry::new(
                                Some(sector),
                                match key_type {
                                    KeyType::A => KeyKind::A,
                                    KeyType::B => KeyKind::B,
                                },
                                key,
                                KeySource::KeyScan,
                            ))?;
                            break;
                        }
                        Err(error) if is_fatal_scan_error(&error) => return Err(error),
                        Err(_) => continue,
                    },
                    Err(NfcError::AuthenticationFailed) => continue,
                    Err(error) => return Err(error),
                }
            }
        }
        sectors.push(found);
    }
    Ok(ClassicKeyscanResult {
        sectors,
        store,
        attempts,
        verified,
    })
}

fn push_unique(keys: &mut Vec<Key>, key: Key) {
    if !keys.contains(&key) {
        keys.push(key);
    }
}

fn is_fatal_scan_error(error: &NfcError) -> bool {
    matches!(
        error,
        NfcError::Cancelled
            | NfcError::NoCard
            | NfcError::NoReader
            | NfcError::Timeout(_)
            | NfcError::CardChanged
            | NfcError::Unsupported(_)
    )
}
