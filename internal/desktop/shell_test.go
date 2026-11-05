package desktop

import (
	"fmt"
	"testing"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo/ui"
)

type fakeBackend struct {
	dashboard app.DashboardDTO
	workbench app.WorkbenchDTO
	keys      []app.KeyDTO
}

func (f *fakeBackend) GetDashboard() app.DashboardDTO { return f.dashboard }
func (f *fakeBackend) RefreshDevices() ([]app.DeviceDTO, error) {
	return f.dashboard.Devices, nil
}
func (f *fakeBackend) ConnectDevice(string) error  { return nil }
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
func (f *fakeBackend) StartReadCard(bool) (app.TaskDTO, error) {
	return app.TaskDTO{ID: "read-1", Name: "read", Status: "running"}, nil
}
func (f *fakeBackend) StartKeyScan(bool) (app.TaskDTO, error) {
	return app.TaskDTO{ID: "keyscan-1", Name: "keyscan", Status: "running"}, nil
}
func (f *fakeBackend) CancelTask(string) error { return nil }

func fixtureBackend() *fakeBackend {
	device := app.DeviceDTO{
		ID: "reader-1", Name: "PN532 UART", ConnString: "pn532_uart:/dev/ttyUSB0",
		Transport: "UART / PN532", Available: true, Nested: true, Darkside: true, Hardnested: true,
	}
	card := app.CardDTO{
		Present: true, UID: "04 A1 B2 C3", UIDLength: 4, ATQA: "00 04", SAK: "08",
		Type: "MIFARE Classic 1K", TypeCode: "mifare_classic_1k", Inferred: true,
	}
	workbench := app.WorkbenchDTO{
		Loaded: true, Layout: "classic_1k", Complete: true,
		Blocks: []app.WorkbenchBlockDTO{
			{Block: 0, Sector: 0, Kind: "manufacturer", Hex: "04 A1 B2 C3 D4 E5 F6 88 04 00 00 00 00 00 00 00", ASCII: "........"},
			{Block: 1, Sector: 0, Kind: "data", Hex: "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F", ASCII: "................", Status: "read"},
		},
	}
	keys := []app.KeyDTO{
		{ID: "builtin", Value: "FF FF FF FF FF FF", Builtin: true, Sources: []string{"builtin"}},
		{ID: "custom", Value: "A0 A1 A2 A3 A4 A5", Removable: true, Sources: []string{"user_input"}},
	}
	return &fakeBackend{
		dashboard: app.DashboardDTO{
			Devices: []app.DeviceDTO{device}, SelectedDevice: device.ID,
			Connection: app.ConnectionDTO{Status: "polling"},
			Card:       card,
		},
		workbench: workbench,
		keys:      keys,
	}
}

func TestShellRendersCoreScreensAndEvents(t *testing.T) {
	backend := fixtureBackend()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1100, 760)

	if !tester.HasText("Readers") || !tester.HasText("PN532 UART") {
		t.Fatalf("devices page texts: %q", tester.Texts())
	}
	if err := tester.Click("Card"); err != nil {
		t.Fatal(err)
	}
	if !tester.HasText("MIFARE Classic 1K") || !tester.HasText("04 A1 B2 C3") {
		t.Fatalf("card page texts: %q", tester.Texts())
	}

	if err := tester.Click("Classic blocks"); err != nil {
		t.Fatal(err)
	}
	if !tester.HasText("04 A1 B2 C3 D4 E5 F6 88 04 00 00 00 00 00 00 00") {
		t.Fatalf("block table texts: %q", tester.Texts())
	}

	readResult := backend.workbench
	readResult.Blocks = append([]app.WorkbenchBlockDTO(nil), backend.workbench.Blocks...)
	readResult.Blocks[1].Hex = "DE AD BE EF 00 00 00 00 00 00 00 00 00 00 00 00"
	model.applyEvent(app.WorkbenchEventName, readResult)
	tester.Frame()
	if !tester.HasText("DE AD BE EF 00 00 00 00 00 00 00 00 00 00 00 00") {
		t.Fatalf("workbench event did not update the table: %q", tester.Texts())
	}

	if err := tester.Click("Key catalog"); err != nil {
		t.Fatal(err)
	}
	if !tester.HasText("A0 A1 A2 A3 A4 A5") {
		t.Fatalf("key page texts: %q", tester.Texts())
	}

	model.applyEvent(app.TaskEventName, app.TaskEventDetailDTO{
		TaskID: "read-1", Kind: "read", Type: app.TaskEventProgress,
		Progress: 42, Message: "Reading sector 3",
	})
	if err := tester.Click("Activity"); err != nil {
		t.Fatal(err)
	}
	if !tester.HasText("Reading sector 3") {
		t.Fatalf("activity page texts: %q", tester.Texts())
	}

	if err := tester.Click("Devices"); err != nil {
		t.Fatal(err)
	}
	model.locale = "zh-Hans"
	tester.Frame()
	if !tester.HasText("读卡器") || !tester.HasText("正在检测卡片") {
		t.Fatalf("Simplified Chinese texts: %q", tester.Texts())
	}
}

func TestCatalogTranslationsAreComplete(t *testing.T) {
	en := locales.In("en")
	zh := locales.In("zh-Hans")
	checks := []struct {
		name string
		en   string
		zh   string
	}{
		{"navDevices", en.NavDevices(), zh.NavDevices()},
		{"keysTitle", en.KeysTitle(), zh.KeysTitle()},
		{"activityTitle", en.ActivityTitle(), zh.ActivityTitle()},
		{"classicEmpty", en.ClassicEmpty(), zh.ClassicEmpty()},
		{"cardEmpty", en.CardEmpty(), zh.CardEmpty()},
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
