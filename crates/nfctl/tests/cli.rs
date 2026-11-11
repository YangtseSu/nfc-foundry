//! CLI smoke tests against the scripted mock reader.

use std::path::Path;
use std::process::{Command, Output};

use nfc_classic::{DumpFormat, exchange};

fn nfctl(args: &[&str]) -> Output {
    let config = tempfile::tempdir().expect("config dir");
    Command::new(env!("CARGO_BIN_EXE_nfctl"))
        .args(args)
        .env("XDG_CONFIG_HOME", config.path())
        .output()
        .expect("run nfctl")
}

fn nfctl_chinese_locale(args: &[&str]) -> Output {
    let config = tempfile::tempdir().expect("config dir");
    Command::new(env!("CARGO_BIN_EXE_nfctl"))
        .args(args)
        .env("XDG_CONFIG_HOME", config.path())
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .env("LC_MESSAGES", "zh_CN.UTF-8")
        .output()
        .expect("run nfctl under a Chinese locale")
}

#[test]
fn help_and_errors_stay_english_under_a_chinese_locale() {
    let output = nfctl_chinese_locale(&["--help"]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(
        text.contains("NFC Foundry command-line workbench"),
        "{text}"
    );
    assert!(text.contains("Usage:"), "{text}");
    assert!(!text.contains('读'), "{text}");

    let output = nfctl_chinese_locale(&["--mock", "--device", "mock:missing", "scan"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("invalid argument"), "{text}");
    assert!(!text.contains('读'), "{text}");
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
        "--yes",
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
    assert!(stderr(&output).contains("needs confirmation"));
}

#[test]
fn keyscan_uses_explicit_key_files_when_common_keys_are_disabled() {
    let directory = tempfile::tempdir().expect("temp dir");
    let dictionary = directory.path().join("keys.dic");
    let store = directory.path().join("keys.json");
    std::fs::write(&dictionary, "A0A1A2A3A4A5\n").expect("dictionary");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--no-common-keys",
        "--key-file",
        dictionary.to_str().expect("path"),
        "classic",
        "keyscan",
        "--types",
        "A",
        "--save",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(stdout(&output).contains("sector 0"), "{}", stdout(&output));

    let output = nfctl(&[
        "classic",
        "keys",
        "list",
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("A0A1A2A3A4A5"), "{text}");
    assert!(!text.contains("FFFFFFFFFFFF"), "{text}");
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

#[test]
fn dump_auto_uses_pm3_raw_and_keys_export_round_trips() {
    let directory = tempfile::tempdir().expect("temp dir");
    let dump = directory.path().join("card-dump.bin");
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
    let text = stdout(&output);
    assert!(text.contains("(1024 bytes, raw)"), "{text}");
    assert_eq!(std::fs::metadata(&dump).expect("dump").len(), 1024);

    let store = directory.path().join("keys.json");
    let dic = directory.path().join("keys.dic");
    let output = nfctl(&[
        "classic",
        "keys",
        "add",
        "a0a1a2a3a4a5",
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    let output = nfctl(&[
        "classic",
        "keys",
        "export",
        "--format",
        "dic",
        "--output",
        dic.to_str().expect("path"),
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    let exported = std::fs::read_to_string(&dic).expect("dic");
    assert_eq!(exported, "A0A1A2A3A4A5\n");

    let imported = directory.path().join("imported.json");
    let output = nfctl(&[
        "classic",
        "keys",
        "import",
        dic.to_str().expect("path"),
        "--key-store",
        imported.to_str().expect("path"),
    ]);
    assert_success(&output);
    let output = nfctl(&[
        "classic",
        "keys",
        "list",
        "--key-store",
        imported.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(stdout(&output).contains("A0A1A2A3A4A5"));

    let output = nfctl(&[
        "classic",
        "keys",
        "export",
        "--format",
        "json",
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(json["version"], 2);
    assert_eq!(json["keys"][0]["key"], "A0A1A2A3A4A5");
}

#[test]
fn read_and_project_dump_preserve_verified_keys() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = directory.path().join("keys.json");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--key-store",
        store.to_str().expect("path"),
        "classic",
        "read",
        "--block",
        "4",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("captured 1 key(s)"),
        "{}",
        stdout(&output)
    );
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&store).expect("store")).expect("json");
    assert_eq!(json["version"], 2);
    assert_eq!(json["verifications"][0]["card"], "04A2248A/0004/08");
    assert_eq!(json["verifications"][0]["sector"], 1);

    let project = directory.path().join("card.nfc-foundry.json");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--key-store",
        store.to_str().expect("path"),
        "classic",
        "dump",
        "--format",
        "project",
        "--output",
        project.to_str().expect("path"),
    ]);
    assert_success(&output);
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).expect("project"))
            .expect("project json");
    assert_eq!(json["version"], 2);
    assert_eq!(json["device"], "mock:classic-1k");
    assert!(json["keys"].as_array().is_some_and(|keys| keys.len() == 16));
    assert_eq!(json["keys"][0]["source"], "common");
}

#[test]
fn key_import_reports_bad_lines_and_keeps_valid_keys() {
    let directory = tempfile::tempdir().expect("temp dir");
    let file = directory.path().join("keys.dic");
    std::fs::write(
        &file,
        "A0A1A2A3A4A5\n\
         not-a-key\n\
         FFFFFFFFFFFF\n\
         FFFFFFFFFFFF\n",
    )
    .expect("dictionary");
    let store = directory.path().join("keys.json");
    let output = nfctl(&[
        "classic",
        "keys",
        "import",
        file.to_str().expect("path"),
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("imported 2 key(s)"), "{text}");
    assert!(text.contains("1 duplicate"), "{text}");
    assert!(text.contains("1 issue"), "{text}");
    let output = nfctl(&[
        "classic",
        "keys",
        "list",
        "--key-store",
        store.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(stdout(&output).contains("A0A1A2A3A4A5"));
    assert!(stdout(&output).contains("FFFFFFFFFFFF"));
}

#[test]
fn auto_recovery_scans_every_sector_and_verifies_the_read() {
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--yes",
        "recover",
        "auto",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("verified sector 0 key A:"), "{text}");
    assert!(text.contains("verified sector 15 key B:"), "{text}");
    assert!(text.contains("sector keys 32/32"), "{text}");
    assert!(text.contains("read verification 16/16"), "{text}");
}

#[test]
fn recovery_requires_authorization_without_yes() {
    let output = nfctl(&["--mock", "--device", "mock:classic-1k", "recover", "auto"]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("needs confirmation"), "{text}");
    assert!(text.contains("pass --yes"), "{text}");
}

#[test]
fn nested_recovery_without_parity_fails_honestly() {
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--key",
        "A0A1A2A3A4A5",
        "--yes",
        "recover",
        "nested",
        "--known-block",
        "4",
        "--known-type",
        "B",
        "--target-block",
        "0",
    ]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("recovery parity") || text.contains("unsupported"),
        "{text}"
    );
}

#[test]
fn type2_info_and_dump_describe_the_ntag() {
    let directory = tempfile::tempdir().expect("temp dir");
    let dump = directory.path().join("ntag.json");

    let output = nfctl(&["--mock", "--device", "mock:ntag215", "type2", "info"]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("product: NTAG215"), "{text}");
    assert!(text.contains("NDEF data area: 496 bytes"), "{text}");
    assert!(text.contains("NDEF: empty"), "{text}");
    assert!(text.contains("AUTH0: FF"), "{text}");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "type2",
        "dump",
        "--output",
        dump.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("135/135 pages"),
        "{}",
        stdout(&output)
    );
    assert!(dump.exists());
}

#[test]
fn type2_auth_flags_are_explicit_and_never_echo_secrets() {
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--type2-password",
        "11223344",
        "type2",
        "read",
    ]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("must be supplied together"), "{text}");
    assert!(!text.contains("11223344"), "{text}");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--type2-password",
        "FFFFFFFF",
        "--type2-pack",
        "0000",
        "type2",
        "read",
        "--page",
        "4",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("page 4"), "{text}");
    assert!(!text.contains("11223344"), "{text}");
    assert!(!text.contains("AABB"), "{text}");
}

