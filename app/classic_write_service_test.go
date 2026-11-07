package app

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

type classicWriteFixture struct {
	mu       sync.Mutex
	card     nfc.CardInfo
	blocks   [64][nfc.BlockSize]byte
	writes   []int
	trailers []int
}

func newClassicWriteFixture() *classicWriteFixture {
	card := nfc.CardInfo{UID: []byte{0x04, 0x11, 0x22, 0x33}, ATQA: [2]byte{0x00, 0x04}, SAK: 0x08}
	fixture := &classicWriteFixture{card: card}
	copy(fixture.blocks[0][0:4], card.UID)
	fixture.blocks[0][4] = card.UID[0] ^ card.UID[1] ^ card.UID[2] ^ card.UID[3]
	for sector := range mifare.Classic1KSectors {
		trailer := sector*4 + 3
		copy(fixture.blocks[trailer][0:6], []byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF})
		fixture.blocks[trailer][6], fixture.blocks[trailer][7], fixture.blocks[trailer][8], fixture.blocks[trailer][9] = 0xFF, 0x07, 0x80, 0x69
		copy(fixture.blocks[trailer][10:16], []byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF})
	}
	return fixture
}

func (f *classicWriteFixture) reader() *mock.Reader {
	return mock.NewReaderWithFuncs(mock.ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			return f.card.Clone(), nil
		},
		Authenticate: func(context.Context, byte, nfc.KeyType, nfc.Key) error { return nil },
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
			f.writes = append(f.writes, int(block))
			f.blocks[block] = data
			return nil
		},
		WriteTrailer: func(_ context.Context, block byte, data [nfc.BlockSize]byte) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			if int(block) >= len(f.blocks) {
				return nfc.ErrInvalidArgument
			}
			f.trailers = append(f.trailers, int(block))
			f.blocks[block] = data
			return nil
		},
		WriteManufacturer: func(_ context.Context, data [nfc.BlockSize]byte) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			f.blocks[0] = data
			return nil
		},
	})
}

func (f *classicWriteFixture) setUID(uid []byte) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.card.UID = append([]byte(nil), uid...)
}

func (f *classicWriteFixture) writeCount() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.writes) + len(f.trailers)
}

func (f *classicWriteFixture) dumpResult(t *testing.T, service *Service) workflow.DumpResult {
	t.Helper()
	layout := mifare.Classic1K
	image, err := mifare.NewDump(layout)
	if err != nil {
		t.Fatal(err)
	}
	f.mu.Lock()
	blocks := f.blocks
	card := f.card.Clone()
	f.mu.Unlock()
	for index := range image.Blocks {
		image.Blocks[index].Data = blocks[index]
		image.Blocks[index].KnownMask = mifare.AllBytesKnown
		image.Blocks[index].Status = mifare.BlockRead
		if index%4 == 3 {
			image.Blocks[index].Status = mifare.BlockSynthetic
		}
	}
	key := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	keys := make([]workflow.VerifiedSectorKeys, mifare.Classic1KSectors)
	for sector := range keys {
		keys[sector] = workflow.VerifiedSectorKeys{Sector: sector, KeyA: &key}
	}
	return workflow.DumpResult{
		Dump: image, Card: card, Device: nfc.DeviceInfo{Name: "Mock Classic reader", ConnString: "mock:classic-write"},
		Keys: keys, StartedAt: time.Now().UTC(), FinishedAt: time.Now().UTC(),
	}
}

