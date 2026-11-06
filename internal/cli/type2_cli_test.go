package cli_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/cli"
	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	runtimebundle "github.com/YangtseSu/nfc-foundry/internal/runtime"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

func newType2Harness(t *testing.T, tag *mock.Type2Tag) (*testHarness, *mock.Reader) {
	t.Helper()
	dir := t.TempDir()
	keyPath := filepath.Join(dir, "keys.json")
	lockDir := filepath.Join(dir, "locks")
	uidBackupDir := filepath.Join(dir, "uid-backups")
	type2BackupDir := filepath.Join(dir, "type2-backups")
	locator := runtimebundle.NewLocator(filepath.Join(dir, "runtime"))
	enumerator := mock.NewEnumerator([]nfc.DeviceInfo{{Name: "Mock Type 2 reader", ConnString: testConnString}}, nil)
	reader := mock.NewType2Reader(tag)
	harness := &testHarness{t: t, keys: keyPath, dir: dir}
	harness.deps = cli.Dependencies{NewService: func(emitter app.EventEmitter) *app.Service {
		return app.NewServiceWithOptions(app.Options{
			Emitter:    emitter,
			Enumerator: enumerator,
			NewReader:  func() nfc.Reader { return reader },
			DeviceConfig: device.Config{
				PollInterval: 2 * time.Millisecond, PollTimeout: 50 * time.Millisecond,
				OpenTimeout: time.Second, RemovalMisses: 2,
				ReconnectBackoffs: []time.Duration{time.Millisecond}, LockDir: lockDir,
			},
			Locator:         locator,
			KeyStorePath:    keyPath,
			UIDBackupRoot:   uidBackupDir,
			Type2BackupRoot: type2BackupDir,
		})
	}}
	return harness, reader
}

func decodeResult(t *testing.T, envelope testEnvelope, target any) {
	t.Helper()
	if err := json.Unmarshal(envelope.Result, target); err != nil {
		t.Fatalf("decode result %s: %v", envelope.Result, err)
	}
}

func TestCLIType2InfoReadAndPageWrite(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	harness, _ := newType2Harness(t, tag)

	code, envelope := harness.run("type2", "info", "--json")
	if code != 0 {
		t.Fatalf("info exit %d: %s", code, harness.stderr.String())
	}
	var info app.Type2InfoDTO
	decodeResult(t, envelope, &info)
	if info.Product != "ntag213" || info.TotalPages != 45 || info.UserBytes != 144 {
		t.Fatalf("info = %+v", info)
	}

	code, envelope = harness.run("type2", "read", "--json")
	if code != 0 {
		t.Fatalf("read exit %d: %s", code, harness.stderr.String())
	}
	var dump app.Type2DumpDTO
	decodeResult(t, envelope, &dump)
	if !dump.Loaded || len(dump.Pages) != 45 || !dump.CC.Valid || dump.CC.Capacity != 144 {
		t.Fatalf("dump = %+v", dump)
	}

	code, envelope = harness.run("type2", "read", "--page", "3", "--json")
	if code != 0 {
		t.Fatalf("page read exit %d: %s", code, harness.stderr.String())
	}
	var page struct {
		Page app.Type2PageDTO `json:"page"`
	}
	decodeResult(t, envelope, &page)
	if page.Page.Page != 3 || page.Page.Region != "capability" || page.Page.Hex != "E1101200" {
		t.Fatalf("page = %+v", page.Page)
	}

	code, envelope = harness.run("type2", "write", "--page", "4", "--data", "DEADBEEF", "--yes", "--json")
	if code != 0 {
		t.Fatalf("write exit %d: %s", code, harness.stderr.String())
	}
	var write app.Type2WriteResultDTO
	decodeResult(t, envelope, &write)
	if write.Backup == "" || len(write.Written) != 1 || len(write.Verified) != 1 {
		t.Fatalf("write = %+v", write)
	}
	page4, _ := tag.Page(4)
	if page4 != [4]byte{0xDE, 0xAD, 0xBE, 0xEF} {
		t.Fatalf("page 4 = % X", page4)
	}
}

