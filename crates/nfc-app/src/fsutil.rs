//! Filesystem helpers for application-owned files.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nfc_core::{NfcError, Result};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            NfcError::Io(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    let temp = temporary_path(path);
    fs::write(&temp, bytes)
        .map_err(|error| NfcError::Io(format!("could not write {}: {error}", temp.display())))?;
    restrict_permissions(&temp)?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(NfcError::Io(format!(
            "could not replace {}: {error}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|error| NfcError::Io(format!("could not read {}: {error}", path.display())))
}

pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    fs::read(path)
        .map_err(|error| NfcError::Io(format!("could not read {}: {error}", path.display())))
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

fn temporary_path(path: &Path) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    path.with_file_name(format!(".{name}.{}.{counter}.tmp", std::process::id()))
}
