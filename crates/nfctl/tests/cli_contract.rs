//! End-to-end tests for the `nfctl` machine contract.
//!
//! Every test runs the built binary against the scripted mock readers with an
//! isolated configuration and data home, so no test reads or writes the
//! developer's key store, backups, or hardware.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    directory: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().expect("create sandbox"),
        }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }

    fn command(&self, arguments: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nfctl"));
        self.configure(&mut command);
        command.args(arguments);
        command.output().expect("run nfctl")
    }

    fn spawn(&self, arguments: &[&str]) -> std::process::Child {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nfctl"));
        self.configure(&mut command);
        command
            .args(arguments)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn nfctl")
    }

    fn configure(&self, command: &mut Command) {
        command.env_clear();
        command.env("HOME", self.directory.path());
        command.env("XDG_CONFIG_HOME", self.directory.path().join("config"));
        command.env("XDG_DATA_HOME", self.directory.path().join("data"));
        command.env("XDG_RUNTIME_DIR", self.directory.path().join("run"));
    }

    fn json(&self, arguments: &[&str]) -> (Value, Output) {
        let output = self.command(arguments);
        let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "stdout is not JSON: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (value, output)
    }
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exit code")
}

fn error_code(value: &Value) -> &str {
    value["error"]["code"].as_str().expect("error code")
}

fn assert_envelope(value: &Value) {
    let object = value.as_object().expect("envelope object");
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    assert_eq!(
        keys,
        [
            "card", "command", "device", "error", "ok", "result", "warnings"
        ]
    );
    assert!(value["warnings"].is_array());
}

fn assert_no_key_material(text: &str) {
    for key in ["FFFFFFFFFFFF", "A0A1A2A3A4A5", "D3F7D3F7D3F7"] {
        assert!(
            !text.contains(key),
            "machine output leaked key material {key}: {text}"
        );
    }
}

#[test]
fn list_json_is_a_stable_envelope() {
    let sandbox = Sandbox::new();
    let (value, output) = sandbox.json(&["--mock", "list", "--json"]);
    assert_eq!(code(&output), 0);
    assert_envelope(&value);
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "list");
    assert_eq!(value["device"], Value::Null);
    assert_eq!(value["card"], Value::Null);
    assert_eq!(value["error"], Value::Null);
    let devices = value["result"]["devices"].as_array().expect("devices");
    assert!(!devices.is_empty());
    for device in devices {
        assert!(device["name"].is_string());
        assert!(device["connString"].is_string());
    }
}

#[test]
fn scan_json_reports_card_facts() {
    let sandbox = Sandbox::new();
    let (value, output) =
        sandbox.json(&["--mock", "--device", "mock:classic-1k", "scan", "--json"]);
    assert_eq!(code(&output), 0);
    assert_envelope(&value);
    assert_eq!(value["command"], "scan");
    assert_eq!(value["device"], "mock:classic-1k");
    assert_eq!(value["card"]["uid"], "04A2248A");
    assert_eq!(value["card"]["atqa"], "0004");
    assert_eq!(value["card"]["sak"], "08");
    assert_eq!(value["card"]["type"], "mifare_classic_1k");
    assert_eq!(value["result"]["card"], value["card"]);
}

