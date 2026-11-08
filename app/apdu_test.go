package app

import (
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestType4WriteRequestVerifiesCardIdentity(t *testing.T) {
	card := nfc.CardInfo{UID: []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66}, ATQA: [2]byte{0x00, 0x44}, SAK: 0x20}
	request := Type4NDEFWriteRequestDTO{
		ExpectedUID: "04 11 22 33 44 55 66", ExpectedATQA: "00 44", ExpectedSAK: "20",
	}
	if err := request.verifyCard(card); err != nil {
		t.Fatalf("matching card rejected: %v", err)
	}
	request.ExpectedUID = "04 11 22 33 44 55 67"
	if err := request.verifyCard(card); !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("changed card error = %v; want ErrCardChanged", err)
	}
	request = Type4NDEFWriteRequestDTO{}
	if err := request.verifyCard(card); err != nil {
		t.Fatalf("empty identity should preserve the CLI path: %v", err)
	}
}
