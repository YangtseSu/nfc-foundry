package desktop

import (
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo/ui"
)

type fakeBackend struct {
	mu                sync.Mutex
	dashboard         app.DashboardDTO
	workbench         app.WorkbenchDTO
	keys              []app.KeyDTO
	connects          []string
	readStarts        int
	scanStarts        int
	type2Starts       int
	type2ReadPassword string
	type2Writes       []app.Type2WriteStartRequestDTO
	type2NDEFWrites   []app.Type2NDEFWriteStartRequestDTO
	type2RawWrites    []app.Type2NDEFRawRequestDTO
	type2Records      []app.Type2NDEFRecordsRequestDTO
	type2Plans        []app.Type2LifecycleRequestDTO
	type2Lifecycles   []app.Type2LifecycleRequestDTO
	type2ProtPlans    []app.Type2ProtectionWriteRequestDTO
	type2ProtWrites   []app.Type2ProtectionWriteRequestDTO
	type2Source       app.Type2SourceDTO
	importedNDEF      *app.Type2NDEFImportDTO
	exportedNDEF      string
	type2             app.Type2DumpDTO
	classicEdits      []classicEditCall
	classicPreflights []app.WritePreflightRequestDTO
	classicWrites     []app.WriteStartRequestDTO
	uidPreflights     []string
	uidWrites         []app.UIDWriteStartRequestDTO
	dumpLoads         int
	dumpSaves         int
}

