package cli_test

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/cli"
	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	runtimebundle "github.com/YangtseSu/nfc-foundry/internal/runtime"
)

const testConnString = "mock:classic-reader"

const cliExitUsage = 2

type fakeClassic struct {
	mu     sync.Mutex
	card   nfc.CardInfo
	blocks [64][nfc.BlockSize]byte
}

func newFakeClassic() *fakeClassic {
	card := nfc.CardInfo{
		UID:  []byte{0x04, 0x11, 0x22, 0x33},
		ATQA: [2]byte{0x00, 0x04},
		SAK:  0x08,
	}
	fake := &fakeClassic{card: card}
	copy(fake.blocks[0][0:4], card.UID)
	fake.blocks[0][4] = card.UID[0] ^ card.UID[1] ^ card.UID[2] ^ card.UID[3]
	for sector := range 16 {
		trailer := sector*4 + 3
		copy(fake.blocks[trailer][0:6], []byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF})
		fake.blocks[trailer][6] = 0xFF
		fake.blocks[trailer][7] = 0x07
		fake.blocks[trailer][8] = 0x80
		fake.blocks[trailer][9] = 0x69
		copy(fake.blocks[trailer][10:16], []byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF})
	}
	return fake
}

func (f *fakeClassic) reader() *mock.Reader {
	return mock.NewReaderWithFuncs(mock.ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			return f.card.Clone(), nil
		},
		Authenticate: func(context.Context, byte, nfc.KeyType, nfc.Key) error {
			return nil
		},
		ReadBlock: func(_ context.Context, block byte) ([nfc.BlockSize]byte, error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			if int(block) >= len(f.blocks) {
				return [nfc.BlockSize]byte{}, nfc.ErrInvalidArgument
			}
			return f.blocks[block], nil
		},
		WriteBlock: func(_ context.Context, block byte, data [nfc.BlockSize]byte) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			if int(block) >= len(f.blocks) {
				return nfc.ErrInvalidArgument
			}
			f.blocks[block] = data
			return nil
		},
		WriteTrailer: func(_ context.Context, block byte, data [nfc.BlockSize]byte) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			if int(block) >= len(f.blocks) {
				return nfc.ErrInvalidArgument
			}
			f.blocks[block] = data
			return nil
		},
		WriteManufacturer: func(_ context.Context, data [nfc.BlockSize]byte) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			f.blocks[0] = data
			f.card.UID = append([]byte(nil), data[0:4]...)
			return nil
		},
	})
}

func (f *fakeClassic) block(number int) [nfc.BlockSize]byte {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.blocks[number]
}

func (f *fakeClassic) uid() string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return strings.ToUpper(hex.EncodeToString(f.card.UID))
}

type testEnvelope struct {
	OK       bool            `json:"ok"`
	Command  string          `json:"command"`
	Device   string          `json:"device"`
	Card     *app.CardDTO    `json:"card"`
	Warnings []string        `json:"warnings"`
	Error    *testError      `json:"error"`
	Result   json.RawMessage `json:"result"`
}

type testError struct {
	Code     string `json:"code"`
	Message  string `json:"message"`
	ExitCode int    `json:"exitCode"`
}

type testHarness struct {
	t      *testing.T
	deps   cli.Dependencies
	keys   string
	dir    string
	stdout bytes.Buffer
	stderr bytes.Buffer
}

func newHarness(t *testing.T, fake *fakeClassic) *testHarness {
	t.Helper()
	dir := t.TempDir()
	keyPath := filepath.Join(dir, "keys.json")
	lockDir := filepath.Join(dir, "locks")
	backupDir := filepath.Join(dir, "uid-backups")
	locator := runtimebundle.NewLocator(filepath.Join(dir, "runtime"))
	enumerator := mock.NewEnumerator([]nfc.DeviceInfo{{Name: "Mock Classic reader", ConnString: testConnString}}, nil)
	reader := fake.reader()
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
			Locator:           locator,
			KeyStorePath:      keyPath,
			UIDBackupRoot:     backupDir,
			ClassicBackupRoot: filepath.Join(dir, "classic-backups"),
		})
	}}
	return harness
}

