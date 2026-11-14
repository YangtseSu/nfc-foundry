//! Developer raw diagnostics: feature gate, warnings, and exchange output.
//!
//! The default build compiles the "absent" tests; a `developer-tools` build
//! compiles the "present" tests. Both sides run against the scripted mock
//! readers, so neither touches hardware or the user's files.

#[cfg(feature = "developer-tools")]
use std::io::Write as _;
use std::path::Path;
#[cfg(feature = "developer-tools")]
use std::process::Stdio;
use std::process::{Command, Output};

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

    fn command(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nfctl"));
        command.env_clear();
        command.env("HOME", self.path());
        command.env("XDG_CONFIG_HOME", self.path().join("config"));
        command.env("XDG_DATA_HOME", self.path().join("data"));
        command.env("XDG_RUNTIME_DIR", self.path().join("run"));
        command.envs(environment.iter().copied());
        command.args(arguments);
        command
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.run_env(arguments, &[])
    }

    fn run_env(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
        self.command(arguments, environment)
            .output()
            .expect("run nfctl")
    }

    #[cfg(feature = "developer-tools")]
    fn run_stdin(&self, arguments: &[&str], input: &str, environment: &[(&str, &str)]) -> Output {
        let mut child = self
            .command(arguments, environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn nfctl");
        child
            .stdin
            .as_mut()
            .expect("stdin pipe")
            .write_all(input.as_bytes())
            .expect("write console input");
        child.wait_with_output().expect("wait for nfctl")
    }
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exit code")
}

#[cfg(not(feature = "developer-tools"))]
mod without_feature {
    use super::*;

    #[test]
    fn the_developer_commands_are_absent_from_help_and_parsing() {
        let sandbox = Sandbox::new();
        let help = sandbox.run(&["--help"]);
        assert_eq!(code(&help), 0);
        let text = String::from_utf8_lossy(&help.stdout).to_string();
        for name in ["pn532", "apdu"] {
            assert!(
                !text.contains(name),
                "release help advertises `{name}`:\n{text}"
            );
        }

        let shell = sandbox.run(&["--mock", "pn532", "shell"]);
        assert_eq!(code(&shell), 2);
        let apdu = sandbox.run(&["--mock", "apdu", "--raw", "00A4040000"]);
        assert_eq!(code(&apdu), 2);
    }
}

#[cfg(feature = "developer-tools")]
mod with_feature {
    use serde_json::Value;

    use super::*;

    #[test]
    fn the_developer_commands_are_listed_in_help() {
        let sandbox = Sandbox::new();
        let help = sandbox.run(&["--help"]);
        assert_eq!(code(&help), 0);
        let text = String::from_utf8_lossy(&help.stdout).to_string();
        assert!(text.contains("pn532"), "{text}");
        assert!(text.contains("apdu"), "{text}");
    }

    #[test]
    fn the_console_warns_and_keeps_secrets_out_of_the_output() {
        let sandbox = Sandbox::new();
        let output = sandbox.run_stdin(
            &[
                "--mock",
                "--device",
                "mock:empty",
                "--key",
                "A0A1A2A3A4A5",
                "pn532",
                "shell",
            ],
            "02\nhelp\nquit\n",
            &[("NFC_FOUNDRY_TYPE2_PASSWORD", "DEADBEEF")],
        );
        assert_eq!(code(&output), 0, "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(stderr.contains("developer console"), "{stderr}");
        assert!(stderr.contains("bypass card workflows"), "{stderr}");
        assert!(stdout.contains("response: (empty)"), "{stdout}");
        for secret in ["A0A1A2A3A4A5", "DEADBEEF"] {
            assert!(
                !stdout.contains(secret) && !stderr.contains(secret),
                "console output leaked {secret}"
            );
        }
    }

    #[test]
    fn a_raw_apdu_returns_data_and_status_word() {
        let sandbox = Sandbox::new();
        let output = sandbox.run(&[
            "--mock",
            "--device",
            "mock:type4-desfire",
            "apdu",
            "--raw",
            "00A4040007D276000085010100",
            "--json",
        ]);
        assert_eq!(code(&output), 0, "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
        assert_eq!(value["command"], "apdu.raw");
        assert_eq!(value["result"]["sw"], "9000");
        assert_eq!(value["result"]["statusOk"], true);
        let warnings = value["warnings"].as_array().expect("warnings array");
        assert!(
            warnings
                .iter()
                .filter_map(Value::as_str)
                .any(|warning| warning.contains("not a safe write path")),
            "{warnings:?}"
        );
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        assert!(
            !stdout.contains("D276"),
            "the request APDU must not be echoed: {stdout}"
        );
    }

    #[test]
    fn the_console_rejects_machine_output() {
        let sandbox = Sandbox::new();
        let output = sandbox.run(&["--mock", "pn532", "shell", "--json"]);
        assert_eq!(code(&output), 2, "{output:?}");
    }
}
