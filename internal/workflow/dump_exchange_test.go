package workflow_test

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func complete4KDumpResult(t *testing.T) workflow.DumpResult {
	t.Helper()
	image, _ := mifare.NewDump(mifare.Classic4K)
	for index := range image.Blocks {
		image.Blocks[index].Data[0] = byte(index)
		image.Blocks[index].Data[1] = byte(index >> 8)
		image.Blocks[index].KnownMask = mifare.AllBytesKnown
		image.Blocks[index].Status = mifare.BlockRead
		if index%4 == 3 || index%16 == 15 {
			image.Blocks[index].Data = transportTrailer(0xff)
		}
	}
	image.Blocks[0].Data[0], image.Blocks[0].Data[1], image.Blocks[0].Data[2], image.Blocks[0].Data[3] = 1, 2, 3, 4
	image.Blocks[0].Data[4] = 1 ^ 2 ^ 3 ^ 4
	return workflow.DumpResult{Dump: image, Card: classic1KCard(1)}
}

func dumpRawBytes(t *testing.T, result workflow.DumpResult) []byte {
	t.Helper()
	raw, err := result.Dump.Raw()
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func requireSameRaw(t *testing.T, want, got workflow.DumpResult) {
	t.Helper()
	if !got.Complete() {
		t.Fatal("loaded dump is incomplete")
	}
	if string(dumpRawBytes(t, want)) != string(dumpRawBytes(t, got)) {
		t.Fatal("raw image changed across the interchange round trip")
	}
}

func writeFixture(t *testing.T, name string, data []byte) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestDumpFormatNamesAndPaths(t *testing.T) {
	for _, name := range []string{"project", "raw", "mct", "eml", "mct-json", "proxmark3", "sidecar"} {
		format, err := workflow.ParseDumpFormat(name)
		if err != nil || string(format) != name {
			t.Fatalf("ParseDumpFormat(%q) = %q, %v", name, format, err)
		}
	}
	if _, err := workflow.ParseDumpFormat("mfu"); err == nil {
		t.Fatal("ParseDumpFormat accepted an unsupported format")
	}
	for path, want := range map[string]workflow.DumpFormat{
		"card.bin": workflow.DumpFormatRaw, "card.mfd": workflow.DumpFormatRaw,
		"card.mct": workflow.DumpFormatMCT, "card.eml": workflow.DumpFormatEML,
		"card.json": workflow.DumpFormatProject,
	} {
		got, err := workflow.DumpFormatForPath(path)
		if err != nil || got != want {
			t.Fatalf("DumpFormatForPath(%q) = %q, %v", path, got, err)
		}
	}
	if _, err := workflow.DumpFormatForPath("card.txt"); err == nil {
		t.Fatal("DumpFormatForPath accepted an unknown extension")
	}
}

func TestMCTTextRoundTrip(t *testing.T) {
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "card.mct")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatMCT, want); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if load.Format != workflow.DumpFormatMCT || len(load.Warnings) == 0 {
		t.Fatalf("load = format %q warnings %v", load.Format, load.Warnings)
	}
	requireSameRaw(t, want, load.Result)
	if len(load.Result.Card.UID) != 0 {
		t.Fatalf("MCT text import invented card identity: %x", load.Result.Card.UID)
	}
}

func TestMCTText4KRoundTrip(t *testing.T) {
	want := complete4KDumpResult(t)
	path := filepath.Join(t.TempDir(), "card4k.mct")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatMCT, want); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if load.Result.Dump.Layout != mifare.Classic4K {
		t.Fatalf("layout = %v", load.Result.Dump.Layout)
	}
	requireSameRaw(t, want, load.Result)
}

func TestEMLRoundTrip(t *testing.T) {
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "card.eml")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatEML, want); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if load.Format != workflow.DumpFormatEML {
		t.Fatalf("format = %q", load.Format)
	}
	requireSameRaw(t, want, load.Result)
}

func TestMCTJSONRoundTrip(t *testing.T) {
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "card.mfcard.json")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatMCTJSON, want); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if load.Format != workflow.DumpFormatMCTJSON || len(load.Warnings) == 0 {
		t.Fatalf("load = format %q warnings %v", load.Format, load.Warnings)
	}
	requireSameRaw(t, want, load.Result)
}