func newClassicWriteAppService(t *testing.T, backupRoot string) (*Service, *Bindings, *classicWriteFixture, *appEventRecorder, string) {
	t.Helper()
	dir := t.TempDir()
	if backupRoot == "" {
		backupRoot = filepath.Join(dir, "classic-backups")
	}
	fixture := newClassicWriteFixture()
	reader := fixture.reader()
	enumerator := mock.NewEnumerator([]nfc.DeviceInfo{{Name: "Mock Classic reader", ConnString: "mock:classic-write"}}, nil)
	recorder := newAppEventRecorder()
	service := NewServiceWithOptions(Options{
		Emitter:    recorder,
		Enumerator: enumerator,
		NewReader:  func() nfc.Reader { return reader },
		DeviceConfig: device.Config{
			PollInterval: 2 * time.Millisecond, PollTimeout: 50 * time.Millisecond,
			OpenTimeout: time.Second, RemovalMisses: 2,
			ReconnectBackoffs: []time.Duration{time.Millisecond}, LockDir: filepath.Join(dir, "locks"),
		},
		KeyStorePath:      filepath.Join(dir, "keys.json"),
		UIDBackupRoot:     filepath.Join(dir, "uid-backups"),
		Type2BackupRoot:   filepath.Join(dir, "type2-backups"),
		ClassicBackupRoot: backupRoot,
	})
	t.Cleanup(service.Shutdown)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := service.ConnectDevice("mock:classic-write"); err != nil {
		t.Fatal(err)
	}
	if _, err := service.WaitForCard(ctx); err != nil {
		t.Fatal(err)
	}
	result := fixture.dumpResult(t, service)
	path := filepath.Join(dir, "card.nfcx.json")
	if err := workflow.SaveDumpProject(path, result); err != nil {
		t.Fatal(err)
	}
	if _, err := service.LoadDumpFile(path, true); err != nil {
		t.Fatal(err)
	}
	seedVerifiedDumpKeys(t, service, fixture.card, result.Keys)
	return service, NewBindings(service), fixture, recorder, path
}

func seedVerifiedDumpKeys(t *testing.T, service *Service, card nfc.CardInfo, entries []workflow.VerifiedSectorKeys) {
	t.Helper()
	cardID := keys.CardID(card)
	for _, sector := range entries {
		for _, candidate := range []struct {
			kind nfc.KeyType
			key  *nfc.Key
		}{{nfc.KeyTypeA, sector.KeyA}, {nfc.KeyTypeB, sector.KeyB}} {
			if candidate.key == nil {
				continue
			}
			if _, err := service.keyStore.MergeVerified(cardID, sector.Sector, candidate.kind, *candidate.key, keys.SourceReadContext); err != nil {
				t.Fatal(err)
			}
		}
	}
}

func TestClassicBlockPreflightWritesOnlySelectedBlock(t *testing.T) {
	_, bindings, fixture, recorder, _ := newClassicWriteAppService(t, "")
	preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "block", Blocks: []int{1}})
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.Valid || preflight.BackupPath != "" || len(preflight.Blocks) != 1 || preflight.Blocks[0] != 1 {
		t.Fatalf("preflight = %+v", preflight)
	}
	if _, err := bindings.StartWrite(WriteStartRequestDTO{Token: preflight.Token}); err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	fixture.mu.Lock()
	defer fixture.mu.Unlock()
	if len(fixture.writes) != 1 || fixture.writes[0] != 1 || len(fixture.trailers) != 0 {
		t.Fatalf("writes = %v trailers = %v", fixture.writes, fixture.trailers)
	}
}

func TestClassicBulkWriteBacksUpCurrentCardBeforeWriting(t *testing.T) {
	_, bindings, fixture, recorder, _ := newClassicWriteAppService(t, "")
	edit := "DEADBEEF00112233445566778899AABB"
	if _, err := bindings.EditBlockHex(1, edit); err != nil {
		t.Fatal(err)
	}
	preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "changes"})
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.Valid || preflight.BackupPath == "" || len(preflight.Blocks) != 1 || preflight.Blocks[0] != 1 {
		t.Fatalf("preflight = %+v", preflight)
	}
	if _, err := bindings.StartWrite(WriteStartRequestDTO{Token: preflight.Token}); err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	backup, err := workflow.LoadDumpProject(preflight.BackupPath)
	if err != nil {
		t.Fatal(err)
	}
	if !backup.Complete() {
		t.Fatal("pre-write backup is incomplete")
	}
	if got := backup.Dump.Blocks[1].Data; got != [nfc.BlockSize]byte{} {
		t.Fatalf("backup block 1 = % X; want the unedited card value", got)
	}
	if !strings.Contains(recorder.findTaskEvent(t, "pre-write backup saved").Message, preflight.BackupPath) {
		t.Fatalf("backup event did not expose the path")
	}
	fixture.mu.Lock()
	defer fixture.mu.Unlock()
	if len(fixture.writes) != 1 || fixture.writes[0] != 1 {
		t.Fatalf("writes = %v", fixture.writes)
	}
}

