package app

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func recoveredRef(keyID string, sector int, keyType nfc.KeyType, status workflow.RecoveredKeyStatus) workflow.RecoveredKeyRef {
	return workflow.RecoveredKeyRef{KeyID: keyID, Sector: sector, KeyType: keyType, Status: status}
}

func findSaveResult(t *testing.T, recorder *appEventRecorder) RecoverySaveResultDTO {
	t.Helper()
	recorder.mu.Lock()
	defer recorder.mu.Unlock()
	for _, event := range recorder.taskEvents {
		if event.RecoverySave != nil {
			return *event.RecoverySave
		}
	}
	t.Fatalf("recovery save result not found in %+v", recorder.taskEvents)
	return RecoverySaveResultDTO{}
}

type persistedStoreV2 struct {
	Version int `json:"version"`
	Keys    []struct {
		Value string `json:"value"`
		Name  string `json:"name"`
	} `json:"keys"`
	Verifications []struct {
		CardID  string `json:"cardId"`
		Sector  int    `json:"sector"`
		KeyType string `json:"keyType"`
	} `json:"verifications"`
	KeySets []struct {
		Name   string   `json:"name"`
		CardID string   `json:"cardId"`
		KeyIDs []string `json:"keyIds"`
	} `json:"keySets"`
}

func TestReadCardScanFailureStopsBeforeReading(t *testing.T) {
	service, _, fixture, recorder, _ := newClassicWriteAppService(t, "")
	fixture.setAuthError(nfc.ErrNoCard)
	_, err := service.StartReadCard(ReadCardStartRequestDTO{DiscardDirty: true, ScanKeys: true})
	if err != nil {
		t.Fatal(err)
	}
	event := recorder.waitTask(t)
	if event.Type != "failed" {
		t.Fatalf("task event = %+v", event)
	}
	if service.Workbench().Loaded {
		t.Fatal("read continued after the scan failed")
	}
}