func TestProxmarkJSONRoundTrip(t *testing.T) {
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "card.mfc.json")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatProxmark, want); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if load.Format != workflow.DumpFormatProxmark || len(load.Warnings) != 0 {
		t.Fatalf("load = format %q warnings %v", load.Format, load.Warnings)
	}
	requireSameRaw(t, want, load.Result)
	if string(load.Result.Card.UID) != string(want.Card.UID) || load.Result.Card.SAK != want.Card.SAK {
		t.Fatalf("card identity = %+v", load.Result.Card)
	}
	if len(load.Result.Keys) != mifare.Classic1KSectors {
		t.Fatalf("sector keys = %d", len(load.Result.Keys))
	}
	if load.Result.Keys[0].KeyA == nil || load.Result.Keys[15].KeyB == nil {
		t.Fatalf("sector keys were not preserved: %+v", load.Result.Keys)
	}
	if *load.Result.Keys[0].KeyA != (nfc.Key{0xff, 0xff, 0xff, 0xff, 0xff, 0xff}) {
		t.Fatalf("Key A = %x", *load.Result.Keys[0].KeyA)
	}
}

func TestProxmarkJSON4KRoundTrip(t *testing.T) {
	want := complete4KDumpResult(t)
	path := filepath.Join(t.TempDir(), "card4k.mfc.json")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatProxmark, want); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if load.Result.Dump.Layout != mifare.Classic4K || len(load.Result.Keys) != mifare.Classic4KSectors {
		t.Fatalf("loaded layout=%v keys=%d", load.Result.Dump.Layout, len(load.Result.Keys))
	}
	requireSameRaw(t, want, load.Result)
	if load.Result.Keys[39].KeyA == nil {
		t.Fatal("sector 39 keys were not preserved")
	}
}

func TestKeyArrayRoundTrip(t *testing.T) {
	want := completeDumpResult(t)
	trailer := &want.Dump.Blocks[3]
	copy(trailer.Data[0:6], []byte{0x01, 0x02, 0x03, 0x04, 0x05, 0x06})
	copy(trailer.Data[10:16], []byte{0x11, 0x12, 0x13, 0x14, 0x15, 0x16})
	data, err := workflow.EncodeKeyArray(want)
	if err != nil {
		t.Fatal(err)
	}
	if len(data) != mifare.Classic1KSectors*2*nfc.KeySize {
		t.Fatalf("1K key array size = %d", len(data))
	}
	if !bytes.Equal(data[0:6], []byte{0x01, 0x02, 0x03, 0x04, 0x05, 0x06}) {
		t.Fatalf("sector 0 Key A = % X", data[0:6])
	}
	if !bytes.Equal(data[mifare.Classic1KSectors*nfc.KeySize:mifare.Classic1KSectors*nfc.KeySize+6], []byte{0x11, 0x12, 0x13, 0x14, 0x15, 0x16}) {
		t.Fatalf("sector 0 Key B = % X", data[mifare.Classic1KSectors*nfc.KeySize:mifare.Classic1KSectors*nfc.KeySize+6])
	}
	array, err := workflow.ParseKeyArray(data)
	if err != nil {
		t.Fatal(err)
	}
	if array.Layout != mifare.Classic1K || len(array.Sectors) != mifare.Classic1KSectors {
		t.Fatalf("parsed array = layout %v sectors %d", array.Layout, len(array.Sectors))
	}
	if array.Sectors[0].KeyA == nil || array.Sectors[0].KeyB == nil ||
		*array.Sectors[0].KeyA != (nfc.Key{0x01, 0x02, 0x03, 0x04, 0x05, 0x06}) ||
		*array.Sectors[0].KeyB != (nfc.Key{0x11, 0x12, 0x13, 0x14, 0x15, 0x16}) {
		t.Fatalf("parsed sector 0 = %+v", array.Sectors[0])
	}

	fourK := complete4KDumpResult(t)
	data, err = workflow.EncodeKeyArray(fourK)
	if err != nil {
		t.Fatal(err)
	}
	if len(data) != mifare.Classic4KSectors*2*nfc.KeySize {
		t.Fatalf("4K key array size = %d", len(data))
	}
	array, err = workflow.ParseKeyArray(data)
	if err != nil {
		t.Fatal(err)
	}
	if array.Layout != mifare.Classic4K || len(array.Sectors) != mifare.Classic4KSectors {
		t.Fatalf("parsed 4K array = layout %v sectors %d", array.Layout, len(array.Sectors))
	}
	if array.Sectors[39].KeyA == nil || array.Sectors[39].KeyB == nil {
		t.Fatalf("parsed sector 39 = %+v", array.Sectors[39])
	}
}