#[test]
fn type2_password_accepts_current_credential_aliases() {
    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--backup-dir",
        backups.to_str().expect("path"),
        "--current-pwd",
        "FFFFFFFF",
        "--current-pack",
        "0000",
        "type2",
        "password",
        "--auth0",
        "32",
        "--access",
        "80",
        "--pwd",
        "11223344",
        "--pack",
        "AABB",
        "--yes",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("password configuration written"), "{text}");
    assert!(!text.contains("11223344"), "{text}");
    assert!(!text.contains("AABB"), "{text}");
}

#[test]
fn type2_page_write_is_verified_and_advanced_write_needs_confirmation() {
    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--backup-dir",
        backups.to_str().expect("path"),
        "type2",
        "write",
        "--page",
        "8",
        "--data",
        "A1A2A3A4",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("written and verified"),
        "{}",
        stdout(&output)
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "type2",
        "write",
        "--page",
        "3",
        "--data",
        "E1103E00",
        "--advanced",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("needs confirmation"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn ndef_text_write_read_and_format_round_trip() {
    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--backup-dir",
        backups.to_str().expect("path"),
        "ndef",
        "write",
        "--text",
        "hello",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("NDEF written and verified"),
        "{}",
        stdout(&output)
    );

    // Each `--mock` invocation starts from the scripted blank tag.
    let output = nfctl(&["--mock", "--device", "mock:ntag215", "ndef", "read"]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("NDEF: empty"),
        "{}",
        stdout(&output)
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--backup-dir",
        backups.to_str().expect("path"),
        "ndef",
        "format",
        "--yes",
    ]);
    assert_success(&output);

    let output = nfctl(&["--mock", "--device", "mock:ntag215", "ndef", "read"]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("NDEF: empty"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn type2_dump_restore_and_lock_round_trip() {
    let directory = tempfile::tempdir().expect("temp dir");
    let dump = directory.path().join("ntag.json");
    let backups = directory.path().join("backups");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "type2",
        "dump",
        "--output",
        dump.to_str().expect("path"),
    ]);
    assert_success(&output);

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--backup-dir",
        backups.to_str().expect("path"),
        "type2",
        "restore",
        "--input",
        dump.to_str().expect("path"),
        "--yes",
    ]);
    assert_success(&output);
    assert!(stdout(&output).contains("unchanged"), "{}", stdout(&output));

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "--backup-dir",
        backups.to_str().expect("path"),
        "type2",
        "lock",
        "--pages",
        "4",
        "--yes",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("lock bits written"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn classic_ndef_reads_reports_and_writes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let output_file = directory.path().join("message.bin");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-ndef",
        "classic",
        "ndef",
        "info",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("mapping: 1.0"), "{text}");
    assert!(text.contains("NFC sectors: 1, 2, 3"), "{text}");
    assert!(text.contains("NDEF: 1 record(s)"), "{text}");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-ndef",
        "classic",
        "ndef",
        "read",
        "--output",
        output_file.to_str().expect("path"),
    ]);
    assert_success(&output);
    assert!(output_file.exists());
    assert!(stdout(&output).contains("text"), "{}", stdout(&output));

    let backups = directory.path().join("backups");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-ndef",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "ndef",
        "write",
        "--text",
        "updated",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("NDEF written"), "{text}");
    assert!(text.contains("backup"), "{text}");
}

