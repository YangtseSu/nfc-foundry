//! MIFARE Classic key store and candidate ordering.

use std::path::{Path, PathBuf};

use nfc_classic::DumpKey;
use nfc_core::{CardInfo, Key, KeyType, NfcError, Result};
use serde::{Deserialize, Serialize};

use crate::fsutil::{read_text, write_atomic};

const STORE_VERSION: u32 = 2;
const STORE_MIN_VERSION: u32 = 1;
const MAX_SECTOR: usize = 39;

/// Well-known factory and common keys. They are authentication candidates
/// only; a key is trusted after the card accepts it and a trailer read
/// succeeds. The set matches the previous NFC Foundry conservative defaults.
pub const COMMON_KEYS: [&str; 57] = [
    "FFFFFFFFFFFF",
    "000000000000",
    "A0A1A2A3A4A5",
    "B0B1B2B3B4B5",
    "C0C1C2C3C4C5",
    "D0D1D2D3D4D5",
    "D3F7D3F7D3F7",
    "4D3A99C351DD",
    "1A982C7E459A",
    "AABBCCDDEEFF",
    "714C5C886E97",
    "587EE5F9350F",
    "A0478CC39091",
    "533CB6C723F6",
    "8FD0A4F256E9",
    "B5FF67CAB8E0",
    "96A301B48D30",
    "A64598A77478",
    "26940B21FF5D",
    "FC00018778F7",
    "00000FFE2488",
    "5C598C9C58B5",
    "E4D2770A89BE",
    "434F4D4D4F41",
    "434F4D4D4F42",
    "47524F555041",
    "47524F555042",
    "54726176656C",
    "776974687573",
    "4B4559313233",
    "A396EFA4E24F",
    "A31667A8CEC1",
    "123456789ABC",
    "1234567890AB",
    "000000000001",
    "112233445566",
    "A1B2C3D4E5F6",
    "AA55AA55AA55",
    "55AA55AA55AA",
    "ABABABABABAB",
    "BABABABABABA",
    "010203040506",
    "060504030201",
    "ABCDEF123456",
    "123456ABCDEF",
    "FEDCBA987654",
    "13579BDF0246",
    "02468ACE1357",
    "6B4C5C886E97",
    "4E3A99C351DD",
    "6E7A2C7E459A",
    "B0C0D0E0F000",
    "A1A2A3A4A5A6",
    "B1B2B3B4B5B6",
    "C1C2C3C4C5C6",
    "D1D2D3D4D5D6",
    "E0E1E2E3E4E5",
];

/// Parsed common keys. Entries in [`COMMON_KEYS`] are valid by construction.
pub fn common_keys() -> Vec<Key> {
    COMMON_KEYS
        .iter()
        .filter_map(|hex| Key::from_hex(hex).ok())
        .collect()
}

/// Standard keys checked on the first key scan. This matches MCT's
/// `std.keys`: factory default, MAD public Key A, and NFC Forum NDEF Key A.
pub const DEFAULT_KEYS: [&str; 3] = ["FFFFFFFFFFFF", "A0A1A2A3A4A5", "D3F7D3F7D3F7"];

/// Near-standard keys kept out of the first-run selection: the blank key and
/// the de facto NFC Forum MAD Key B values. MCT keeps these in
/// `extended-std.keys`.
pub const DEFAULT_EXTENDED_KEYS: [&str; 4] = [
    "000000000000",
    "B0B1B2B3B4B5",
    "89ECA97F8C2A",
    "A5A4A3A2A1A0",
];

/// NXP MIFARE Classic EV1 originality-signature keys for hidden blocks 16/17.
pub const EV1_SIGNATURE_KEYS: [&str; 4] = [
    "5C8FF9990DA2",
    "75CCB59C9BED",
    "D01AFEEB890A",
    "4B791BEA7BCC",
];

fn parse_keys(values: &[&str]) -> Vec<Key> {
    values
        .iter()
        .filter_map(|hex| Key::from_hex(hex).ok())
        .collect()
}

/// Standard keys checked by default on the first scan.
pub fn default_keys() -> Vec<Key> {
    parse_keys(&DEFAULT_KEYS)
}

/// Near-standard keys, unchecked by default.
pub fn default_extended_keys() -> Vec<Key> {
    parse_keys(&DEFAULT_EXTENDED_KEYS)
}

/// EV1 signature keys, used only for the hidden signature blocks.
pub fn ev1_signature_keys() -> Vec<Key> {
    parse_keys(&EV1_SIGNATURE_KEYS)
}

/// Built-in dictionary entries outside the curated default groups.
pub fn common_only_keys() -> Vec<Key> {
    let default = default_keys();
    let extended = default_extended_keys();
    let ev1 = ev1_signature_keys();
    common_keys()
        .into_iter()
        .filter(|key| !default.contains(key) && !extended.contains(key) && !ev1.contains(key))
        .collect()
}

/// Which authentication key type an entry may be used for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyKind {
    A,
    B,
    #[default]
    Any,
}

impl KeyKind {
    pub const ALL: [Self; 3] = [Self::A, Self::B, Self::Any];

    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::Any => "any",
        }
    }

    pub const fn includes(self, key_type: KeyType) -> bool {
        matches!(
            (self, key_type),
            (Self::Any, _) | (Self::A, KeyType::A) | (Self::B, KeyType::B)
        )
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "a" => Ok(Self::A),
            "b" => Ok(Self::B),
            "any" | "*" => Ok(Self::Any),
            other => Err(NfcError::InvalidArgument(format!(
                "unknown key type {other:?}; use A, B, or any"
            ))),
        }
    }

    fn stored(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::Any => "any",
        }
    }
}

/// Where a key came from. Provenance is informational and survives storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeySource {
    #[default]
    Manual,
    KeyFile,
    KeyScan,
    Recovery,
    Common,
    ReadContext,
}

impl KeySource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::KeyFile => "key file",
            Self::KeyScan => "scan",
            Self::Recovery => "recovery",
            Self::Common => "common",
            Self::ReadContext => "read",
        }
    }

    pub const fn stored(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::KeyFile => "key_file",
            Self::KeyScan => "key_scan",
            Self::Recovery => "recovery",
            Self::Common => "common",
            Self::ReadContext => "read_context",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "" | "manual" => Ok(Self::Manual),
            "key_file" | "file" => Ok(Self::KeyFile),
            "key_scan" | "scan" => Ok(Self::KeyScan),
            "recovery" => Ok(Self::Recovery),
            "common" => Ok(Self::Common),
            "read_context" | "read" => Ok(Self::ReadContext),
            other => Err(NfcError::InvalidArgument(format!(
                "unknown key source {other:?}"
            ))),
        }
    }
}

