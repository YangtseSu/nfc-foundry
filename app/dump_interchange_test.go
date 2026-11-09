package app

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
)

func TestSaveDumpFileAsInterchangeFormats(t *testing.T) {
	service, _, fixture, _, _ := newClassicWriteAppService(t, "")
	dir := t.TempDir()

	proxmarkPath := filepath.Join(dir, "card.mfc.json")
	if _, err := service.SaveDumpFileAs(proxmarkPath, DumpFormatProxmark); err != nil {
		t.Fatal(err)
	}
	loaded, err := service.LoadDumpFile(proxmarkPath, true)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Warnings) != 0 || loaded.SourceUID == "" {
		t.Fatalf("Proxmark3 load = warnings %v source UID %q", loaded.Warnings, loaded.SourceUID)
	}

	mctPath := filepath.Join(dir, "card.mct")
	saved, err := service.SaveDumpFileAs(mctPath, DumpFormatMCT)
	if err != nil {
		t.Fatal(err)
	}
	if saved.Format != string(DumpFormatMCT) || saved.Path != "card.mct" {
		t.Fatalf("saved mct = %+v", saved)
	}
	loaded, err = service.LoadDumpFile(mctPath, true)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Warnings) == 0 {
		t.Fatal("MCT text load did not report the missing card identity")
	}

	if _, err := service.SaveDumpFile(filepath.Join(dir, "card.eml")); err != nil {
		t.Fatal(err)
	}

	keyArrayPath := filepath.Join(dir, "card-keys.bin")
	if _, err := service.SaveDumpFileAs(keyArrayPath, DumpFormatKeyArray); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(keyArrayPath)
	if err != nil || info.Size() != mifare.Classic1KSectors*2*6 {
		t.Fatalf("key array size = %v, err = %v", info, err)
	}

	partial := fixture.dumpResult(t, service)
	partial.Dump.Blocks[9].KnownMask = 0
	partial.Dump.Blocks[9].Status = mifare.BlockReadFailed
	if err := service.bench.Load(partial, ""); err != nil {
		t.Fatal(err)
	}
	rejectedPath := filepath.Join(dir, "partial.mct")
	if _, err := service.SaveDumpFileAs(rejectedPath, DumpFormatMCT); err == nil {
		t.Fatal("partial dump was exported as MCT text")
	}
	if _, err := os.Stat(rejectedPath); !os.IsNotExist(err) {
		t.Fatal("partial MCT export created a file")
	}
	if _, err := service.SaveDumpFileAs(filepath.Join(dir, "partial.json"), DumpFormatProject); err != nil {
		t.Fatalf("partial project export failed: %v", err)
	}
}

func TestImportKeyArrayWithSet(t *testing.T) {
	service, _, _, _, _ := newClassicWriteAppService(t, "")
	dir := t.TempDir()
	data := make([]byte, mifare.Classic1KSectors*2*6)
	for sector := range mifare.Classic1KSectors {
		copy(data[sector*6:], []byte{0xD0, 0x00, byte(sector), 0x11, 0x22, 0x33})
		copy(data[(mifare.Classic1KSectors+sector)*6:], []byte{0xD1, 0x00, byte(sector), 0x11, 0x22, 0x33})
	}
	path := filepath.Join(dir, "card-keys.bin")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	report, err := service.ImportKeyFileWithSet(path, "keyarray", "PM3 raw")
	if err != nil {
		t.Fatal(err)
	}
	if report.Valid != 32 || report.Added != 32 || report.KeySet == nil || report.KeySet.KeyCount != 32 {
		t.Fatalf("key array import = %+v set = %+v", report, report.KeySet)
	}
	report, err = service.ImportKeyFileWithSet(path, "keyarray", "PM3 raw")
	if err != nil {
		t.Fatal(err)
	}
	if report.Added != 0 || report.Duplicates != 32 || report.KeySet == nil || report.KeySet.KeyCount != 32 {
		t.Fatalf("duplicate key array import = %+v set = %+v", report, report.KeySet)
	}
	if _, err := service.ImportKeyFileWithSet(path, "bogus", ""); err == nil {
		t.Fatal("unknown key-file format was accepted")
	}
}

func TestImportKeyDictionaryWithSet(t *testing.T) {
	service, _, _, _, _ := newClassicWriteAppService(t, "")
	dir := t.TempDir()
	first := filepath.Join(dir, "first.keys")
	if err := os.WriteFile(first, []byte("# comment\nD00D1234BEEF\nCAFEBABE0001\nD00D1234BEEF\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	report, err := service.ImportKeyDictionaryFileWithSet(first, "Imported batch")
	if err != nil {
		t.Fatal(err)
	}
	if report.KeySet == nil || report.KeySet.Name != "Imported batch" || report.KeySet.KeyCount != 2 {
		t.Fatalf("import report = %+v set = %+v", report, report.KeySet)
	}
	if report.Added != 2 || report.Duplicates != 1 {
		t.Fatalf("added=%d duplicates=%d", report.Added, report.Duplicates)
	}

	second := filepath.Join(dir, "second.dic")
	if err := os.WriteFile(second, []byte("D00D1234BEEF\nFEEDFACE1234\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	report, err = service.ImportKeyDictionaryFileWithSet(second, "Imported batch")
	if err != nil {
		t.Fatal(err)
	}
	if report.KeySet == nil || report.KeySet.KeyCount != 3 {
		t.Fatalf("extended set = %+v", report.KeySet)
	}
	sets := service.KeySets()
	if len(sets) != 1 || sets[0].Name != "Imported batch" || sets[0].KeyCount != 3 {
		t.Fatalf("key sets = %+v", sets)
	}
}
