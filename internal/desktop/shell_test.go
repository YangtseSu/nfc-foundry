package desktop

import (
	"fmt"
	"sync"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo/ui"
)

type fakeBackend struct {
	mu          sync.Mutex
	dashboard   app.DashboardDTO
	workbench   app.WorkbenchDTO
	keys        []app.KeyDTO
	connects    []string
	readStarts  int
	scanStarts  int
	type2Starts int
	type2       app.Type2DumpDTO
}

func (f *fakeBackend) GetDashboard() app.DashboardDTO { return f.dashboard }
func (f *fakeBackend) RefreshDevices() ([]app.DeviceDTO, error) {
	return f.dashboard.Devices, nil
}
func (f *fakeBackend) ConnectDevice(connString string) error {
	f.mu.Lock()
	f.connects = append(f.connects, connString)
	f.mu.Unlock()
	return nil
}
func (f *fakeBackend) DisconnectDevice() error     { return nil }
func (f *fakeBackend) GetKeyCatalog() []app.KeyDTO { return f.keys }
func (f *fakeBackend) AddKey(string) ([]app.KeyDTO, error) {
	return f.keys, nil
}
func (f *fakeBackend) DeleteKey(string) ([]app.KeyDTO, error) {
	return f.keys, nil
}
func (f *fakeBackend) ImportKeyDictionary() (app.ImportReportDTO, error) {
	return app.ImportReportDTO{Keys: f.keys}, nil
}
func (f *fakeBackend) ExportKeyDictionary() (app.ExportResultDTO, error) {
	return app.ExportResultDTO{Count: len(f.keys)}, nil
}
func (f *fakeBackend) GetWorkbench() app.WorkbenchDTO { return f.workbench }
func (f *fakeBackend) GetType2Dump() app.Type2DumpDTO { return f.type2 }
func (f *fakeBackend) StartType2Read(string) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2Starts++
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-1", Name: "type2", Status: "running"}, nil
}
func (f *fakeBackend) StartReadCard(bool) (app.TaskDTO, error) {
	f.mu.Lock()
	f.readStarts++
	f.mu.Unlock()
	return app.TaskDTO{ID: "read-1", Name: "read", Status: "running"}, nil
}
func (f *fakeBackend) StartKeyScan(bool) (app.TaskDTO, error) {
	f.mu.Lock()
	f.scanStarts++
	f.mu.Unlock()
	return app.TaskDTO{ID: "keyscan-1", Name: "keyscan", Status: "running"}, nil
}
func (f *fakeBackend) CancelTask(string) error { return nil }

func (f *fakeBackend) connectCalls() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string(nil), f.connects...)
}

func (f *fakeBackend) taskStarts() (int, int) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.readStarts, f.scanStarts
}

