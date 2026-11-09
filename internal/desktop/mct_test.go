package desktop

import (
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func waitFor(t *testing.T, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if condition() {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("condition was not reached")
}

func TestClassicDiffTabRendersSummaryAndRows(t *testing.T) {
	backend := classicFixture()
	backend.diff = app.DumpDiffDTO{
		Layout: "classic_1k", ChangedBlocks: 1, EqualBlocks: 63, TrailerChanges: 1,
		Before: app.DumpIdentityDTO{UID: "04 A1 B2 C3", Layout: "classic_1k", Complete: true, ReadableBlocks: 64, TotalBlocks: 64},
		After:  app.DumpIdentityDTO{UID: "04 A1 B2 C3", Layout: "classic_1k", Complete: true, ReadableBlocks: 64, TotalBlocks: 64},
		Blocks: []app.DumpBlockDiffDTO{
			{Block: 1, Sector: 0, Kind: "data", State: "changed", BeforeHex: "00000000", AfterHex: "DEADBEEF"},
			{Block: 3, Sector: 0, Kind: "trailer", State: "changed", BeforeHex: "FF", AfterHex: "00",
				Trailer: &app.DumpTrailerDiffDTO{Valid: true, KeyAChanged: true, BeforeKeyA: "FFFFFFFFFFFF", AfterKeyA: "000000000000"}},
		},
	}
	model := newShell(backend)
	model.selectedTab = tabDiff
	model.diffLoaded = true
	model.diff = backend.diff
	model.diffPath = "comparison.json"
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	for _, text := range []string{"Dump comparison", "1 changed", "63 equal", "comparison.json", "DE AD BE EF", "00 00 00 00"} {
		if !tester.HasText(text) {
			t.Fatalf("missing %q in diff view: %q", text, tester.Texts())
		}
	}
	if err := tester.Click("Trailers only"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if tester.HasText("DE AD BE EF") {
		t.Fatalf("data row survived the trailer filter: %q", tester.Texts())
	}
}

func TestClassicValueEditorPreviewsAndStartsBlockWrite(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	model.selectedTab = tabBlocks
	model.blockChosen = 1
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	model.valueInput = "100"
	model.valueAddressInput = "1"
	tester.Frame()
	if err := tester.Click("Preview value"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool { return model.classicDialogOpen && model.classicIntent != nil })
	tester.Frame()
	if !tester.HasText("Write block 1") {
		t.Fatalf("value write confirmation missing: %q", tester.Texts())
	}
	if err := tester.Click("Confirm block write"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.classicWrites) == 1
	})
	backend.mu.Lock()
	defer backend.mu.Unlock()
	if len(backend.valuePreviews) != 1 || backend.valuePreviews[0].Block != 1 {
		t.Fatalf("value previews = %+v", backend.valuePreviews)
	}
	if len(backend.classicPreflights) == 0 || backend.classicPreflights[len(backend.classicPreflights)-1].Mode != "block" {
		t.Fatalf("preflights = %+v", backend.classicPreflights)
	}
}

func TestClassicTrailerEditorRequiresTypedConfirmation(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	model.selectedTab = tabBlocks
	model.blockChosen = 3
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if err := tester.Click("Preview trailer"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool { return model.trailerDialogOpen })
	tester.Frame()
	if !tester.HasText("Write sector trailer 3") {
		t.Fatalf("trailer confirmation missing: %q", tester.Texts())
	}
	model.trailerConfirm = "3"
	tester.Frame()
	if err := tester.Click("Write trailer"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.classicWrites) == 1
	})
	backend.mu.Lock()
	defer backend.mu.Unlock()
	write := backend.classicWrites[0]
	if !write.ConfirmTrailers || write.Confirmation != "3" {
		t.Fatalf("trailer write request = %+v", write)
	}
}

func TestCloneDialogRequiresSourceUID(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if err := tester.Click("Clone card…"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool { return model.cloneOpen })
	tester.Frame()
	if !tester.HasText("Clone card") || !tester.HasText("Type the source UID 04 A1 B2 C3 to confirm.") {
		t.Fatalf("clone dialog missing: %q", tester.Texts())
	}
	model.cloneConfirm = "04 A1 B2 C3"
	tester.Frame()
	if err := tester.Click("Clone"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.cloneStarts) == 1
	})
	backend.mu.Lock()
	defer backend.mu.Unlock()
	if backend.cloneStarts[0].Confirmation != "04A1B2C3" {
		t.Fatalf("clone start = %+v", backend.cloneStarts[0])
	}
}

func TestFactoryDialogRequiresFORMAT(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if err := tester.Click("Factory format"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool { return model.factoryOpen && model.factoryPlan != nil })
	tester.Frame()
	if !tester.HasText("Sets every sector trailer to Key A/B FFFFFFFFFFFF and access bytes FF 07 80. Block 0 is not written.") {
		t.Fatalf("factory dialog missing: %q", tester.Texts())
	}
	model.factoryConfirm = "FORMAT"
	tester.Frame()
	if err := tester.Click("Run format"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.factoryStarts) == 1
	})
}

func TestKeyFileEditorShowsLineNumbersAndProblems(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.selectedTab = tabKeys
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if err := tester.Click("Edit key file…"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Key file editor") {
		t.Fatalf("key file editor missing: %q", tester.Texts())
	}
	if err := tester.Click("Open…"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool { return len(model.keyFileLines) == 1 })
	tester.Frame()
	if !tester.HasText("Line") || !tester.HasText("Key") || !tester.HasText("valid") {
		t.Fatalf("key file lines missing: %q", tester.Texts())
	}
}

func TestPartialReadBannerOffersScanAndReadAgain(t *testing.T) {
	backend := classicFixture()
	backend.workbench.Complete = false
	backend.workbench.ReadableBlocks = 60
	backend.workbench.TotalBlocks = 64
	backend.workbench.ReadableSectors = 14
	backend.workbench.TotalSectors = 16
	backend.workbench.SkippedSectors = []int{3, 7}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if !tester.HasText("60 of 64 blocks readable · 2 sectors skipped · Skipped sectors: 3, 7") {
		t.Fatalf("partial read banner missing: %q", tester.Texts())
	}
	if err := tester.Click("Scan keys and read again"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.readRequests) == 1
	})
	backend.mu.Lock()
	defer backend.mu.Unlock()
	if !backend.readRequests[0].ScanKeys || !backend.readRequests[0].DiscardDirty {
		t.Fatalf("read-again request = %+v", backend.readRequests[0])
	}
}
