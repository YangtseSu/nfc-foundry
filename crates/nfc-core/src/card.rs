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

impl CardType {
    /// Conservative inference from ISO/IEC 14443A selection data. The cascade
    /// bit is ignored; only unambiguous Classic SAK values, the standard Type 2
    /// signature, and the ISO/IEC 14443-4 SAK bit produce a known family.
    pub fn infer(atqa: [u8; 2], sak: u8) -> Self {
        let sak = sak & !0x04;
        match sak {
            0x08 => Self::MifareClassic1K,
            0x18 => Self::MifareClassic4K,
            0x00 if atqa == [0x00, 0x44] => Self::Type2,
            0x00 => Self::Unknown,
            _ if sak & 0x20 != 0 => Self::Iso14443,
            _ => Self::Unknown,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_classic_and_iso14443_families() {
        assert_eq!(
            CardType::infer([0x00, 0x04], 0x08),
            CardType::MifareClassic1K
        );
        assert_eq!(
            CardType::infer([0x00, 0x04], 0x0C),
            CardType::MifareClassic1K
        );
        assert_eq!(
            CardType::infer([0x00, 0x02], 0x18),
            CardType::MifareClassic4K
        );
        assert_eq!(CardType::infer([0x00, 0x44], 0x20), CardType::Iso14443);
        assert_eq!(CardType::infer([0x00, 0x04], 0x28), CardType::Iso14443);
    }

    #[test]
    fn infers_type2_only_from_the_standard_signature() {
        assert_eq!(CardType::infer([0x00, 0x44], 0x00), CardType::Type2);
        assert_eq!(CardType::infer([0x44, 0x00], 0x00), CardType::Unknown);
        assert_eq!(CardType::infer([0x00, 0x04], 0x00), CardType::Unknown);
        assert_eq!(CardType::infer([0x00, 0x44], 0x09), CardType::Unknown);
    }
}
