//! Pre-mutation backups stored as project-format dumps.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nfc_classic::{Dump, DumpFormat, exchange};
use nfc_core::Result;

use crate::fsutil::write_atomic;

static BACKUP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Directory-backed store for project-format snapshots.
#[derive(Clone, Debug)]
pub struct BackupStore {
    directory: PathBuf,
}

impl Default for BackupStore {
    fn default() -> Self {
        Self::new(Self::default_directory())
    }
}

impl BackupStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// `$XDG_DATA_HOME/nfc-foundry/backups`, falling back to
    /// `$HOME/.local/share/nfc-foundry/backups`.
    pub fn default_directory() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("share"))
            })
            .unwrap_or_else(std::env::temp_dir);
        base.join("nfc-foundry").join("backups")
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Writes a project-format snapshot atomically and returns its path.
    /// Unknown bytes and block statuses are preserved by the project format.
    pub fn save(&self, dump: &Dump, label: &str) -> Result<BackupRecord> {
        let bytes = exchange::export(dump, DumpFormat::Project)?;
        self.save_bytes(label, &bytes)
    }

    /// Writes an already-encoded project snapshot atomically. Type 2 images
    /// use their own JSON project format through this path.
    pub fn save_bytes(&self, label: &str, bytes: &[u8]) -> Result<BackupRecord> {
        let path = self
            .directory
            .join(format!("{}-{}.json", sanitize(label), unique()));
        write_atomic(&path, bytes)?;
        Ok(BackupRecord {
            path,
            bytes: bytes.len(),
        })
    }
}

/// Result of a successful backup write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BackupRecord {
    pub path: PathBuf,
    pub bytes: usize,
}

fn sanitize(label: &str) -> String {
    let mut sanitized: String = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    sanitized.truncate(80);
    if sanitized.is_empty() {
        "classic".to_owned()
    } else {
        sanitized
    }
}

fn unique() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let counter = BACKUP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{:09}-{}-{counter}",
        now.as_secs(),
        now.subsec_nanos(),
        std::process::id()
    )
}

#[cfg(test)]
mod tests {
    use nfc_classic::Layout;

    use super::*;
    use crate::fsutil::read_text;

    fn dump() -> Dump {
        let mut dump = Dump::new(Layout::Classic1K);
        dump.blocks[0].data[0..4].copy_from_slice(&[1, 2, 3, 4]);
        dump.blocks[0].known_mask = 0x0F;
        dump.blocks[1].data[0..4].copy_from_slice(&[9, 8, 7, 6]);
        dump.blocks[1].known_mask = 0x0F;
        dump
    }

    #[test]
    fn backup_round_trips_partial_state_through_the_project_format() {
        let directory = tempfile::tempdir().expect("temp dir");
        let store = BackupStore::new(directory.path());
        let source = dump();
        let record = store.save(&source, "04A2248A/write").expect("backup");
        assert!(record.path.starts_with(directory.path()));
        assert!(
            record
                .path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .starts_with("04A2248A-write-")
        );
        let text = read_text(&record.path).expect("backup text");
        let loaded = exchange::load(text.as_bytes()).expect("load backup");
        assert_eq!(loaded.dump, source);
        assert!(!loaded.dump.blocks[1].complete());
        assert_eq!(loaded.dump.blocks[1].known_mask, 0x000F);
        assert!(!loaded.dump.blocks[2].complete());
        assert!(record.bytes > 0);
    }

    #[test]
    fn backup_names_are_unique_and_sanitized() {
        let directory = tempfile::tempdir().expect("temp dir");
        let store = BackupStore::new(directory.path());
        let first = store.save(&dump(), "card: 1").expect("backup");
        let second = store.save(&dump(), "card: 1").expect("backup");
        assert_ne!(first.path, second.path);
        assert!(
            first
                .path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .starts_with("card--1-")
        );
    }

    #[test]
    fn default_directory_ends_with_the_product_backup_folder() {
        let directory = BackupStore::default_directory();
        assert!(directory.ends_with("nfc-foundry/backups"));
    }
}