#[test]
fn classic_read_and_dump_json_are_typed() {
    let sandbox = Sandbox::new();
    let (read, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "read",
        "--block",
        "4",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(read["command"], "classic.read");
    assert_eq!(read["result"]["block"], 4);
    assert_eq!(read["result"]["hex"].as_str().expect("hex").len(), 47);
    assert!(read["result"]["ascii"].is_string());

    let destination = sandbox.path().join("dump.json");
    let (dump, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "dump",
        "--output",
        destination.to_str().expect("utf8 path"),
        "--format",
        "project",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(dump["command"], "classic.dump");
    assert_eq!(dump["result"]["path"], destination.to_str().expect("utf8"));
    assert_eq!(dump["result"]["format"], "project");
    assert_eq!(dump["result"]["blocksReadable"], 64);
    assert_eq!(dump["result"]["sectorsReadable"], 16);
    assert!(destination.exists());
    assert_no_key_material(&String::from_utf8_lossy(&output.stdout));
}

#[test]
fn keyscan_and_key_list_redact_key_values() {
    let sandbox = Sandbox::new();
    let store = sandbox.path().join("keys.json");
    let (scan, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--key-store",
        store.to_str().expect("utf8 path"),
        "classic",
        "keyscan",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(scan["command"], "classic.keyscan");
    assert_eq!(scan["result"]["cancelled"], false);
    assert!(scan["result"]["sectors"][0]["keyA"].is_boolean());
    assert!(scan["result"]["attempts"].as_u64().expect("attempts") > 0);
    assert_no_key_material(&String::from_utf8_lossy(&output.stdout));

    let (added, output) = sandbox.json(&[
        "--mock",
        "--key-store",
        store.to_str().expect("utf8 path"),
        "classic",
        "keys",
        "add",
        "A0A1A2A3A4A5",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(added["result"]["added"], true);

    let (list, output) = sandbox.json(&[
        "--mock",
        "--key-store",
        store.to_str().expect("utf8 path"),
        "classic",
        "keys",
        "list",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    let entries = list["result"]["entries"].as_array().expect("entries");
    assert!(
        entries
            .iter()
            .any(|entry| entry["keyType"] == "any" && entry["source"] == "manual")
    );
    assert_no_key_material(&String::from_utf8_lossy(&output.stdout));

    let export = sandbox.command(&[
        "--mock",
        "--key-store",
        store.to_str().expect("utf8 path"),
        "classic",
        "keys",
        "export",
        "--json",
    ]);
    assert_eq!(code(&export), 2);
    let value: Value = serde_json::from_slice(&export.stdout).expect("json");
    assert_eq!(error_code(&value), "usage");
    assert!(
        value["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--output")
    );
}

#[test]
fn type2_and_ndef_json_are_typed() {
    let sandbox = Sandbox::new();
    let (info, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "type2",
        "info",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(info["command"], "type2.info");
    assert_eq!(info["result"]["product"], "NTAG215");
    assert!(info["result"]["pageCount"].as_u64().expect("pages") > 0);
    assert_eq!(info["result"]["ndef"]["state"], "empty");

    let (read, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "ndef",
        "read",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(read["command"], "ndef.read");
    assert_eq!(read["result"]["ndef"]["state"], "empty");
    assert!(
        read["result"]["ndef"]["capacity"]
            .as_u64()
            .expect("capacity")
            > 0
    );

    let (written, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "ndef",
        "write",
        "--text",
        "hello",
        "--yes",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(written["command"], "ndef.write");
    assert!(written["result"]["written"].as_u64().expect("written") > 0);
    assert!(written["result"]["backup"].is_string());
}

#[test]
fn desfire_info_json_is_typed() {
    let sandbox = Sandbox::new();
    let (info, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:desfire",
        "desfire",
        "info",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(info["command"], "desfire.info");
    assert!(info["result"]["version"]["name"].is_string());
    assert!(info["result"]["applications"].is_array());
}

#[test]
fn recovery_and_diag_json_work() {
    let sandbox = Sandbox::new();
    let (recovery, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "recover",
        "auto",
        "--yes",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(recovery["command"], "recover.auto");
    assert_eq!(recovery["result"]["strategy"], "auto");
    assert!(
        !recovery["result"]["verifiedKeys"]
            .as_array()
            .expect("keys")
            .is_empty()
    );
    assert_no_key_material(&String::from_utf8_lossy(&output.stdout));

    let (diag, output) = sandbox.json(&["--mock", "diag", "--json"]);
    assert_eq!(code(&output), 0);
    assert_eq!(diag["command"], "diag");
    assert!(diag["result"]["app"]["version"].is_string());
    assert!(diag["result"]["drivers"].is_array());
}

#[test]
fn dry_run_is_rejected_for_reads_and_plans_writes() {
    let sandbox = Sandbox::new();
    let (rejected, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "read",
        "--block",
        "4",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code(&output), 2);
    assert_eq!(error_code(&rejected), "usage");
    assert!(
        rejected["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--dry-run")
    );

    let (planned, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "write",
        "--block",
        "4",
        "--data",
        "00000000000000000000000000000000",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(planned["result"]["dryRun"], true);
    assert_eq!(planned["result"]["plan"]["blocks"][0]["block"], 4);
}

#[test]
fn usage_and_hardware_failures_have_stable_codes() {
    let sandbox = Sandbox::new();
    let (conflict, output) = sandbox.json(&["--mock", "--json", "--jsonl", "list"]);
    assert_eq!(code(&output), 2);
    assert_eq!(error_code(&conflict), "usage");

    let (unknown_reader, output) =
        sandbox.json(&["--mock", "--device", "mock:missing", "scan", "--json"]);
    assert_eq!(code(&output), 2);
    assert_eq!(error_code(&unknown_reader), "usage");

    let (malformed, output) = sandbox.json(&["--mock", "--json", "classic", "read"]);
    assert_eq!(code(&output), 2);
    assert_eq!(error_code(&malformed), "usage");

    let (missing_card, output) =
        sandbox.json(&["--mock", "--device", "mock:empty", "scan", "--json"]);
    assert_eq!(code(&output), 4);
    assert_eq!(error_code(&missing_card), "no_card");
}

#[test]
fn timeout_expiry_is_a_deadline_failure() {
    let sandbox = Sandbox::new();
    let output = sandbox.command(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "scan",
        "--watch",
        "--timeout",
        "250ms",
        "--json",
    ]);
    assert_eq!(code(&output), 1);
    let value: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(error_code(&value), "deadline_exceeded");
    assert_eq!(value["error"]["exitCode"], 1);
}

#[test]
fn jsonl_streams_progress_and_one_terminal_result() {
    let sandbox = Sandbox::new();
    let store = sandbox.path().join("keys.json");
    let output = sandbox.command(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "--key-store",
        store.to_str().expect("utf8 path"),
        "classic",
        "keyscan",
        "--jsonl",
    ]);
    assert_eq!(code(&output), 0);
    let lines = String::from_utf8(output.stdout)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json line"))
        .collect::<Vec<_>>();
    assert!(lines.len() > 1);
    let (last, progress) = lines.split_last().expect("final line");
    assert_eq!(last["type"], "result");
    assert_eq!(last["ok"], true);
    for line in progress {
        assert_eq!(line["type"], "progress");
        assert!(line["phase"].is_string());
        assert!(line["message"].is_string());
        assert!(line["elapsed_ms"].is_u64());
    }
    let text = serde_json::to_string(&Value::Array(lines)).expect("encode");
    assert_no_key_material(&text);
}

#[test]
fn signal_cancellation_exits_six() {
    let sandbox = Sandbox::new();
    let child = Command::new(env!("CARGO_BIN_EXE_nfctl"))
        .env_clear()
        .env("HOME", sandbox.path())
        .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
        .env("XDG_DATA_HOME", sandbox.path().join("data"))
        .env("XDG_RUNTIME_DIR", sandbox.path().join("run"))
        .args([
            "--mock",
            "--device",
            "mock:classic-1k",
            "scan",
            "--watch",
            "--jsonl",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn nfctl");
    std::thread::sleep(std::time::Duration::from_millis(700));
    let status = Command::new("sh")
        .args(["-c", &format!("kill -INT {}", child.id())])
        .status()
        .expect("send SIGINT");
    assert!(status.success());
    let output = child.wait_with_output().expect("wait for nfctl");
    assert_eq!(output.status.code(), Some(6));
    let lines = String::from_utf8(output.stdout).expect("utf8");
    let last = lines.lines().last().expect("terminal line");
    let value: Value = serde_json::from_str(last).expect("json line");
    assert_eq!(value["type"], "result");
    assert_eq!(error_code(&value), "cancelled");
    assert_eq!(value["error"]["exitCode"], 6);
}

#[test]
fn a_second_process_cannot_claim_the_same_reader() {
    use std::io::BufRead;

    let sandbox = Sandbox::new();
    let mut holder = sandbox.spawn(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "scan",
        "--watch",
        "--jsonl",
    ]);
    let stdout = holder.stdout.take().expect("holder stdout");
    let mut stdout = std::io::BufReader::new(stdout);
    let mut first_line = String::new();
    stdout
        .read_line(&mut first_line)
        .expect("holder prints its first event");
    assert!(
        first_line.contains("\"type\":\"card\""),
        "holder did not open the reader: {first_line}"
    );

    let (value, output) =
        sandbox.json(&["--mock", "--device", "mock:classic-1k", "scan", "--json"]);
    assert_eq!(code(&output), 4);
    assert_eq!(error_code(&value), "reader_busy");
    assert_eq!(value["error"]["exitCode"], 4);
    let message = value["error"]["message"].as_str().expect("message");
    assert!(message.contains("in use"), "{message}");

    let status = Command::new("sh")
        .args(["-c", &format!("kill -TERM {}", holder.id())])
        .status()
        .expect("stop the holder");
    assert!(status.success());
    assert_eq!(holder.wait().expect("wait for holder").code(), Some(6));

    let (_, output) = sandbox.json(&["--mock", "--device", "mock:classic-1k", "scan", "--json"]);
    assert_eq!(code(&output), 0, "the released reader stays usable");
}

#[test]
fn health_diff_and_classic_ndef_json_are_typed() {
    let sandbox = Sandbox::new();
    let first = sandbox.path().join("first.json");
    let second = sandbox.path().join("second.json");
    for (source, destination) in [("mock:classic-1k", &first), ("mock:classic-1k", &second)] {
        let (_, output) = sandbox.json(&[
            "--mock",
            "--device",
            source,
            "classic",
            "dump",
            "--output",
            destination.to_str().expect("utf8 path"),
            "--format",
            "project",
            "--json",
        ]);
        assert_eq!(code(&output), 0);
    }

    let (health, output) = sandbox.json(&[
        "--mock",
        "card",
        "health",
        "--input",
        first.to_str().expect("utf8 path"),
        "--format",
        "project",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(health["command"], "card.health");
    assert_eq!(health["result"]["layout"], "MIFARE Classic 1K");
    assert_eq!(health["result"]["readableBlocks"], 64);
    assert_eq!(health["result"]["valid"], true);
    assert_eq!(health["card"]["uid"], "04A2248A");

    let (diff, output) = sandbox.json(&[
        "--mock",
        "classic",
        "diff",
        first.to_str().expect("utf8 path"),
        second.to_str().expect("utf8 path"),
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(diff["command"], "classic.diff");
    assert_eq!(diff["result"]["changed"], 0);
    assert!(diff["result"]["equal"].as_u64().expect("equal") > 0);
    assert_eq!(
        diff["result"]["blocks"].as_array().expect("blocks").len(),
        0
    );

    let (ndef, output) = sandbox.json(&[
        "--mock",
        "--device",
        "mock:classic-ndef",
        "classic",
        "ndef",
        "read",
        "--json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(ndef["command"], "classic.ndef.read");
    assert_eq!(ndef["result"]["ndef"]["state"], "message");
    let records = ndef["result"]["ndef"]["records"]
        .as_array()
        .expect("records");
    assert!(!records.is_empty());
    assert!(records[0]["kind"].is_string());
}

#[test]
fn human_output_keeps_stdout_result_and_stderr_diagnostics() {
    let sandbox = Sandbox::new();
    let output = sandbox.command(&[
        "--mock",
        "--device",
        "mock:classic-1k",
        "classic",
        "read",
        "--block",
        "4",
    ]);
    assert_eq!(code(&output), 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("block 4:"));
    assert!(stdout.contains("ascii:"));
    assert!(!stdout.contains("warning:"));
}

#[test]
fn backup_paths_stay_inside_the_sandbox() {
    let sandbox = Sandbox::new();
    let output = sandbox.command(&[
        "--mock",
        "--device",
        "mock:ntag215",
        "ndef",
        "format",
        "--yes",
    ]);
    assert_eq!(code(&output), 0);
    let mut backups = Vec::new();
    collect_files(&sandbox.path().join("data"), &mut backups);
    assert!(!backups.is_empty(), "a backup should be written");
    for path in backups {
        assert!(path.starts_with(sandbox.path()));
    }
}

fn collect_files(directory: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, paths);
        } else {
            paths.push(path);
        }
    }
}