func (h *testHarness) run(args ...string) (int, testEnvelope) {
	h.t.Helper()
	h.stdout.Reset()
	h.stderr.Reset()
	code := cli.Run(context.Background(), args, &h.stdout, &h.stderr, h.deps)
	var envelope testEnvelope
	if h.stdout.Len() != 0 && (contains(args, "--json") || contains(args, "--jsonl")) {
		if err := json.Unmarshal(h.stdout.Bytes(), &envelope); err != nil {
			h.t.Fatalf("decode JSON output %q: %v (stderr: %s)", h.stdout.String(), err, h.stderr.String())
		}
	}
	return code, envelope
}

func contains(values []string, wanted string) bool {
	for _, value := range values {
		if value == wanted {
			return true
		}
	}
	return false
}

func TestListJSONEnumeratesReaders(t *testing.T) {
	harness := newHarness(t, newFakeClassic())
	code, envelope := harness.run("list", "--json")
	if code != 0 || !envelope.OK || envelope.Command != "list" {
		t.Fatalf("code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var result struct {
		Devices []app.DeviceDTO `json:"devices"`
	}
	if err := json.Unmarshal(envelope.Result, &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Devices) != 1 || result.Devices[0].ConnString != testConnString {
		t.Fatalf("devices = %+v", result.Devices)
	}
}

func TestScanAndCardInfoExposeCardFacts(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	code, envelope := harness.run("scan", "--json")
	if code != 0 || !envelope.OK || envelope.Card == nil || envelope.Card.UID != "04 11 22 33" {
		t.Fatalf("scan code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	code, envelope = harness.run("card", "info", "--json")
	if code != 0 || !envelope.OK || envelope.Command != "card.info" || envelope.Card == nil {
		t.Fatalf("card info code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
}

func TestKeyscanPersistsAndDumpReadsInANewProcess(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	code, envelope := harness.run("classic", "keyscan", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("keyscan code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var keyscan struct {
		Verified int `json:"verified"`
		Total    int `json:"total"`
	}
	if err := json.Unmarshal(envelope.Result, &keyscan); err != nil {
		t.Fatal(err)
	}
	if keyscan.Verified != 32 || keyscan.Total != 32 {
		t.Fatalf("keyscan = %+v", keyscan)
	}

	dumpPath := filepath.Join(harness.dir, "card.bin")
	code, envelope = harness.run("classic", "dump", dumpPath, "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("dump code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var dump struct {
		Path             string `json:"path"`
		Sidecar          string `json:"sidecar"`
		Complete         bool   `json:"complete"`
		Blocks           int    `json:"blocks"`
		VerifiedKeySlots int    `json:"verifiedKeySlots"`
		KeySlots         int    `json:"keySlots"`
	}
	if err := json.Unmarshal(envelope.Result, &dump); err != nil {
		t.Fatal(err)
	}
	if !dump.Complete || dump.Blocks != 64 || dump.VerifiedKeySlots != 32 || dump.KeySlots != 32 {
		t.Fatalf("dump = %+v", dump)
	}
	raw, err := os.ReadFile(dumpPath)
	if err != nil || len(raw) != 1024 {
		t.Fatalf("raw dump length = %d, err = %v", len(raw), err)
	}
	if dump.Sidecar == "" {
		t.Fatal("dump metadata sidecar was not reported")
	}
	if _, err := os.Stat(dump.Sidecar); err != nil {
		t.Fatalf("metadata sidecar missing: %v", err)
	}
}

func TestDumpJSONPathStoresProject(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	if code, envelope := harness.run("classic", "keyscan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("keyscan failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}
	path := filepath.Join(harness.dir, "card.json")
	code, envelope := harness.run("classic", "dump", path, "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("dump: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var result struct {
		Sidecar string `json:"sidecar"`
	}
	if err := json.Unmarshal(envelope.Result, &result); err != nil {
		t.Fatal(err)
	}
	if result.Sidecar != "" {
		t.Fatalf("a .json dump must not create a raw sidecar: %+v", result)
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(raw) == 0 || raw[0] != '{' {
		t.Fatalf("a .json dump is not a JSON project: %q", raw[:min(len(raw), 40)])
	}
}

func TestClassicWriteRequiresYesAndRejectsProtectedBlocks(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	if code, envelope := harness.run("classic", "keyscan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("keyscan failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}

	code, envelope := harness.run("classic", "write", "--block", "8", "--data", "DEADBEEF00112233445566778899AABB", "--json")
	if code != cliExitUsage || envelope.Error == nil || envelope.Error.Code != "usage" {
		t.Fatalf("write without --yes: code=%d envelope=%+v", code, envelope)
	}
	code, envelope = harness.run("classic", "write", "--block", "8", "--data", "DEADBEEF00112233445566778899AABB", "--yes", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("write: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	if got := fake.block(8); got != [16]byte{0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB} {
		t.Fatalf("block 8 = % X", got)
	}
	code, _ = harness.run("classic", "write", "--block", "3", "--data", "00000000000000000000000000000000", "--yes", "--json")
	if code != 3 {
		t.Fatalf("trailer write code = %d, want validation", code)
	}
}

func TestClassicRestoreDryRunThenVerifiedRestore(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	if code, envelope := harness.run("classic", "keyscan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("keyscan failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}
	dumpPath := filepath.Join(harness.dir, "card.bin")
	if code, envelope := harness.run("classic", "dump", dumpPath, "--json"); code != 0 || !envelope.OK {
		t.Fatalf("dump failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}

	code, envelope := harness.run("classic", "restore", dumpPath, "--dry-run", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("restore dry run: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var dryRun struct {
		Preflight *app.WritePreflightDTO `json:"preflight"`
	}
	if err := json.Unmarshal(envelope.Result, &dryRun); err != nil {
		t.Fatal(err)
	}
	if dryRun.Preflight == nil || dryRun.Preflight.BackupPath == "" {
		t.Fatalf("dry-run preflight has no backup path: %+v", dryRun.Preflight)
	}
	if _, err := os.Stat(dryRun.Preflight.BackupPath); !os.IsNotExist(err) {
		t.Fatal("dry run created a pre-write backup")
	}
	if fake.block(8) != [16]byte{} {
		t.Fatal("dry run modified the card")
	}
	code, _ = harness.run("classic", "restore", dumpPath, "--json")
	if code != cliExitUsage {
		t.Fatalf("restore without --yes code = %d", code)
	}
	code, _ = harness.run("classic", "restore", dumpPath, "--yes", "--json")
	if code != cliExitUsage {
		t.Fatalf("restore without --confirm-trailers code = %d", code)
	}
	code, envelope = harness.run("classic", "restore", dumpPath, "--yes", "--confirm-trailers", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("restore: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var restore struct {
		Preflight *app.WritePreflightDTO `json:"preflight"`
		Restore   *app.RestoreResultDTO  `json:"restore"`
	}
	if err := json.Unmarshal(envelope.Result, &restore); err != nil {
		t.Fatal(err)
	}
	if restore.Restore == nil || len(restore.Restore.VerifiedBlocks) == 0 {
		t.Fatalf("restore result = %+v", restore)
	}
	if restore.Preflight == nil || restore.Preflight.BackupPath == "" {
		t.Fatalf("restore preflight has no backup path: %+v", restore.Preflight)
	}
	backup, err := os.Stat(restore.Preflight.BackupPath)
	if err != nil {
		t.Fatalf("pre-write backup is missing: %v", err)
	}
	if backup.Mode().Perm()&0o077 != 0 {
		t.Fatalf("pre-write backup permissions = %v", backup.Mode().Perm())
	}
}

func TestKeysListHidesValuesUnlessRequested(t *testing.T) {
	harness := newHarness(t, newFakeClassic())
	code, envelope := harness.run("keys", "list", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("keys list: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	if bytes.Contains(envelope.Result, []byte("FFFFFFFFFFFF")) || bytes.Contains(envelope.Result, []byte(`"value"`)) {
		t.Fatalf("key values leaked by default: %s", envelope.Result)
	}
	code, envelope = harness.run("keys", "list", "--show-keys", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("keys list --show-keys: code=%d envelope=%+v", code, envelope)
	}
	if !bytes.Contains(envelope.Result, []byte("FFFFFFFFFFFF")) {
		t.Fatalf("key values were not requested: %s", envelope.Result)
	}
}

func TestUIDSetDryRunThenVerifiedWrite(t *testing.T) {
	fake := newFakeClassic()
	harness := newHarness(t, fake)
	if code, envelope := harness.run("classic", "keyscan", "--json"); code != 0 || !envelope.OK {
		t.Fatalf("keyscan failed: code=%d %+v %s", code, envelope, harness.stderr.String())
	}
	code, envelope := harness.run("uid", "set", "04AABBCC", "--dry-run", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("uid dry run: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	var uidDryRun struct {
		BackupPath string `json:"backupPath"`
	}
	if err := json.Unmarshal(envelope.Result, &uidDryRun); err != nil {
		t.Fatal(err)
	}
	if uidDryRun.BackupPath == "" {
		t.Fatal("UID dry run has no pre-write backup path")
	}
	if _, err := os.Stat(uidDryRun.BackupPath); !os.IsNotExist(err) {
		t.Fatal("UID dry run created a pre-write backup")
	}
	if fake.uid() != "04112233" {
		t.Fatal("dry run changed the UID")
	}
	code, _ = harness.run("uid", "set", "04AABBCC", "--yes", "--json")
	if code != cliExitUsage {
		t.Fatalf("uid without confirmation code = %d", code)
	}
	code, envelope = harness.run("uid", "set", "04AABBCC", "--yes", "--confirm-uid", "04AABBCC", "--json")
	if code != 0 || !envelope.OK {
		t.Fatalf("uid write: code=%d envelope=%+v stderr=%s", code, envelope, harness.stderr.String())
	}
	if fake.uid() != "04AABBCC" {
		t.Fatalf("UID = %s, want 04AABBCC", fake.uid())
	}
	var result struct {
		BackupPath string                 `json:"backupPath"`
		Result     *app.UIDWriteResultDTO `json:"result"`
	}
	if err := json.Unmarshal(envelope.Result, &result); err != nil {
		t.Fatal(err)
	}
	if result.Result == nil || !result.Result.Verified {
		t.Fatalf("UID result = %+v", result)
	}
	if result.BackupPath == "" {
		t.Fatal("UID write has no pre-write backup path")
	}
	if _, err := os.Stat(result.BackupPath); err != nil {
		t.Fatalf("UID pre-write backup is missing: %v", err)
	}
}

func TestRecoverAndUnknownCommandsFailWithUsage(t *testing.T) {
	harness := newHarness(t, newFakeClassic())
	code, envelope := harness.run("recover", "nested", "--json")
	if code != cliExitUsage || envelope.Error == nil || envelope.Error.Code != "usage" {
		t.Fatalf("recover without --yes: code=%d envelope=%+v", code, envelope)
	}
	code, envelope = harness.run("bogus", "--json")
	if code != cliExitUsage || envelope.Error == nil {
		t.Fatalf("unknown command: code=%d envelope=%+v", code, envelope)
	}
}

func TestJSONLStreamsProgressAndOneResult(t *testing.T) {
	harness := newHarness(t, newFakeClassic())
	harness.stdout.Reset()
	harness.stderr.Reset()
	code := cli.Run(context.Background(), []string{"classic", "keyscan", "--jsonl"}, &harness.stdout, &harness.stderr, harness.deps)
	if code != 0 {
		t.Fatalf("keyscan jsonl code=%d stderr=%s", code, harness.stderr.String())
	}
	lines := strings.Split(strings.TrimSpace(harness.stdout.String()), "\n")
	var progress, results int
	var resultOK bool
	for _, line := range lines {
		var header struct {
			Type string `json:"type"`
			OK   bool   `json:"ok"`
		}
		if err := json.Unmarshal([]byte(line), &header); err != nil {
			t.Fatalf("decode line %q: %v", line, err)
		}
		switch header.Type {
		case "progress":
			progress++
		case "result":
			results++
			resultOK = header.OK
		}
	}
	if progress == 0 || results != 1 || !resultOK {
		t.Fatalf("jsonl progress=%d results=%d ok=%t output=%s", progress, results, resultOK, harness.stdout.String())
	}
}
