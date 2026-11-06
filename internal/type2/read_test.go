package type2_test

import (
	"context"
	"encoding/hex"
	"errors"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

func textRecord(text string) []byte {
	payload := append([]byte{0x02, 'e', 'n'}, []byte(text)...)
	return append([]byte{0xD1, 0x01, byte(len(payload)), 'T'}, payload...)
}

func TestReadNTAGProducts(t *testing.T) {
	for _, test := range []struct {
		product  type2.Product
		pages    int
		capacity int
		cc       string
	}{
		{type2.ProductNTAG213, 45, 144, "E1101200"},
		{type2.ProductNTAG215, 135, 496, "E1103E00"},
		{type2.ProductNTAG216, 231, 872, "E1106D00"},
	} {
		uid := []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66}
		tag := mock.NewType2Tag(test.product, uid)
		document, err := type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{})
		if err != nil {
			t.Fatalf("%s read: %v", test.product, err)
		}
		if document.Product != test.product || document.ProductName != strings.ToUpper(string(test.product)) {
			t.Fatalf("%s product name = %q", test.product, document.ProductName)
		}
		if len(document.Pages) != test.pages || document.CC.Capacity != test.capacity || !document.CC.Valid {
			t.Fatalf("%s pages=%d capacity=%d valid=%v", test.product, len(document.Pages), document.CC.Capacity, document.CC.Valid)
		}
		if document.Pages[3] != test.cc {
			t.Fatalf("%s page 3 = %s; want %s", test.product, document.Pages[3], test.cc)
		}
		if document.NDEF != nil && document.NDEF.Present {
			t.Fatalf("%s unexpected NDEF", test.product)
		}
	}
}

func TestReadUltralightFallbackAndFormat(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductUltralight, nil)
	document, err := type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if document.Product != type2.ProductUltralight || document.CC.Valid || document.NDEFError == "" {
		t.Fatalf("fallback document = %+v", document)
	}
	tag.Format()
	document, err = type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if !document.CC.Valid || document.CC.Capacity != 48 || document.NDEF == nil || document.NDEF.Present {
		t.Fatalf("formatted document = %+v", document)
	}
}

func TestReadPasswordAuthentication(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	tag.SetPassword([4]byte{1, 2, 3, 4}, [2]byte{0xAA, 0xBB})
	if _, err := type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{
		Password: []byte{1, 2, 3, 4}, Pack: []byte{0xAA, 0xBB},
	}); err != nil {
		t.Fatal(err)
	}
	_, err := type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{
		Password: []byte{1, 2, 3, 4}, Pack: []byte{0xAA, 0xBC},
	})
	if !errors.Is(err, nfc.ErrAuthenticationFailed) {
		t.Fatalf("wrong PACK error = %v", err)
	}
}

func TestNDEFWritePlanProducesReadableMessage(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	document, err := type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	message := textRecord("hello")
	writes, err := type2.PlanNDEFWrites(document, message)
	if err != nil {
		t.Fatal(err)
	}
	if len(writes) == 0 {
		t.Fatal("NDEF plan is empty")
	}
	session, err := type2.OpenSession(context.Background(), tag)
	if err != nil {
		t.Fatal(err)
	}
	for _, write := range writes {
		data, err := write.Data()
		if err != nil {
			t.Fatal(err)
		}
		if err := session.WritePageVerified(context.Background(), write.Page, data); err != nil {
			t.Fatal(err)
		}
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	document, err = type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if document.NDEF == nil || !document.NDEF.Present || document.NDEF.Text != "hello" {
		t.Fatalf("NDEF after write = %+v (error %q)", document.NDEF, document.NDEFError)
	}
	if document.NDEF.Raw != strings.ToUpper(hex.EncodeToString(message)) {
		t.Fatalf("NDEF raw = %s", document.NDEF.Raw)
	}
}

func TestDocumentRejectsWrongCard(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG215, nil)
	document, err := type2.Read(context.Background(), tag, tag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	other := tag.CardInfo()
	other.UID[6] ^= 0xFF
	if err := document.ValidateForCard(other); !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("wrong-card validation error = %v", err)
	}
}