/// A key that authenticated a sector during a read or scan. The key is not a
/// persistent record; `KeyStore::record_verified` assigns card and time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedKey {
    pub sector: usize,
    pub key_type: KeyType,
    pub key: Key,
    pub source: KeySource,
}

/// Card-scoped key verification. One record exists per card, sector, and key
/// type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyVerification {
    pub sector: usize,
    pub key_type: KeyType,
    pub key: Key,
    pub source: KeySource,
    pub verified_at: u64,
}

/// Non-fatal issue found while importing a key file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyImportIssue {
    pub line: usize,
    pub message: String,
}

/// Result of importing a key file. Malformed entries do not abort the import.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyImportReport {
    pub added: usize,
    pub duplicates: usize,
    pub ignored: usize,
    pub issues: Vec<KeyImportIssue>,
}

/// One key entry. A missing sector applies the key to every sector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyEntry {
    pub sector: Option<usize>,
    pub kind: KeyKind,
    pub key: Key,
    pub source: KeySource,
}

impl KeyEntry {
    pub const fn new(sector: Option<usize>, kind: KeyKind, key: Key, source: KeySource) -> Self {
        Self {
            sector,
            kind,
            key,
            source,
        }
    }

    pub fn sector_label(&self) -> String {
        match self.sector {
            Some(sector) => format!("sector {sector}"),
            None => "all sectors".to_owned(),
        }
    }
}

/// Where an exported candidate key came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateSource {
    Stored,
    Builtin,
}

impl CandidateSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stored => "stored",
            Self::Builtin => "builtin",
        }
    }
}

/// One key in the stored-plus-built-in candidate set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyCandidate {
    pub key: Key,
    pub source: CandidateSource,
}

/// In-memory key store with stable candidate ordering: explicit sector keys
/// first, then wildcard keys, each in insertion order. Verifications are
/// card-scoped and survive storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyStore {
    entries: Vec<KeyEntry>,
    verifications: Vec<StoredVerificationRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredVerificationRecord {
    card: String,
    verification: KeyVerification,
}

