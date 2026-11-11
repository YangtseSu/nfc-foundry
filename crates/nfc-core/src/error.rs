use thiserror::Error;

/// Shared error type across readers, card families, recovery, and application
/// services. CLI exit-code mapping must stay stable.
#[derive(Debug, Error)]
pub enum NfcError {
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("no card is present")]
    NoCard,
    #[error("no reader is selected")]
    NoReader,
    #[error("reader timed out: {0}")]
    Timeout(String),
    #[error("authentication failed")]
    AuthenticationFailed,
    #[error("authentication required: {0}")]
    AuthenticationRequired(String),
    #[error("the card identity changed")]
    CardChanged,
    #[error("verification failed: {0}")]
    VerificationFailed(String),
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
}

pub type Result<T, E = NfcError> = std::result::Result<T, E>;