func TestClassicBulkWriteBackupFailureAbortsBeforeWriting(t *testing.T) {
	blocked := filepath.Join(t.TempDir(), "blocked")
	if err := os.WriteFile(blocked, []byte("not a directory"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, bindings, fixture, recorder, _ := newClassicWriteAppService(t, blocked)
	if _, err := bindings.EditBlockHex(1, "DEADBEEF00112233445566778899AABB"); err != nil {
		t.Fatal(err)
	}
	preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "changes"})
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.Valid {
		t.Fatalf("preflight = %+v", preflight)
	}
	if _, err := bindings.StartWrite(WriteStartRequestDTO{Token: preflight.Token}); err != nil {
		t.Fatal(err)
	}
	event := recorder.waitTask(t)
	if event.Type != "failed" || !strings.Contains(event.Message, "pre-write backup") || !strings.Contains(event.Message, "not modified") {
		t.Fatalf("task event = %+v", event)
	}
	if fixture.writeCount() != 0 {
		t.Fatal("a write was attempted without a pre-write backup")
	}
}

func TestClassicWriteRejectsCardChangeAfterPreflight(t *testing.T) {
	service, bindings, fixture, _, _ := newClassicWriteAppService(t, "")
	preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "block", Blocks: []int{1}})
	if err != nil || !preflight.Valid {
		t.Fatalf("preflight = %+v error = %v", preflight, err)
	}
	uid := []byte{0x04, 0xAA, 0xBB, 0xCC}
	fixture.setUID(uid)
	want := formatHex(uid)
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if card := service.Dashboard().Card; card.Present && card.UID == want {
			break
		}
		time.Sleep(time.Millisecond)
	}
	if _, err := bindings.StartWrite(WriteStartRequestDTO{Token: preflight.Token}); !errors.Is(err, ErrPreflightRequired) {
		t.Fatalf("StartWrite() error = %v, want ErrPreflightRequired", err)
	}
	if fixture.writeCount() != 0 {
		t.Fatal("a write was attempted after the card changed")
	}
}

func TestClassicBulkPreflightExcludesBlockZero(t *testing.T) {
	_, bindings, _, _, _ := newClassicWriteAppService(t, "")
	block0 := "04 11 22 33 04 00 00 00 00 00 00 00 00 00 00 01"
	if _, err := bindings.EditBlockHex(0, block0); err != nil {
		t.Fatal(err)
	}
	if _, err := bindings.EditBlockHex(1, "DEADBEEF00112233445566778899AABB"); err != nil {
		t.Fatal(err)
	}
	preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "changes"})
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.Valid || len(preflight.Blocks) != 1 || preflight.Blocks[0] != 1 {
		t.Fatalf("preflight = %+v", preflight)
	}
	for _, block := range preflight.Blocks {
		if block == 0 {
			t.Fatal("bulk preflight included block 0")
		}
	}
}

func TestClassicPreflightRejectsDumpFromDifferentCard(t *testing.T) {
	service, bindings, fixture, _, _ := newClassicWriteAppService(t, "")
	result := fixture.dumpResult(t, service)
	result.Card.UID = []byte{0x04, 0x99, 0x88, 0x77}
	if err := service.bench.Load(result, "other-card.nfcx.json"); err != nil {
		t.Fatal(err)
	}
	preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "block", Blocks: []int{1}})
	if err != nil {
		t.Fatal(err)
	}
	if preflight.Valid || !strings.Contains(preflight.Error, "different card") {
		t.Fatalf("preflight = %+v", preflight)
	}
}

func TestClassicPreflightRefusesBlockEditorOnProtectedBlocks(t *testing.T) {
	_, bindings, _, _, _ := newClassicWriteAppService(t, "")
	for _, block := range []int{0, 3} {
		preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "block", Blocks: []int{block}})
		if err != nil {
			t.Fatal(err)
		}
		if preflight.Valid || !strings.Contains(preflight.Error, "ordinary data block") {
			t.Fatalf("block %d preflight = %+v", block, preflight)
		}
	}
}

func TestClassicBindingsSaveAndLoadDump(t *testing.T) {
	_, bindings, _, _, source := newClassicWriteAppService(t, "")
	target := filepath.Join(t.TempDir(), "saved.nfcx.json")
	result, err := bindings.SaveDumpFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if result.Path != filepath.Base(target) {
		t.Fatalf("save result = %+v", result)
	}
	loaded, err := bindings.LoadDumpFile(target, true)
	if err != nil {
		t.Fatal(err)
	}
	if !loaded.Loaded || loaded.Name != filepath.Base(target) {
		t.Fatalf("loaded = %+v", loaded)
	}
	if same, err := workflow.LoadDumpProject(source); err != nil || !same.Complete() {
		t.Fatalf("source dump = %+v error = %v", same, err)
	}
}
