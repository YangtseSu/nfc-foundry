//! Amiibo CLI file and mock-reader tests.

use std::path::Path;
use std::process::{Command, Output};

use nfc_amiibo::{AmiiboKeys, AmiiboPlain, AmiiboTag, bcc0, bcc1, pack};

const SOURCE_UID: [u8; 7] = [0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];

fn nfctl(args: &[&str]) -> Output {
    let config = tempfile::tempdir().expect("config dir");
    let data = tempfile::tempdir().expect("data dir");
    Command::new(env!("CARGO_BIN_EXE_nfctl"))
        .args(args)
        .env("XDG_CONFIG_HOME", config.path())
        .env("XDG_DATA_HOME", data.path())
        .env("XDG_RUNTIME_DIR", config.path().join("runtime"))
        .output()
        .expect("run nfctl")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "nfctl failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn synthetic_key_bytes() -> Vec<u8> {
    let mut data = vec![0u8; 80];
    for (index, value) in data.iter_mut().take(16).enumerate() {
        *value = index as u8;
    }
    data[16..30].copy_from_slice(b"unfixed infos\0");
    data[31] = 14;
    for index in 0..16 {
        data[32 + index] = 0x20 + index as u8;
    }
    for index in 0..32 {
        data[48 + index] = 0x30 + index as u8;
    }
    let mut tag = vec![0u8; 80];
    for (index, value) in tag.iter_mut().take(16).enumerate() {
        *value = 0x50 + index as u8;
    }
    tag[16..30].copy_from_slice(b"locked secret\0");
    tag[31] = 16;
    for index in 0..16 {
        tag[32 + index] = 0x60 + index as u8;
    }
    for index in 0..32 {
        tag[48 + index] = 0x70 + index as u8;
    }
    let mut combined = data;
    combined.extend_from_slice(&tag);
    combined
}

fn synthetic_keys() -> AmiiboKeys {
    AmiiboKeys::from_combined(&synthetic_key_bytes()).expect("keys")
}

fn source_tag() -> AmiiboTag {
    let mut bytes = [0u8; 540];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index * 7 + 3) as u8;
    }
    let uid = SOURCE_UID;
    bytes[..8].copy_from_slice(&[bcc1(&uid), 0x48, 0x0F, 0xE0, 0xF1, 0x10, 0xFF, 0xEE]);
    bytes[0x29..0x2B].copy_from_slice(&[0x00, 0x05]);
    bytes[0x1D4..0x1DC].copy_from_slice(&[
        uid[0],
        uid[1],
        uid[2],
        bcc0(&uid),
        uid[3],
        uid[4],
        uid[5],
        uid[6],
    ]);
    bytes[0x1DC..0x1E4].copy_from_slice(&[0x12, 0x34, 0x01, 0x02, 0x0A, 0xBC, 0x03, 0x02]);
    bytes[0x208..0x21C].copy_from_slice(&[
        0x01, 0x00, 0x0F, 0xBD, 0x00, 0x00, 0x00, 0x04, 0x5F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    let plain = AmiiboPlain::from_bytes(&bytes).expect("plain");
    pack(&synthetic_keys(), &plain).expect("tag")
}

fn write_fixtures(directory: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let key = directory.join("key_retail.bin");
    let dump = directory.join("amiibo.bin");
    std::fs::write(&key, synthetic_key_bytes()).expect("key");
    std::fs::write(&dump, source_tag().as_bytes()).expect("dump");
    (key, dump)
}

#[test]
fn decrypt_encrypt_round_trip_and_key_forms() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (key, dump) = write_fixtures(directory.path());
    let plain = directory.path().join("plain.bin");
    let rebuilt = directory.path().join("rebuilt.bin");

    let output = nfctl(&[
        "--json",
        "--amiibo-keys",
        key.to_str().expect("key"),
        "amiibo",
        "decrypt",
        "--input",
        dump.to_str().expect("dump"),
        "--output",
        plain.to_str().expect("plain"),
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("\"command\": \"amiibo.decrypt\""), "{text}");
    assert!(text.contains("\"sourceUid\": \"04112233445566\""), "{text}");
    assert!(text.contains("\"gameCharacterId\": \"1234\""), "{text}");
    assert!(plain.exists());

    let output = nfctl(&[
        "--json",
        "--amiibo-keys",
        key.to_str().expect("key"),
        "amiibo",
        "encrypt",
        "--input",
        plain.to_str().expect("plain"),
        "--output",
        rebuilt.to_str().expect("rebuilt"),
    ]);
    assert_success(&output);
    assert_eq!(
        std::fs::read(&rebuilt).expect("rebuilt"),
        std::fs::read(&dump).expect("dump")
    );

    let data_key = directory.path().join("unfixed-info.bin");
    let tag_key = directory.path().join("locked-secret.bin");
    let combined = std::fs::read(&key).expect("combined");
    std::fs::write(&data_key, &combined[..80]).expect("data key");
    std::fs::write(&tag_key, &combined[80..]).expect("tag key");
    let second = directory.path().join("plain-two.bin");
    let output = nfctl(&[
        "--amiibo-data-key",
        data_key.to_str().expect("data key"),
        "--amiibo-tag-key",
        tag_key.to_str().expect("tag key"),
        "amiibo",
        "decrypt",
        "--input",
        dump.to_str().expect("dump"),
        "--output",
        second.to_str().expect("output"),
    ]);
    assert_success(&output);
    assert_eq!(
        std::fs::read(&second).expect("second"),
        std::fs::read(&plain).expect("plain")
    );
}

#[test]
fn missing_or_wrong_keys_are_reported_without_secrets() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (key, dump) = write_fixtures(directory.path());
    let output = nfctl(&["amiibo", "decrypt", "--input", dump.to_str().expect("dump")]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("supply --amiibo-keys"));

    let mut wrong = std::fs::read(&key).expect("key");
    wrong[0] ^= 0x01;
    let wrong_path = directory.path().join("wrong.bin");
    std::fs::write(&wrong_path, &wrong).expect("wrong");
    let output = nfctl(&[
        "--amiibo-keys",
        wrong_path.to_str().expect("wrong"),
        "amiibo",
        "decrypt",
        "--input",
        dump.to_str().expect("dump"),
    ]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("signatures"), "{text}");
    assert!(!text.contains(&format!("{wrong:02X?}")), "{text}");
}

