//! NTAG PWD_AUTH credentials and the authentication state of one operation.
//!
//! The password is only used to build the PWD_AUTH frame. It is redacted from
//! `Debug`, is never written to an image, and does not outlive the request that
//! carried it.

use std::fmt;

use nfc_core::{CancelToken, NfcError, PAGE_SIZE, Result};
use nfc_transport::Reader;

/// PWD_AUTH credentials for one selected tag.
///
/// The expected PACK is mandatory: a correct password that returns a different
/// PACK means the caller is not talking to the tag it expected.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Type2Auth {
    password: [u8; PAGE_SIZE],
    pack: [u8; 2],
}

impl Type2Auth {
    pub const fn new(password: [u8; PAGE_SIZE], pack: [u8; 2]) -> Self {
        Self { password, pack }
    }

    pub const fn pack(&self) -> [u8; 2] {
        self.pack
    }

    /// Authenticate once and require the configured PACK.
    fn authenticate(&self, reader: &mut dyn Reader, cancel: &CancelToken) -> Result<()> {
        cancel.check()?;
        let pack = reader.password_auth(self.password)?;
        cancel.check()?;
        if pack != self.pack {
            return Err(NfcError::VerificationFailed(
                "PWD_AUTH returned a PACK that does not match the expected value".to_owned(),
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for Type2Auth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Type2Auth")
            .field("password", &"<redacted>")
            .field("pack", &"<redacted>")
            .finish()
    }
}

/// Authentication state for one reader operation.
///
/// The first protected access sends PWD_AUTH; later accesses in the same
/// operation reuse the authenticated session. A new operation, a reselection,
/// or a field reset must use a fresh session.
#[derive(Clone, Copy, Debug, Default)]
pub struct Type2AuthSession<'a> {
    auth: Option<&'a Type2Auth>,
    authenticated: bool,
}

impl<'a> Type2AuthSession<'a> {
    pub const fn none() -> Self {
        Self {
            auth: None,
            authenticated: false,
        }
    }

    pub const fn new(auth: &'a Type2Auth) -> Self {
        Self {
            auth: Some(auth),
            authenticated: false,
        }
    }

    pub fn from_option(auth: Option<&'a Type2Auth>) -> Self {
        Self {
            auth,
            authenticated: false,
        }
    }

    pub const fn has_credentials(&self) -> bool {
        self.auth.is_some()
    }

    pub const fn authenticated(&self) -> bool {
        self.authenticated
    }

    /// Forget the session after a reselection or field loss.
    pub fn reset(&mut self) {
        self.authenticated = false;
    }

    pub fn authenticate(&mut self, reader: &mut dyn Reader, cancel: &CancelToken) -> Result<()> {
        if self.authenticated {
            return Ok(());
        }
        let auth = self.auth.ok_or_else(|| {
            NfcError::AuthenticationRequired(
                "the tag is password protected; supply the Type 2 password and PACK".to_owned(),
            )
        })?;
        auth.authenticate(reader, cancel)?;
        self.authenticated = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nfc_core::{CardInfo, CardType};
    use nfc_transport::DeviceInfo;
    use nfc_transport::mock::MockReader;

    use super::*;

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )
    }

    fn reader() -> MockReader {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:ntag"), Some(card()));
        reader.set_password([1, 2, 3, 4], [0xAA, 0x55]);
        reader
    }

    #[test]
    fn debug_never_contains_the_password_or_pack() {
        let auth = Type2Auth::new([0xDE, 0xAD, 0xBE, 0xEF], [0xCA, 0xFE]);
        let text = format!("{auth:?}");
        assert!(!text.contains("DEADBEEF"), "{text}");
        assert!(!text.contains("CAFE"), "{text}");
        assert!(text.contains("redacted"), "{text}");
    }

    #[test]
    fn authenticates_once_and_checks_the_pack() {
        let mut reader = reader();
        let auth = Type2Auth::new([1, 2, 3, 4], [0xAA, 0x55]);
        let mut session = Type2AuthSession::new(&auth);
        session
            .authenticate(&mut reader, &CancelToken::new())
            .expect("correct password");
        assert!(session.authenticated());
        assert_eq!(reader.password_auths(), 1);
        session
            .authenticate(&mut reader, &CancelToken::new())
            .expect("already authenticated");
        assert_eq!(reader.password_auths(), 1);

        session.reset();
        session
            .authenticate(&mut reader, &CancelToken::new())
            .expect("reauthenticate");
        assert_eq!(reader.password_auths(), 2);
    }

    #[test]
    fn wrong_password_and_pack_fail_without_mutating_anything() {
        let mut reader = reader();
        let wrong_password = Type2Auth::new([9, 9, 9, 9], [0xAA, 0x55]);
        let mut session = Type2AuthSession::new(&wrong_password);
        assert!(matches!(
            session.authenticate(&mut reader, &CancelToken::new()),
            Err(NfcError::AuthenticationFailed)
        ));

        let wrong_pack = Type2Auth::new([1, 2, 3, 4], [0x00, 0x00]);
        let mut session = Type2AuthSession::new(&wrong_pack);
        let error = session
            .authenticate(&mut reader, &CancelToken::new())
            .expect_err("wrong PACK");
        assert!(matches!(error, NfcError::VerificationFailed(_)));
        assert!(!format!("{error}").contains("AA"));
    }

    #[test]
    fn missing_credentials_are_reported_before_io() {
        let mut reader = reader();
        let mut session = Type2AuthSession::none();
        assert!(matches!(
            session.authenticate(&mut reader, &CancelToken::new()),
            Err(NfcError::AuthenticationRequired(_))
        ));
        assert_eq!(reader.password_auths(), 0);
    }
}
