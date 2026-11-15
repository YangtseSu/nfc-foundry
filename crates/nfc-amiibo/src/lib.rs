//! Amiibo / NTAG215 cryptography and plaintext model.
//!
//! The crate implements the published Amiibo AES-CTR/HMAC-SHA256 format for
//! user-supplied master keys. It never contains Nintendo key material, Amiibo
//! identity data, or hardware I/O. The application layer loads the keys and
//! decides which tag may be read or written.

mod crypto;
mod identity;
mod keys;
mod tag;

pub use identity::AmiiboIdentity;
pub use keys::{AmiiboKeys, COMBINED_KEY_SIZE, MASTER_KEY_SIZE};
pub use tag::{AmiiboPlain, AmiiboTag, PLAIN_SIZE, TAG_SIZE, UID_SIZE, bcc0, bcc1, pack, unpack};
