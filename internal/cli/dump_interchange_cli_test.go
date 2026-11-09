package cli_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/app"
)

func TestCLIClassicDumpAndExportInterchangeFormats(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)

	mctPath := filepath.Join(harness.dir, "card.mct")
	code, envelope := harness.run("classic", "dump", mctPath, "--scan", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("dump mct: code=%d error=%+v stderr=%s", code, envelope.Error, harness.stderr.String())
	}
	var dumped struct {
		Path   string `json:"path"`
		Format string `json:"format"`
	}
	decodeResult(t, envelope, &dumped)
	if dumped.Format != "mct" {
		t.Fatalf("dump format = %q", dumped.Format)
	}
	mct, err := os.ReadFile(mctPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(string(mct), "+Sector: 0\n") {
		t.Fatalf("MCT dump = %q", mct)
	}

	proxmarkPath := filepath.Join(harness.dir, "card.mfc.json")
	code, envelope = harness.run("classic", "export", proxmarkPath, "--format", "proxmark3", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("export proxmark3: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var exported struct {
		Path   string `json:"path"`
		Format string `json:"format"`
	}
	decodeResult(t, envelope, &exported)
	if exported.Format != "proxmark3" {
		t.Fatalf("export format = %q", exported.Format)
	}
	document, err := os.ReadFile(proxmarkPath)
	if err != nil {
		t.Fatal(err)
	}
	var proxmark map[string]any
	if err := json.Unmarshal(document, &proxmark); err != nil {
		t.Fatal(err)
	}
	if proxmark["FileType"] != "mfc v2" {
		t.Fatalf("FileType = %v", proxmark["FileType"])
	}
	card := proxmark["Card"].(map[string]any)
	if card["UID"] != fake.uid() {
		t.Fatalf("Card UID = %v, want %s", card["UID"], fake.uid())
	}
	if _, ok := proxmark["SectorKeys"].(map[string]any); !ok {
		t.Fatalf("SectorKeys missing: %v", proxmark["SectorKeys"])
	}

	code, envelope = harness.run("classic", "restore", proxmarkPath, "--dry-run", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("restore proxmark3 dry run: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var restored struct {
		Preflight *app.WritePreflightDTO `json:"preflight"`
		Warnings  []string               `json:"warnings"`
	}
	decodeResult(t, envelope, &restored)
	if restored.Preflight == nil || !restored.Preflight.Valid || len(restored.Warnings) != 0 {
		t.Fatalf("restore preflight = %+v warnings = %v", restored.Preflight, restored.Warnings)
	}

	emlPath := filepath.Join(harness.dir, "card.eml")
	if code, envelope = harness.run("classic", "export", emlPath, "--format", "eml", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("export eml: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	eml, err := os.ReadFile(emlPath)
	if err != nil {
		t.Fatal(err)
	}
	if lines := strings.Count(strings.TrimSpace(string(eml)), "\n") + 1; lines != 64 {
		t.Fatalf("EML line count = %d", lines)
	}

	mctJSONPath := filepath.Join(harness.dir, "card.mfcard.json")
	if code, envelope = harness.run("classic", "export", mctJSONPath, "--format", "mct-json", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("export mct json: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	document, err = os.ReadFile(mctJSONPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(document), `"FileType": "mfcard"`) {
		t.Fatalf("MCT JSON = %s", document)
	}

	keyArrayPath := filepath.Join(harness.dir, "card-keys.bin")
	if code, envelope = harness.run("classic", "export", keyArrayPath, "--format", "keyarray", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("export key array: code=%d error=%+v stderr=%s", code, envelope.Error, harness.stderr.String())
	}
	keyArrayInfo, err := os.Stat(keyArrayPath)
	if err != nil || keyArrayInfo.Size() != 16*2*6 {
		t.Fatalf("key array size = %v, err = %v", keyArrayInfo, err)
	}

	sidecarPath := filepath.Join(harness.dir, "card.bin")
	if code, envelope = harness.run("classic", "export", sidecarPath, "--format", "sidecar", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("export sidecar: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	if _, err := os.Stat(sidecarPath + ".nfcx.json"); err != nil {
		t.Fatal(err)
	}

	if code, _ = harness.run("classic", "export", proxmarkPath, "--json"); code != cliExitUsage {
		t.Fatalf("export without --format code = %d", code)
	}
}

func TestCLIClassicDumpEmitsTextFormatWarnings(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	path := filepath.Join(harness.dir, "card.eml")
	if code, envelope := harness.run("classic", "dump", path, "--scan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("dump eml: code=%d error=%+v stderr=%s", code, envelope.Error, harness.stderr.String())
	}
	code, envelope := harness.run("classic", "restore", path, "--dry-run", "--json")
	if code == 0 {
		t.Fatalf("restore from an identity-less EML dump unexpectedly succeeded: %+v", envelope)
	}
	if envelope.Error == nil || !strings.Contains(envelope.Error.Message, "no card identity") {
		t.Fatalf("restore error = %+v", envelope.Error)
	}
}

func TestCLIKeyArrayImport(t *testing.T) {
	harness := newHarness(t, newFakeClassic())
	data := make([]byte, 16*2*6)
	for sector := range 16 {
		copy(data[sector*6:], []byte{0xD0, 0x00, byte(sector), 0x11, 0x22, 0x33})
		copy(data[(16+sector)*6:], []byte{0xD1, 0x00, byte(sector), 0x11, 0x22, 0x33})
	}
	path := filepath.Join(harness.dir, "card-keys.bin")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	code, envelope := harness.run("keys", "import", path, "--format", "keyarray", "--set", "PM3 raw", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("keys import keyarray: code=%d error=%+v stderr=%s", code, envelope.Error, harness.stderr.String())
	}
	var imported struct {
		Format string         `json:"format"`
		Valid  int            `json:"valid"`
		Added  int            `json:"added"`
		KeySet *app.KeySetDTO `json:"keySet"`
	}
	decodeResult(t, envelope, &imported)
	if imported.Format != "keyarray" || imported.Valid != 32 || imported.Added != 32 ||
		imported.KeySet == nil || imported.KeySet.KeyCount != 32 {
		t.Fatalf("key array import = %+v set = %+v", imported, imported.KeySet)
	}
	if code, _ = harness.run("keys", "import", path, "--format", "bogus", "--json"); code != cliExitUsage {
		t.Fatalf("invalid key import format code = %d", code)
	}
}

func TestCLIKeyImportSetAndExportFormats(t *testing.T) {
	harness := newHarness(t, newFakeClassic())
	keyFile := filepath.Join(harness.dir, "import.keys")
	if err := os.WriteFile(keyFile, []byte("# synthetic\nd00d1234beef\nCAFEBABE0001\nD00D1234BEEF\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	code, envelope := harness.run("keys", "import", keyFile, "--set", "Imported batch", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("keys import --set: code=%d error=%+v stderr=%s", code, envelope.Error, harness.stderr.String())
	}
	var imported struct {
		Added  int            `json:"added"`
		KeySet *app.KeySetDTO `json:"keySet"`
	}
	decodeResult(t, envelope, &imported)
	if imported.Added != 2 || imported.KeySet == nil || imported.KeySet.Name != "Imported batch" || imported.KeySet.KeyCount != 2 {
		t.Fatalf("import result = %+v", imported)
	}

	exportPath := filepath.Join(harness.dir, "export.keys")
	code, envelope = harness.run("keys", "export", exportPath, "--set", "Imported batch", "--format", "keys", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("keys export --set: code=%d error=%+v stderr=%s", code, envelope.Error, harness.stderr.String())
	}
	values, err := os.ReadFile(exportPath)
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Fields(string(values))
	if len(lines) != 2 {
		t.Fatalf("exported key lines = %v", lines)
	}
	if strings.Contains(string(values), "#") || strings.Contains(string(values), "D00D1234BEEF\nD00D1234BEEF") {
		t.Fatalf("exported key text = %q", values)
	}

	code, _ = harness.run("keys", "export", exportPath, "--format", "json", "--json")
	if code != cliExitUsage {
		t.Fatalf("keys export --format json code = %d", code)
	}
}