#[test]
fn classic_ndef_format_requires_confirmation() {
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "ndef",
        "format",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("needs confirmation"),
        "{}",
        stderr(&output)
    );

    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "ndef",
        "format",
        "--yes",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("NDEF: empty"), "{text}");
    assert!(text.contains("16 trailer(s)"), "{text}");
    assert!(text.contains("backup"), "{text}");
}

#[test]
fn classic_ndef_format_can_set_read_only() {
    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "ndef",
        "format",
        "--yes",
        "--read-only",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("read-only"), "{text}");
    assert!(text.contains("NDEF: empty"), "{text}");
}

#[test]
fn version_and_diag_report_the_release_version() {
    let version = env!("CARGO_PKG_VERSION");

    let output = nfctl(&["--version"]);
    assert_success(&output);
    assert_eq!(stdout(&output).trim(), format!("nfctl {version}"));

    let output = nfctl(&["diag"]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains(&format!("NFC Foundry {version}")), "{text}");
    assert!(text.contains(&format!("nfctl {version}")), "{text}");
    assert!(text.contains("pn532_uart:"), "{text}");
    assert!(text.contains("acr122_usb:"), "{text}");
    assert!(text.contains("build:"), "{text}");
    assert!(!text.contains("skeleton"), "{text}");

    let output = nfctl(&["--mock", "diag"]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("reader: Mock Classic 1K"), "{text}");
    assert!(text.contains("mock:classic-1k"), "{text}");
}

