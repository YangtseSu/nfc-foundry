//! Session preferences for the workbench.
//!
//! The file is independent of `nfctl`: only the GUI reads and writes it. It is
//! written atomically with `0600` permissions so a crash cannot leave a
//! half-written preference file.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::Language;
use nfc_core::{NfcError, Result};

/// User-visible workbench preferences.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    /// Language code; unknown values fall back to the detected locale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Classic key store JSON path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_store: Option<PathBuf>,
    /// Directory for pre-write backups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<PathBuf>,
    /// Hardnested table directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardnested_tables: Option<PathBuf>,
    /// Default Classic dump format name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_dump_format: Option<String>,
    /// Whether candidate keys include the built-in common keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_common_keys: Option<bool>,
}

impl UiPreferences {
    /// Load a preference file. A missing file yields defaults; malformed JSON
    /// is an error the shell reports instead of silently discarding settings.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                NfcError::Io(format!("could not parse {}: {error}", path.display()))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(NfcError::Io(format!(
                "could not read {}: {error}",
                path.display()
            ))),
        }
    }

    /// Save atomically with `0600` permissions.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|error| NfcError::Io(format!("could not encode preferences: {error}")))?;
        text.push('\n');
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
                NfcError::Io(format!("could not create {}: {error}", parent.display()))
            })?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text).map_err(|error| {
            NfcError::Io(format!("could not write {}: {error}", temporary.display()))
        })?;
        restrict_permissions(&temporary)?;
        fs::rename(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            NfcError::Io(format!("could not replace {}: {error}", path.display()))
        })
    }

    /// Saved language, if the stored code is supported.
    pub fn language(&self) -> Option<Language> {
        self.language.as_deref().and_then(Language::from_code)
    }
}

/// `$XDG_CONFIG_HOME/nfc-foundry`, falling back to `~/.config/nfc-foundry`.
pub fn config_directory() -> Option<PathBuf> {
    if let Some(directory) = env::var_os("XDG_CONFIG_HOME")
        && !directory.is_empty()
    {
        return Some(PathBuf::from(directory).join("nfc-foundry"));
    }
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join("nfc-foundry"))
}

/// `$XDG_CONFIG_HOME/nfc-foundry/ui.json`.
pub fn preferences_path() -> Option<PathBuf> {
    config_directory().map(|directory| directory.join("ui.json"))
}

fn restrict_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions).map_err(|error| {
            NfcError::Io(format!(
                "could not restrict permissions on {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_and_restrict_permissions() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("nfc-foundry/ui.json");
        let preferences = UiPreferences {
            language: Some("zh-CN".to_owned()),
            key_store: Some(PathBuf::from("/tmp/keys.json")),
            backup_dir: Some(PathBuf::from("/tmp/backups")),
            hardnested_tables: Some(PathBuf::from("/tmp/tables")),
            default_dump_format: Some("mct".to_owned()),
            use_common_keys: Some(false),
        };
        preferences.save(&path).expect("save");
        assert_eq!(UiPreferences::load(&path).expect("load"), preferences);
        assert_eq!(preferences.language(), Some(Language::ZhCn));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_missing_file_returns_defaults_and_bad_json_is_an_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        let missing = directory.path().join("missing.json");
        assert_eq!(
            UiPreferences::load(&missing).expect("default"),
            UiPreferences::default()
        );
        let broken = directory.path().join("broken.json");
        fs::write(&broken, "not json").expect("write");
        assert!(UiPreferences::load(&broken).is_err());
    }

    #[test]
    fn an_unknown_language_code_falls_back_to_locale_detection() {
        let preferences = UiPreferences {
            language: Some("xx-YY".to_owned()),
            ..UiPreferences::default()
        };
        assert_eq!(preferences.language(), None);
    }
}