func classicFixture() *fakeBackend {
	device := app.DeviceDTO{
		ID: "reader-1", Name: "PN532 UART", ConnString: "pn532_uart:/dev/ttyUSB0",
		Transport: "UART / PN532", Available: true, Nested: true, Darkside: true, Hardnested: true,
	}
	card := app.CardDTO{
		Present: true, UID: "04 A1 B2 C3", UIDLength: 4, ATQA: "00 04", SAK: "08",
		Type: "MIFARE Classic 1K", TypeCode: "mifare_classic_1k", Inferred: true,
	}
	blocks := []app.WorkbenchBlockDTO{
		{Block: 0, Sector: 0, Kind: "manufacturer", Hex: "04 A1 B2 C3 D4 E5 F6 88 04 00 00 00 00 00 00 00", ASCII: "........", Status: "read"},
		{Block: 1, Sector: 0, Kind: "data", Hex: "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F", ASCII: "................", Status: "read"},
		{Block: 2, Sector: 0, Kind: "data", Hex: "AA BB CC DD EE FF 00 11 22 33 44 55 66 77 88 99", ASCII: ".. .#DUfw..", Status: "read"},
		{Block: 3, Sector: 0, Kind: "trailer", Hex: "FF FF FF FF FF FF FF 07 80 69 FF FF FF FF FF FF", ASCII: "........i......", Status: "read",
			Access: &app.AccessSummaryDTO{Valid: true, Groups: []app.AccessGroupDTO{{Group: 0, Code: "000", DataWriteKeys: "A|B", TrailerWriteKeys: "A", KeyBReadable: true}}}},
		{Block: 4, Sector: 1, Kind: "data", Hex: "10 20 30 40 50 60 70 80 90 A0 B0 C0 D0 E0 F0 00", ASCII: "..0@P`p.........", Status: "read"},
		{Block: 5, Sector: 1, Kind: "data", Hex: "01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10", ASCII: "................", Status: "read"},
		{Block: 6, Sector: 1, Kind: "data", Hex: "DE AD BE EF 00 00 00 00 00 00 00 00 00 00 00 00", ASCII: "................", Status: "read"},
		{Block: 7, Sector: 1, Kind: "trailer", Hex: "FF FF FF FF FF FF FF 07 80 69 FF FF FF FF FF FF", ASCII: "........i......", Status: "read"},
	}
	workbench := app.WorkbenchDTO{
		Loaded: true, Layout: "classic_1k", Complete: true, Name: "card.nfcx",
		Blocks: blocks,
		Validation: app.WorkbenchValidationDTO{
			Valid: true, Errors: []string{}, Warnings: []string{},
		},
	}
	keys := []app.KeyDTO{
		{ID: "builtin", Value: "FF FF FF FF FF FF", Builtin: true, Sources: []string{"builtin"}},
		{ID: "custom", Value: "A0 A1 A2 A3 A4 A5", Removable: true, Sources: []string{"user_input"}},
	}
	sectorKeys := []app.SectorKeyDTO{
		{Sector: 0, KeyAStatus: "verified", KeyA: "FF FF FF FF FF FF", KeyAID: "builtin", KeyBStatus: "unknown"},
		{Sector: 1, KeyAStatus: "unknown", KeyBStatus: "unknown"},
	}
	return &fakeBackend{
		dashboard: app.DashboardDTO{
			Devices: []app.DeviceDTO{device}, SelectedDevice: device.ID,
			Connection: app.ConnectionDTO{Status: "polling"},
			Card:       card,
			Keys:       sectorKeys,
		},
		workbench: workbench,
		keys:      keys,
	}
}

func type2Fixture() *fakeBackend {
	device := app.DeviceDTO{
		ID: "reader-1", Name: "PN532 UART", ConnString: "pn532_uart:/dev/ttyUSB0",
		Transport: "UART / PN532", Available: true,
	}
	card := app.CardDTO{
		Present: true, UID: "04 E1 9C 2A 7B 54 80", UIDLength: 7, ATQA: "00 44", SAK: "00",
		Type: "NTAG215", TypeCode: "ntag215", Inferred: true,
	}
	return &fakeBackend{
		dashboard: app.DashboardDTO{
			Devices: []app.DeviceDTO{device}, SelectedDevice: device.ID,
			Connection: app.ConnectionDTO{Status: "polling"},
			Card:       card,
		},
	}
}

func TestShellRendersWorkbenchAndClassicTabs(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)

	if !tester.HasText("PN532 UART") || !tester.HasText("MIFARE Classic 1K") ||
		!tester.HasText("04 A1 B2 C3") || !tester.HasText("Sector 0") ||
		!tester.HasText("00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F") {
		t.Fatalf("workbench texts: %q", tester.Texts())
	}
	if tester.HasText("Memory") || tester.HasText("Protection") {
		t.Fatalf("Classic workbench exposed future tabs: %q", tester.Texts())
	}

	if err := tester.Click("Keys"); err != nil {
		t.Fatal(err)
	}
	if !tester.HasText("Sector keys") || !tester.HasText("Key catalog") ||
		!tester.HasText("A0 A1 A2 A3 A4 A5") {
		t.Fatalf("key view texts: %q", tester.Texts())
	}

	if err := tester.Click("Details"); err != nil {
		t.Fatal(err)
	}
	if !tester.HasText("Card facts") || !tester.HasText("Geometry") ||
		!tester.HasText("classic_1k") {
		t.Fatalf("details texts: %q", tester.Texts())
	}

	model.applyEvent(app.TaskEventName, app.TaskEventDetailDTO{
		TaskID: "read-1", Kind: "read", Type: app.TaskEventProgress,
		Progress: 42, Message: "Reading sector 3",
	})
	tester.Frame()
	if !tester.HasText("Reading sector 3") {
		t.Fatalf("activity did not stay visible: %q", tester.Texts())
	}

	if err := tester.Click("Blocks"); err != nil {
		t.Fatal(err)
	}
	if err := tester.Click("00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if model.blockChosen != 1 {
		t.Fatalf("selected block = %d, want 1", model.blockChosen)
	}
	if !tester.HasText("Block 1 of 8") {
		t.Fatalf("block inspector texts: %q", tester.Texts())
	}

	updated := backend.workbench
	updated.Blocks = append([]app.WorkbenchBlockDTO(nil), backend.workbench.Blocks...)
	updated.Blocks[1].Hex = "DE AD BE EF 00 00 00 00 00 00 00 00 00 00 00 00"
	model.applyEvent(app.WorkbenchEventName, updated)
	tester.Frame()
	if !tester.HasText("DE AD BE EF 00 00 00 00 00 00 00 00 00 00 00 00") {
		t.Fatalf("workbench event did not update the table: %q", tester.Texts())
	}

	model.locale = "zh-Hans"
	tester.Frame()
	if !tester.HasText("卡片工作区") || !tester.HasText("扇区区块") {
		t.Fatalf("Simplified Chinese texts: %q", tester.Texts())
	}
}