func TestKeyArrayRejectsPartialAndWrongSizes(t *testing.T) {
	partial := completeDumpResult(t)
	partial.Dump.Blocks[9].KnownMask = 0
	if _, err := workflow.EncodeKeyArray(partial); !errors.Is(err, mifare.ErrIncompleteDump) {
		t.Fatalf("partial key array error = %v", err)
	}
	for _, size := range []int{0, 191, 193, 479, 481} {
		if _, err := workflow.ParseKeyArray(make([]byte, size)); err == nil {
			t.Fatalf("ParseKeyArray accepted %d bytes", size)
		}
	}
}

func TestProxmarkV3DropsATSAndSignatureWithWarning(t *testing.T) {
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "card-v3.json")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatProxmark, want); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var object map[string]any
	if err := json.Unmarshal(data, &object); err != nil {
		t.Fatal(err)
	}
	object["FileType"] = "mfc v3"
	card := object["Card"].(map[string]any)
	card["ATS"] = "6A"
	card["SIGNATURE"] = "01020304"
	encoded, _ := json.Marshal(object)
	if err := os.WriteFile(path, encoded, 0o600); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(load.Warnings) != 2 {
		t.Fatalf("warnings = %v", load.Warnings)
	}
	requireSameRaw(t, want, load.Result)
}

func TestExternalExportRejectsPartialDump(t *testing.T) {
	partial := completeDumpResult(t)
	partial.Dump.Blocks[9].KnownMask = 0
	partial.Dump.Blocks[9].Status = mifare.BlockReadFailed
	for _, format := range []workflow.DumpFormat{
		workflow.DumpFormatMCT, workflow.DumpFormatEML, workflow.DumpFormatMCTJSON, workflow.DumpFormatProxmark,
	} {
		path := filepath.Join(t.TempDir(), "partial.dump")
		if _, err := workflow.ExportDumpFile(path, format, partial); !errors.Is(err, mifare.ErrIncompleteDump) {
			t.Fatalf("%s export error = %v", format, err)
		}
		if _, err := os.Stat(path); !os.IsNotExist(err) {
			t.Fatalf("%s export created %s for a partial dump", format, path)
		}
	}
}

func TestMalformedMCTRejected(t *testing.T) {
	truncated := "+Sector: 0\n" + strings.Repeat("00000000000000000000000000000000\n", 4)
	if _, err := workflow.LoadDumpFile(writeFixture(t, "truncated.mct", []byte(truncated))); err == nil || !strings.Contains(err.Error(), "missing sector") {
		t.Fatalf("truncated MCT error = %v", err)
	}
	duplicate := strings.Replace(truncated, "+Sector: 0\n", "+Sector: 0\n", 1) + "+Sector: 0\n"
	if _, err := workflow.LoadDumpFile(writeFixture(t, "duplicate.mct", []byte(duplicate))); err == nil || !strings.Contains(err.Error(), "more than once") {
		t.Fatalf("duplicate MCT error = %v", err)
	}
}

func TestMalformedEMLRejected(t *testing.T) {
	short := strings.Repeat("00000000000000000000000000000000\n", 63)
	if _, err := workflow.LoadDumpFile(writeFixture(t, "short.eml", []byte(short))); err == nil {
		t.Fatal("short EML was accepted")
	}
	nonHex := strings.Repeat("ZZ000000000000000000000000000000\n", 64)
	if _, err := workflow.LoadDumpFile(writeFixture(t, "nonhex.eml", []byte(nonHex))); err == nil {
		t.Fatal("non-hex EML was accepted")
	}
}

