/// The card families supported by this product. Unknown cards are reported
/// explicitly and are never guessed into a supported family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardType {
    #[default]
    Unknown,
    MifareClassic1K,
    MifareClassic4K,
    /// SAK `0x09` identifies a MIFARE Mini. The product reports its identity
    /// only: no card layout, read, or write path is claimed for it.
    MifareMini,
    Type2,
    Iso14443,
}

impl CardType {
    /// Conservative inference from ISO/IEC 14443A selection data. The cascade
    /// bit is ignored; only unambiguous SAK values, the standard Type 2
    /// signature, and the ISO/IEC 14443-4 SAK bit produce a known family.
    pub fn infer(atqa: [u8; 2], sak: u8) -> Self {
        let sak = sak & !0x04;
        match sak {
            0x08 => Self::MifareClassic1K,
            0x09 => Self::MifareMini,
            0x18 => Self::MifareClassic4K,
            0x00 if atqa == [0x00, 0x44] => Self::Type2,
            0x00 => Self::Unknown,
            _ if sak & 0x20 != 0 => Self::Iso14443,
            _ => Self::Unknown,
        }
    }

    /// Stable user-facing family name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown card",
            Self::MifareClassic1K => "MIFARE Classic 1K",
            Self::MifareClassic4K => "MIFARE Classic 4K",
            Self::MifareMini => "MIFARE Mini",
            Self::Type2 => "NFC Forum Type 2",
            Self::Iso14443 => "ISO/IEC 14443A",
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

    /// Stable selection identity used to scope key verifications:
    /// `UID/ATQA/SAK` in uppercase hexadecimal. UID and ATQA are concatenated
    /// without separators; the three fields are separated by `/`.
    pub fn identity_key(&self) -> String {
        let uid = self
            .uid
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();
        format!(
            "{uid}/{:02X}{:02X}/{:02X}",
            self.atqa[0], self.atqa[1], self.sak
        )
    }

    /// Same selection identity: UID, ATQA, and SAK.
    pub fn same_identity(&self, other: &Self) -> bool {
        self.uid == other.uid && self.atqa == other.atqa && self.sak == other.sak
    }

    /// Same ISO-DEP card identity, allowing a random-UID prefix.
    ///
    /// DESFire EV3 and other ISO/IEC 14443-4 cards may randomize the leading
    /// UID bytes on every selection while keeping the last four bytes stable.
    /// Non-ISO-DEP cards still use the strict identity.
    pub fn same_iso_dep_identity(&self, other: &Self) -> bool {
        if self.card_type != CardType::Iso14443 || other.card_type != CardType::Iso14443 {
            return self.same_identity(other);
        }
        if self.atqa != other.atqa || self.sak != other.sak || self.uid.len() != other.uid.len() {
            return false;
        }
        let stable = self.uid.len().min(4);
        self.uid[self.uid.len() - stable..] == other.uid[other.uid.len() - stable..]
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
    fn identifies_mifare_mini_without_claiming_a_layout() {
        for sak in [0x09, 0x0D] {
            assert_eq!(CardType::infer([0x00, 0x04], sak), CardType::MifareMini);
        }
        assert_eq!(CardType::MifareMini.label(), "MIFARE Mini");
    }

    #[test]
    fn infers_type2_only_from_the_standard_signature() {
        assert_eq!(CardType::infer([0x00, 0x44], 0x00), CardType::Type2);
        assert_eq!(CardType::infer([0x44, 0x00], 0x00), CardType::Unknown);
        assert_eq!(CardType::infer([0x00, 0x04], 0x00), CardType::Unknown);
        assert_eq!(CardType::infer([0x00, 0x44], 0x09), CardType::MifareMini);
    }

    #[test]
    fn identity_key_is_stable_across_uid_lengths() {
        let four = CardInfo::new(
            vec![0x04, 0xA2, 0x24, 0x8A],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        );
        assert_eq!(four.identity_key(), "04A2248A/0004/08");

        let seven = CardInfo::new(
            vec![0x04, 0x52, 0xF5, 0x67, 0xD3, 0x2A, 0x81],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        );
        assert_eq!(seven.identity_key(), "0452F567D32A81/0044/00");

        let ten = CardInfo::new(
            vec![0x04, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            [0x00, 0x02],
            0x18,
            CardType::MifareClassic4K,
        );
        assert_eq!(ten.identity_key(), "04010203040506070809/0002/18");
    }

    #[test]
    fn iso_dep_identity_allows_a_random_uid_prefix() {
        let first = CardInfo::new(
            vec![0x04, 0x77, 0x1F, 0xA2, 0xBB, 0x17, 0x90],
            [0x00, 0x44],
            0x20,
            CardType::Iso14443,
        );
        let reselected = CardInfo::new(
            vec![0x04, 0x4D, 0x09, 0xA2, 0xBB, 0x17, 0x90],
            [0x00, 0x44],
            0x20,
            CardType::Iso14443,
        );
        assert!(!first.same_identity(&reselected));
        assert!(first.same_iso_dep_identity(&reselected));

        let different = CardInfo::new(
            vec![0x04, 0x4D, 0x09, 0xA2, 0xBB, 0x17, 0x91],
            [0x00, 0x44],
            0x20,
            CardType::Iso14443,
        );
        assert!(!first.same_iso_dep_identity(&different));

        let classic = CardInfo::new(
            vec![0xFA, 0xB9, 0xCF, 0xF4],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        );
        let other_classic = CardInfo::new(
            vec![0xFA, 0xB9, 0xCF, 0xF5],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        );
        assert!(!classic.same_iso_dep_identity(&other_classic));
    }
}