func TestType2TabsAreLiveWithoutFakeData(t *testing.T) {
	backend := type2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)

	if model.selectedTab != tabMemory {
		t.Fatalf("selected tab = %q, want memory", model.selectedTab)
	}
	for _, text := range []string{"Memory", "NDEF", "Protection", "Details"} {
		if !tester.HasText(text) {
			t.Fatalf("missing %q in type 2 workbench: %q", text, tester.Texts())
		}
	}
	if tester.HasText("Phase 05") {
		t.Fatalf("type 2 workbench still advertises Phase 05: %q", tester.Texts())
	}
	if !tester.HasText("Read the tag to load Type 2 memory.") {
		t.Fatalf("missing Type 2 read hint: %q", tester.Texts())
	}
	model.selectedTab = tabNDEF
	tester.Frame()
	if model.selectedTab != tabNDEF {
		t.Fatalf("NDEF tab selection = %q", model.selectedTab)
	}
	if !tester.HasText("Read the tag to load Type 2 memory.") {
		t.Fatalf("missing NDEF empty state: %q", tester.Texts())
	}
}

func TestType2ReadActionStartsType2Task(t *testing.T) {
	backend := type2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Read card"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		starts := backend.type2Starts
		backend.mu.Unlock()
		if starts == 1 {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("Read card did not start a Type 2 task")
}

func TestShellConnectsExactReader(t *testing.T) {
	backend := classicFixture()
	backend.dashboard.SelectedDevice = ""
	backend.dashboard.Connection = app.ConnectionDTO{Status: "disconnected"}
	backend.dashboard.Card = app.CardDTO{}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)

	if err := tester.Click("Connect"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for len(backend.connectCalls()) == 0 && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	calls := backend.connectCalls()
	if len(calls) != 1 || calls[0] != "pn532_uart:/dev/ttyUSB0" {
		t.Fatalf("connect calls = %v", calls)
	}
}

func TestClassicActionsStartTasks(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)

	if err := tester.Click("Read card"); err != nil {
		t.Fatal(err)
	}
	waitForTaskStarts(t, backend, 1, 0)
	model.applyEvent(app.TaskEventName, app.TaskEventDetailDTO{
		TaskID: "read-1", Kind: "read", Type: app.TaskEventStarted,
		Message: "Read card started",
	})
	waitForText(t, tester, "Reading the card…")

	model.applyEvent(app.TaskEventName, app.TaskEventDetailDTO{
		TaskID: "read-1", Kind: "read", Type: app.TaskEventCompleted, Progress: 100,
		Message: "Read card completed",
	})
	tester.Frame()

	if err := tester.Click("Scan keys"); err != nil {
		t.Fatal(err)
	}
	waitForTaskStarts(t, backend, 1, 1)
	model.applyEvent(app.TaskEventName, app.TaskEventDetailDTO{
		TaskID: "keyscan-1", Kind: "keyscan", Type: app.TaskEventStarted,
		Message: "Key scan started",
	})
	waitForText(t, tester, "Scanning keys…")
}