func TestCLIType2WriteGuards(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	harness, _ := newType2Harness(t, tag)

	code, _ := harness.run("type2", "write", "--page", "4", "--data", "DEADBEEF", "--json")
	if code != cliExitUsage {
		t.Fatalf("missing --yes exit = %d", code)
	}
	code, _ = harness.run("type2", "write", "--page", "41", "--data", "04000000", "--yes", "--json")
	if code != 3 {
		t.Fatalf("protected write exit = %d (%s)", code, harness.stderr.String())
	}
	page41, _ := tag.Page(41)
	if page41 != [4]byte{0x04, 0x00, 0x00, 0xFF} {
		t.Fatalf("protected page changed: % X", page41)
	}

	code, envelope := harness.run("type2", "write", "--page", "4", "--data", "CAFEBABE", "--dry-run", "--json")
	if code != 0 {
		t.Fatalf("dry run exit = %d: %s", code, harness.stderr.String())
	}
	var write app.Type2WriteResultDTO
	decodeResult(t, envelope, &write)
	if len(write.Planned) != 1 || write.Planned[0] != 4 || len(write.Written) != 0 {
		t.Fatalf("dry run = %+v", write)
	}
	page4, _ := tag.Page(4)
	if page4 != [4]byte{} {
		t.Fatalf("dry run modified page 4: % X", page4)
	}
}

func TestCLINDEFWriteAndRead(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG215, nil)
	harness, _ := newType2Harness(t, tag)
	message := "D101085402656E68656C6C6F"

	code, envelope := harness.run("ndef", "write", "--hex", message, "--yes", "--json")
	if code != 0 {
		t.Fatalf("ndef write exit %d: %s", code, harness.stderr.String())
	}
	var write app.Type2WriteResultDTO
	decodeResult(t, envelope, &write)
	if len(write.Written) == 0 || len(write.Verified) != len(write.Written) {
		t.Fatalf("ndef write = %+v", write)
	}

	code, envelope = harness.run("ndef", "read", "--json")
	if code != 0 {
		t.Fatalf("ndef read exit %d: %s", code, harness.stderr.String())
	}
	var read struct {
		NDEF *app.Type2NDEFDTO `json:"ndef"`
	}
	decodeResult(t, envelope, &read)
	if read.NDEF == nil || read.NDEF.Text != "hello" || read.NDEF.Raw != message {
		t.Fatalf("ndef read = %+v", read.NDEF)
	}
}

func TestCLINDEFReadRejectsUnformattedTag(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductUltralight, nil)
	harness, _ := newType2Harness(t, tag)
	code, _ := harness.run("ndef", "read", "--json")
	if code != 3 {
		t.Fatalf("unformatted ndef read exit = %d (%s)", code, harness.stderr.String())
	}
	code, envelope := harness.run("type2", "read", "--json")
	if code != 0 {
		t.Fatalf("ultralight read exit = %d: %s", code, harness.stderr.String())
	}
	var dump app.Type2DumpDTO
	decodeResult(t, envelope, &dump)
	if dump.Info.Product != "mifare_ultralight" || dump.CC.Valid || dump.NDEFError == "" {
		t.Fatalf("ultralight dump = %+v", dump)
	}
}

func TestCLIType2DumpAndRestore(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG216, nil)
	harness, _ := newType2Harness(t, tag)
	path := filepath.Join(t.TempDir(), "tag.json")

	code, _ := harness.run("type2", "dump", path, "--json")
	if code != 0 {
		t.Fatalf("dump exit %d: %s", code, harness.stderr.String())
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), `"format": "nfc-foundry/type2"`) {
		t.Fatalf("dump file = %s", data)
	}

	tag.SetPage(4, [4]byte{1, 2, 3, 4})
	code, envelope := harness.run("type2", "restore", path, "--yes", "--json")
	if code != 0 {
		t.Fatalf("restore exit %d: %s", code, harness.stderr.String())
	}
	var restore app.Type2WriteResultDTO
	decodeResult(t, envelope, &restore)
	if restore.Backup == "" || len(restore.Written) == 0 {
		t.Fatalf("restore = %+v", restore)
	}
	page4, _ := tag.Page(4)
	if page4 != [4]byte{} {
		t.Fatalf("page 4 after restore = % X", page4)
	}

	tag.SetPage(4, [4]byte{9, 9, 9, 9})
	code, envelope = harness.run("type2", "restore", path, "--yes", "--dry-run", "--json")
	if code != 0 {
		t.Fatalf("dry restore exit %d: %s", code, harness.stderr.String())
	}
	var dry app.Type2WriteResultDTO
	decodeResult(t, envelope, &dry)
	if len(dry.Planned) != 1 || dry.Planned[0] != 4 || len(dry.Written) != 0 {
		t.Fatalf("dry restore = %+v", dry)
	}
	page4, _ = tag.Page(4)
	if page4 != [4]byte{9, 9, 9, 9} {
		t.Fatalf("dry restore modified page 4: % X", page4)
	}
}

