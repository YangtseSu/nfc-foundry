//! Persisted workbench preferences.
//!
//! The file belongs to the GUI only. It is written atomically with `0600`
//! permissions so a crash cannot leave a half-written preference file and a
//! key-store path is not exposed to other users.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use nfc_app::classic::KeyStore;
use nfc_classic::DumpFormat;
use nfc_core::{NfcError, Result};
use serde::{Deserialize, Serialize};

/// User-visible workbench preferences. Missing fields use defaults; a missing
/// file is not an error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    /// Classic key store JSON path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_store: Option<PathBuf>,
    /// Directory for pre-write backups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<PathBuf>,
    /// Hardnested table directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardnested_tables: Option<PathBuf>,
    /// Whether card operations include the built-in common keys.
    pub use_common_keys: bool,
    /// Hex keys checked in the key-scan selector. `None` selects the three
    /// built-in default keys on first run; `Some([])` means the user cleared
    /// every key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyscan_keys: Option<Vec<String>>,
    /// Default Classic dump format name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_dump_format: Option<String>,
    /// Workbench language code. `None` uses the system locale on first run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            key_store: None,
            backup_dir: None,
            hardnested_tables: None,
            use_common_keys: true,
            keyscan_keys: None,
            default_dump_format: None,
            language: None,
        }
    }
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
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "ui.json".to_owned());
        let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
        fs::write(&temporary, text).map_err(|error| {
            NfcError::Io(format!("could not write {}: {error}", temporary.display()))
        })?;
        if let Err(error) = restrict_permissions(&temporary) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        fs::rename(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            NfcError::Io(format!("could not replace {}: {error}", path.display()))
        })
    }

    /// Concrete default dump format. `auto` resolves per image at save time.
    pub fn dump_format(&self) -> Result<DumpFormat> {
        match &self.default_dump_format {
            Some(name) => DumpFormat::parse(name),
            None => Ok(DumpFormat::Auto),
        }
    }

    pub fn set_dump_format(&mut self, format: DumpFormat) {
        self.default_dump_format = Some(format.name().to_owned());
    }

    /// Rejects paths and formats that cannot be applied. Existing key stores
    /// must parse; table directories must contain usable tables.
    pub fn validate(&self) -> Result<()> {
        if let Some(path) = &self.key_store {
            if path.as_os_str().is_empty() {
                return Err(NfcError::InvalidArgument(
                    "the key store path is empty".to_owned(),
                ));
            }
            if path.exists() {
                if !path.is_file() {
                    return Err(NfcError::InvalidArgument(format!(
                        "the key store path {} is not a file",
                        path.display()
                    )));
                }
                KeyStore::load(path)?;
            } else {
                validate_parent(path, "key store")?;
            }
        }
        if let Some(directory) = &self.backup_dir {
            if directory.as_os_str().is_empty() {
                return Err(NfcError::InvalidArgument(
                    "the backup directory is empty".to_owned(),
                ));
            }
            if directory.exists() {
                if !directory.is_dir() {
                    return Err(NfcError::InvalidArgument(format!(
                        "the backup path {} is not a directory",
                        directory.display()
                    )));
                }
            } else {
                validate_parent(directory, "backup directory")?;
            }
        }
        if let Some(directory) = &self.hardnested_tables {
            if directory.as_os_str().is_empty() {
                return Err(NfcError::InvalidArgument(
                    "the hardnested table directory is empty".to_owned(),
                ));
            }
            if !has_table_files(directory) {
                return Err(NfcError::InvalidArgument(format!(
                    "the hardnested table directory {} contains no bitflip state tables",
                    directory.display()
                )));
            }
        }
        self.dump_format()?;
        if let Some(code) = &self.language
            && crate::i18n::Language::from_code(code).is_none()
        {
            return Err(NfcError::InvalidArgument(format!(
                "the language {code} is not supported"
            )));
        }
        Ok(())
    }

    /// Effective workbench language: the saved choice, then the system
    /// locale, then English.
    pub fn language(&self) -> crate::i18n::Language {
        let saved = self
            .language
            .as_deref()
            .and_then(crate::i18n::Language::from_code);
        crate::i18n::detect_language(saved, |variable| std::env::var(variable).ok())
    }

    pub fn set_language(&mut self, language: crate::i18n::Language) {
        self.language = Some(language.code().to_owned());
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

fn has_table_files(directory: &Path) -> bool {
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("bitflip_") && name.contains("_states.bin")
    })
}

fn validate_parent(path: &Path, label: &str) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    let mut ancestor = parent;
    while !ancestor.exists() {
        let Some(next) = ancestor.parent() else {
            return Ok(());
        };
        if next.as_os_str().is_empty() {
            return Ok(());
        }
        ancestor = next;
    }
    if ancestor.is_dir() {
        Ok(())
    } else {
        Err(NfcError::InvalidArgument(format!(
            "the {label} parent {} is not a directory",
            ancestor.display()
        )))
    }
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
        let mut preferences = UiPreferences {
            key_store: Some(directory.path().join("classic-keys.json")),
            backup_dir: Some(directory.path().to_path_buf()),
            hardnested_tables: None,
            use_common_keys: false,
            keyscan_keys: Some(vec!["D3F7D3F7D3F7".to_owned()]),
            default_dump_format: None,
            language: None,
        };
        preferences.set_dump_format(DumpFormat::Mct);
        preferences.set_language(crate::i18n::Language::ZhCn);
        preferences.save(&path).expect("save");
        assert_eq!(UiPreferences::load(&path).expect("load"), preferences);
        assert_eq!(preferences.dump_format().expect("format"), DumpFormat::Mct);
        assert_eq!(preferences.language(), crate::i18n::Language::ZhCn);

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
        assert!(UiPreferences::default().use_common_keys);
        let broken = directory.path().join("broken.json");
        fs::write(&broken, "not json").expect("write");
        assert!(UiPreferences::load(&broken).is_err());
    }

    #[test]
    fn validation_rejects_unreadable_stores_and_missing_tables() {
        let directory = tempfile::tempdir().expect("temp dir");
        let broken = directory.path().join("keys.json");
        fs::write(&broken, "not json").expect("write");
        let preferences = UiPreferences {
            key_store: Some(broken),
            ..UiPreferences::default()
        };
        assert!(preferences.validate().is_err());

        let preferences = UiPreferences {
            hardnested_tables: Some(directory.path().join("tables")),
            ..UiPreferences::default()
        };
        assert!(preferences.validate().is_err());

        let preferences = UiPreferences {
            default_dump_format: Some("unknown".to_owned()),
            ..UiPreferences::default()
        };
        assert!(preferences.validate().is_err());

        let preferences = UiPreferences {
            language: Some("fr".to_owned()),
            ..UiPreferences::default()
        };
        assert!(preferences.validate().is_err());
    }

    #[test]
    fn an_unsaved_language_uses_the_system_locale() {
        let preferences = UiPreferences::default();
        let expected = crate::i18n::detect_language(None, |variable| std::env::var(variable).ok());
        assert_eq!(preferences.language(), expected);
    }

    #[test]
    fn validation_allows_missing_output_paths_with_existing_parents() {
        let directory = tempfile::tempdir().expect("temp dir");
        let preferences = UiPreferences {
            key_store: Some(directory.path().join("new-keys.json")),
            backup_dir: Some(directory.path().join("backups")),
            ..UiPreferences::default()
        };
        preferences.validate().expect("valid paths");
    }
}
