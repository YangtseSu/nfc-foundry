package nfc_test

import (
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestInferCardTypeIsConservative(t *testing.T) {
	type2ATQA := [2]byte{0x00, 0x44}
	tests := []struct {
		atqa [2]byte
		sak  byte
		want nfc.CardType
	}{
		{[2]byte{0x00, 0x04}, 0x09, nfc.CardTypeMIFAREMini},
		{[2]byte{0x00, 0x04}, 0x08, nfc.CardTypeMIFAREClassic1K},
		{[2]byte{0x00, 0x02}, 0x18, nfc.CardTypeMIFAREClassic4K},
		{[2]byte{0x00, 0x04}, 0x0c, nfc.CardTypeMIFAREClassic1K}, // cascade bit is not product identity
		{type2ATQA, 0x00, nfc.CardTypeType2},
		{[2]byte{0x00, 0x04}, 0x00, nfc.CardTypeUnknown},
		{type2ATQA, 0x20, nfc.CardTypeUnknown},
	}
	for _, test := range tests {
		if got := nfc.InferCardType(nfc.CardInfo{ATQA: test.atqa, SAK: test.sak}); got != test.want {
			t.Errorf("InferCardType(ATQA=% X, SAK=%02X) = %q; want %q", test.atqa, test.sak, got, test.want)
		}
	}
}

func TestSameCardUsesCompleteSelectionIdentity(t *testing.T) {
	base := nfc.CardInfo{UID: []byte{1, 2, 3, 4}, ATQA: [2]byte{0, 4}, SAK: 0x08}
	if !nfc.SameCard(base, base.Clone()) {
		t.Fatal("identical card selections did not match")
	}
	for _, changed := range []nfc.CardInfo{
		{UID: []byte{1, 2, 3, 5}, ATQA: base.ATQA, SAK: base.SAK},
		{UID: base.UID, ATQA: [2]byte{0, 2}, SAK: base.SAK},
		{UID: base.UID, ATQA: base.ATQA, SAK: 0x18},
	} {
		if nfc.SameCard(base, changed) {
			t.Fatalf("changed card matched base: %+v", changed)
		}
	}
}
