//! Process-level runtime: output-mode pre-scan, durations, deadlines, and
//! interrupt handling.

use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nfc_core::CancelToken;

use crate::output::Mode;

/// Reads `--json`/`--jsonl` before clap so usage errors still honor the
/// requested mode. Stops at `--`, where the tokens become positionals.
pub fn prescan_mode(args: &[OsString]) -> Mode {
    let mut mode = Mode::Human;
    for argument in args.iter().skip(1) {
        if argument == "--" {
            break;
        }
        match argument.to_str() {
            Some("--json") => mode = Mode::Json,
            Some("--jsonl") => mode = Mode::JsonLines,
            _ => {}
        }
    }
    mode
}

/// Parses `500ms`, `30s`, `5m`, `1h`, or a sum such as `1m30s`.
pub fn parse_duration(value: &str) -> Result<Duration, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("the duration is empty".to_owned());
    }
    let mut nanos: u128 = 0;
    let mut rest = value;
    let mut saw_part = false;
    while !rest.is_empty() {
        let digits = rest
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits == 0 {
            return Err(format!("expected a number in {value:?}"));
        }
        let amount: u128 = rest[..digits]
            .parse()
            .map_err(|_| format!("invalid number in {value:?}"))?;
        rest = &rest[digits..];
        let (scale, remaining) = if let Some(rest) = rest.strip_prefix("ms") {
            (1_000_000u128, rest)
        } else if let Some(rest) = rest.strip_prefix('s') {
            (1_000_000_000u128, rest)
        } else if let Some(rest) = rest.strip_prefix('m') {
            (60 * 1_000_000_000u128, rest)
        } else if let Some(rest) = rest.strip_prefix('h') {
            (3_600 * 1_000_000_000u128, rest)
        } else {
            return Err(format!(
                "missing or unknown unit in {value:?}; use ms, s, m, or h"
            ));
        };
        nanos = nanos
            .checked_add(amount.saturating_mul(scale))
            .ok_or_else(|| format!("{value:?} is too long"))?;
        rest = remaining;
        saw_part = true;
    }
    if !saw_part {
        return Err(format!("invalid duration {value:?}"));
    }
    let seconds =
        u64::try_from(nanos / 1_000_000_000).map_err(|_| format!("{value:?} is too long"))?;
    let nanos =
        u32::try_from(nanos % 1_000_000_000).map_err(|_| format!("{value:?} is too long"))?;
    let duration = Duration::new(seconds, nanos);
    if duration.is_zero() {
        return Err("the timeout must be greater than zero".to_owned());
    }
    Ok(duration)
}

/// Cancels the operation token when the deadline passes. The watchdog thread
/// is detached: the process exits before it wakes on short commands.
pub fn start_deadline(token: &CancelToken, timeout: Duration) -> Arc<AtomicBool> {
    let expired = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&expired);
    let token = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        flag.store(true, Ordering::Release);
        token.cancel();
    });
    expired
}

/// Installs SIGINT/SIGTERM handling. The first signal cancels the operation
/// token; a second signal exits immediately instead of waiting for a stuck
/// exchange. A failed registration leaves the default dispositions in place.
pub fn install_signal_handler(token: &CancelToken) {
    let mut signals = match signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ]) {
        Ok(signals) => signals,
        Err(_) => return,
    };
    let token = token.clone();
    std::thread::spawn(move || {
        let mut first = true;
        for _signal in signals.forever() {
            if first {
                first = false;
                token.cancel();
            } else {
                std::process::exit(130);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_units_and_sums() {
        assert_eq!(
            parse_duration("500ms").expect("ms"),
            Duration::from_millis(500)
        );
        assert_eq!(parse_duration("30s").expect("s"), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").expect("m"), Duration::from_secs(300));
        assert_eq!(parse_duration("1h").expect("h"), Duration::from_secs(3600));
        assert_eq!(
            parse_duration("1m30s").expect("sum"),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn rejects_missing_units_zero_and_garbage() {
        for value in ["", "10", "abc", "0s", "1x", "-5s", "s"] {
            assert!(parse_duration(value).is_err(), "{value:?}");
        }
    }

    #[test]
    fn prescan_reads_mode_before_the_subcommand_and_stops_at_separator() {
        assert_eq!(prescan_mode(&os(&["nfctl", "--json", "list"])), Mode::Json);
        assert_eq!(
            prescan_mode(&os(&["nfctl", "list", "--jsonl"])),
            Mode::JsonLines
        );
        assert_eq!(
            prescan_mode(&os(&["nfctl", "diff", "--", "--json"])),
            Mode::Human
        );
    }
}
