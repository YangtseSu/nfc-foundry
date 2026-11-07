package nfc

import "bytes"

// CardType is a conservative inference from ISO/IEC 14443A selection data.
// It is a hint for the UI, never proof of a card's exact product or capacity.
type CardType string

const (
	CardTypeUnknown         CardType = "unknown"
	CardTypeMIFAREMini      CardType = "mifare_classic_mini"
	CardTypeMIFAREClassic1K CardType = "mifare_classic_1k"
	CardTypeMIFAREClassic4K CardType = "mifare_classic_4k"
	CardTypeType2           CardType = "nfc_type2"
	CardTypeISO14443        CardType = "iso14443_4"
)

// InferCardType recognizes only unambiguous Classic-family SAK values, the
// standard Type 2 selection signature (ATQA 00 44, SAK 00), and the
// ISO/IEC 14443-4 SAK bit. The Type 2 hint is refined to an exact NTAG product
// only after GET_VERSION.
func InferCardType(card CardInfo) CardType {
	sak := card.SAK &^ 0x04 // Ignore the ISO14443A cascade bit if a backend retains it.
	switch sak {
	case 0x09:
		return CardTypeMIFAREMini
	case 0x08:
		return CardTypeMIFAREClassic1K
	case 0x18:
		return CardTypeMIFAREClassic4K
	case 0x00:
		if card.ATQA == [ATQALength]byte{0x00, 0x44} {
			return CardTypeType2
		}
		return CardTypeUnknown
	}
	if sak&0x20 != 0 {
		return CardTypeISO14443
	}
	return CardTypeUnknown
}

// SameCard reports whether two selections identify the same ISO14443A card.
// ATQA and SAK are included so a UID collision cannot silently change the
// operation's assumed card type or capacity.
func SameCard(left, right CardInfo) bool {
	return bytes.Equal(left.UID, right.UID) && left.ATQA == right.ATQA && left.SAK == right.SAK
}
