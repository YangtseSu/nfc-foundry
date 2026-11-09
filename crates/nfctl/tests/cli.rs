//! CLI smoke tests against the scripted mock reader.

use std::path::Path;
use std::process::{Command, Output};

fn nfctl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nfctl"))
        .args(args)
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

#[test]
fn list_and_scan_keep_the_mock_contract() {
    let output = nfctl(&["--mock", "list"]);
    assert_success(&output);
    assert!(stdout(&output).contains("mock:classic-1k"));

    let output = nfctl(&["--mock", "--device", "mock:classic-1k", "scan"]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("MIFARE Classic 1K"), "{text}");
    assert!(text.contains("04 A2 24 8A"), "{text}");
}

#[test]
fn dump_read_and_diff_work_against_the_mock_card() {
    let directory = tempfile::tempdir().expect("temp dir");
    let dump = directory.path().join("card.json");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "dump",
        "--output",
        dump.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("64/64 blocks"),
        "{}",
        stdout(&output)
    );
    assert!(dump.exists());

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "read",
        "--block",
        "4",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("block 4: 04 04 04 04"),
        "{}",
        stdout(&output)
    );

    let output = nfctl(&[
        "classic",
        "diff",
        dump.to_str().expect("path"),
        dump.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("0 changed, 64 equal"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn write_restore_and_backups_are_reported() {
    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "write",
        "--block",
        "4",
        "--data",
        "000102030405060708090A0B0C0D0E0F",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("block 4 written and verified"), "{text}");
    assert!(text.contains("backup:"), "{text}");
    assert!(
        std::fs::read_dir(&backups)
            .expect("backup dir")
            .next()
            .is_some()
    );

    let dump = directory.path().join("card.json");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "dump",
        "--output",
        dump.to_str().expect("path"),
    ]);
    assert_success(&output);

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "restore",
        "--input",
        dump.to_str().expect("path"),
        "--confirm",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("0 written, 63 unchanged"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn destructive_actions_require_confirmation() {
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "write",
        "--block",
        "0",
        "--data",
        "00000000000000000000000000000000",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("manufacturer block 0"));

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "write",
        "--block",
        "7",
        "--data",
        "FFFFFFFFFFFF FF0780 69 FFFFFFFFFFFF",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("explicit confirmation"));

    let directory = tempfile::tempdir().expect("temp dir");
    let dump = directory.path().join("card.json");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "dump",
        "--output",
        dump.to_str().expect("path"),
    ]);
    assert_success(&output);
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "restore",
        "--input",
        dump.to_str().expect("path"),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("explicit confirmation"));
}

#[test]
fn keyscan_persists_discovered_keys_and_keys_commands_read_them() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = directory.path().join("keys.json");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "keyscan",
        "--types",
        "A",
        "--save",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(stdout(&output).contains("sector 0"), "{}", stdout(&output));
    assert!(store.exists());

    let output = nfctl(&[
        "classic",
        "keys",
        "list",
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("FFFFFFFFFFFF"), "{text}");

    let output = nfctl(&[
        "classic",
        "keys",
        "add",
        "DEADBEEF0000",
        "--sector",
        "3",
        "--key-type",
        "B",
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    let output = nfctl(&[
        "classic",
        "keys",
        "list",
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(stdout(&output).contains("DEADBEEF0000"));
}

#[test]
fn key_file_import_is_visible_to_read_commands() {
    let directory = tempfile::tempdir().expect("temp dir");
    let keys = directory.path().join("keys.txt");
    std::fs::write(&keys, "FFFFFFFFFFFF\n").expect("key file");
    let store = directory.path().join("keys.json");

    let output = nfctl(&[
        "classic",
        "keys",
        "import",
        keys.to_str().expect("path"),
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(Path::new(&store).exists());

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--key-store",
        store.to_str().expect("path"),
        "--no-common-keys",
        "classic",
        "read",
        "--block",
        "4",
    ]);
    assert_success(&output);
    assert!(stdout(&output).contains("block 4:"));
}
