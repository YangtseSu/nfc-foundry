use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{NfcError, Result};

/// Shared cancellation flag passed through reader operations.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Returns `NfcError::Cancelled` once the token has been cancelled.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(NfcError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared_between_clones() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!token.is_cancelled());
        clone.cancel();
        assert!(token.is_cancelled());
        assert!(matches!(token.check(), Err(NfcError::Cancelled)));
    }
}