func TestMalformedJSONRejected(t *testing.T) {
	duplicate := `{"FileType":"mfcard","blocks":{"0":"` + strings.Repeat("00", 16) + `","0":"` + strings.Repeat("11", 16) + `"}}`
	if _, err := workflow.LoadDumpFile(writeFixture(t, "duplicate.json", []byte(duplicate))); err == nil || !strings.Contains(err.Error(), "duplicate key") {
		t.Fatalf("duplicate JSON error = %v", err)
	}
	unknown := `{"FileType":"mfu","blocks":{}}`
	if _, err := workflow.LoadDumpFile(writeFixture(t, "unknown.json", []byte(unknown))); err == nil || !strings.Contains(err.Error(), "unsupported JSON dump FileType") {
		t.Fatalf("unknown FileType error = %v", err)
	}
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "truncated.json")
	if err := workflow.SaveDumpProject(path, want); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(path)
	if err := os.WriteFile(path, data[:len(data)/2], 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := workflow.LoadDumpFile(path); err == nil {
		t.Fatal("truncated JSON was accepted")
	}
}

func TestRawImportRejectsBadTrailerAndBCC(t *testing.T) {
	want := completeDumpResult(t)
	raw := dumpRawBytes(t, want)
	raw[3*mifare.BlockSize+6] = 0
	if _, err := workflow.LoadDumpFile(writeFixture(t, "bad-trailer.bin", raw)); err == nil {
		t.Fatal("raw import accepted invalid access bits")
	}

	path := filepath.Join(t.TempDir(), "bad-bcc.json")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatProxmark, want); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(path)
	var object map[string]any
	if err := json.Unmarshal(data, &object); err != nil {
		t.Fatal(err)
	}
	blocks := object["blocks"].(map[string]any)
	block0 := blocks["0"].(string)
	blocks["0"] = block0[:8] + "00" + block0[10:]
	encoded, _ := json.Marshal(object)
	if err := os.WriteFile(path, encoded, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := workflow.LoadDumpFile(path); err == nil || !strings.Contains(err.Error(), "BCC") {
		t.Fatalf("bad BCC error = %v", err)
	}
}

func TestProxmarkSectorKeyConflictRejected(t *testing.T) {
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "conflict.json")
	if _, err := workflow.ExportDumpFile(path, workflow.DumpFormatProxmark, want); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(path)
	var object map[string]any
	if err := json.Unmarshal(data, &object); err != nil {
		t.Fatal(err)
	}
	sectorKeys := object["SectorKeys"].(map[string]any)
	sectorKeys["0"].(map[string]any)["KeyA"] = "000000000000"
	encoded, _ := json.Marshal(object)
	if err := os.WriteFile(path, encoded, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := workflow.LoadDumpFile(path); err == nil || !strings.Contains(err.Error(), "does not match the trailer") {
		t.Fatalf("sector key conflict error = %v", err)
	}
}

func TestOversizedDumpRejected(t *testing.T) {
	path := writeFixture(t, "huge.bin", make([]byte, 2<<20))
	if _, err := workflow.LoadDumpFile(path); err == nil || !strings.Contains(err.Error(), "too large") {
		t.Fatalf("oversized dump error = %v", err)
	}
}

func TestRawWithoutSidecarWarnsAndSidecarCanBeExported(t *testing.T) {
	want := completeDumpResult(t)
	path := filepath.Join(t.TempDir(), "card.bin")
	if err := os.WriteFile(path, dumpRawBytes(t, want), 0o600); err != nil {
		t.Fatal(err)
	}
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(load.Warnings) != 1 || len(load.Result.Card.UID) != 0 {
		t.Fatalf("raw load = warnings %v uid %x", load.Warnings, load.Result.Card.UID)
	}
	if _, err := workflow.SaveDumpSidecar(path, want); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path + ".nfcx.json"); err != nil {
		t.Fatal(err)
	}
	load, err = workflow.LoadDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(load.Warnings) != 0 || string(load.Result.Card.UID) != string(want.Card.UID) {
		t.Fatalf("sidecar load = warnings %v uid %x", load.Warnings, load.Result.Card.UID)
	}
}
