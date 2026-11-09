package cli_test

import (
	"encoding/hex"
	"encoding/json"
	"path/filepath"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func dumpClassicProject(t *testing.T, harness *testHarness, fake *fakeClassic, name string) string {
	t.Helper()
	if code, envelope := harness.run("classic", "keyscan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("keyscan failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}
	path := filepath.Join(harness.dir, name)
	if code, envelope := harness.run("classic", "dump", path, "--json"); code != 0 || !envelope.OK {
		t.Fatalf("dump failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}
	return path
}

func TestClassicDiffReportsChangedTrailerAndIdentity(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	before := dumpClassicProject(t, harness, fake, "before.json")

	fake.mu.Lock()
	fake.blocks[1][0] = 0xde
	fake.blocks[3][0] = 0xa0
	fake.mu.Unlock()
	after := dumpClassicProject(t, harness, fake, "after.json")

	code, envelope := harness.run("classic", "diff", before, after, "--json")
	if code != 0 || !envelope.OK || envelope.Command != "classic.diff" {
		t.Fatalf("diff: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var diff struct {
		ChangedBlocks  int `json:"changedBlocks"`
		TrailerChanges int `json:"trailerChanges"`
		Blocks         []struct {
			Block   int    `json:"block"`
			State   string `json:"state"`
			Trailer *struct {
				KeyAChanged bool `json:"keyAChanged"`
			} `json:"trailer"`
		} `json:"blocks"`
	}
	if err := json.Unmarshal(envelope.Result, &diff); err != nil {
		t.Fatal(err)
	}
	if diff.ChangedBlocks != 2 || diff.TrailerChanges != 1 {
		t.Fatalf("diff = %+v", diff)
	}
	if diff.Blocks[1].State != "changed" || diff.Blocks[3].Trailer == nil || !diff.Blocks[3].Trailer.KeyAChanged {
		t.Fatalf("diff blocks = %+v", diff.Blocks)
	}
}

func TestClassicValueSetWritesEncodedValueBlock(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	path := dumpClassicProject(t, harness, fake, "value.json")
	code, envelope := harness.run("classic", "value", "set", path, "--block", "1", "--value", "100", "--address", "1", "--yes", "--json")
	if code != 0 || !envelope.OK || envelope.Command != "classic.value.set" {
		t.Fatalf("value set: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	want := [nfc.BlockSize]byte{0x64, 0, 0, 0, 0x9b, 0xff, 0xff, 0xff, 0x64, 0, 0, 0, 1, 0xfe, 1, 0xfe}
	if got := fake.block(1); got != want {
		t.Fatalf("block 1 = %x, want %x", got, want)
	}
	code, _ = harness.run("classic", "value", "set", path, "--block", "1", "--value", "100", "--json")
	if code != cliExitUsage {
		t.Fatalf("value set without --yes code = %d", code)
	}
}

func TestClassicTrailerSetRequiresTypedConfirmation(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	path := dumpClassicProject(t, harness, fake, "trailer.json")
	code, _ := harness.run("classic", "trailer", "set", path, "--block", "3",
		"--key-a", "FFFFFFFFFFFF", "--key-b", "A0A1A2A3A4A5", "--access", "000,000,000,001", "--yes", "--json")
	if code != cliExitUsage {
		t.Fatalf("trailer set without --confirm code = %d", code)
	}
	code, envelope := harness.run("classic", "trailer", "set", path, "--block", "3",
		"--key-a", "FFFFFFFFFFFF", "--key-b", "A0A1A2A3A4A5", "--access", "000,000,000,001",
		"--confirm", "3", "--yes", "--json")
	if code != 0 || !envelope.OK || envelope.Command != "classic.trailer.set" {
		t.Fatalf("trailer set: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	if got := fake.block(3); hex.EncodeToString(got[0:6]) != "ffffffffffff" || hex.EncodeToString(got[10:16]) != "a0a1a2a3a4a5" {
		t.Fatalf("trailer = %x", got)
	}
}

func TestClassicCloneRestoresCompleteImage(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	if code, envelope := harness.run("classic", "keyscan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("keyscan failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}
	source := dumpClassicProject(t, harness, fake, "source.json")
	fake.mu.Lock()
	fake.blocks[1] = [nfc.BlockSize]byte{0xde, 0xad}
	fake.blocks[3][0] = 0x99
	fake.mu.Unlock()

	code, envelope := harness.run("classic", "clone", source, "--yes", "--confirm", "04112233", "--json")
	if code != 0 || !envelope.OK || envelope.Command != "classic.clone" {
		t.Fatalf("clone: code=%d envelope=%+v error=%+v stderr=%s", code, envelope, envelope.Error, harness.stderr.String())
	}
	if got := fake.block(1); got != [nfc.BlockSize]byte{} {
		t.Fatalf("cloned block 1 = %x", got)
	}
	load, err := workflow.LoadDumpFile(source)
	if err != nil {
		t.Fatal(err)
	}
	for index := range load.Result.Dump.Blocks {
		if want := load.Result.Dump.Blocks[index].Data; fake.block(index) != want {
			t.Fatalf("cloned block %d = %x, want %x", index, fake.block(index), want)
		}
	}
}

func TestClassicFormatResetsTrailers(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	if code, envelope := harness.run("classic", "keyscan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("keyscan failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}
	fake.mu.Lock()
	fake.blocks[1] = [nfc.BlockSize]byte{0xde, 0xad}
	fake.blocks[3][0] = 0x99
	fake.mu.Unlock()

	code, envelope := harness.run("classic", "format", "--yes", "--confirm", "FORMAT", "--json")
	if code != 0 || !envelope.OK || envelope.Command != "classic.format" {
		t.Fatalf("format: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	if got := fake.block(1); got != [nfc.BlockSize]byte{} {
		t.Fatalf("formatted block 1 = %x", got)
	}
	want := [nfc.BlockSize]byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x07, 0x80, 0x69, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff}
	if got := fake.block(3); got != want {
		t.Fatalf("formatted trailer = %x", got)
	}
	if fake.uid() != "04112233" {
		t.Fatalf("format changed the UID: %s", fake.uid())
	}
}
