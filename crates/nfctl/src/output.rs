//! Output modes and the stable JSON envelope.
//!
//! Human results and machine results are produced by the same command handler:
//! [`Output::line`] and [`Output::inline`] are muted outside human mode, and
//! warnings are collected for the envelope. stdout carries only the requested
//! result; diagnostics always go to stderr.

use std::fmt;
use std::io::{self, Write as _};

use nfc_core::{CardInfo, CardType};
use serde_json::{Value, json};

use crate::error::{Classification, CliError, classify};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Human,
    Json,
    JsonLines,
}

impl Mode {
    pub fn is_human(self) -> bool {
        self == Self::Human
    }
}

/// Per-invocation output state. The command name, selected device, and card
/// travel into the envelope; handlers set them as soon as the facts exist.
pub struct Output {
    mode: Mode,
    verbose: bool,
    command: Option<&'static str>,
    device: Option<String>,
    card: Option<Value>,
    warnings: Vec<String>,
}

impl Output {
    pub fn new(mode: Mode, verbose: bool) -> Self {
        Self {
            mode,
            verbose,
            command: None,
            device: None,
            card: None,
            warnings: Vec::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Names the command for the envelope. Safe to call more than once; the
    /// innermost handler is the most specific name.
    pub fn set_command(&mut self, command: &'static str) {
        self.command = Some(command);
    }

    pub fn set_device(&mut self, conn_string: impl Into<String>) {
        self.device = Some(conn_string.into());
    }

    pub fn set_card(&mut self, card: &CardInfo) {
        self.card = Some(card_facts(card));
    }

    /// Human-readable result line on stdout. Suppressed for JSON modes.
    pub fn line(&self, args: fmt::Arguments<'_>) {
        if self.mode.is_human() {
            let mut stdout = io::stdout().lock();
            let _ = stdout.write_fmt(args);
            let _ = stdout.write_all(b"\n");
        }
    }

    /// Human-readable result fragment without a trailing newline.
    pub fn inline(&self, args: fmt::Arguments<'_>) {
        if self.mode.is_human() {
            let _ = io::stdout().lock().write_fmt(args);
        }
    }

    /// Warning: printed on stderr in human mode, collected in JSON modes.
    pub fn warn(&mut self, args: fmt::Arguments<'_>) {
        match self.mode {
            Mode::Human => {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_all(b"warning: ");
                let _ = stderr.write_fmt(args);
                let _ = stderr.write_all(b"\n");
            }
            Mode::Json | Mode::JsonLines => self.warnings.push(args.to_string()),
        }
    }

    /// Diagnostic note: stderr in every mode, never part of the envelope.
    pub fn note(&self, args: fmt::Arguments<'_>) {
        let mut stderr = io::stderr().lock();
        let _ = stderr.write_fmt(args);
        let _ = stderr.write_all(b"\n");
    }

    /// Reader and driver detail, only with `--verbose`.
    pub fn verbose(&self, args: fmt::Arguments<'_>) {
        if self.verbose {
            let mut stderr = io::stderr().lock();
            let _ = stderr.write_fmt(args);
            let _ = stderr.write_all(b"\n");
        }
    }

    /// One JSONL event on stdout. Only `--jsonl` writes it.
    pub fn event(&self, event: Value) {
        emit_event(self.mode, event);
    }

    /// Prints the success result and returns the process exit code.
    pub fn finish(self, result: Value) -> u8 {
        if self.mode != Mode::Human {
            self.write_envelope(self.envelope(true, Some(result), None));
        }
        0
    }

    /// Prints the failure and returns the classified process exit code.
    pub fn fail(self, error: &CliError, deadline_expired: bool) -> u8 {
        let classification = classify(error, deadline_expired);
        self.emit_failure(error, classification)
    }

    fn emit_failure(self, error: &CliError, classification: Classification) -> u8 {
        let message = error.message();
        match self.mode {
            Mode::Human => {
                let mut stderr = io::stderr().lock();
                let _ = writeln!(stderr, "error: {message}");
            }
            Mode::Json | Mode::JsonLines => {
                let error = json!({
                    "code": classification.code,
                    "message": message,
                    "exitCode": classification.exit_code,
                });
                let envelope = self.envelope(false, None, Some(error));
                self.write_envelope(envelope);
            }
        }
        classification.exit_code
    }

    fn write_envelope(&self, envelope: Value) {
        let mut stdout = io::stdout().lock();
        match self.mode {
            Mode::Human => {}
            Mode::Json => {
                let _ = serde_json::to_writer_pretty(&mut stdout, &envelope);
                let _ = stdout.write_all(b"\n");
            }
            Mode::JsonLines => {
                let mut line = serde_json::Map::new();
                line.insert("type".to_owned(), Value::String("result".to_owned()));
                if let Value::Object(fields) = envelope {
                    line.extend(fields);
                }
                let _ = serde_json::to_writer(&mut stdout, &Value::Object(line));
                let _ = stdout.write_all(b"\n");
            }
        }
    }

    fn envelope(&self, ok: bool, result: Option<Value>, error: Option<Value>) -> Value {
        json!({
            "ok": ok,
            "command": self.command,
            "device": self.device,
            "card": self.card,
            "warnings": self.warnings,
            "error": error,
            "result": result,
        })
    }
}

/// Compact JSONL event for progress sinks that cannot borrow [`Output`].
pub fn emit_event(mode: Mode, event: Value) {
    if mode == Mode::JsonLines {
        let mut stdout = io::stdout().lock();
        let _ = serde_json::to_writer(&mut stdout, &event);
        let _ = stdout.write_all(b"\n");
    }
}

/// Stable card facts used by the envelope and by `scan` results.
pub fn card_facts(card: &CardInfo) -> Value {
    json!({
        "uid": compact_hex(&card.uid),
        "atqa": format!("{:02X}{:02X}", card.atqa[0], card.atqa[1]),
        "sak": format!("{:02X}", card.sak),
        "type": card_type_code(card.card_type),
    })
}

pub fn card_type_code(card_type: CardType) -> &'static str {
    match card_type {
        CardType::Unknown => "unknown",
        CardType::MifareClassic1K => "mifare_classic_1k",
        CardType::MifareClassic4K => "mifare_classic_4k",
        CardType::MifareMini => "mifare_mini",
        CardType::Type2 => "type2",
        CardType::Iso14443 => "iso14443a",
    }
}

pub fn compact_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}
