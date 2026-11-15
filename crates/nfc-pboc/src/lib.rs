//! Read-only ISO 7816-4 public-data adapters for payment and transit cards.
//!
//! The crate implements the China T-Union transit profile and an EMV
//! payment-card public-data profile over ISO-DEP Type A. It parses BER-TLV file
//! control information, selects an application, and reads only public card
//! data. It has no credit, purchase, PIN, balance, transaction-history,
//! top-up, or card-write command.
//!
//! `nfc-transport` stays below this crate: every exchange is an ISO 7816-4
//! command APDU, and the caller chooses the reader. Card workflows and output
//! policy live in `nfc-app`.

pub mod apdu;
pub mod emv;
pub mod fci;
pub mod mock;
pub mod tlv;
pub mod tunion;

pub use emv::{EmvApplication, EmvCard, EmvExpiry, TerminalParameters};
pub use tunion::{
    CardDate, CardDateTime, TUnionCard, TUnionCardType, TUnionTransaction, TUnionTransactionKind,
    TUnionTravel, TUnionTravelKind, format_cny, mask_card_number,
};
