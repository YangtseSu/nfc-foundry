package app

import (
	"path/filepath"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func TestLoadDumpDoesNotVerifyMetadataKeysOnCurrentCard(t *testing.T) {
	service, _, fixture, _, _ := newClassicWriteAppService(t, "")
	card := fixture.card.Clone()
	cardID := keys.CardID(card)
	original := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	record, _, err := service.keyStore.Add(original, keys.SourceReadContext)
	if err != nil {
		t.Fatal(err)
	}
	if err := service.keyStore.Verify(cardID, 1, nfc.KeyTypeB, record.ID); err != nil {
		t.Fatal(err)
	}
	result := fixture.dumpResult(t, service)
	secret := nfc.Key{0x11, 0x22, 0x33, 0x44, 0x55, 0x66}
	result.Keys[1].KeyB = &secret
	path := filepath.Join(t.TempDir(), "dump.nfcx.json")
	if err := workflow.SaveDumpProject(path, result); err != nil {
		t.Fatal(err)
	}
	if _, err := service.LoadDumpFile(path, true); err != nil {
		t.Fatal(err)
	}
	verified := verifiedSectorKeys(service.keyStore, card, mifare.Classic1K)
	if verified[1].KeyB == nil || *verified[1].KeyB != original {
		t.Fatalf("sector 1 Key B verification = %v; want the authenticated FF key", verified[1].KeyB)
	}
}