#[test]
fn restore_dry_run_blocks_and_changes_only_share_the_plan() {
    let directory = tempfile::tempdir().expect("temp dir");
    let dump_path = directory.path().join("card.json");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "dump",
        "--output",
        dump_path.to_str().expect("path"),
        "--format",
        "project",
    ]);
    assert_success(&output);
    let mut dump = exchange::load(&std::fs::read(&dump_path).expect("dump"))
        .expect("load dump")
        .dump;
    dump.blocks[4].set_read([0x41; 16]);
    dump.blocks[6].set_read([0x46; 16]);
    let source = directory.path().join("changed.json");
    std::fs::write(
        &source,
        exchange::export(&dump, DumpFormat::Project).expect("export"),
    )
    .expect("write source");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "write",
        "--block",
        "4",
        "--data",
        "AA0102030405060708090A0B0C0D0E0F",
        "--dry-run",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("block 4"), "{text}");
    assert!(
        text.contains("dry run: no card writes were performed"),
        "{text}"
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--blocks",
        "4,6",
        "--dry-run",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("block 4"), "{text}");
    assert!(text.contains("block 6"), "{text}");
    assert!(!text.contains("block 5"), "{text}");
    assert!(
        text.contains("dry run: no card writes were performed"),
        "{text}"
    );

    let backups = directory.path().join("backups");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--blocks",
        "4,6",
        "--yes",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("2 written"), "{text}");
    assert!(text.contains("not selected: 62"), "{text}");
    assert_eq!(std::fs::read_dir(&backups).expect("backup dir").count(), 1);

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "restore",
        "--input",
        dump_path.to_str().expect("path"),
        "--changes-only",
        "--yes",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("no changed blocks"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn uid_write_gates_and_verifies_on_the_mock_card() {
    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "uid",
        "--uid",
        "DEADBEEF",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--advanced"),
        "{}",
        stderr(&output)
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "uid",
        "--uid",
        "DEADBEEF",
        "--advanced",
    ]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--yes"), "{}", stderr(&output));

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "uid",
        "--uid",
        "not-a-uid",
        "--advanced",
        "--yes",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("hexadecimal"),
        "{}",
        stderr(&output)
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "uid",
        "--uid",
        "DEADBEEF",
        "--advanced",
        "--dry-run",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("target DE AD BE EF"), "{text}");
    assert!(text.contains("old "), "{text}");
    assert!(text.contains("new "), "{text}");
    assert!(text.contains("dry run"), "{text}");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "uid",
        "--uid",
        "DEADBEEF",
        "--advanced",
        "--yes",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(
        text.contains("UID changed: 04 A2 24 8A -> DE AD BE EF"),
        "{text}"
    );
    assert!(text.contains("block 0 verified"), "{text}");
    assert!(text.contains("BCC valid"), "{text}");
    assert!(text.contains("backup:"), "{text}");
    assert_eq!(
        std::fs::read_dir(&backups)
            .expect("backup dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count(),
        1
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "uid",
        "--uid",
        "04A2248A",
        "--advanced",
        "--yes",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("already matches block 0"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn restore_with_uid_dry_run_gates_and_executes_on_the_mock_card() {
    let directory = tempfile::tempdir().expect("temp dir");
    let backups = directory.path().join("backups");
    let dump_path = directory.path().join("card.json");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "dump",
        "--output",
        dump_path.to_str().expect("path"),
        "--format",
        "project",
    ]);
    assert_success(&output);
    let mut dump = exchange::load(&std::fs::read(&dump_path).expect("dump"))
        .expect("load dump")
        .dump;
    dump.blocks[4].set_read([0x41; 16]);
    let source = directory.path().join("changed.json");
    std::fs::write(
        &source,
        exchange::export(&dump, DumpFormat::Project).expect("export"),
    )
    .expect("write source");

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--uid",
        "DEADBEEF",
        "--advanced",
        "--dry-run",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("UID target DEADBEEF"), "{text}");
    assert!(text.contains("new block 0"), "{text}");
    assert!(
        text.contains("dry run: no card writes were performed"),
        "{text}"
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--uid",
        "DEADBEEF",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--advanced"),
        "{}",
        stderr(&output)
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--advanced",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--uid"), "{}", stderr(&output));

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--blocks",
        "4",
        "--uid",
        "DEADBEEF",
        "--advanced",
        "--yes",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("full restore"),
        "{}",
        stderr(&output)
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--uid",
        "DEADBEEF",
        "--advanced",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("writing UID DEADBEEF"),
        "{}",
        stderr(&output)
    );

    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--backup-dir",
        backups.to_str().expect("path"),
        "classic",
        "restore",
        "--input",
        source.to_str().expect("path"),
        "--uid",
        "DE:AD:BE:EF",
        "--advanced",
        "--yes",
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("1 written"), "{text}");
    assert!(text.contains("UID changed: 04A2248A -> DEADBEEF"), "{text}");
    assert!(text.contains("BCC valid"), "{text}");
    assert!(text.contains("UID backup:"), "{text}");
}