func TestReadClearsStaleVerificationForRejectedKey(t *testing.T) {
	service, _, fixture, recorder, _ := newClassicWriteAppService(t, "")
	stale := nfc.Key{0x11, 0x22, 0x33, 0x44, 0x55, 0x66}
	record, _, err := service.keyStore.Add(stale, keys.SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	card := fixture.card.Clone()
	cardID := keys.CardID(card)
	if err := service.keyStore.Verify(cardID, 5, nfc.KeyTypeA, record.ID); err != nil {
		t.Fatal(err)
	}
	fixture.rejectKey(stale)
	_, err = service.StartReadCard(ReadCardStartRequestDTO{DiscardDirty: true})
	if err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	for _, match := range service.keyStore.Verified(cardID) {
		if match.Sector == 5 && match.KeyType == nfc.KeyTypeA {
			t.Fatalf("stale verification survived the read: %+v", match)
		}
	}
	if _, ok := service.keyStore.Record(record.ID); !ok {
		t.Fatal("clearing the verification deleted the key record")
	}
}

func TestSaveRecoveryReviewPersistsReloadsAndVerifies(t *testing.T) {
	service, bindings, fixture, recorder, _ := newClassicWriteAppService(t, "")
	custom := nfc.Key{0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01}
	record, _, err := service.keyStore.Add(custom, keys.SourceAttackEngine)
	if err != nil {
		t.Fatal(err)
	}
	card := fixture.card.Clone()
	cardID := keys.CardID(card)
	if err := service.keyStore.Verify(cardID, 0, nfc.KeyTypeA, record.ID); err != nil {
		t.Fatal(err)
	}
	service.setRecoveryReview(card, []workflow.RecoveredKeyRef{
		recoveredRef(record.ID, 0, nfc.KeyTypeA, workflow.RecoveredKeyNew),
	})
	_, err = bindings.SaveRecoveryReview(RecoverySaveRequestDTO{
		KeySetName: "Cloned access card", KeyNames: map[string]string{record.ID: "Sector 0 Key A"},
		SelectedKeyIDs: []string{record.ID},
	})
	if err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	result := findSaveResult(t, recorder)
	if !result.Reloaded || result.VerifiedSlots != 1 || result.TotalSlots != 1 || len(result.Rejected) != 0 {
		t.Fatalf("save result = %+v", result)
	}
	encoded, err := os.ReadFile(service.keyStorePath)
	if err != nil {
		t.Fatal(err)
	}
	var stored persistedStoreV2
	if err := json.Unmarshal(encoded, &stored); err != nil {
		t.Fatal(err)
	}
	if stored.Version != 2 || len(stored.KeySets) != 1 || stored.KeySets[0].Name != "Cloned access card" {
		t.Fatalf("persisted store = %+v", stored)
	}
	foundName := false
	for _, key := range stored.Keys {
		if key.Name == "Sector 0 Key A" {
			foundName = true
		}
	}
	if !foundName {
		t.Fatalf("key name was not persisted: %+v", stored.Keys)
	}
	if review := service.PendingRecoveryReview(); review != nil {
		t.Fatalf("pending review survived save: %+v", review)
	}
	reloaded, ok := service.keyStore.Record(record.ID)
	if !ok || reloaded.Name != "Sector 0 Key A" {
		t.Fatalf("reloaded key = %+v ok=%v", reloaded, ok)
	}
}

func TestSaveRecoveryReviewUnverifiesRejectedKeys(t *testing.T) {
	service, bindings, fixture, recorder, _ := newClassicWriteAppService(t, "")
	custom := nfc.Key{0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F}
	record, _, err := service.keyStore.Add(custom, keys.SourceAttackEngine)
	if err != nil {
		t.Fatal(err)
	}
	card := fixture.card.Clone()
	cardID := keys.CardID(card)
	if err := service.keyStore.Verify(cardID, 2, nfc.KeyTypeB, record.ID); err != nil {
		t.Fatal(err)
	}
	service.setRecoveryReview(card, []workflow.RecoveredKeyRef{
		recoveredRef(record.ID, 2, nfc.KeyTypeB, workflow.RecoveredKeyNew),
	})
	fixture.rejectKey(custom)
	_, err = bindings.SaveRecoveryReview(RecoverySaveRequestDTO{
		KeySetName: "Rejected set", SelectedKeyIDs: []string{record.ID},
	})
	if err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	result := findSaveResult(t, recorder)
	if result.VerifiedSlots != 0 || len(result.Rejected) != 1 {
		t.Fatalf("save result = %+v", result)
	}
	for _, match := range service.keyStore.Verified(cardID) {
		if match.Sector == 2 && match.KeyType == nfc.KeyTypeB {
			t.Fatalf("rejected verification survived: %+v", match)
		}
	}
	encoded, err := os.ReadFile(service.keyStorePath)
	if err != nil {
		t.Fatal(err)
	}
	if len(encoded) == 0 {
		t.Fatal("corrected store was not written")
	}
}

func TestDiscardRecoveryReviewKeepsPreExistingKeys(t *testing.T) {
	service, _, fixture, _, _ := newClassicWriteAppService(t, "")
	card := fixture.card.Clone()
	cardID := keys.CardID(card)
	existing, _, err := service.keyStore.Add(nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}, keys.SourceBuiltin)
	if err != nil {
		t.Fatal(err)
	}
	if err := service.keyStore.Verify(cardID, 1, nfc.KeyTypeA, existing.ID); err != nil {
		t.Fatal(err)
	}
	fresh := nfc.Key{0x42, 0x42, 0x42, 0x42, 0x42, 0x42}
	record, _, err := service.keyStore.Add(fresh, keys.SourceAttackEngine)
	if err != nil {
		t.Fatal(err)
	}
	if err := service.keyStore.Verify(cardID, 0, nfc.KeyTypeA, record.ID); err != nil {
		t.Fatal(err)
	}
	service.setRecoveryReview(card, []workflow.RecoveredKeyRef{
		recoveredRef(record.ID, 0, nfc.KeyTypeA, workflow.RecoveredKeyNew),
		recoveredRef(existing.ID, 1, nfc.KeyTypeA, workflow.RecoveredKeyExisting),
	})
	if err := service.DiscardRecoveryReview(); err != nil {
		t.Fatal(err)
	}
	if _, ok := service.keyStore.Record(record.ID); ok {
		t.Fatal("discard kept the newly introduced key")
	}
	if _, ok := service.keyStore.Record(existing.ID); !ok {
		t.Fatal("discard removed a pre-existing key")
	}
	for _, match := range service.keyStore.Verified(cardID) {
		if match.Sector == 0 && match.KeyType == nfc.KeyTypeA {
			t.Fatalf("newly introduced verification survived discard: %+v", match)
		}
		if match.Sector == 1 && match.KeyType == nfc.KeyTypeA && match.KeyID != existing.ID {
			t.Fatalf("pre-existing verification changed: %+v", match)
		}
	}
	if _, err := os.Stat(service.keyStorePath); !os.IsNotExist(err) {
		t.Fatalf("discard wrote the key store: %v", err)
	}
}

