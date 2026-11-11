//! Embeds the source commit and build date used by the self-check report.
//!
//! `NFC_FOUNDRY_GIT_COMMIT` and `NFC_FOUNDRY_BUILD_DATE` override the values a
//! packaging system supplies explicitly. `SOURCE_DATE_EPOCH` is honoured for
//! reproducible builds. Missing tools or metadata produce `unknown` rather than
//! failing the build.

use std::env;
use std::process::Command;

fn main() {
    for name in [
        "NFC_FOUNDRY_GIT_COMMIT",
        "NFC_FOUNDRY_BUILD_DATE",
        "SOURCE_DATE_EPOCH",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    let commit = env::var("NFC_FOUNDRY_GIT_COMMIT")
        .ok()
        .or_else(|| command("git", &["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    let build_date = env::var("NFC_FOUNDRY_BUILD_DATE")
        .ok()
        .or_else(source_date_epoch)
        .or_else(|| command("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]))
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=NFC_FOUNDRY_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=NFC_FOUNDRY_BUILD_DATE={build_date}");
}

fn source_date_epoch() -> Option<String> {
    let seconds = env::var("SOURCE_DATE_EPOCH").ok()?;
    command(
        "date",
        &["-u", "-d", &format!("@{seconds}"), "+%Y-%m-%dT%H:%M:%SZ"],
    )
    .or(Some(format!("@{seconds}")))
}

fn command(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    if text.is_empty() { None } else { Some(text) }
}