#[test]
fn diag_json_is_a_stable_self_check_without_card_material() {
    let output = nfctl(&["--mock", "diag", "--json"]);
    assert_success(&output);
    let text = stdout(&output);
    let value: serde_json::Value = serde_json::from_str(&text).expect("self-check JSON");
    let object = value.as_object().expect("object");
    assert_eq!(
        object.keys().cloned().collect::<Vec<_>>(),
        ["app", "drivers", "hardnested_tables", "platform", "readers"]
    );
    assert_eq!(value["app"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(value["app"]["commit"].is_string(), "{text}");
    assert!(value["app"]["build_date"].is_string(), "{text}");
    assert_eq!(value["platform"]["os"], std::env::consts::OS);
    assert_eq!(value["drivers"][0]["conn_string_prefix"], "pn532_uart:");
    assert_eq!(value["readers"]["available"], true);
    assert!(
        value["readers"]["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .any(|device| device["conn_string"] == "mock:classic-1k"),
        "{text}"
    );
    assert!(value["hardnested_tables"]["available"].is_boolean());
    for forbidden in ["A0A1A2A3A4A5", "FFFFFFFFFFFF", "\"uid\"", "\"dump\""] {
        assert!(!text.contains(forbidden), "self-check leaked {forbidden}");
    }
}

#[test]
fn card_health_reports_the_saved_image_and_key_coverage() {
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
        "--format",
        "project",
    ]);
    assert_success(&output);

    let output = nfctl(&[
        "--mock",
        "--key-store",
        directory.path().join("keys.json").to_str().expect("path"),
        "card",
        "health",
        "--input",
        dump.to_str().expect("path"),
    ]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("card: MIFARE Classic 1K"), "{text}");
    assert!(text.contains("UID 04 A2 24 8A (4 bytes)"), "{text}");
    assert!(text.contains("ATQA 0004 · SAK 08"), "{text}");
    assert!(text.contains("layout: MIFARE Classic 1K"), "{text}");
    assert!(
        text.contains("completeness: 64/64 blocks · 16/16 sectors · complete"),
        "{text}"
    );
    assert!(text.contains("keys: 16/16 sectors covered"), "{text}");
    assert!(text.contains("dirty blocks: none"), "{text}");
    assert!(text.contains("validation: ok"), "{text}");
}

#[test]
fn key_delete_removes_one_entry_and_keeps_the_rest() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = directory.path().join("keys.json");
    let store = store.to_str().expect("path");
    for (key, sector, kind) in [("A0A1A2A3A4A5", "0", "A"), ("B0B1B2B3B4B5", "1", "B")] {
        let output = nfctl(&[
            "--key-store",
            store,
            "classic",
            "keys",
            "add",
            key,
            "--sector",
            sector,
            "--key-type",
            kind,
        ]);
        assert_success(&output);
    }

    let output = nfctl(&[
        "--key-store",
        store,
        "classic",
        "keys",
        "delete",
        "A0A1A2A3A4A5",
        "--sector",
        "0",
        "--key-type",
        "A",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("deleted A key A0A1A2A3A4A5"),
        "{}",
        stdout(&output)
    );

    let output = nfctl(&["--key-store", store, "classic", "keys", "list", "--builtin"]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(
        !text
            .lines()
            .any(|line| line.contains("A0A1A2A3A4A5") && line.contains("manual")),
        "{text}"
    );
    assert!(text.contains("B0B1B2B3B4B5"), "{text}");
    assert!(text.contains("FFFFFFFFFFFF (common)"), "{text}");

    let output = nfctl(&[
        "--key-store",
        store,
        "classic",
        "keys",
        "delete",
        "FFFFFFFFFFFF",
    ]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("no stored any key FFFFFFFFFFFF"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn mini_reports_its_identity_and_refuses_a_classic_read() {
    let output = nfctl(&["--mock", "--device", "mock:mini", "card", "info"]);
    assert_success(&output);
    let text = stdout(&output);
    assert!(text.contains("type: MIFARE Mini"), "{text}");
    assert!(text.contains("identified only"), "{text}");

    let directory = tempfile::tempdir().expect("temp dir");
    let dump = directory.path().join("mini.mfd");
    let output = nfctl(&[
        "--mock",
        "--device",
        "mock:mini",
        "classic",
        "dump",
        "--output",
        dump.to_str().expect("path"),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("MIFARE Mini"),
        "{}",
        stderr(&output)
    );
    assert!(!dump.exists(), "no dump may be written for a Mini card");
}
