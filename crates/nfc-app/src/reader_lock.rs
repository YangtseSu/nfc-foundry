//! Cross-process advisory reader lock.
//!
//! The GUI and `nfctl` share one lock per physical reader so two processes
//! cannot claim the same device. The lock is a POSIX `flock` on a per-reader
//! file under `$XDG_RUNTIME_DIR/nfc-foundry`, or a per-user temporary
//! directory when `XDG_RUNTIME_DIR` is absent. `flock` releases the lock when
//! the descriptor closes, so a crash never leaves a blocking stale file.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nfc_core::{NfcError, Result};
use nfc_transport::DeviceInfo;
use rustix::fs::{FlockOperation, flock};
use rustix::io::Errno;

/// An exclusive advisory lock held for one open reader connection.
///
/// Dropping the value closes the descriptor and releases the lock. The lock
/// file itself is kept so the next connection can reuse it.
#[derive(Debug)]
pub struct ReaderLock {
    _file: File,
}

impl ReaderLock {
    /// Acquire the lock for a physical reader. The device must carry a
    /// `physical_id`; scripted transports never reach this path.
    pub fn acquire(device: &DeviceInfo) -> Result<Self> {
        Self::acquire_in(&lock_dir()?, device)
    }

    pub(crate) fn acquire_in(directory: &Path, device: &DeviceInfo) -> Result<Self> {
        let key = device
            .physical_id
            .as_deref()
            .ok_or_else(|| NfcError::Io("reader lock requires a physical identity".to_owned()))?;
        prepare_directory(directory)?;
        let path = directory.join(lock_file_name(key));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                NfcError::Io(format!(
                    "could not open reader lock {}: {error}",
                    path.display()
                ))
            })?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                NfcError::Io(format!(
                    "could not secure reader lock {}: {error}",
                    path.display()
                ))
            })?;
        match flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {}
            Err(error) if error == Errno::WOULDBLOCK || error == Errno::AGAIN => {
                return Err(busy_error(device, &path));
            }
            Err(error) => {
                return Err(NfcError::Io(format!(
                    "could not lock reader {}: {error}",
                    path.display()
                )));
            }
        }
        record_holder(&file, device);
        Ok(Self { _file: file })
    }
}

/// The lock root: the runtime directory when set, otherwise a per-user
/// temporary directory with private permissions.
fn lock_dir() -> Result<PathBuf> {
    if let Some(directory) = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        return Ok(directory.join("nfc-foundry"));
    }
    let uid = rustix::process::getuid();
    Ok(std::env::temp_dir().join(format!("nfc-foundry-{}", uid.as_raw())))
}

fn prepare_directory(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory).map_err(|error| {
        NfcError::Io(format!(
            "could not create reader lock directory {}: {error}",
            directory.display()
        ))
    })?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
        NfcError::Io(format!(
            "could not secure reader lock directory {}: {error}",
            directory.display()
        ))
    })
}

/// Filename for one physical identity. The readable prefix helps diagnostics;
/// the stable FNV-1a suffix keeps distinct identities from colliding after
/// sanitization and makes the name independent of the Rust toolchain.
fn lock_file_name(key: &str) -> String {
    let mut stem = String::with_capacity(48);
    for character in key.chars().take(48) {
        stem.push(if character.is_ascii_alphanumeric() {
            character
        } else {
            '_'
        });
    }
    format!("reader-{stem}-{:016x}.lock", fnv1a(key.as_bytes()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn record_holder(file: &File, device: &DeviceInfo) {
    let program = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "nfc-foundry".to_owned());
    let record = format!(
        "pid={}\nprogram={program}\nreader={}\n",
        std::process::id(),
        device.conn_string
    );
    let _ = file.set_len(0);
    let mut file = file;
    let _ = file.write_all(record.as_bytes());
    let _ = file.flush();
}

fn busy_error(device: &DeviceInfo, path: &Path) -> NfcError {
    let holder = read_holder(path);
    let who = match (holder.program, holder.pid) {
        (Some(program), Some(pid)) => format!(" by {program} (pid {pid})"),
        (None, Some(pid)) => format!(" by process {pid}"),
        _ => " by another NFC Foundry process".to_owned(),
    };
    NfcError::ReaderBusy(format!(
        "{} is in use{who}; close that session or wait for it to exit",
        device.conn_string
    ))
}

#[derive(Debug, Default)]
struct Holder {
    pid: Option<u32>,
    program: Option<String>,
}

fn read_holder(path: &Path) -> Holder {
    let Ok(mut file) = File::open(path) else {
        return Holder::default();
    };
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return Holder::default();
    }
    let mut holder = Holder::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "pid" => holder.pid = value.parse().ok(),
            "program" => holder.program = Some(value.to_owned()),
            _ => {}
        }
    }
    holder
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(physical_id: &str) -> DeviceInfo {
        DeviceInfo::new("Mock reader", "mock:reader").with_physical_id(physical_id)
    }

    #[test]
    fn second_holder_is_busy_until_the_first_lock_drops() {
        let directory = tempfile::tempdir().expect("tempdir");
        let reader = device("test:physical:1");
        let first = ReaderLock::acquire_in(directory.path(), &reader).expect("first lock");

        let error = ReaderLock::acquire_in(directory.path(), &reader).expect_err("second lock");
        let message = error.to_string();
        assert!(matches!(error, NfcError::ReaderBusy(_)), "{message}");
        assert!(message.contains("mock:reader"), "{message}");
        assert!(
            message.contains(&format!("pid {}", std::process::id())),
            "{message}"
        );

        drop(first);
        ReaderLock::acquire_in(directory.path(), &reader).expect("lock after release");
    }

    #[test]
    fn stale_lock_file_does_not_block_after_release() {
        let directory = tempfile::tempdir().expect("tempdir");
        let reader = device("test:physical:2");
        drop(ReaderLock::acquire_in(directory.path(), &reader).expect("first lock"));

        let entries = std::fs::read_dir(directory.path())
            .expect("read lock directory")
            .count();
        assert_eq!(entries, 1, "the lock file is kept for reuse");
        ReaderLock::acquire_in(directory.path(), &reader).expect("reused lock file");
    }

    #[test]
    fn lock_files_are_private_and_named_by_physical_identity() {
        let directory = tempfile::tempdir().expect("tempdir");
        let reader = device("test:physical:3");
        let lock = ReaderLock::acquire_in(directory.path(), &reader).expect("lock");
        assert_eq!(
            std::fs::metadata(directory.path())
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let entries = std::fs::read_dir(directory.path())
            .expect("read lock directory")
            .collect::<std::io::Result<Vec<_>>>()
            .expect("entries");
        assert_eq!(entries.len(), 1);
        let path = entries[0].path();
        let name = path.file_name().expect("file name").to_string_lossy();
        assert!(name.starts_with("reader-test_physical_3-"), "{name}");
        assert!(name.ends_with(".lock"), "{name}");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            path,
            directory.path().join(lock_file_name("test:physical:3"))
        );
        drop(lock);
    }

    #[test]
    fn lock_name_is_stable_across_calls_and_distinguishes_keys() {
        assert_eq!(lock_file_name("a/b:c"), lock_file_name("a/b:c"));
        assert_ne!(lock_file_name("a/b:c"), lock_file_name("a_b_c"));
        assert_eq!(fnv1a(b"hello"), 0xa430_d846_80aa_bd0b);
    }
}