#[test]
fn write_requires_advanced_and_dry_run_writes_nothing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (key, dump) = write_fixtures(directory.path());
    let key = key.to_str().expect("key");
    let dump = dump.to_str().expect("dump");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:amiibo",
        "--amiibo-keys",
        key,
        "amiibo",
        "write",
        "--input",
        dump,
    ]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--advanced"));

    let data = tempfile::tempdir().expect("data dir");
    let output = nfctl_with_data(
        &[
            "--json",
            "--mock",
            "--device",
            "mock:amiibo",
            "--amiibo-keys",
            key,
            "--dry-run",
            "amiibo",
            "write",
            "--input",
            dump,
            "--advanced",
        ],
        data.path(),
    );
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("\"command\": \"amiibo.write\""), "{text}");
    assert!(text.contains("\"dryRun\": true"), "{text}");
    assert!(text.contains("\"targetUid\": \"045B1C2A3B4C80\""), "{text}");
    assert!(text.contains("\"sourceUid\": \"04112233445566\""), "{text}");
    assert!(
        std::fs::read_dir(data.path().join("nfc-foundry/backups"))
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true),
        "dry run wrote a backup"
    );
}

#[test]
fn write_executes_and_reports_the_backup() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (key, dump) = write_fixtures(directory.path());
    let data = tempfile::tempdir().expect("data dir");
    let output = nfctl_with_data(
        &[
            "--mock",
            "--device",
            "mock:amiibo",
            "--amiibo-keys",
            key.to_str().expect("key"),
            "amiibo",
            "write",
            "--input",
            dump.to_str().expect("dump"),
            "--advanced",
            "--yes",
        ],
        data.path(),
    );
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("Amiibo written and verified"), "{text}");
    assert!(text.contains("backup:"), "{text}");
    let backups = data.path().join("nfc-foundry/backups");
    assert!(
        std::fs::read_dir(&backups)
            .expect("backups")
            .next()
            .is_some(),
        "no backup"
    );
}

fn nfctl_with_data(args: &[&str], data: &Path) -> Output {
    let config = tempfile::tempdir().expect("config dir");
    Command::new(env!("CARGO_BIN_EXE_nfctl"))
        .args(args)
        .env("XDG_CONFIG_HOME", config.path())
        .env("XDG_DATA_HOME", data)
        .env("XDG_RUNTIME_DIR", config.path().join("runtime"))
        .output()
        .expect("run nfctl")
}
