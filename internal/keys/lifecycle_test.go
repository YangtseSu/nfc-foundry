package keys

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func writeStoreFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestVersionOneStoreLoadsAndSavesAsVersionTwo(t *testing.T) {
	key := nfc.Key{1, 2, 3, 4, 5, 6}
	id := keyID(key)
	path := filepath.Join("testdata", "keys-v1.json")
	store := NewStore()
	if err := store.Load(path); err != nil {
		t.Fatal(err)
	}
	if _, ok := store.Record(id); !ok {
		t.Fatal("version 1 key was not loaded")
	}
	if matches := store.Verified("01020304/0400/08"); len(matches) != 1 || matches[0].KeyID != id {
		t.Fatalf("verifications = %+v", matches)
	}
	out := filepath.Join(t.TempDir(), "keys-v2.json")
	if err := store.Save(out); err != nil {
		t.Fatal(err)
	}
	encoded, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	var stored persistedStore
	if err := json.Unmarshal(encoded, &stored); err != nil {
		t.Fatal(err)
	}
	if stored.Version != 2 {
		t.Fatalf("saved version = %d", stored.Version)
	}
	if len(stored.KeySets) != 0 {
		t.Fatalf("unexpected key sets: %+v", stored.KeySets)
	}
}

func TestKeyNamesAndSetsRoundTrip(t *testing.T) {
	store := NewStore()
	cardID := "01020304/0400/08"
	first, _, err := store.Add(nfc.Key{0x11, 0x22, 0x33, 0x44, 0x55, 0x66}, SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	second, _, err := store.Add(nfc.Key{0x66, 0x55, 0x44, 0x33, 0x22, 0x11}, SourceAttackEngine)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.RenameKey(first.ID, "Sector 0 Key A"); err != nil {
		t.Fatal(err)
	}
	builtin, _, err := store.Add(nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}, SourceBuiltin)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.RenameKey(builtin.ID, "Transport key"); err != nil {
		t.Fatal(err)
	}
	set, err := store.CreateKeySet("Cloned access card", cardID, KeySetSourceRecovery)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SetKeySetKeys(set.ID, []string{first.ID, second.ID, first.ID}); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "keys.json")
	if err := store.Save(path); err != nil {
		t.Fatal(err)
	}
	loaded := NewStore()
	report, err := loaded.LoadWithReport(path)
	if err != nil {
		t.Fatal(err)
	}
	if report.Version != 2 || report.Changed() {
		t.Fatalf("load report = %+v", report)
	}
	record, ok := loaded.Record(first.ID)
	if !ok || record.Name != "Sector 0 Key A" {
		t.Fatalf("key name = %+v ok=%v", record, ok)
	}
	builtinRecord, ok := loaded.Record(builtin.ID)
	if !ok || builtinRecord.Name != "Transport key" || !builtinRecord.HasSource(SourceBuiltin) {
		t.Fatalf("builtin name = %+v ok=%v", builtinRecord, ok)
	}
	sets := loaded.KeySets()
	if len(sets) != 1 || sets[0].Name != "Cloned access card" || len(sets[0].KeyIDs) != 2 {
		t.Fatalf("key sets = %+v", sets)
	}
	if containing := loaded.SetsContaining(second.ID); len(containing) != 1 || containing[0].ID != sets[0].ID {
		t.Fatalf("sets containing key = %+v", containing)
	}
}

