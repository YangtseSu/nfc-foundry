//! Shared card identity, errors, and ISO/IEC 14443A primitives.

pub mod cancel;
pub mod card;
pub mod error;
pub mod iso14443a;

pub use cancel::CancelToken;
pub use card::{CardInfo, CardType};
pub use error::{NfcError, Result};
pub use iso14443a::{
    BLOCK_SIZE, KEY_SIZE, Key, KeyType, PAGE_QUAD_SIZE, PAGE_SIZE, TYPE2_VERSION_SIZE,
};
