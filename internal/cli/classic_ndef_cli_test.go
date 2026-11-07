package cli_test

import (
	"encoding/hex"
	"os"
	"path/filepath"
	"testing"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

func (f *fakeClassic) formatNDEF(t *testing.T, sectors []int, message []byte) {
	t.Helper()
	f.mu.Lock()
	defer f.mu.Unlock()
	mad := mifare.MAD{Present: true, Version: 1, MultiApplication: true, AIDs: make(map[int]uint16)}
	for _, sector := range sectors {
		mad.AIDs[sector] = mifare.NDEFAID
	}
	encoded, err := mad.Blocks()
	if err != nil {
		t.Fatal(err)
	}
	f.blocks[1], f.blocks[2] = encoded.Sector0[0], encoded.Sector0[1]
	f.blocks[3] = [mifare.BlockSize]byte{0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x78, 0x77, 0x88, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	areaBlocks, err := mifare.NDEFAreaBlocks(mifare.Classic1K, sectors)
	if err != nil {
		t.Fatal(err)
	}
	if message == nil {
		message, _ = type2.EncodeTextMessage("hello", "en")
	}
	area, err := type2.BuildNDEFArea(len(areaBlocks)*mifare.BlockSize, message)
	if err != nil {
		t.Fatal(err)
	}
	for index, block := range areaBlocks {
		copy(f.blocks[block][:], area[index*mifare.BlockSize:(index+1)*mifare.BlockSize])
	}
	for _, sector := range sectors {
		trailer := sector*4 + 3
		f.blocks[trailer] = [mifare.BlockSize]byte{0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7, 0x7F, 0x07, 0x88, 0x40, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	}
}

func TestCLIClassicNDEFReadWrite(t *testing.T) {
	fake := newFakeClassic()
	fake.formatNDEF(t, []int{1}, nil)
	harness := newHarness(t, fake)

	code, envelope := harness.run("ndef", "read", "--json")
	if code != 0 {
		t.Fatalf("ndef read exit %d: %s", code, harness.stderr.String())
	}
	var read struct {
		Family  string             `json:"family"`
		MAD     *app.ClassicMADDTO `json:"mad"`
		NDEF    *app.Type2NDEFDTO  `json:"ndef"`
		Sectors []int              `json:"sectors"`
	}
	decodeResult(t, envelope, &read)
	if read.Family != "classic" || read.MAD == nil || !read.MAD.Present || read.NDEF == nil || read.NDEF.Text != "hello" {
		t.Fatalf("read = %+v", read)
	}

	message, err := type2.EncodeTextMessage("updated", "en")
	if err != nil {
		t.Fatal(err)
	}
	raw := hex.EncodeToString(message)
	code, envelope = harness.run("ndef", "write", "--hex", raw, "--yes", "--json")
	if code != 0 {
		t.Fatalf("ndef write exit %d: %s", code, harness.stderr.String())
	}
	var write app.ClassicNDEFWriteResultDTO
	decodeResult(t, envelope, &write)
	if write.Backup == "" || len(write.Written) == 0 || len(write.Verified) != len(write.Written) {
		t.Fatalf("write = %+v", write)
	}

	code, envelope = harness.run("ndef", "read", "--json")
	if code != 0 {
		t.Fatalf("ndef re-read exit %d: %s", code, harness.stderr.String())
	}
	decodeResult(t, envelope, &read)
	if read.NDEF == nil || read.NDEF.Text != "updated" {
		t.Fatalf("re-read = %+v", read.NDEF)
	}
}

func TestCLIClassicNDEFFormat(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)

	keyFile := filepath.Join(harness.dir, "default-keys.txt")
	if err := os.WriteFile(keyFile, []byte("FFFFFFFFFFFF\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	code, envelope := harness.run("ndef", "format", "--sectors", "1-2", "--keys", keyFile, "--dry-run", "--json")
	if code != 0 {
		t.Fatalf("format dry-run exit %d: %s stdout=%s", code, harness.stderr.String(), harness.stdout.String())
	}
	var preview app.ClassicNDEFFormatPreviewDTO
	decodeResult(t, envelope, &preview)
	if !preview.Valid || len(preview.Sectors) != 2 || preview.Capacity != 96 {
		t.Fatalf("preview = %+v", preview)
	}

	code, _ = harness.run("ndef", "format", "--sectors", "1-2", "--keys", keyFile, "--json")
	if code != cliExitUsage {
		t.Fatalf("missing confirmations exit = %d", code)
	}

	code, envelope = harness.run("ndef", "format", "--sectors", "1-2", "--keys", keyFile, "--yes", "--confirm-format", "--json")
	if code != 0 {
		t.Fatalf("format exit %d: %s", code, harness.stderr.String())
	}
	var result app.ClassicNDEFWriteResultDTO
	decodeResult(t, envelope, &result)
	if !result.Formatted || result.Backup == "" {
		t.Fatalf("format result = %+v", result)
	}

	code, envelope = harness.run("ndef", "read", "--json")
	if code != 0 {
		t.Fatalf("ndef read exit %d: %s", code, harness.stderr.String())
	}
	var read struct {
		Family      string             `json:"family"`
		MAD         *app.ClassicMADDTO `json:"mad"`
		Application bool               `json:"application"`
		NDEF        *app.Type2NDEFDTO  `json:"ndef"`
	}
	decodeResult(t, envelope, &read)
	if read.Family != "classic" || read.MAD == nil || !read.MAD.Present || !read.Application || read.NDEF == nil || read.NDEF.Present {
		t.Fatalf("formatted read = %+v mad=%+v ndef=%+v", read, *read.MAD, *read.NDEF)
	}
}

func TestCLIClassicNDEFFormatGuards(t *testing.T) {
	harness := newHarness(t, newFakeClassic())
	code, _ := harness.run("ndef", "format", "--sectors", "16", "--dry-run", "--json")
	if code != cliExitUsage {
		t.Fatalf("sector 16 exit = %d", code)
	}
	code, _ = harness.run("ndef", "format", "--sectors", "3-1", "--dry-run", "--json")
	if code != cliExitUsage {
		t.Fatalf("descending range exit = %d", code)
	}
}