func waitForTaskStarts(t *testing.T, backend *fakeBackend, reads, scans int) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		gotReads, gotScans := backend.taskStarts()
		if gotReads >= reads && gotScans >= scans {
			return
		}
		time.Sleep(time.Millisecond)
	}
	gotReads, gotScans := backend.taskStarts()
	t.Fatalf("task starts = %d reads, %d scans; want at least %d, %d", gotReads, gotScans, reads, scans)
}

func waitForText(t *testing.T, tester *ui.Tester, text string) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		tester.Frame()
		if tester.HasText(text) {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatalf("timed out waiting for %q: %q", text, tester.Texts())
}

func TestCatalogTranslationsAreComplete(t *testing.T) {
	en := locales.In("en")
	zh := locales.In("zh-Hans")
	checks := []struct {
		name string
		en   string
		zh   string
	}{
		{"tabBlocks", en.TabBlocks(), zh.TabBlocks()},
		{"keysPanelTitle", en.KeysPanelTitle(), zh.KeysPanelTitle()},
		{"activityTitle", en.ActivityTitle(), zh.ActivityTitle()},
		{"blocksEmpty", en.BlocksEmpty(), zh.BlocksEmpty()},
		{"cardEmpty", en.CardEmpty(), zh.CardEmpty()},
		{"railHealth", en.RailHealth(), zh.RailHealth()},
		{"type2NoData", en.Type2NoData(), zh.Type2NoData()},
		{"type2MemoryPanel", en.Type2MemoryPanel(), zh.Type2MemoryPanel()},
	}
	for _, check := range checks {
		if check.en == "" || check.zh == "" {
			t.Errorf("%s is empty: en=%q zh=%q", check.name, check.en, check.zh)
		}
		if check.en == check.zh {
			t.Errorf("%s was not translated: %q", check.name, check.en)
		}
	}
}

func TestPercent(t *testing.T) {
	if percent(0) != "" || percent(42) != fmt.Sprintf("%d%%", 42) {
		t.Fatalf("percent = %q", percent(42))
	}
}

func TestType2ViewsRenderLoadedDump(t *testing.T) {
	backend := type2Fixture()
	backend.type2 = app.Type2DumpDTO{
		Loaded: true,
		Info: app.Type2InfoDTO{
			Product: "ntag213", ProductName: "NTAG213", UID: "04 E1 9C 2A 7B 54 80",
			TotalPages: 45, UserPages: app.Type2PageRangeDTO{Start: 4, End: 39}, UserBytes: 144,
		},
		Pages: []app.Type2PageDTO{
			{Page: 0, Region: "uid", Hex: "04E19C2A", ASCII: "...."},
			{Page: 3, Region: "capability", Hex: "E1101200", ASCII: "...."},
			{Page: 4, Region: "user", Hex: "D101085402656E68656C6C6F", ASCII: "...."},
		},
		CC:         app.Type2CCDTO{Valid: true, Raw: "E1101200", Capacity: 144, Writable: true},
		Protection: app.Type2ProtectionDTO{StaticLock: "0000", DynamicLock: "000000", Auth0: 255, Access: 0x00},
		NDEF: &app.Type2NDEFDTO{
			Present: true, Length: 12, Raw: "D101085402656E68656C6C6F", Text: "hello",
			Records: []app.Type2NDEFRecordDTO{{TNF: 1, Type: "T", Text: "hello"}},
		},
	}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)

	if !tester.HasText("Page memory") || !tester.HasText("E1 10 12 00") || !tester.HasText("capability") {
		t.Fatalf("memory view texts: %q", tester.Texts())
	}
	if !tester.HasText("NTAG213 · 45 pages · 144 bytes user memory") {
		t.Fatalf("Type 2 subtitle: %q", tester.Texts())
	}
	model.selectedTab = tabNDEF
	tester.Frame()
	if !tester.HasText("hello") || !tester.HasText("NDEF message") {
		t.Fatalf("NDEF view texts: %q", tester.Texts())
	}
	model.selectedTab = tabProtection
	tester.Frame()
	if !tester.HasText("Static lock") || !tester.HasText("CFGLCK") || !tester.HasText("AUTH0") {
		t.Fatalf("protection view texts: %q", tester.Texts())
	}
}