func TestLoadPrunesMissingSetMembersAndDropsEmptySets(t *testing.T) {
	key := nfc.Key{0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC}
	validID := keyID(key)
	path := filepath.Join(t.TempDir(), "keys.json")
	writeStoreFile(t, path, `{
  "version": 2,
  "keys": [{"value": "123456789ABC", "sources": ["user_input"]}],
  "keySets": [
    {"id": "set-valid", "name": "Partial set", "cardId": "01020304/0400/08", "keyIds": ["`+validID+`", "key-missing"], "createdAt": "2026-01-02T03:04:05Z", "source": "manual"},
    {"id": "set-empty", "name": "Gone set", "cardId": "01020304/0400/08", "keyIds": ["key-missing"], "createdAt": "2026-01-02T03:04:05Z", "source": "manual"}
  ]
}`)
	store := NewStore()
	report, err := store.LoadWithReport(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(report.PrunedKeyIDs) != 1 || report.PrunedKeyIDs[0] != "key-missing" {
		t.Fatalf("pruned = %+v", report.PrunedKeyIDs)
	}
	if len(report.DroppedKeySets) != 1 || report.DroppedKeySets[0] != "set-empty" {
		t.Fatalf("dropped = %+v", report.DroppedKeySets)
	}
	set, ok := store.KeySet("set-valid")
	if !ok || len(set.KeyIDs) != 1 || set.KeyIDs[0] != validID {
		t.Fatalf("partial set = %+v ok=%v", set, ok)
	}
	if _, ok := store.KeySet("set-empty"); ok {
		t.Fatal("empty set was kept")
	}
}

func TestLoadRejectsDuplicateSetIdentity(t *testing.T) {
	for name, content := range map[string]string{
		"duplicate id": `{"version":2,"keys":[],"keySets":[
			{"id":"set-a","name":"One","keyIds":["key-x"],"source":"manual"},
			{"id":"set-a","name":"Two","keyIds":["key-y"],"source":"manual"}]}`,
		"duplicate name": `{"version":2,"keys":[],"keySets":[
			{"id":"set-a","name":"Same","cardId":"card","keyIds":["key-x"],"source":"manual"},
			{"id":"set-b","name":"Same","cardId":"card","keyIds":["key-y"],"source":"manual"}]}`,
	} {
		t.Run(name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "keys.json")
			writeStoreFile(t, path, content)
			if err := NewStore().Load(path); err == nil {
				t.Fatal("duplicate key set identity was accepted")
			}
		})
	}
}

func TestDeleteKeyRemovesSetMembership(t *testing.T) {
	store := NewStore()
	first, _, _ := store.Add(nfc.Key{1, 1, 1, 1, 1, 1}, SourceUserInput)
	second, _, _ := store.Add(nfc.Key{2, 2, 2, 2, 2, 2}, SourceUserInput)
	set, err := store.CreateKeySet("Pair", "", KeySetSourceManual)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SetKeySetKeys(set.ID, []string{first.ID, second.ID}); err != nil {
		t.Fatal(err)
	}
	if err := store.Delete(first.ID); err != nil {
		t.Fatal(err)
	}
	remaining, ok := store.KeySet(set.ID)
	if !ok || len(remaining.KeyIDs) != 1 || remaining.KeyIDs[0] != second.ID {
		t.Fatalf("set after first delete = %+v ok=%v", remaining, ok)
	}
	if err := store.Delete(second.ID); err != nil {
		t.Fatal(err)
	}
	if _, ok := store.KeySet(set.ID); ok {
		t.Fatal("empty key set survived key deletion")
	}
}

func TestReloadReplacesInMemoryState(t *testing.T) {
	store := NewStore()
	path := filepath.Join(t.TempDir(), "keys.json")
	first, _, err := store.Add(nfc.Key{0x10, 0x20, 0x30, 0x40, 0x50, 0x60}, SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Save(path); err != nil {
		t.Fatal(err)
	}
	second, _, err := store.Add(nfc.Key{0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0}, SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.Reload(path); err != nil {
		t.Fatal(err)
	}
	if _, ok := store.Record(first.ID); !ok {
		t.Fatal("reload dropped the persisted key")
	}
	if _, ok := store.Record(second.ID); ok {
		t.Fatal("reload kept an unpersisted key")
	}
}

func TestKeyAndSetNameValidation(t *testing.T) {
	store := NewStore()
	record, _, err := store.Add(nfc.Key{9, 9, 9, 9, 9, 9}, SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.RenameKey(record.ID, strings.Repeat("a", 65)); err == nil {
		t.Fatal("oversized key name was accepted")
	}
	if _, err := store.RenameKey(record.ID, "bad\nname"); err == nil {
		t.Fatal("control character in key name was accepted")
	}
	if _, err := store.CreateKeySet("  ", "", KeySetSourceManual); err == nil {
		t.Fatal("empty key set name was accepted")
	}
	if _, err := store.CreateKeySet("Set", "", KeySetSource("other")); err == nil {
		t.Fatal("unknown key set source was accepted")
	}
	set, err := store.CreateKeySet("Set", "", KeySetSourceManual)
	if err != nil {
		t.Fatal(err)
	}
	other, err := store.CreateKeySet("Other", "", KeySetSourceManual)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.RenameKeySet(other.ID, "Set"); err == nil {
		t.Fatal("duplicate set name was accepted")
	}
	if set.Name != "Set" {
		t.Fatalf("set = %+v", set)
	}
}
