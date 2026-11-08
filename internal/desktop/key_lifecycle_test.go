package desktop

import (
	"testing"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func TestKeysScanBeforeReadIsOptIn(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Keys"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click("Read card"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.readRequests) == 1
	})
	backend.mu.Lock()
	first := backend.readRequests[0]
	backend.mu.Unlock()
	if first.ScanKeys {
		t.Fatal("scan-before-read was enabled by default")
	}

	if err := tester.Click("Scan keys before reading"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click("Read card"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.readRequests) == 2
	})
	backend.mu.Lock()
	second := backend.readRequests[1]
	backend.mu.Unlock()
	if !second.ScanKeys {
		t.Fatal("scan-before-read option was not passed to the read task")
	}
}

func TestRecoveryReviewDialogJoinsCatalogAndSaves(t *testing.T) {
	backend := classicFixture()
	backend.recoveryReview = &app.RecoveryReviewDTO{
		Card: backend.dashboard.Card, CardID: "04112233/0004/08",
		Recovered: []app.RecoveredKeyRefDTO{
			{KeyID: "custom", Sector: 5, KeyType: "A", Status: "new"},
		},
		DefaultKeySetName: "Recovered 04112233 2026-11-08",
	}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if !tester.HasText("Review recovered keys") {
		t.Fatalf("review modal missing: %q", tester.Texts())
	}
	if !tester.HasText("A0 A1 A2 A3 A4 A5") {
		t.Fatalf("review did not join the key value: %q", tester.Texts())
	}
	if !tester.HasText("New") {
		t.Fatalf("review did not show the key status: %q", tester.Texts())
	}
	model.recoveryKeyNames["custom"] = stringPtr("Sector 5 Key A")
	tester.Frame()
	if err := tester.Click("Save and verify"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.savedReviews) == 1
	})
	backend.mu.Lock()
	request := backend.savedReviews[0]
	backend.mu.Unlock()
	if request.KeySetName != "Recovered 04112233 2026-11-08" || len(request.SelectedKeyIDs) != 1 || request.SelectedKeyIDs[0] != "custom" {
		t.Fatalf("save request = %+v", request)
	}
	if request.KeyNames["custom"] != "Sector 5 Key A" {
		t.Fatalf("key name = %q", request.KeyNames["custom"])
	}
}

func TestRecoveryReviewDiscard(t *testing.T) {
	backend := classicFixture()
	backend.recoveryReview = &app.RecoveryReviewDTO{
		Card: backend.dashboard.Card, CardID: "04112233/0004/08",
		Recovered:         []app.RecoveredKeyRefDTO{},
		DefaultKeySetName: "Recovered set",
	}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if err := tester.Click("Discard"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return backend.discardedReviews == 1
	})
}

func TestKeySetsPanelShowsSetsAndReloadsStore(t *testing.T) {
	backend := classicFixture()
	backend.keySets = []app.KeySetDTO{{ID: "set-1", Name: "Cloned access card", KeyCount: 2, CreatedAt: "2026-11-08T15:10:00Z", Source: "recovery"}}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Keys"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Cloned access card") || !tester.HasText("Key sets") {
		t.Fatalf("key set panel missing: %q", tester.Texts())
	}
	if err := tester.Click("Reload key store"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return backend.keyReloads == 1
	})
}

func stringPtr(value string) *string { return &value }

func TestKeyRenameDialogSavesName(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Keys"); err != nil {
		t.Fatal(err)
	}
	model.keyChosen = "custom"
	model.keyChosenRow = 1
	tester.Frame()
	if err := tester.Click("Rename"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Rename key") {
		t.Fatalf("rename dialog missing: %q", tester.Texts())
	}
	model.keyRenameInput = "Sector 5 Key A"
	tester.Frame()
	if err := tester.Click("Save"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return backend.keyRenames["custom"] == "Sector 5 Key A"
	})
}

func TestKeySetCreateDialogUsesVerifiedKeys(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Keys"); err != nil {
		t.Fatal(err)
	}
	if err := tester.Click("Save verified keys"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Save verified keys as a key set") {
		t.Fatalf("key set dialog missing: %q", tester.Texts())
	}
	model.keySetNameInput = "Cloned access card"
	tester.Frame()
	if err := tester.Click("Save"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.keySets) == 1 && backend.keySets[0].Name == "Cloned access card"
	})
}

func TestKeySetExportAndDeleteActions(t *testing.T) {
	backend := classicFixture()
	backend.keySets = []app.KeySetDTO{{ID: "set-1", Name: "Cloned access card", KeyCount: 2, CreatedAt: "2026-11-08T15:10:00Z", Source: "recovery"}}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Keys"); err != nil {
		t.Fatal(err)
	}
	model.keySetExportID = "set-1"
	model.keySetExportOpen = true
	tester.Frame()
	if !tester.HasText("Dictionary (.dic)") {
		t.Fatalf("export dialog missing: %q", tester.Texts())
	}
	model.keySetExportOpen = false
	model.exportKeySet("set-1", "dic")
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.exportedKeySets) == 1 && backend.exportedKeySets[0] == "set-1/dic"
	})

	model.keySetDeleteID = "set-1"
	model.keySetDeleteOpen = true
	tester.Frame()
	if err := tester.Click("Delete key set"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.keySets) == 0
	})
}