/// Card identity used to scope key verifications.
pub fn card_id(card: &CardInfo) -> String {
    card.identity_key()
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl KeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store used by the CLI and GUI. `None` when neither `XDG_CONFIG_HOME`
    /// nor `HOME` is available.
    pub fn default_path() -> Option<PathBuf> {
        Some(config_dir()?.join("nfc-foundry").join("classic-keys.json"))
    }

    /// Key store written by the previous application. It is imported once,
    /// when present, so existing keys survive the rewrite.
    pub fn legacy_path() -> Option<PathBuf> {
        Some(config_dir()?.join("NFC Foundry").join("keys.json"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = read_text(path)?;
        Self::from_json(&text)
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &self.to_json()?)
    }

    /// Internal JSON store as UTF-8 bytes, including sector, key type,
    /// provenance, and card-scoped verifications.
    pub fn to_json(&self) -> Result<Vec<u8>> {
        let document = StoreDocument {
            version: STORE_VERSION,
            keys: self
                .entries
                .iter()
                .map(|entry| StoredKey {
                    sector: entry.sector,
                    kind: entry.kind.stored().to_owned(),
                    key: entry.key.to_hex(),
                    source: entry.source.stored().to_owned(),
                })
                .collect(),
            verifications: self
                .verifications
                .iter()
                .map(|record| StoredVerification {
                    card: record.card.clone(),
                    sector: record.verification.sector,
                    kind: record.verification.key_type.label().to_string(),
                    key: record.verification.key.to_hex(),
                    source: record.verification.source.stored().to_owned(),
                    verified_at: record.verification.verified_at,
                })
                .collect(),
        };
        let mut bytes = serde_json::to_vec_pretty(&document)
            .map_err(|error| NfcError::Io(format!("could not serialize the key store: {error}")))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Proxmark3 `.dic` text: unique uppercase keys in candidate order, one
    /// per line. Sector, key type, and provenance are not representable.
    pub fn export_dic(&self) -> String {
        let mut text = String::new();
        for key in self.all_keys() {
            text.push_str(&key.to_hex());
            text.push('\n');
        }
        text
    }

    /// Unique union of stored keys and the built-in dictionary, sorted by
    /// uppercase hexadecimal. Stored provenance wins when a key is both.
    /// Reading candidates never mutates the store.
    pub fn candidate_keys(&self) -> Vec<KeyCandidate> {
        let mut candidates: Vec<KeyCandidate> = self
            .all_keys()
            .into_iter()
            .map(|key| KeyCandidate {
                key,
                source: CandidateSource::Stored,
            })
            .collect();
        for key in common_keys() {
            if !candidates.iter().any(|candidate| candidate.key == key) {
                candidates.push(KeyCandidate {
                    key,
                    source: CandidateSource::Builtin,
                });
            }
        }
        candidates.sort_by_cached_key(|candidate| candidate.key.to_hex());
        candidates
    }

    /// `.dic` text for [`Self::candidate_keys`]: one uppercase key per line.
    pub fn export_candidate_dic(&self) -> String {
        let mut text = String::new();
        for candidate in self.candidate_keys() {
            text.push_str(&candidate.key.to_hex());
            text.push('\n');
        }
        text
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let document: StoreDocument = serde_json::from_str(text).map_err(|error| {
            NfcError::InvalidArgument(format!("invalid key store JSON: {error}"))
        })?;
        if document.version < STORE_MIN_VERSION || document.version > STORE_VERSION {
            return Err(NfcError::InvalidArgument(format!(
                "unsupported key store version {}; expected {}..={STORE_VERSION}",
                document.version, STORE_MIN_VERSION
            )));
        }
        let mut store = Self::default();
        for (index, stored) in document.keys.into_iter().enumerate() {
            if let Some(sector) = stored.sector
                && sector > MAX_SECTOR
            {
                return Err(NfcError::InvalidArgument(format!(
                    "key store entry {index} names sector {sector}, outside 0..={MAX_SECTOR}"
                )));
            }
            let kind = KeyKind::parse(&stored.kind)?;
            let key = Key::from_hex(&stored.key).map_err(|error| {
                NfcError::InvalidArgument(format!("key store entry {index}: {error}"))
            })?;
            let source = KeySource::parse(&stored.source)?;
            store.add(KeyEntry::new(stored.sector, kind, key, source))?;
        }
        for (index, stored) in document.verifications.into_iter().enumerate() {
            let card = stored.card.trim().to_owned();
            if card.is_empty() {
                return Err(NfcError::InvalidArgument(format!(
                    "key verification {index} has an empty card identity"
                )));
            }
            if stored.sector > MAX_SECTOR {
                return Err(NfcError::InvalidArgument(format!(
                    "key verification {index} names sector {}, outside 0..={MAX_SECTOR}",
                    stored.sector
                )));
            }
            let key_type = match stored.kind.trim().to_ascii_uppercase().as_str() {
                "A" => KeyType::A,
                "B" => KeyType::B,
                other => {
                    return Err(NfcError::InvalidArgument(format!(
                        "key verification {index} has key type {other:?}; use A or B"
                    )));
                }
            };
            let key = Key::from_hex(&stored.key).map_err(|error| {
                NfcError::InvalidArgument(format!("key verification {index}: {error}"))
            })?;
            let source = KeySource::parse(&stored.source)?;
            if store.verifications.iter().any(|record| {
                record.card == card
                    && record.verification.sector == stored.sector
                    && record.verification.key_type == key_type
            }) {
                return Err(NfcError::InvalidArgument(format!(
                    "key verification {index} repeats card {card} sector {} key {}",
                    stored.sector,
                    key_type.label()
                )));
            }
            store.verifications.push(StoredVerificationRecord {
                card,
                verification: KeyVerification {
                    sector: stored.sector,
                    key_type,
                    key,
                    source,
                    verified_at: stored.verified_at,
                },
            });
        }
        Ok(store)
    }

    pub fn entries(&self) -> &[KeyEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Adds an entry. Returns whether it was inserted; an identical entry is
    /// ignored.
    pub fn add(&mut self, entry: KeyEntry) -> Result<bool> {
        if let Some(sector) = entry.sector
            && sector > MAX_SECTOR
        {
            return Err(NfcError::InvalidArgument(format!(
                "sector {sector} is outside 0..={MAX_SECTOR}"
            )));
        }
        if self.entries.iter().any(|existing| {
            existing.sector == entry.sector
                && existing.kind == entry.kind
                && existing.key == entry.key
        }) {
            return Ok(false);
        }
        self.entries.push(entry);
        Ok(true)
    }

    /// Adds a wildcard key from a text key file.
    pub fn import_text(&mut self, path: &Path) -> Result<KeyImportReport> {
        let text = read_text(path)?;
        self.import_text_str(&text)
    }

    /// Imports a key file by content: JSON documents use the structured
    /// importer, everything else uses the Proxmark3 `.dic` text dialect.
    pub fn import_text_str(&mut self, text: &str) -> Result<KeyImportReport> {
        if text.trim_start().starts_with('{') {
            return self.import_json_str(text);
        }
        self.import_dic_str(text)
    }

    /// Imports Proxmark3 `.dic` text: one 12-hex key per line, blank lines and
    /// `#` comments accepted, inline comments allowed. Keys become wildcard
    /// `any` entries. Malformed lines are reported and skipped.
    pub fn import_dic_str(&mut self, text: &str) -> Result<KeyImportReport> {
        let mut report = KeyImportReport::default();
        for (index, line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                report.ignored += 1;
                continue;
            }
            let mut fields = line.split_whitespace();
            let Some(hex) = fields.next() else {
                report.ignored += 1;
                continue;
            };
            if fields.next().is_some() {
                report.ignored += 1;
                report.issues.push(KeyImportIssue {
                    line: line_number,
                    message: "expected one 12-digit key per line".to_owned(),
                });
                continue;
            }
            let key = match Key::from_hex(hex) {
                Ok(key) => key,
                Err(error) => {
                    report.ignored += 1;
                    report.issues.push(KeyImportIssue {
                        line: line_number,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            match self.add(KeyEntry::new(None, KeyKind::Any, key, KeySource::KeyFile)) {
                Ok(true) => report.added += 1,
                Ok(false) => report.duplicates += 1,
                Err(error) => {
                    report.ignored += 1;
                    report.issues.push(KeyImportIssue {
                        line: line_number,
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// Import a JSON key file. Accepts the previous application's
    /// `{"keys":[{"value":"…"}]}` document and this store's own schema;
    /// sector/type/provenance are preserved when present.
    pub fn import_json_str(&mut self, text: &str) -> Result<KeyImportReport> {
        let document: ImportDocument = serde_json::from_str(text)
            .map_err(|error| NfcError::InvalidArgument(format!("invalid key JSON: {error}")))?;
        let mut report = KeyImportReport::default();
        if document.keys.is_empty() {
            report.issues.push(KeyImportIssue {
                line: 0,
                message: "the JSON key file contains no keys".to_owned(),
            });
            return Ok(report);
        }
        for (index, stored) in document.keys.into_iter().enumerate() {
            let line = index + 1;
            let Some(hex) = stored.value.or(stored.key) else {
                report.ignored += 1;
                report.issues.push(KeyImportIssue {
                    line,
                    message: "missing value or key".to_owned(),
                });
                continue;
            };
            let kind = match stored.kind.as_deref().map(KeyKind::parse).transpose() {
                Ok(kind) => kind.unwrap_or(KeyKind::Any),
                Err(error) => {
                    report.ignored += 1;
                    report.issues.push(KeyImportIssue {
                        line,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let source = match stored.source.as_deref().map(KeySource::parse).transpose() {
                Ok(source) => source.unwrap_or(KeySource::KeyFile),
                Err(error) => {
                    report.ignored += 1;
                    report.issues.push(KeyImportIssue {
                        line,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let key = match Key::from_hex(&hex) {
                Ok(key) => key,
                Err(error) => {
                    report.ignored += 1;
                    report.issues.push(KeyImportIssue {
                        line,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            match self.add(KeyEntry::new(stored.sector, kind, key, source)) {
                Ok(true) => report.added += 1,
                Ok(false) => report.duplicates += 1,
                Err(error) => {
                    report.ignored += 1;
                    report.issues.push(KeyImportIssue {
                        line,
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// Candidate keys for a sector, with the key types that may be tried.
    pub fn candidates_for(&self, sector: usize) -> Vec<(KeyType, Key)> {
        let mut candidates = Vec::new();
        for sector_specific in [true, false] {
            for entry in &self.entries {
                let applies = if sector_specific {
                    entry.sector == Some(sector)
                } else {
                    entry.sector.is_none()
                };
                if !applies {
                    continue;
                }
                for key_type in [KeyType::A, KeyType::B] {
                    if entry.kind.includes(key_type) && !candidates.contains(&(key_type, entry.key))
                    {
                        candidates.push((key_type, entry.key));
                    }
                }
            }
        }
        candidates
    }

    /// Candidate keys for one card and sector, with provenance. Verified keys
    /// for this card come first, then stored sector entries and wildcards.
    pub fn candidates_for_card(
        &self,
        card: &CardInfo,
        sector: usize,
    ) -> Vec<(KeyType, Key, KeySource)> {
        let card = card_id(card);
        let mut candidates = Vec::new();
        for record in &self.verifications {
            if record.card != card || record.verification.sector != sector {
                continue;
            }
            let candidate = (
                record.verification.key_type,
                record.verification.key,
                record.verification.source,
            );
            if !candidates.iter().any(|entry: &(KeyType, Key, KeySource)| {
                entry.0 == candidate.0 && entry.1 == candidate.1
            }) {
                candidates.push(candidate);
            }
        }
        for sector_specific in [true, false] {
            for entry in &self.entries {
                let applies = if sector_specific {
                    entry.sector == Some(sector)
                } else {
                    entry.sector.is_none()
                };
                if !applies {
                    continue;
                }
                for key_type in [KeyType::A, KeyType::B] {
                    if !entry.kind.includes(key_type) {
                        continue;
                    }
                    let candidate = (key_type, entry.key, entry.source);
                    if !candidates.iter().any(|entry: &(KeyType, Key, KeySource)| {
                        entry.0 == candidate.0 && entry.1 == candidate.1
                    }) {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates
    }

    /// Verification records for one card.
    pub fn verifications_for(&self, card: &CardInfo) -> Vec<KeyVerification> {
        let card = card_id(card);
        self.verifications
            .iter()
            .filter(|record| record.card == card)
            .map(|record| record.verification)
            .collect()
    }

    /// Number of verified key slots for one card.
    pub fn verified_count(&self, card: &CardInfo) -> usize {
        let card = card_id(card);
        self.verifications
            .iter()
            .filter(|record| record.card == card)
            .count()
    }

    /// Records authenticated keys for a card and persists them in memory.
    /// Returns the number of keys processed.
    pub fn record_verified(&mut self, card: &CardInfo, keys: &[AuthenticatedKey]) -> Result<usize> {
        let card = card_id(card);
        let now = now_seconds();
        let mut recorded = 0;
        for entry in keys {
            if entry.sector > MAX_SECTOR {
                return Err(NfcError::InvalidArgument(format!(
                    "sector {} is outside 0..={MAX_SECTOR}",
                    entry.sector
                )));
            }
            let kind = match entry.key_type {
                KeyType::A => KeyKind::A,
                KeyType::B => KeyKind::B,
            };
            let already_applies = self.entries.iter().any(|stored| {
                stored.key == entry.key
                    && stored.kind.includes(entry.key_type)
                    && (stored.sector.is_none() || stored.sector == Some(entry.sector))
            });
            if !already_applies {
                self.add(KeyEntry::new(
                    Some(entry.sector),
                    kind,
                    entry.key,
                    entry.source,
                ))?;
            }
            self.upsert_verification(
                card.clone(),
                KeyVerification {
                    sector: entry.sector,
                    key_type: entry.key_type,
                    key: entry.key,
                    source: entry.source,
                    verified_at: now,
                },
            );
            recorded += 1;
        }
        Ok(recorded)
    }

    /// Records keys loaded from a dump's metadata. Unknown source labels fall
    /// back to `manual` because metadata is informational.
    pub fn record_dump_keys(&mut self, card: &CardInfo, keys: &[DumpKey]) -> Result<usize> {
        let authenticated: Vec<_> = keys
            .iter()
            .map(|entry| AuthenticatedKey {
                sector: entry.sector,
                key_type: entry.key_type,
                key: entry.key,
                source: entry
                    .source
                    .as_deref()
                    .and_then(|source| KeySource::parse(source).ok())
                    .unwrap_or(KeySource::Manual),
            })
            .collect();
        self.record_verified(card, &authenticated)
    }

    fn upsert_verification(&mut self, card: String, verification: KeyVerification) {
        if let Some(existing) = self.verifications.iter_mut().find(|record| {
            record.card == card
                && record.verification.sector == verification.sector
                && record.verification.key_type == verification.key_type
        }) {
            existing.verification = verification;
        } else {
            self.verifications
                .push(StoredVerificationRecord { card, verification });
        }
    }

    /// Candidates for recovery: every stored key tried as both key types,
    /// followed by the built-in common keys when requested. The order follows
    /// the store and then the declared common-key order so factory keys such as
    /// `FFFFFFFFFFFF` are tried first.
    pub fn recovery_candidates(&self, include_common_keys: bool) -> Vec<(KeyType, Key)> {
        let mut candidates = Vec::new();
        for entry in &self.entries {
            for key_type in [KeyType::A, KeyType::B] {
                let candidate = (key_type, entry.key);
                if entry.kind.includes(key_type) && !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        if include_common_keys {
            for key in common_keys() {
                for key_type in [KeyType::A, KeyType::B] {
                    let candidate = (key_type, key);
                    if !candidates.contains(&candidate) {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates
    }

    /// Every distinct key stored, in insertion order.
    pub fn all_keys(&self) -> Vec<Key> {
        let mut keys = Vec::new();
        for entry in &self.entries {
            if !keys.contains(&entry.key) {
                keys.push(entry.key);
            }
        }
        keys
    }

    /// First stored key usable for a sector and key type. Sector-specific
    /// entries take precedence over wildcards.
    pub fn find_key(&self, sector: usize, key_type: KeyType) -> Option<Key> {
        for sector_specific in [true, false] {
            if let Some(entry) = self.entries.iter().find(|entry| {
                let applies = if sector_specific {
                    entry.sector == Some(sector)
                } else {
                    entry.sector.is_none()
                };
                applies && entry.kind.includes(key_type)
            }) {
                return Some(entry.key);
            }
        }
        None
    }

    pub fn merge(&mut self, other: &Self) -> Result<usize> {
        let mut added = 0;
        for entry in &other.entries {
            if self.add(*entry)? {
                added += 1;
            }
        }
        for record in &other.verifications {
            if let Some(existing) = self.verifications.iter_mut().find(|existing| {
                existing.card == record.card
                    && existing.verification.sector == record.verification.sector
                    && existing.verification.key_type == record.verification.key_type
            }) {
                if record.verification.verified_at >= existing.verification.verified_at {
                    existing.verification = record.verification;
                }
            } else {
                self.verifications.push(record.clone());
            }
        }
        Ok(added)
    }

    pub fn remove(&mut self, index: usize) -> Option<KeyEntry> {
        if index >= self.entries.len() {
            return None;
        }
        let removed = self.entries.remove(index);
        if let Some(sector) = removed.sector {
            self.verifications.retain(|record| {
                !(record.verification.sector == sector
                    && record.verification.key == removed.key
                    && removed.kind.includes(record.verification.key_type))
            });
        }
        Some(removed)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.verifications.clear();
    }
}

/// One per-card key file in Proxmark3 `.dic` format.
///
/// The text format is one uppercase 12-hex key per line. `#` comments carry
/// the optional set name and the card identity, which PM3 ignores:
///
/// ```text
/// # name: Home door
/// # card: 04A2248A/0004/08
/// FFFFFFFFFFFF
/// A0A1A2A3A4A5
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFile {
    pub path: PathBuf,
    pub name: String,
    pub card: Option<CardInfo>,
    pub keys: Vec<Key>,
}

impl KeyFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = read_text(path)?;
        Self::parse(path, &text)
    }

    pub fn parse(path: &Path, text: &str) -> Result<Self> {
        let mut name = String::new();
        let mut card = None;
        let mut keys = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = line.trim();
            if let Some(comment) = line.strip_prefix('#') {
                let comment = comment.trim();
                if let Some(value) = comment.strip_prefix("name:") {
                    name = value.trim().to_owned();
                } else if let Some(value) = comment.strip_prefix("card:") {
                    card = parse_card_identity(value.trim());
                }
                continue;
            }
            let value = line.split('#').next().unwrap_or("").trim();
            if value.is_empty() {
                continue;
            }
            if value.split_whitespace().count() != 1 {
                return Err(NfcError::InvalidArgument(format!(
                    "{} line {line_number}: expected one 12-digit key",
                    path.display()
                )));
            }
            let key = Key::from_hex(value).map_err(|error| {
                NfcError::InvalidArgument(format!("{} line {line_number}: {error}", path.display()))
            })?;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        if name.is_empty() {
            name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("keys")
                .to_owned();
        }
        Ok(Self {
            path: path.to_owned(),
            name,
            card,
            keys,
        })
    }

    pub fn save(&self) -> Result<()> {
        let mut text = String::from("# NFC Foundry MIFARE Classic key file\n");
        if !self.name.is_empty() {
            text.push_str(&format!("# name: {}\n", self.name));
        }
        if let Some(card) = &self.card {
            text.push_str(&format!("# card: {}\n", card.identity_key()));
        }
        for key in &self.keys {
            text.push_str(&key.to_hex());
            text.push('\n');
        }
        write_atomic(&self.path, text.as_bytes())
    }

    /// `<dump>.dic` next to the dump file. Used as a fallback when no XDG data
    /// directory is available.
    pub fn sibling_path(dump: &Path) -> PathBuf {
        dump.with_extension("dic")
    }

    /// Default key file for a dump: `keys/<dump stem>.dic` under the XDG data
    /// directory, falling back to a sibling `.dic`.
    pub fn path_for_dump(dump: &Path) -> PathBuf {
        crate::paths::key_path_for_dump(dump).unwrap_or_else(|| Self::sibling_path(dump))
    }

    /// Loads the key file paired with a dump. The XDG keys directory wins;
    /// a sibling `.dic` is accepted for older files.
    pub fn load_for_dump(dump: &Path) -> Result<Option<Self>> {
        let primary = Self::path_for_dump(dump);
        if primary.exists() {
            return Self::load(&primary).map(Some);
        }
        let fallback = Self::sibling_path(dump);
        if fallback != primary && fallback.exists() {
            return Self::load(&fallback).map(Some);
        }
        Ok(None)
    }

    /// Loads `<dump>.dic` when it exists.
    pub fn load_sibling(dump: &Path) -> Result<Option<Self>> {
        let path = Self::sibling_path(dump);
        if !path.exists() {
            return Ok(None);
        }
        Self::load(&path).map(Some)
    }

    pub fn from_keys(
        path: PathBuf,
        name: impl Into<String>,
        card: Option<CardInfo>,
        keys: impl IntoIterator<Item = Key>,
    ) -> Self {
        let mut unique = Vec::new();
        for key in keys {
            if !unique.contains(&key) {
                unique.push(key);
            }
        }
        let mut name = name.into();
        if name.is_empty() {
            name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("keys")
                .to_owned();
        }
        Self {
            path,
            name,
            card,
            keys: unique,
        }
    }

    pub fn from_authenticated(
        path: PathBuf,
        name: impl Into<String>,
        card: Option<CardInfo>,
        keys: &[AuthenticatedKey],
    ) -> Self {
        Self::from_keys(path, name, card, keys.iter().map(|entry| entry.key))
    }

    pub fn from_dump_keys(
        path: PathBuf,
        name: impl Into<String>,
        card: Option<CardInfo>,
        keys: &[DumpKey],
    ) -> Self {
        Self::from_keys(path, name, card, keys.iter().map(|entry| entry.key))
    }

    /// Wildcard store used for reads and scans against this card.
    pub fn to_store(&self) -> KeyStore {
        let mut store = KeyStore::new();
        for key in &self.keys {
            let _ = store.add(KeyEntry::new(None, KeyKind::Any, *key, KeySource::KeyFile));
        }
        store
    }

    pub fn merge_into(&self, store: &mut KeyStore) -> Result<usize> {
        store.merge(&self.to_store())
    }

    /// Appends keys that are not already present.
    pub fn merge_keys(&mut self, keys: impl IntoIterator<Item = Key>) {
        for key in keys {
            if !self.keys.contains(&key) {
                self.keys.push(key);
            }
        }
    }
}

fn parse_card_identity(value: &str) -> Option<CardInfo> {
    let mut fields = value.split('/');
    let uid = decode_hex_bytes(fields.next()?)?;
    let atqa = decode_hex_bytes(fields.next()?)?;
    let sak = u8::from_str_radix(fields.next()?, 16).ok()?;
    if fields.next().is_some() || atqa.len() != 2 {
        return None;
    }
    let atqa = [atqa[0], atqa[1]];
    Some(CardInfo::new(
        uid,
        atqa,
        sak,
        nfc_core::CardType::infer(atqa, sak),
    ))
}

fn decode_hex_bytes(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

#[derive(Serialize, Deserialize)]
struct StoreDocument {
    version: u32,
    #[serde(default)]
    keys: Vec<StoredKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    verifications: Vec<StoredVerification>,
}

#[derive(Deserialize)]
struct ImportDocument {
    #[serde(default)]
    keys: Vec<ImportedKey>,
}

#[derive(Deserialize)]
struct ImportedKey {
    #[serde(default)]
    sector: Option<usize>,
    #[serde(default)]
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })
}

#[derive(Serialize, Deserialize)]
struct StoredKey {
    #[serde(default)]
    sector: Option<usize>,
    #[serde(default)]
    #[serde(rename = "type")]
    kind: String,
    key: String,
    #[serde(default)]
    source: String,
}

#[derive(Serialize, Deserialize)]
struct StoredVerification {
    card: String,
    sector: usize,
    #[serde(rename = "type")]
    kind: String,
    key: String,
    #[serde(default)]
    source: String,
    #[serde(default, rename = "verifiedAt")]
    verified_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> Key {
        Key::new([byte; 6])
    }

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0xA2, 0x24, 0x8A],
            [0x00, 0x04],
            0x08,
            nfc_core::CardType::MifareClassic1K,
        )
    }

    #[test]
    fn candidate_order_prefers_sector_keys_then_wildcards() {
        let mut store = KeyStore::new();
        store
            .add(KeyEntry::new(
                None,
                KeyKind::Any,
                key(1),
                KeySource::KeyFile,
            ))
            .expect("wildcard");
        store
            .add(KeyEntry::new(
                Some(2),
                KeyKind::A,
                key(2),
                KeySource::Manual,
            ))
            .expect("sector key");
        store
            .add(KeyEntry::new(
                Some(2),
                KeyKind::B,
                key(3),
                KeySource::KeyScan,
            ))
            .expect("sector key");

        let candidates = store.candidates_for(2);
        assert_eq!(
            candidates,
            vec![
                (KeyType::A, key(2)),
                (KeyType::B, key(3)),
                (KeyType::A, key(1)),
                (KeyType::B, key(1)),
            ]
        );
        assert_eq!(
            store.candidates_for(3),
            vec![(KeyType::A, key(1)), (KeyType::B, key(1)),]
        );
        assert_eq!(store.find_key(2, KeyType::A), Some(key(2)));
        assert_eq!(store.find_key(2, KeyType::B), Some(key(3)));
        assert_eq!(store.find_key(3, KeyType::B), Some(key(1)));
        assert_eq!(store.find_key(4, KeyType::B), Some(key(1)));
    }

    #[test]
    fn recovery_candidates_expand_both_types_and_add_common_keys() {
        let mut store = KeyStore::new();
        store
            .add(KeyEntry::new(
                Some(2),
                KeyKind::Any,
                key(0x22),
                KeySource::Recovery,
            ))
            .expect("sector key");
        let candidates = store.recovery_candidates(false);
        assert_eq!(
            candidates,
            vec![(KeyType::A, key(0x22)), (KeyType::B, key(0x22))]
        );

        let with_common = store.recovery_candidates(true);
        assert_eq!(with_common.len(), 2 + COMMON_KEYS.len() * 2);
        assert_eq!(
            &with_common[..2],
            &[(KeyType::A, key(0x22)), (KeyType::B, key(0x22))]
        );
        let first_common = Key::from_hex(COMMON_KEYS[0]).expect("common key");
        assert_eq!(
            &with_common[2..4],
            &[(KeyType::A, first_common), (KeyType::B, first_common)]
        );
        assert_eq!(
            with_common
                .iter()
                .filter(|(key_type, _)| *key_type == KeyType::A)
                .count(),
            1 + COMMON_KEYS.len()
        );
    }

    #[test]
    fn store_round_trips_with_wildcards_and_sources() {
        let mut store = KeyStore::new();
        store
            .add(KeyEntry::new(
                Some(7),
                KeyKind::B,
                key(0xAB),
                KeySource::KeyScan,
            ))
            .expect("sector key");
        store
            .add(KeyEntry::new(
                None,
                KeyKind::Any,
                key(0xCD),
                KeySource::Manual,
            ))
            .expect("wildcard");
        let json = serde_json::to_string(&StoreDocument {
            version: STORE_VERSION,
            keys: store
                .entries
                .iter()
                .map(|entry| StoredKey {
                    sector: entry.sector,
                    kind: entry.kind.stored().to_owned(),
                    key: entry.key.to_hex(),
                    source: entry.source.stored().to_owned(),
                })
                .collect(),
            verifications: Vec::new(),
        })
        .expect("serialize");
        let loaded = KeyStore::from_json(&json).expect("load");
        assert_eq!(loaded, store);
        assert!(loaded.entries()[0].sector_label().contains("7"));
        assert_eq!(loaded.entries()[1].sector_label(), "all sectors");
    }

    #[test]
    fn json_import_accepts_previous_and_structured_stores() {
        let mut store = KeyStore::new();
        let report = store
            .import_text_str(
                r#"{"version":1,"keys":[
                    {"value":"D3F7D3F7D3F7","sources":["read_context"]},
                    {"sector":2,"type":"B","key":"A0A1A2A3A4A5","source":"manual"}
                ]}"#,
            )
            .expect("import");
        assert_eq!(report.added, 2);
        assert_eq!(store.entries()[0].sector, None);
        assert_eq!(store.entries()[0].kind, KeyKind::Any);
        assert_eq!(store.entries()[0].source, KeySource::KeyFile);
        assert_eq!(store.entries()[1].sector, Some(2));
        assert_eq!(store.entries()[1].kind, KeyKind::B);
        assert_eq!(store.entries()[1].source, KeySource::Manual);
        assert_eq!(store.len(), 2);

        let duplicate = store
            .import_text_str(r#"{"keys":[{"value":"D3F7D3F7D3F7"}]}"#)
            .expect("reimport");
        assert_eq!(duplicate.added, 0);
        assert_eq!(duplicate.duplicates, 1);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn text_import_accepts_comments_and_deduplicates() {
        let mut store = KeyStore::new();
        let report = store
            .import_text_str(
                "# factory keys\n\
                 FFFFFFFFFFFF\n\
                 a0a1a2a3a4a5  # inline comment\n\
                 FFFFFFFFFFFF\n",
            )
            .expect("import");
        assert_eq!(report.added, 2);
        assert_eq!(report.duplicates, 1);
        assert_eq!(store.len(), 2);
        assert_eq!(store.candidates_for(0).len(), 4);
    }

    #[test]
    fn candidate_export_unions_stored_and_builtin_keys_deterministically() {
        let mut store = KeyStore::new();
        let empty = store.candidate_keys();
        assert_eq!(empty.len(), COMMON_KEYS.len());
        assert!(
            empty
                .iter()
                .all(|candidate| candidate.source == CandidateSource::Builtin)
        );
        assert_eq!(
            empty
                .iter()
                .map(|candidate| candidate.key.to_hex())
                .collect::<Vec<_>>(),
            {
                let mut keys = common_keys()
                    .into_iter()
                    .map(Key::to_hex)
                    .collect::<Vec<_>>();
                keys.sort();
                keys
            }
        );

        store
            .add(KeyEntry::new(
                None,
                KeyKind::Any,
                key(0xCD),
                KeySource::Manual,
            ))
            .expect("custom");
        store
            .add(KeyEntry::new(
                Some(0),
                KeyKind::A,
                Key::from_hex("FFFFFFFFFFFF").expect("builtin"),
                KeySource::KeyScan,
            ))
            .expect("builtin duplicate");
        let candidates = store.candidate_keys();
        assert_eq!(candidates.len(), COMMON_KEYS.len() + 1);
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.key == key(0xCD))
                .count(),
            1
        );
        let factory = candidates
            .iter()
            .find(|candidate| candidate.key == Key::from_hex("FFFFFFFFFFFF").expect("key"))
            .expect("factory key");
        assert_eq!(factory.source, CandidateSource::Stored);
        assert_eq!(store.len(), 2);
        assert_eq!(store.entries().len(), 2);

        let dic = store.export_candidate_dic();
        let lines: Vec<_> = dic.lines().collect();
        assert_eq!(lines.len(), candidates.len());
        assert!(lines.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(lines.iter().all(|line| line.len() == 12));
        assert!(lines.iter().all(|line| {
            line.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        }));
        assert!(dic.ends_with('\n'));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn dic_export_is_unique_uppercase_candidate_order() {
        let mut store = KeyStore::new();
        store
            .add(KeyEntry::new(
                Some(4),
                KeyKind::B,
                key(0xAB),
                KeySource::KeyScan,
            ))
            .expect("sector key");
        store
            .add(KeyEntry::new(
                None,
                KeyKind::Any,
                key(0xCD),
                KeySource::KeyFile,
            ))
            .expect("wildcard");
        store
            .add(KeyEntry::new(
                Some(6),
                KeyKind::A,
                key(0xAB),
                KeySource::Manual,
            ))
            .expect("duplicate key");
        assert_eq!(
            store.export_dic(),
            format!("{0}\n{1}\n", "AB".repeat(6), "CD".repeat(6))
        );
    }

    #[test]
    fn dic_import_skips_blank_and_inline_comments_and_reports_bad_lines() {
        let mut store = KeyStore::new();
        let report = store
            .import_dic_str(
                "\n# PM3 dictionary\r\n\
                 ffffffffffff\r\n\
                 A0A1A2A3A4A5 # factory\n\
                 # trailing comment\n\
                 ffffffffffff\n",
            )
            .expect("dic import");
        assert_eq!(report.added, 2);
        assert_eq!(report.duplicates, 1);
        assert_eq!(store.len(), 2);
        assert_eq!(store.entries()[0].key.to_hex(), "FFFFFFFFFFFF");
        assert_eq!(store.entries()[0].kind, KeyKind::Any);
        assert_eq!(store.entries()[0].source, KeySource::KeyFile);
        assert_eq!(store.entries()[1].key.to_hex(), "A0A1A2A3A4A5");

        let report = store
            .import_dic_str("FFFFFFFFFFFF extra\n")
            .expect("bad line");
        assert_eq!(report.added, 0);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].line, 1);

        let report = store.import_dic_str("FFFFFFFF\n").expect("short key");
        assert_eq!(report.issues.len(), 1);
        let report = store.import_dic_str("GGGGGGGGGGGG\n").expect("non-hex key");
        assert_eq!(report.issues.len(), 1);
        let report = store
            .import_dic_str("# only comments\n")
            .expect("empty import");
        assert_eq!(report.added, 0);
        assert_eq!(report.ignored, 1);
    }

    #[test]
    fn malformed_entries_are_rejected() {
        let json = r#"{"version":1,"keys":[{"sector":40,"type":"A","key":"FFFFFFFFFF"}]}"#;
        assert!(KeyStore::from_json(json).is_err());
        let json = r#"{"version":1,"keys":[{"sector":1,"type":"C","key":"FFFFFFFFFF"}]}"#;
        assert!(KeyStore::from_json(json).is_err());
        let json = r#"{"version":3,"keys":[]}"#;
        assert!(KeyStore::from_json(json).is_err());
        assert!(KeyStore::from_json(r#"{"version":2,"keys":[]}"#).is_ok());
        let mut store = KeyStore::new();
        let report = store
            .import_text_str("FFFFFFFFFFFF extra")
            .expect("bad line is reported");
        assert_eq!(report.issues.len(), 1);
        assert!(
            store
                .add(KeyEntry::new(
                    Some(40),
                    KeyKind::A,
                    key(1),
                    KeySource::Manual
                ))
                .is_err()
        );
    }

    #[test]
    fn store_file_round_trips_and_creates_parent_directories() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("nested").join("keys.json");
        let mut store = KeyStore::new();
        store
            .add(KeyEntry::new(
                Some(4),
                KeyKind::B,
                key(0x11),
                KeySource::KeyScan,
            ))
            .expect("key");
        store.save(&path).expect("save");
        assert_eq!(KeyStore::load(&path).expect("load"), store);
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn version_one_store_loads_with_no_verifications() {
        let store = KeyStore::from_json(
            r#"{"version":1,"keys":[{"sector":1,"type":"A","key":"FFFFFFFFFFFF","source":"manual"}]}"#,
        )
        .expect("v1 load");
        assert_eq!(store.len(), 1);
        assert!(store.verifications_for(&card()).is_empty());
    }

    #[test]
    fn verified_keys_are_card_scoped_and_tried_first() {
        let mut store = KeyStore::new();
        store
            .add(KeyEntry::new(
                None,
                KeyKind::Any,
                key(1),
                KeySource::KeyFile,
            ))
            .expect("wildcard");
        let card = card();
        store
            .record_verified(
                &card,
                &[AuthenticatedKey {
                    sector: 2,
                    key_type: KeyType::B,
                    key: key(2),
                    source: KeySource::ReadContext,
                }],
            )
            .expect("record");
        let candidates = store.candidates_for_card(&card, 2);
        assert_eq!(candidates[0], (KeyType::B, key(2), KeySource::ReadContext));
        assert!(candidates.contains(&(KeyType::A, key(1), KeySource::KeyFile)));
        assert_eq!(store.verified_count(&card), 1);

        let other = CardInfo::new(
            vec![0x11, 0x22, 0x33, 0x44],
            [0x00, 0x04],
            0x08,
            nfc_core::CardType::MifareClassic1K,
        );
        assert_eq!(store.verified_count(&other), 0);
        assert!(store.verifications_for(&other).is_empty());
    }

    #[test]
    fn verifications_round_trip_and_replace_the_same_slot() {
        let card = card();
        let mut store = KeyStore::new();
        store
            .record_verified(
                &card,
                &[AuthenticatedKey {
                    sector: 3,
                    key_type: KeyType::A,
                    key: key(1),
                    source: KeySource::ReadContext,
                }],
            )
            .expect("first");
        store
            .record_verified(
                &card,
                &[AuthenticatedKey {
                    sector: 3,
                    key_type: KeyType::A,
                    key: key(2),
                    source: KeySource::Recovery,
                }],
            )
            .expect("replacement");
        let verifications = store.verifications_for(&card);
        assert_eq!(verifications.len(), 1);
        assert_eq!(verifications[0].key, key(2));
        assert_eq!(verifications[0].source, KeySource::Recovery);

        let json = store.to_json().expect("json");
        let loaded = KeyStore::from_json(std::str::from_utf8(&json).expect("utf8")).expect("load");
        assert_eq!(loaded.verifications_for(&card), verifications);
    }

    #[test]
    fn removing_the_sector_entry_removes_its_verification() {
        let card = card();
        let mut store = KeyStore::new();
        store
            .record_verified(
                &card,
                &[AuthenticatedKey {
                    sector: 4,
                    key_type: KeyType::A,
                    key: key(1),
                    source: KeySource::ReadContext,
                }],
            )
            .expect("record");
        assert_eq!(store.verified_count(&card), 1);
        let index = store
            .entries()
            .iter()
            .position(|entry| entry.sector == Some(4))
            .expect("entry");
        store.remove(index).expect("remove");
        assert_eq!(store.verified_count(&card), 0);
    }

    #[test]
    fn common_keys_are_well_formed() {
        let keys = common_keys();
        assert_eq!(keys.len(), COMMON_KEYS.len());
        assert_eq!(keys[0].to_hex(), "FFFFFFFFFFFF");
        assert!(keys.iter().all(|key| key.as_bytes().len() == 6));
    }

    #[test]
    fn built_in_groups_are_disjoint() {
        let default = default_keys();
        let extended = default_extended_keys();
        let common = common_only_keys();
        assert_eq!(default.len(), 3);
        assert_eq!(extended.len(), 4);
        assert!(default.iter().all(|key| !extended.contains(key)));
        assert!(
            common
                .iter()
                .all(|key| !default.contains(key) && !extended.contains(key))
        );
    }

    #[test]
    fn store_deduplicates_by_key_identity_not_source() {
        let mut store = KeyStore::new();
        let key = key(7);
        assert!(
            store
                .add(KeyEntry::new(None, KeyKind::Any, key, KeySource::Manual))
                .expect("add")
        );
        assert!(
            !store
                .add(KeyEntry::new(None, KeyKind::Any, key, KeySource::KeyScan))
                .expect("duplicate")
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn key_file_round_trips_pm3_text() {
        let directory = tempfile::tempdir().expect("dir");
        let path = directory.path().join("home.dic");
        let file = KeyFile::from_keys(
            path.clone(),
            "Home door",
            Some(card()),
            [key(1), key(2), key(1)],
        );
        file.save().expect("save");
        let text = std::fs::read_to_string(&path).expect("text");
        assert!(text.starts_with("# NFC Foundry"));
        assert!(text.contains("# name: Home door"));
        assert!(text.contains("# card: 04A2248A/0004/08"));
        let loaded = KeyFile::load(&path).expect("load");
        assert_eq!(loaded.name, "Home door");
        assert_eq!(loaded.card, Some(card()));
        assert_eq!(loaded.keys, vec![key(1), key(2)]);
        assert_eq!(loaded.to_store().len(), 2);
    }

    #[test]
    fn key_file_accepts_mct_style_comments_and_deduplicates() {
        let directory = tempfile::tempdir().expect("dir");
        let path = directory.path().join("keys.dic");
        std::fs::write(
            &path,
            "# Standard keys\nffffffffffff\nA0A1A2A3A4A5 # MAD\n\nFFFFFFFFFFFF\n",
        )
        .expect("write");
        let file = KeyFile::load(&path).expect("load");
        assert_eq!(file.name, "keys");
        assert_eq!(file.card, None);
        assert_eq!(
            file.keys,
            vec![key(0xFF), Key::from_hex("A0A1A2A3A4A5").expect("key")]
        );
    }
}