type classicEditCall struct {
	Block int
	Hex   string
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
func (f *fakeBackend) GetWorkbench() app.WorkbenchDTO {
	f.mu.Lock()
	defer f.mu.Unlock()
	return cloneWorkbenchDTO(f.workbench)
}
func (f *fakeBackend) EditBlockHex(block int, hex string) (app.WorkbenchDTO, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.classicEdits = append(f.classicEdits, classicEditCall{Block: block, Hex: hex})
	workbench := cloneWorkbenchDTO(f.workbench)
	if block >= 0 && block < len(workbench.Blocks) {
		workbench.Blocks[block].Hex = testSpacedHex(hex)
		workbench.Blocks[block].Dirty = true
		workbench.Dirty = true
		workbench.DirtyBlocks = append(workbench.DirtyBlocks, block)
	}
	f.workbench = workbench
	return cloneWorkbenchDTO(workbench), nil
}
func (f *fakeBackend) LoadDump(bool) (app.WorkbenchDTO, error) {
	f.mu.Lock()
	f.dumpLoads++
	f.mu.Unlock()
	return f.workbench, nil
}
func (f *fakeBackend) LoadDumpFile(string, bool) (app.WorkbenchDTO, error) {
	f.mu.Lock()
	f.dumpLoads++
	f.mu.Unlock()
	return f.workbench, nil
}
func (f *fakeBackend) SaveDump() (app.SaveResultDTO, error) {
	f.mu.Lock()
	f.dumpSaves++
	f.mu.Unlock()
	return app.SaveResultDTO{Path: "card.nfcx.json"}, nil
}
func (f *fakeBackend) SaveDumpFile(path string) (app.SaveResultDTO, error) {
	f.mu.Lock()
	f.dumpSaves++
	f.mu.Unlock()
	return app.SaveResultDTO{Path: path}, nil
}
func (f *fakeBackend) PreflightWrite(request app.WritePreflightRequestDTO) (app.WritePreflightDTO, error) {
	f.mu.Lock()
	f.classicPreflights = append(f.classicPreflights, request)
	f.mu.Unlock()
	result := app.WritePreflightDTO{Mode: request.Mode, Valid: true, SkippedBlock0: true}
	switch request.Mode {
	case "block":
		result.Token = "write-block"
		result.Blocks = append([]int(nil), request.Blocks...)
	case "changes":
		result.Token = "write-changes"
		result.Blocks = []int{1, 2}
		result.TrailerBlocks = []int{3}
		result.BackupPath = "/tmp/classic-backup.json"
		result.Access = []app.TrailerAccessDTO{{Block: 3, Sector: 0}}
	case "full":
		result.Token = "write-full"
		result.Blocks = []int{1, 2, 3, 4, 5, 6, 7}
		result.TrailerBlocks = []int{3, 7}
		result.BackupPath = "/tmp/classic-backup.json"
		result.Access = []app.TrailerAccessDTO{{Block: 3, Sector: 0}, {Block: 7, Sector: 1}}
	}
	return result, nil
}
func (f *fakeBackend) StartWrite(request app.WriteStartRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.classicWrites = append(f.classicWrites, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "write-1", Name: "write", Status: "running"}, nil
}
func (f *fakeBackend) PreflightUIDWrite(uid string) (app.UIDWritePreflightDTO, error) {
	f.mu.Lock()
	f.uidPreflights = append(f.uidPreflights, uid)
	f.mu.Unlock()
	return app.UIDWritePreflightDTO{
		Token: "uid-token", Valid: true, BackupPath: "/tmp/uid-backup.json",
		Preview: &app.UIDWritePreviewDTO{
			CurrentUID: "04 A1 B2 C3", NewUID: "04 AA BB CC", BCC: "00",
			OldBlock0: "04 A1 B2 C3 00 00 00 00 00 00 00 00 00 00 00 00",
			NewBlock0: "04 AA BB CC 00 00 00 00 00 00 00 00 00 00 00 00",
		},
	}, nil
}
func (f *fakeBackend) StartUIDWrite(request app.UIDWriteStartRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.uidWrites = append(f.uidWrites, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "uid-1", Name: "uid", Status: "running"}, nil
}
func (f *fakeBackend) GetType2Dump() app.Type2DumpDTO { return f.type2 }
func (f *fakeBackend) StartType2Read(password string) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2Starts++
	f.type2ReadPassword = password
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-1", Name: "type2", Status: "running"}, nil
}
func (f *fakeBackend) StartType2Write(request app.Type2WriteStartRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2Writes = append(f.type2Writes, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-write-1", Name: "write", Status: "running"}, nil
}
func (f *fakeBackend) StartType2NDEFWrite(request app.Type2NDEFWriteStartRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2NDEFWrites = append(f.type2NDEFWrites, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-ndef-1", Name: "ndef", Status: "running"}, nil
}
func (f *fakeBackend) PreviewType2NDEF(request app.Type2NDEFWriteStartRequestDTO) (app.Type2NDEFPreviewDTO, error) {
	return app.Type2NDEFPreviewDTO{Template: request.Template, Length: 16}, nil
}
func (f *fakeBackend) GetType2NDEFTemplates() []app.Type2NDEFTemplateDTO {
	return app.Type2NDEFTemplates()
}
func (f *fakeBackend) GetType2NDEFProviders() map[string][]app.Type2ProviderDTO {
	return app.Type2NDEFProviders()
}
func (f *fakeBackend) PreviewType2NDEFRaw(request app.Type2NDEFRawRequestDTO) (app.Type2NDEFPreviewDTO, error) {
	return app.Type2NDEFPreviewDTO{Template: "raw", Length: len(request.Raw) / 2}, nil
}
func (f *fakeBackend) StartType2NDEFRawWrite(request app.Type2NDEFRawRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2RawWrites = append(f.type2RawWrites, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-raw-1", Name: "ndef", Status: "running"}, nil
}
func (f *fakeBackend) PreviewType2NDEFRecords(request app.Type2NDEFRecordsRequestDTO) (app.Type2NDEFPreviewDTO, error) {
	return app.Type2NDEFPreviewDTO{Template: "records", Length: len(request.Records) * 8}, nil
}
func (f *fakeBackend) StartType2NDEFRecordsWrite(request app.Type2NDEFRecordsRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2Records = append(f.type2Records, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-records-1", Name: "ndef", Status: "running"}, nil
}
func (f *fakeBackend) ImportType2NDEF() (app.Type2NDEFImportDTO, error) {
	if f.importedNDEF == nil {
		return app.Type2NDEFImportDTO{Path: "message.ndef", Length: 8, Raw: "D1010454657374"}, nil
	}
	return *f.importedNDEF, nil
}
func (f *fakeBackend) ExportType2NDEF(raw string) (string, error) {
	f.mu.Lock()
	f.exportedNDEF = raw
	f.mu.Unlock()
	return "message.ndef", nil
}
func (f *fakeBackend) LoadType2Source() (app.Type2SourceDTO, error) {
	return f.type2Source, nil
}
func (f *fakeBackend) GetType2Source() app.Type2SourceDTO { return f.type2Source }
func (f *fakeBackend) ExportType2Dump() (string, error)   { return "type2.bin", nil }
func (f *fakeBackend) ImportType2Dump() (app.Type2DumpDTO, error) {
	return f.type2, nil
}
func (f *fakeBackend) PreviewType2Lifecycle(request app.Type2LifecycleRequestDTO) (app.Type2LifecyclePlanDTO, error) {
	f.mu.Lock()
	f.type2Plans = append(f.type2Plans, request)
	f.mu.Unlock()
	return app.Type2LifecyclePlanDTO{
		Operation: request.Operation, CardUID: "04 A1 B2 C3 D4 E5 F6", ProductName: "NTAG215",
		Changes: []app.Type2ChangeDTO{{Target: "password", Old: "set", New: "FF FF FF FF", OneWay: true}},
		Writes:  []app.Type2PageWriteDTO{{Page: 43, Hex: "FFFFFFFF"}}, Token: "type2-plan-token",
	}, nil
}
func (f *fakeBackend) StartType2Lifecycle(request app.Type2LifecycleRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2Lifecycles = append(f.type2Lifecycles, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-lifecycle-1", Name: "lifecycle", Status: "running"}, nil
}
func (f *fakeBackend) PreflightType2Protection(request app.Type2ProtectionWriteRequestDTO) (app.Type2ProtectionPreflightDTO, error) {
	f.mu.Lock()
	f.type2ProtPlans = append(f.type2ProtPlans, request)
	f.mu.Unlock()
	return app.Type2ProtectionPreflightDTO{
		CardUID: "04 A1 B2 C3 D4 E5 F6", Product: "ntag215", ProductName: "NTAG215",
		Changes: []app.Type2ChangeDTO{{Target: "AUTH0", Old: "disabled (FF)", New: "4"}},
		Writes:  []app.Type2PageWriteDTO{{Page: 131, Hex: "00000004"}}, Token: "protect-token",
	}, nil
}
func (f *fakeBackend) StartType2ProtectionWrite(request app.Type2ProtectionWriteRequestDTO) (app.TaskDTO, error) {
	f.mu.Lock()
	f.type2ProtWrites = append(f.type2ProtWrites, request)
	f.mu.Unlock()
	return app.TaskDTO{ID: "type2-protection-1", Name: "protection", Status: "running"}, nil
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

func cloneWorkbenchDTO(value app.WorkbenchDTO) app.WorkbenchDTO {
	clone := value
	clone.DirtyBlocks = append([]int(nil), value.DirtyBlocks...)
	clone.Blocks = append([]app.WorkbenchBlockDTO(nil), value.Blocks...)
	clone.Diffs = append([]app.WorkbenchDiffDTO(nil), value.Diffs...)
	clone.Validation.Errors = append([]string(nil), value.Validation.Errors...)
	clone.Validation.Warnings = append([]string(nil), value.Validation.Warnings...)
	return clone
}

func testSpacedHex(value string) string {
	compact := normalizeClassicHex(value)
	parts := make([]string, 0, len(compact)/2)
	for index := 0; index+1 < len(compact); index += 2 {
		parts = append(parts, compact[index:index+2])
	}
	return strings.Join(parts, " ")
}

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
			Present: true, Length: 12, Raw: "D101085402656E68656C6C6F", Text: "hello", Summary: "hello",
			Records: []app.Type2NDEFRecordDTO{{TNF: 1, Type: "T", Kind: "text", Summary: "hello", Text: "hello"}},
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

func loadedType2Fixture() *fakeBackend {
	backend := type2Fixture()
	backend.type2 = app.Type2DumpDTO{
		Loaded: true,
		Info: app.Type2InfoDTO{
			Product: "ntag213", ProductName: "NTAG213", UID: "04 E1 9C 2A 7B 54 80",
			TotalPages: 45, UserPages: app.Type2PageRangeDTO{Start: 4, End: 39}, UserBytes: 144,
		},
		Pages: []app.Type2PageDTO{
			{Page: 0, Region: "uid", Hex: "04E19C2A", ASCII: "...."},
			{Page: 1, Region: "uid", Hex: "7B548000", ASCII: "...."},
			{Page: 2, Region: "static-lock", Hex: "00000000", ASCII: "...."},
			{Page: 3, Region: "capability", Hex: "E1101200", ASCII: "...."},
			{Page: 4, Region: "user", Hex: "00000000", ASCII: "....", Editable: true},
			{Page: 5, Region: "user", Hex: "11111111", ASCII: "....", Editable: true},
		},
		CC:         app.Type2CCDTO{Valid: true, Raw: "E1101200", Capacity: 144, Writable: true},
		Protection: app.Type2ProtectionDTO{StaticLock: "0000", DynamicLock: "000000", Auth0: 255, Access: 0x00},
	}
	return backend
}

func TestType2PageWriteConfirmationSubmitsRequest(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if model.type2PageChosen != 4 || model.type2PageInput != "00000000" {
		t.Fatalf("initial page selection = %d %q", model.type2PageChosen, model.type2PageInput)
	}
	model.type2PageInput = "DEADBEEF"
	tester.Frame()
	if err := tester.Click("Write page"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Write page 4?") || !tester.HasText("Confirm write") {
		t.Fatalf("confirmation texts: %q", tester.Texts())
	}
	if err := tester.Click("Confirm write"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2Writes)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	writes := append([]app.Type2WriteStartRequestDTO(nil), backend.type2Writes...)
	backend.mu.Unlock()
	if len(writes) != 1 || len(writes[0].Pages) != 1 || writes[0].Pages[0].Page != 4 || writes[0].Pages[0].Hex != "DEADBEEF" {
		t.Fatalf("write request = %+v", writes)
	}
}

func TestType2NDEFWriteConfirmationSubmitsRequest(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	model.selectNdefTemplate("text")
	model.setNdefField("text", "hello")
	tester.Frame()
	if err := tester.Click("Write NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Write Text?") {
		t.Fatalf("confirmation texts: %q", tester.Texts())
	}
	if err := tester.Click("Confirm write"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2NDEFWrites)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	writes := append([]app.Type2NDEFWriteStartRequestDTO(nil), backend.type2NDEFWrites...)
	backend.mu.Unlock()
	if len(writes) != 1 || writes[0].Template != "text" || writes[0].Fields["text"] != "hello" {
		t.Fatalf("NDEF request = %+v", writes)
	}
}

func TestType2WriteCancelStartsNothing(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	model.selectNdefTemplate("text")
	model.setNdefField("text", "hello")
	tester.Frame()
	if err := tester.Click("Write NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click("Cancel"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	backend.mu.Lock()
	count := len(backend.type2NDEFWrites)
	backend.mu.Unlock()
	if count != 0 {
		t.Fatalf("cancel started %d writes", count)
	}
}

func TestType2WriteRejectsInvalidPageAndSystemPage(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.type2PageInput = "XYZ"
	tester.Frame()
	if err := tester.Click("Write page"); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(model.errorText, "8 hexadecimal") {
		t.Fatalf("invalid hex error = %q", model.errorText)
	}
	model.type2PageChosen = 3
	model.type2PageInputPage = -1
	model.syncType2PageSelection(false)
	tester.Frame()
	if !tester.HasText("Select a user-memory page to edit.") {
		t.Fatalf("system page hint missing: %q", tester.Texts())
	}
}

func TestType2WriteFailureShowsBackupPath(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	model.applyEvent(app.TaskEventName, app.TaskEventDetailDTO{
		TaskID: "type2-write-1", Kind: "type2_write", Type: "failed",
		Message: "write failed; full-image backup: /tmp/backup.json",
	})
	if !strings.Contains(model.errorText, "/tmp/backup.json") {
		t.Fatalf("error text = %q", model.errorText)
	}
}

func TestType2MemorySelectionUpdatesEditor(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("11 11 11 11"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if model.type2PageChosen != 5 || model.type2PageInput != "11111111" {
		t.Fatalf("selection = %d %q", model.type2PageChosen, model.type2PageInput)
	}
}

func TestType2NDEFURITemplateSubmitsURI(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	model.selectNdefTemplate("uri")
	model.setNdefField("uri", "https://example.com")
	tester.Frame()
	if err := tester.Click("Write NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click("Confirm write"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2NDEFWrites)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	writes := append([]app.Type2NDEFWriteStartRequestDTO(nil), backend.type2NDEFWrites...)
	backend.mu.Unlock()
	if len(writes) != 1 || writes[0].Template != "uri" || writes[0].Fields["uri"] != "https://example.com" {
		t.Fatalf("URI request = %+v", writes)
	}
}

func TestType2NDEFDecodedRecordView(t *testing.T) {
	backend := loadedType2Fixture()
	backend.type2.NDEF = &app.Type2NDEFDTO{
		Present: true, Length: 120, Raw: "D2...", Summary: "Ada Lovelace",
		Records: []app.Type2NDEFRecordDTO{
			{
				TNF: 2, Type: "text/vcard", Kind: "vcard", Summary: "Ada Lovelace",
				Payload: "424547494E3A5643415244",
				Fields: []app.Type2NDEFRecordFieldDTO{
					{Key: "name", Value: "Ada Lovelace"},
					{Key: "organization", Value: "Analytical Engines"},
					{Key: "email", Value: "ada@example.com"},
				},
			},
			{
				TNF: 2, Type: "application/vnd.wfa.wsc", Kind: "wifi", Summary: "Foundry Lab",
				Fields: []app.Type2NDEFRecordFieldDTO{
					{Key: "ssid", Value: "Foundry Lab"},
					{Key: "auth", Value: "WPA2-Personal"},
					{Key: "encryption", Value: "AES"},
					{Key: "network_key", Value: "••••••"},
					{Key: "mac", Value: "FF:FF:FF:FF:FF:FF"},
				},
			},
		},
	}
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	tester.Frame()
	for _, text := range []string{"Ada Lovelace", "Organization", "Analytical Engines", "SSID", "WPA2-Personal", "AES", "••••••"} {
		if !tester.HasText(text) {
			t.Fatalf("missing %q in decoded NDEF view: %q", text, tester.Texts())
		}
	}
	if tester.HasText("42 45 47 49") {
		t.Fatalf("raw payload shown by default: %q", tester.Texts())
	}
	if err := tester.Click("Show raw"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Raw payload") || !tester.HasText("42 45 47 49") {
		t.Fatalf("raw payload not shown after toggle: %q", tester.Texts())
	}
}

func TestType2NDEFTemplateFieldsRender(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	model.selectNdefTemplate("contact")
	tester.Frame()
	for _, text := range []string{"Name", "Organization", "Job title", "Phone", "Email", "Address", "URL", "Note"} {
		if !tester.HasText(text) {
			t.Fatalf("missing contact field %q: %q", text, tester.Texts())
		}
	}
	model.selectNdefTemplate("wifi")
	tester.Frame()
	for _, text := range []string{"SSID", "Security", "Password", "WPA2-Personal"} {
		if !tester.HasText(text) {
			t.Fatalf("missing Wi-Fi field %q: %q", text, tester.Texts())
		}
	}
	model.selectNdefTemplate("search")
	tester.Frame()
	if !tester.HasText("Search URL prefix") || !tester.HasText("Search query") {
		t.Fatalf("missing search fields: %q", tester.Texts())
	}
	model.selectNdefTemplate("dpp")
	tester.Frame()
	if !tester.HasText("DPP bootstrapping URI") || !tester.HasText("WPA3 / Easy Connect (DPP URI)") {
		t.Fatalf("missing DPP fields: %q", tester.Texts())
	}
}

func TestType2NDEFWiFiTemplateSubmitsFields(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	model.selectNdefTemplate("wifi")
	model.setNdefField("ssid", "Foundry Lab")
	model.setNdefField("password", "password123")
	model.setNdefField("security", "wpa2")
	tester.Frame()
	if err := tester.Click("Write NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click("Confirm write"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2NDEFWrites)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	writes := append([]app.Type2NDEFWriteStartRequestDTO(nil), backend.type2NDEFWrites...)
	backend.mu.Unlock()
	if len(writes) != 1 || writes[0].Template != "wifi" ||
		writes[0].Fields["ssid"] != "Foundry Lab" ||
		writes[0].Fields["password"] != "password123" ||
		writes[0].Fields["security"] != "wpa2" {
		t.Fatalf("Wi-Fi request = %+v", writes)
	}
}

func TestClassicAdvancedToggleRequiresConfirmationAndResets(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if tester.HasText("Write changed blocks") {
		t.Fatal("advanced actions were visible before enabling advanced mode")
	}
	if err := tester.Click("Advanced operations"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Enable advanced operations?") {
		t.Fatalf("advanced warning missing: %q", tester.Texts())
	}
	if err := tester.Click("Enable advanced"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !model.advanced || !tester.HasText("Write changed blocks") {
		t.Fatalf("advanced mode not enabled: %q", tester.Texts())
	}
	model.applyCardEvent(app.CardEventDTO{Type: "removed"})
	if model.advanced {
		t.Fatal("advanced mode survived card removal")
	}
	tester.Frame()
	if tester.HasText("Write changed blocks") {
		t.Fatal("advanced actions remained visible after card removal")
	}
}

func TestClassicBlockEditorWritesSelectedBlock(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if !tester.HasText("Select an ordinary data block to edit.") {
		t.Fatalf("protected block hint missing: %q", tester.Texts())
	}
	model.blockChosen = 1
	model.syncBlockEditor(true)
	tester.Frame()
	if model.blockInput != "000102030405060708090A0B0C0D0E0F" {
		t.Fatalf("block input = %q", model.blockInput)
	}
	model.blockInput = "DEADBEEF00112233445566778899AABB"
	tester.Frame()
	if err := tester.Click("Write block"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Write block 1?") {
		t.Fatalf("confirmation missing: %q", tester.Texts())
	}
	if err := tester.Click("Confirm block write"); err != nil {
		t.Fatal(err)
	}
	writes := waitForClassicWrites(t, backend, 1)
	if len(writes) != 1 || writes[0].Token != "write-block" || writes[0].ConfirmTrailers {
		t.Fatalf("write request = %+v", writes)
	}
	backend.mu.Lock()
	edits := append([]classicEditCall(nil), backend.classicEdits...)
	preflights := append([]app.WritePreflightRequestDTO(nil), backend.classicPreflights...)
	backend.mu.Unlock()
	if len(edits) != 1 || edits[0].Block != 1 || edits[0].Hex != "DEADBEEF00112233445566778899AABB" {
		t.Fatalf("edits = %+v", edits)
	}
	if len(preflights) != 1 || preflights[0].Mode != "block" || len(preflights[0].Blocks) != 1 || preflights[0].Blocks[0] != 1 {
		t.Fatalf("preflights = %+v", preflights)
	}
}

func TestClassicChangedBlocksConfirmationIncludesBackupAndTrailers(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	model.workbench.Dirty = true
	model.workbench.DirtyBlocks = []int{1}
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Write changed blocks"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Write 2 changed blocks?") || !tester.HasText("Backup: /tmp/classic-backup.json") ||
		!tester.HasText("Trailers: 3") {
		t.Fatalf("preflight dialog texts: %q", tester.Texts())
	}
	if err := tester.Click("Write blocks and trailers"); err != nil {
		t.Fatal(err)
	}
	writes := waitForClassicWrites(t, backend, 1)
	if len(writes) != 1 || writes[0].Token != "write-changes" || !writes[0].ConfirmTrailers {
		t.Fatalf("write request = %+v", writes)
	}
}

func TestClassicFullRestoreRequiresTrailerConfirmation(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Full restore"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Restore the full card?") || !tester.HasText("A full restore replaces every writable block") {
		t.Fatalf("full restore dialog texts: %q", tester.Texts())
	}
	if err := tester.Click("Restore card"); err != nil {
		t.Fatal(err)
	}
	writes := waitForClassicWrites(t, backend, 1)
	if len(writes) != 1 || writes[0].Token != "write-full" || !writes[0].ConfirmTrailers {
		t.Fatalf("write request = %+v", writes)
	}
}

func TestClassicUIDWriteRequiresExactConfirmation(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	model.uidInput = "04AABBCC"
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Set UID"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Set UID to 04 AA BB CC?") || !tester.HasText("Confirm UID") ||
		!tester.HasText("Backup: /tmp/uid-backup.json") {
		t.Fatalf("UID confirmation texts: %q", tester.Texts())
	}
	if err := tester.Click("Confirm UID"); err != nil {
		t.Fatal(err)
	}
	writes := waitForUIDWrites(t, backend, 1)
	if len(writes) != 1 || writes[0].Token != "uid-token" || writes[0].Confirmation != "04AABBCC" {
		t.Fatalf("UID write request = %+v", writes)
	}
}

func TestClassicUIDWriteRejectsInvalidUIDBeforePreflight(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.advanced = true
	model.uidInput = "XYZ"
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Set UID"); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(model.errorText, "8 hexadecimal") {
		t.Fatalf("error = %q", model.errorText)
	}
	backend.mu.Lock()
	defer backend.mu.Unlock()
	if len(backend.uidPreflights) != 0 || len(backend.uidWrites) != 0 {
		t.Fatal("invalid UID reached the backend")
	}
}

func TestClassicWriteFailureShowsBackupPath(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.applyEvent(app.TaskEventName, app.TaskEventDetailDTO{
		TaskID: "write-1", Kind: "write", Type: "failed",
		Message: "pre-write backup: /tmp/classic-backup.json",
	})
	if !strings.Contains(model.errorText, "/tmp/classic-backup.json") {
		t.Fatalf("error text = %q", model.errorText)
	}
}

func TestClassicDumpActionsCallBackend(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Save dump"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		saves := backend.dumpSaves
		backend.mu.Unlock()
		if saves == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	if err := tester.Click("Load dump"); err != nil {
		t.Fatal(err)
	}
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		loads := backend.dumpLoads
		backend.mu.Unlock()
		if loads == 1 {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("Load dump did not call the backend")
}

func waitForClassicWrites(t *testing.T, backend *fakeBackend, count int) []app.WriteStartRequestDTO {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		writes := append([]app.WriteStartRequestDTO(nil), backend.classicWrites...)
		backend.mu.Unlock()
		if len(writes) >= count {
			return writes
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("Classic write did not start")
	return nil
}

func waitForUIDWrites(t *testing.T, backend *fakeBackend, count int) []app.UIDWriteStartRequestDTO {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		writes := append([]app.UIDWriteStartRequestDTO(nil), backend.uidWrites...)
		backend.mu.Unlock()
		if len(writes) >= count {
			return writes
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("UID write did not start")
	return nil
}

func ndefSensitiveFixture() *fakeBackend {
	backend := loadedType2Fixture()
	backend.type2.NDEF = &app.Type2NDEFDTO{
		Present: true, Length: 80, Raw: "D101...", Summary: "Foundry",
		Records: []app.Type2NDEFRecordDTO{{
			TNF: 2, Type: "application/vnd.wfa.wsc", Kind: "wifi", Summary: "Foundry",
			Fields: []app.Type2NDEFRecordFieldDTO{
				{Key: "ssid", Value: "Foundry"},
				{Key: "network_key", Value: "secret123", Sensitive: true},
			},
		}},
	}
	return backend
}

func TestType2PasswordReachesReadAndPageWrite(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.type2Password = "01020304"
	tester.Frame()
	if err := tester.Click("Read card"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		starts := backend.type2Starts
		backend.mu.Unlock()
		if starts == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	password := backend.type2ReadPassword
	backend.mu.Unlock()
	if password != "01020304" {
		t.Fatalf("read password = %q", password)
	}

	model.type2PageInput = "DEADBEEF"
	tester.Frame()
	if err := tester.Click("Write page"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click("Confirm write"); err != nil {
		t.Fatal(err)
	}
	deadline = time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2Writes)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	writes := append([]app.Type2WriteStartRequestDTO(nil), backend.type2Writes...)
	backend.mu.Unlock()
	if len(writes) != 1 || writes[0].Password != "01020304" {
		t.Fatalf("page write password = %+v", writes)
	}
}

func TestType2NDEFRawAndMultiRecordWrites(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	model.ndefMode = "raw"
	model.ndefRaw = "D101085402656E68656C6C6F"
	tester.Frame()
	if err := tester.Click("Write NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Write a 12-byte raw message?") {
		t.Fatalf("raw confirmation texts: %q", tester.Texts())
	}
	if err := tester.Click("Confirm write"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2RawWrites)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	rawWrites := append([]app.Type2NDEFRawRequestDTO(nil), backend.type2RawWrites...)
	backend.mu.Unlock()
	if len(rawWrites) != 1 || rawWrites[0].Raw != model.ndefRaw {
		t.Fatalf("raw write = %+v", rawWrites)
	}

	model.ndefMode = "records"
	model.ndefDrafts = []ndefRecordDraft{{Kind: "text", Text: "hello"}, {Kind: "uri", URI: "https://example.com"}}
	tester.Frame()
	if err := tester.Click("Write NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Write 2 records (16 bytes)?") {
		t.Fatalf("records confirmation texts: %q", tester.Texts())
	}
	if err := tester.Click("Confirm write"); err != nil {
		t.Fatal(err)
	}
	deadline = time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2Records)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	records := append([]app.Type2NDEFRecordsRequestDTO(nil), backend.type2Records...)
	backend.mu.Unlock()
	if len(records) != 1 || len(records[0].Records) != 2 || records[0].Records[0].Fields["text"] != "hello" {
		t.Fatalf("record write = %+v", records)
	}
}

func TestType2NDEFRevealAndImportExport(t *testing.T) {
	backend := ndefSensitiveFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabNDEF
	tester.Frame()
	if tester.HasText("secret123") || !tester.HasText("••••••") {
		t.Fatalf("sensitive field was not masked: %q", tester.Texts())
	}
	if err := tester.Click("Reveal secrets"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("secret123") {
		t.Fatalf("reveal did not show the key: %q", tester.Texts())
	}

	if err := tester.Click("Import…"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if model.ndefMode != "raw" || model.ndefRaw != "D1010454657374" {
		t.Fatalf("import state = %q %q", model.ndefMode, model.ndefRaw)
	}
	if err := tester.Click("Export…"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	backend.mu.Lock()
	exported := backend.exportedNDEF
	backend.mu.Unlock()
	if exported != model.ndefRaw {
		t.Fatalf("exported = %q; want %q", exported, model.ndefRaw)
	}
}

func TestType2ProtectionEditorRequiresAdvancedAndPhrase(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 1400)
	model.selectedTab = tabProtection
	tester.Frame()
	if tester.HasText("Protection editor") {
		t.Fatal("protection editor was visible before Advanced mode")
	}
	model.advanced = true
	model.type2ProtAuth0 = "4"
	tester.Frame()
	if !tester.HasText("Protection editor") {
		t.Fatalf("protection editor missing: %q", tester.Texts())
	}
	if err := tester.Click("Review protected write"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2ProtPlans)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	tester.Frame()
	if !tester.HasText("Type PROTECTED to confirm.") || !tester.HasText("AUTH0: disabled (FF) → 4") {
		t.Fatalf("protection confirmation missing: %q", tester.Texts())
	}
	_ = tester.Click("Apply")
	tester.Frame()
	backend.mu.Lock()
	starts := len(backend.type2ProtWrites)
	backend.mu.Unlock()
	if starts != 0 {
		t.Fatalf("protection write started without PROTECTED: %d", starts)
	}
	model.type2ProtConfirm = "PROTECTED"
	tester.Frame()
	if err := tester.Click("Apply"); err != nil {
		t.Fatal(err)
	}
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		starts = len(backend.type2ProtWrites)
		backend.mu.Unlock()
		if starts == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	writes := append([]app.Type2ProtectionWriteRequestDTO(nil), backend.type2ProtWrites...)
	backend.mu.Unlock()
	if len(writes) != 1 || writes[0].Confirmation != "PROTECTED" ||
		writes[0].Token != "protect-token" || writes[0].Auth0 == nil || *writes[0].Auth0 != 4 {
		t.Fatalf("protection request = %+v", writes)
	}
}

func TestType2ProtectionDialogClosesWhenAdvancedIsDisabled(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 1400)
	model.selectedTab = tabProtection
	model.advanced = true
	model.type2ProtAuth0 = "4"
	tester.Frame()
	if err := tester.Click("Review protected write"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2ProtPlans)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	tester.Frame()
	if !tester.HasText("Type PROTECTED to confirm.") {
		t.Fatalf("protection dialog missing: %q", tester.Texts())
	}
	model.toggleAdvanced()
	tester.Frame()
	if model.advanced || model.type2ProtOpen || tester.HasText("Type PROTECTED to confirm.") {
		t.Fatalf("protection dialog survived disabling advanced mode: %q", tester.Texts())
	}
}

func TestType2AdvancedLifecycleRequiresTypedUID(t *testing.T) {
	backend := loadedType2Fixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	model.selectedTab = tabProtection
	tester.Frame()
	if tester.HasText("Advanced lifecycle") {
		t.Fatal("lifecycle controls were visible before Advanced mode")
	}
	model.advanced = true
	tester.Frame()
	if !tester.HasText("Advanced lifecycle") {
		t.Fatalf("lifecycle controls missing: %q", tester.Texts())
	}
	if err := tester.Click("Erase user memory"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		count := len(backend.type2Plans)
		backend.mu.Unlock()
		if count == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	tester.Frame()
	if !tester.HasText("Type the UID 04 A1 B2 C3 D4 E5 F6 to confirm.") {
		t.Fatalf("typed confirmation missing: %q", tester.Texts())
	}
	_ = tester.Click("Apply")
	tester.Frame()
	backend.mu.Lock()
	starts := len(backend.type2Lifecycles)
	backend.mu.Unlock()
	if starts != 0 {
		t.Fatalf("lifecycle started without the typed UID: %d", starts)
	}
	model.type2PlanConfirm = "04 A1 B2 C3 D4 E5 F6"
	tester.Frame()
	if err := tester.Click("Apply"); err != nil {
		t.Fatal(err)
	}
	deadline = time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		backend.mu.Lock()
		starts = len(backend.type2Lifecycles)
		backend.mu.Unlock()
		if starts == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	backend.mu.Lock()
	lifecycles := append([]app.Type2LifecycleRequestDTO(nil), backend.type2Lifecycles...)
	backend.mu.Unlock()
	if len(lifecycles) != 1 || lifecycles[0].Token != "type2-plan-token" || lifecycles[0].Operation != "erase" {
		t.Fatalf("lifecycle request = %+v", lifecycles)
	}
}
