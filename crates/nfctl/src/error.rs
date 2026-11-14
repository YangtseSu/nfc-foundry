//! Typed CLI errors and the stable exit-code contract.
//!
//! Classification is exhaustive over `NfcError`; no variant is matched by its
//! message text. CLI-side argument mistakes are `Usage` so they always map to
//! exit code 2 instead of borrowing a service error class.

use std::io;

use nfc_core::NfcError;

/// One CLI failure with the class needed for the exit-code table.
#[derive(Debug)]
pub enum CliError {
    /// Bad flags, bad hexadecimal, missing confirmation, unsupported dry run.
    Usage(String),
    /// A typed application or transport failure.
    Service(NfcError),
    /// A validation finding assembled by the CLI from a card or dump check.
    Validation(String),
    /// A broken invariant or a serialization failure in the CLI itself.
    Internal(String),
}

impl CliError {
    /// Wraps a parser or validation failure that is the caller's mistake.
    pub fn usage(error: impl std::fmt::Display) -> Self {
        Self::Usage(error.to_string())
    }

    pub fn message(&self) -> String {
        match self {
            Self::Usage(message) | Self::Validation(message) | Self::Internal(message) => {
                message.clone()
            }
            Self::Service(error) => error.to_string(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl From<NfcError> for CliError {
    fn from(error: NfcError) -> Self {
        Self::Service(error)
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Service(NfcError::Io(error.to_string()))
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Internal(format!("could not encode JSON: {error}"))
    }
}

/// Stable machine code and process exit code for one failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Classification {
    pub code: &'static str,
    pub exit_code: u8,
}

impl Classification {
    const fn new(code: &'static str, exit_code: u8) -> Self {
        Self { code, exit_code }
    }
}

/// Classifies a CLI error. `deadline_expired` splits a cancelled token into an
/// operation timeout (exit 1) so scripts can tell `--timeout` expiry from a
/// user interrupt (exit 6).
pub fn classify(error: &CliError, deadline_expired: bool) -> Classification {
    match error {
        CliError::Usage(_) => Classification::new("usage", 2),
        CliError::Validation(_) => Classification::new("validation", 3),
        CliError::Internal(_) => Classification::new("internal", 7),
        CliError::Service(error) => classify_service(error, deadline_expired),
    }
}

fn classify_service(error: &NfcError, deadline_expired: bool) -> Classification {
    match error {
        NfcError::Unsupported(_) => Classification::new("unsupported", 4),
        NfcError::InvalidArgument(_) => Classification::new("validation", 3),
        NfcError::Io(_) => Classification::new("io", 1),
        NfcError::Cancelled => {
            if deadline_expired {
                Classification::new("deadline_exceeded", 1)
            } else {
                Classification::new("cancelled", 6)
            }
        }
        NfcError::NoCard => Classification::new("no_card", 4),
        NfcError::NoReader => Classification::new("no_reader", 4),
        NfcError::ReaderBusy(_) => Classification::new("reader_busy", 4),
        NfcError::Timeout(_) => Classification::new("timeout", 4),
        NfcError::AuthenticationFailed => Classification::new("authentication", 1),
        NfcError::AuthenticationRequired(_) => Classification::new("authentication_required", 1),
        NfcError::CardChanged => Classification::new("card_changed", 3),
        NfcError::VerificationFailed(_) => Classification::new("verification", 1),
        NfcError::NotImplemented(_) => Classification::new("not_implemented", 7),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classified(error: CliError) -> (&'static str, u8) {
        let classification = classify(&error, false);
        (classification.code, classification.exit_code)
    }

    #[test]
    fn every_nfc_error_variant_has_a_stable_exit_code() {
        let cases = [
            (NfcError::Unsupported("x"), ("unsupported", 4)),
            (NfcError::InvalidArgument("x".into()), ("validation", 3)),
            (NfcError::Io("x".into()), ("io", 1)),
            (NfcError::Cancelled, ("cancelled", 6)),
            (NfcError::NoCard, ("no_card", 4)),
            (NfcError::NoReader, ("no_reader", 4)),
            (NfcError::ReaderBusy("in use".into()), ("reader_busy", 4)),
            (NfcError::Timeout("x".into()), ("timeout", 4)),
            (NfcError::AuthenticationFailed, ("authentication", 1)),
            (
                NfcError::AuthenticationRequired("x".into()),
                ("authentication_required", 1),
            ),
            (NfcError::CardChanged, ("card_changed", 3)),
            (
                NfcError::VerificationFailed("x".into()),
                ("verification", 1),
            ),
            (NfcError::NotImplemented("x"), ("not_implemented", 7)),
        ];
        for (error, expected) in cases {
            assert_eq!(classified(CliError::Service(error)), expected);
        }
    }

    #[test]
    fn usage_and_internal_errors_are_not_service_errors() {
        assert_eq!(classified(CliError::Usage("x".into())), ("usage", 2));
        assert_eq!(
            classified(CliError::Validation("x".into())),
            ("validation", 3)
        );
        assert_eq!(classified(CliError::Internal("x".into())), ("internal", 7));
    }

    #[test]
    fn deadline_expiry_is_an_operation_failure_not_a_user_cancel() {
        let classification = classify(&CliError::Service(NfcError::Cancelled), true);
        assert_eq!(classification.code, "deadline_exceeded");
        assert_eq!(classification.exit_code, 1);
    }
}
