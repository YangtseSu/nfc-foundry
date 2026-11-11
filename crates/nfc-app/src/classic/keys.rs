//! MIFARE Classic key store and candidate ordering.

use std::path::{Path, PathBuf};

use nfc_core::{Key, KeyType, NfcError, Result};
use serde::{Deserialize, Serialize};

use crate::fsutil::{read_text, write_atomic};

const STORE_VERSION: u32 = 1;
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
}

impl KeySource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::KeyFile => "key file",
            Self::KeyScan => "scan",
            Self::Recovery => "recovery",
            Self::Common => "common",
        }
    }

    fn stored(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::KeyFile => "key_file",
            Self::KeyScan => "key_scan",
            Self::Recovery => "recovery",
            Self::Common => "common",
        }
    }
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

/// In-memory key store with stable candidate ordering: explicit sector keys
/// first, then wildcard keys, each in insertion order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyStore {
    entries: Vec<KeyEntry>,
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

    /// Internal JSON store as UTF-8 bytes, including sector, key type, and
    /// provenance.
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

    pub fn from_json(text: &str) -> Result<Self> {
        let document: StoreDocument = serde_json::from_str(text).map_err(|error| {
            NfcError::InvalidArgument(format!("invalid key store JSON: {error}"))
        })?;
        if document.version != STORE_VERSION {
            return Err(NfcError::InvalidArgument(format!(
                "unsupported key store version {}; expected {STORE_VERSION}",
                document.version
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
            let source = parse_source(&stored.source)?;
            store
                .entries
                .push(KeyEntry::new(stored.sector, kind, key, source));
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
        if self.entries.contains(&entry) {
            return Ok(false);
        }
        self.entries.push(entry);
        Ok(true)
    }

    /// Adds a wildcard key from a text key file.
    pub fn import_text(&mut self, path: &Path) -> Result<usize> {
        let text = read_text(path)?;
        self.import_text_str(&text)
    }

    /// Imports a key file by content: JSON documents use the structured
    /// importer, everything else uses the Proxmark3 `.dic` text dialect.
    pub fn import_text_str(&mut self, text: &str) -> Result<usize> {
        if text.trim_start().starts_with('{') {
            return self.import_json_str(text);
        }
        self.import_dic_str(text)
    }

    /// Imports Proxmark3 `.dic` text: one 12-hex key per line, blank lines and
    /// `#` comments accepted, inline comments allowed. Keys become wildcard
    /// `any` entries.
    pub fn import_dic_str(&mut self, text: &str) -> Result<usize> {
        let mut added = 0;
        for (index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut fields = line.split_whitespace();
            let Some(hex) = fields.next() else {
                continue;
            };
            if fields.next().is_some() {
                return Err(NfcError::InvalidArgument(format!(
                    "key file line {}: expected one 12-digit key per line",
                    index + 1
                )));
            }
            let key = Key::from_hex(hex).map_err(|error| {
                NfcError::InvalidArgument(format!("key file line {}: {error}", index + 1))
            })?;
            if self.add(KeyEntry::new(None, KeyKind::Any, key, KeySource::KeyFile))? {
                added += 1;
            }
        }
        Ok(added)
    }

    /// Import a JSON key file. Accepts the previous application's
    /// `{"keys":[{"value":"…"}]}` document and this store's own schema;
    /// sector/type/provenance are preserved when present.
    pub fn import_json_str(&mut self, text: &str) -> Result<usize> {
        let document: ImportDocument = serde_json::from_str(text)
            .map_err(|error| NfcError::InvalidArgument(format!("invalid key JSON: {error}")))?;
        if document.keys.is_empty() {
            return Err(NfcError::InvalidArgument(
                "the JSON key file contains no keys".to_owned(),
            ));
        }
        let mut added = 0;
        for (index, stored) in document.keys.into_iter().enumerate() {
            let hex = stored.value.or(stored.key).ok_or_else(|| {
                NfcError::InvalidArgument(format!("key {index}: missing value or key"))
            })?;
            let kind = stored
                .kind
                .as_deref()
                .map(KeyKind::parse)
                .transpose()?
                .unwrap_or(KeyKind::Any);
            let source = stored
                .source
                .as_deref()
                .map(parse_source)
                .transpose()?
                .unwrap_or(KeySource::KeyFile);
            let key = Key::from_hex(&hex)
                .map_err(|error| NfcError::InvalidArgument(format!("key {index}: {error}")))?;
            if self.add(KeyEntry::new(stored.sector, kind, key, source))? {
                added += 1;
            }
        }
        Ok(added)
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
        Ok(added)
    }

    pub fn remove(&mut self, index: usize) -> Option<KeyEntry> {
        if index < self.entries.len() {
            Some(self.entries.remove(index))
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn parse_source(value: &str) -> Result<KeySource> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "" | "manual" => Ok(KeySource::Manual),
        "key_file" => Ok(KeySource::KeyFile),
        "key_scan" | "scan" => Ok(KeySource::KeyScan),
        "recovery" => Ok(KeySource::Recovery),
        "common" => Ok(KeySource::Common),
        other => Err(NfcError::InvalidArgument(format!(
            "unknown key source {other:?}"
        ))),
    }
}

#[derive(Serialize, Deserialize)]
struct StoreDocument {
    version: u32,
    #[serde(default)]
    keys: Vec<StoredKey>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> Key {
        Key::new([byte; 6])
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
        let added = store
            .import_text_str(
                r#"{"version":1,"keys":[
                    {"value":"D3F7D3F7D3F7","sources":["read_context"]},
                    {"sector":2,"type":"B","key":"A0A1A2A3A4A5","source":"manual"}
                ]}"#,
            )
            .expect("import");
        assert_eq!(added, 2);
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
        assert_eq!(duplicate, 0);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn text_import_accepts_comments_and_deduplicates() {
        let mut store = KeyStore::new();
        let added = store
            .import_text_str(
                "# factory keys\n\
                 FFFFFFFFFFFF\n\
                 a0a1a2a3a4a5  # inline comment\n\
                 FFFFFFFFFFFF\n",
            )
            .expect("import");
        assert_eq!(added, 2);
        assert_eq!(store.len(), 2);
        assert_eq!(store.candidates_for(0).len(), 4);
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
    fn dic_import_skips_blank_and_inline_comments_and_rejects_bad_lines() {
        let mut store = KeyStore::new();
        let added = store
            .import_dic_str(
                "\n# PM3 dictionary\r\n\
                 ffffffffffff\r\n\
                 A0A1A2A3A4A5 # factory\n\
                 # trailing comment\n\
                 ffffffffffff\n",
            )
            .expect("dic import");
        assert_eq!(added, 2);
        assert_eq!(store.len(), 2);
        assert_eq!(store.entries()[0].key.to_hex(), "FFFFFFFFFFFF");
        assert_eq!(store.entries()[0].kind, KeyKind::Any);
        assert_eq!(store.entries()[0].source, KeySource::KeyFile);
        assert_eq!(store.entries()[1].key.to_hex(), "A0A1A2A3A4A5");

        assert!(store.import_dic_str("FFFFFFFFFFFF extra\n").is_err());
        assert!(store.import_dic_str("FFFFFFFF\n").is_err());
        assert!(store.import_dic_str("GGGGGGGGGGGG\n").is_err());
        assert_eq!(
            store
                .import_dic_str("# only comments\n")
                .expect("empty import"),
            0
        );
    }

    #[test]
    fn malformed_entries_are_rejected() {
        let json = r#"{"version":1,"keys":[{"sector":40,"type":"A","key":"FFFFFFFFFF"}]}"#;
        assert!(KeyStore::from_json(json).is_err());
        let json = r#"{"version":1,"keys":[{"sector":1,"type":"C","key":"FFFFFFFFFF"}]}"#;
        assert!(KeyStore::from_json(json).is_err());
        let json = r#"{"version":2,"keys":[]}"#;
        assert!(KeyStore::from_json(json).is_err());
        let mut store = KeyStore::new();
        assert!(store.import_text_str("FFFFFFFFFFFF extra").is_err());
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
    fn common_keys_are_well_formed() {
        let keys = common_keys();
        assert_eq!(keys.len(), COMMON_KEYS.len());
        assert_eq!(keys[0].to_hex(), "FFFFFFFFFFFF");
        assert!(keys.iter().all(|key| key.as_bytes().len() == 6));
    }
}
