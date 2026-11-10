//! Standard NDEF record templates.

pub mod bluetooth;
pub mod dpp;
pub mod smart_poster;
pub mod text;
pub mod uri;
pub mod vcard;
pub mod wifi;

pub use bluetooth::{BluetoothLeRecord, LeAddressType, LeRole};
pub use dpp::DppBootstrap;
pub use smart_poster::{SmartPoster, SmartPosterAction};
pub use text::{TextEncoding, TextRecord};
pub use uri::UriRecord;
pub use vcard::VCard;
pub use wifi::{WifiAuthentication, WifiEncryption, WifiRecord};
