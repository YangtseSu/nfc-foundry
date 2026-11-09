/// The card families supported by this product. Unknown cards are reported
/// explicitly and are never guessed into a supported family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardType {
    #[default]
    Unknown,
    MifareClassic1K,
    MifareClassic4K,
    Type2,
    Iso14443,
}

/// Facts obtained from card selection. The UID is stored as bytes because
/// Classic 1K/4K may use four-, seven-, or ten-byte UIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardInfo {
    pub uid: Vec<u8>,
    pub atqa: [u8; 2],
    pub sak: u8,
    pub card_type: CardType,
}

impl CardInfo {
    pub fn new(uid: Vec<u8>, atqa: [u8; 2], sak: u8, card_type: CardType) -> Self {
        Self {
            uid,
            atqa,
            sak,
            card_type,
        }
    }

    pub fn uid_hex(&self) -> String {
        self.uid
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Same selection identity: UID, ATQA, and SAK.
    pub fn same_identity(&self, other: &Self) -> bool {
        self.uid == other.uid && self.atqa == other.atqa && self.sak == other.sak
    }
}