func TestCLIType2ProtectedRestoreRequiresConfirmation(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	harness, _ := newType2Harness(t, tag)
	path := filepath.Join(t.TempDir(), "tag.json")
	if code, _ := harness.run("type2", "dump", path, "--json"); code != 0 {
		t.Fatalf("dump exit %d", code)
	}
	code, _ := harness.run("type2", "restore", path, "--yes", "--protected", "--json")
	if code != cliExitUsage {
		t.Fatalf("protected restore without confirmation exit = %d", code)
	}
	code, _ = harness.run("type2", "restore", path, "--yes", "--protected", "--confirm-protected", "--json")
	if code != 0 {
		t.Fatalf("protected restore exit = %d: %s", code, harness.stderr.String())
	}
}

func TestCLIType2ReadMasksPasswordPages(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	harness, _ := newType2Harness(t, tag)
	code, envelope := harness.run("type2", "read", "--all", "--json")
	if code != 0 {
		t.Fatalf("read exit %d: %s", code, harness.stderr.String())
	}
	var masked app.Type2DumpDTO
	decodeResult(t, envelope, &masked)
	if masked.Pages[43].Hex != "--------" || masked.Pages[44].Hex != "--------" {
		t.Fatalf("password pages are not masked: %+v", masked.Pages[43:45])
	}
	if masked.Pages[4].Hex == "--------" {
		t.Fatal("user page was masked")
	}
	code, envelope = harness.run("type2", "read", "--all", "--show-keys", "--json")
	if code != 0 {
		t.Fatalf("show-keys read exit %d: %s", code, harness.stderr.String())
	}
	var visible app.Type2DumpDTO
	decodeResult(t, envelope, &visible)
	if visible.Pages[43].Hex != "FFFFFFFF" {
		t.Fatalf("show-keys page 43 = %q", visible.Pages[43].Hex)
	}
}

func TestCLIType2NAKAndPasswordFailuresAreOperationErrors(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	harness, _ := newType2Harness(t, tag)
	tag.FailWritePage = 4
	code, _ := harness.run("type2", "write", "--page", "4", "--data", "DEADBEEF", "--yes", "--json")
	if code != 1 {
		t.Fatalf("NAK write exit = %d (%s)", code, harness.stderr.String())
	}
	tag.FailWritePage = -1
	tag.SetPassword([4]byte{1, 2, 3, 4}, [2]byte{0xAA, 0xBB})
	code, _ = harness.run("type2", "read", "--pwd", "00000000", "--json")
	if code != 1 {
		t.Fatalf("wrong password exit = %d (%s)", code, harness.stderr.String())
	}
}

func TestCLIType2VerificationFailureReportsBackup(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	tag.CorruptWritePage = 4
	harness, _ := newType2Harness(t, tag)
	code, envelope := harness.run("type2", "write", "--page", "4", "--data", "DEADBEEF", "--yes", "--json")
	if code != 1 {
		t.Fatalf("verification failure exit = %d", code)
	}
	if envelope.Error == nil || !strings.Contains(envelope.Error.Message, "full-image backup") {
		t.Fatalf("error did not report the backup: %+v (stderr: %s)", envelope.Error, harness.stderr.String())
	}
}