func TestPendingReviewBlocksRecoveryAndReload(t *testing.T) {
	service, _, fixture, _, _ := newClassicWriteAppService(t, "")
	card := fixture.card.Clone()
	record, _, err := service.keyStore.Add(nfc.Key{0x31, 0x32, 0x33, 0x34, 0x35, 0x36}, keys.SourceAttackEngine)
	if err != nil {
		t.Fatal(err)
	}
	service.setRecoveryReview(card, []workflow.RecoveredKeyRef{
		recoveredRef(record.ID, 0, nfc.KeyTypeA, workflow.RecoveredKeyNew),
	})
	if _, err := service.StartKeyRecovery(KeyRecoveryStartRequestDTO{Authorized: true}); !errors.Is(err, ErrRecoveryReviewPending) {
		t.Fatalf("recovery error = %v", err)
	}
	if _, err := service.ReloadKeyStore(); !errors.Is(err, ErrRecoveryReviewPending) {
		t.Fatalf("reload error = %v", err)
	}
	if _, err := service.AddKey("0A0B0C0D0E0F"); !errors.Is(err, ErrRecoveryReviewPending) {
		t.Fatalf("add key error = %v", err)
	}
	if _, err := service.StartReadCard(ReadCardStartRequestDTO{DiscardDirty: true}); !errors.Is(err, ErrRecoveryReviewPending) {
		t.Fatalf("read error = %v", err)
	}
	if err := service.DiscardRecoveryReview(); err != nil {
		t.Fatal(err)
	}
	if review := service.PendingRecoveryReview(); review != nil {
		t.Fatalf("review not cleared: %+v", review)
	}
}

func TestFinishRecoveryAppliesEachSavePolicy(t *testing.T) {
	store := keys.NewStore()
	recorder := newAppEventRecorder()
	path := filepath.Join(t.TempDir(), "keys.json")
	service := &Service{emitter: recorder, keyStore: store, keyStorePath: path}
	card := nfc.CardInfo{UID: []byte{0x04, 0xAA, 0xBB, 0xCC}, ATQA: [2]byte{0, 4}, SAK: 0x08}
	baseline := workflow.CaptureRecoveryBaseline(store, card)
	value := nfc.Key{0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6}
	record, _, err := store.Add(value, keys.SourceAttackEngine)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Verify(keys.CardID(card), 0, nfc.KeyTypeA, record.ID); err != nil {
		t.Fatal(err)
	}

	if err := service.finishRecovery(card, baseline, RecoverySaveNone, "", "", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("none policy wrote the store: %v", err)
	}
	if service.PendingRecoveryReview() != nil {
		t.Fatal("none policy created a review")
	}

	if err := service.finishRecovery(card, baseline, RecoverySavePersist, "Sector 0 Key A", "Cloned access card", nil); err != nil {
		t.Fatal(err)
	}
	encoded, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var stored persistedStoreV2
	if err := json.Unmarshal(encoded, &stored); err != nil {
		t.Fatal(err)
	}
	if stored.Version != 2 || len(stored.KeySets) != 1 || stored.KeySets[0].Name != "Cloned access card" {
		t.Fatalf("persisted store = %+v", stored)
	}
	if record, ok := store.Record(record.ID); !ok || record.Name != "Sector 0 Key A" {
		t.Fatalf("renamed key = %+v ok=%v", record, ok)
	}

	if err := service.finishRecovery(card, baseline, RecoverySaveReview, "", "", nil); err != nil {
		t.Fatal(err)
	}
	review := service.PendingRecoveryReview()
	if review == nil || len(review.Recovered) != 1 || review.Recovered[0].Status != string(workflow.RecoveredKeyExisting) {
		t.Fatalf("review = %+v", review)
	}
}

func TestMalformedKeyStoreIsNotOverwritten(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "keys.json")
	longName := strings.Repeat("x", 65)
	content := `{"version":2,"keys":[{"value":"010203040506","sources":["user_input"],"name":"` + longName + `"}]}`
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	service := NewServiceWithOptions(Options{Emitter: noopEmitter{}, KeyStorePath: path})
	t.Cleanup(service.Shutdown)
	if _, err := service.AddKey("0A0B0C0D0E0F"); err == nil {
		t.Fatal("mutation succeeded after a failed key-store load")
	}
	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != content {
		t.Fatalf("malformed key store was overwritten:\n%s", after)
	}
	if err := os.WriteFile(path, []byte(`{"version":2,"keys":[]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := service.ReloadKeyStore(); err != nil {
		t.Fatal(err)
	}
	if _, err := service.AddKey("0A0B0C0D0E0F"); err != nil {
		t.Fatalf("mutation failed after a successful reload: %v", err)
	}
}
