//! XDG data directories for user-facing files.
//!
//! Dumps, key files, NDEF messages, and pre-write backups live under
//! `$XDG_DATA_HOME/nfc-foundry` (default `~/.local/share/nfc-foundry`).
//! Configuration and the legacy JSON key store stay under
//! `$XDG_CONFIG_HOME/nfc-foundry`.

use std::path::{Path, PathBuf};

use nfc_core::{NfcError, Result};

/// `$XDG_DATA_HOME/nfc-foundry`, defaulting to `~/.local/share/nfc-foundry`.
pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })?;
    Some(base.join("nfc-foundry"))
}

pub fn dumps_dir() -> Option<PathBuf> {
    data_dir().map(|directory| directory.join("dumps"))
}

pub fn keys_dir() -> Option<PathBuf> {
    data_dir().map(|directory| directory.join("keys"))
}

pub fn ndef_dir() -> Option<PathBuf> {
    data_dir().map(|directory| directory.join("ndef"))
}

pub fn backups_dir() -> Option<PathBuf> {
    data_dir().map(|directory| directory.join("backups"))
}

/// Creates a user-facing data directory. Missing parent directories are
/// created too.
pub fn ensure(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)
        .map_err(|error| NfcError::Io(format!("could not create {}: {error}", directory.display())))
}

/// Key file paired with a dump: `keys/<dump stem>.dic` under the data
/// directory. Falls back to a sibling `.dic` when no data directory exists.
pub fn key_path_for_dump(dump: &Path) -> Option<PathBuf> {
    let stem = dump.file_stem()?;
    let name = format!("{}.dic", stem.to_string_lossy());
    match keys_dir() {
        Some(directory) => Some(directory.join(name)),
        None => Some(dump.with_file_name(name)),
    }
}

/// Same pairing rule against an explicit key directory; used by tests and
/// callers that resolve the directory themselves.
pub fn key_path_in(keys_dir: &Path, dump: &Path) -> Option<PathBuf> {
    let stem = dump.file_stem()?;
    Some(keys_dir.join(format!("{}.dic", stem.to_string_lossy())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_path_uses_the_dump_stem_and_keys_directory() {
        let path = key_path_in(
            Path::new("/data/nfc-foundry/keys"),
            Path::new("/data/nfc-foundry/dumps/home.nfc-foundry.json"),
        )
        .expect("path");
        assert_eq!(
            path,
            PathBuf::from("/data/nfc-foundry/keys/home.nfc-foundry.dic")
        );
    }
}
