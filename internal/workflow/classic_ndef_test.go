package workflow_test

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

type classicNDEFCard struct {
	mu       sync.Mutex
	layout   mifare.Layout
	card     nfc.CardInfo
	blocks   map[int][mifare.BlockSize]byte
	keyA     map[int]nfc.Key
	keyB     map[int]nfc.Key
	authed   int
	authType nfc.KeyType
	writes   []int
	trailers []int
	selects  int
	changeAt int
}

func newClassicNDEFCard(t *testing.T, layout mifare.Layout, ndefSectors []int, message []byte) *classicNDEFCard {
	t.Helper()
	card := nfc.CardInfo{UID: []byte{0x04, 0x11, 0x22, 0x33}, ATQA: [2]byte{0x00, 0x04}, SAK: 0x08}
	if layout == mifare.Classic4K {
		card.SAK = 0x18
	}
	fixture := &classicNDEFCard{
		layout: layout, card: card, blocks: make(map[int][mifare.BlockSize]byte),
		keyA: make(map[int]nfc.Key), keyB: make(map[int]nfc.Key), authed: -1,
	}
	sectors, _ := layout.SectorCount()
	transport := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	for sector := 0; sector < sectors; sector++ {
		fixture.keyA[sector] = transport
		fixture.keyB[sector] = transport
		trailer, _ := layout.TrailerBlock(sector)
		fixture.blocks[trailer] = [mifare.BlockSize]byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x07, 0x80, 0x69, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	}
	manufacturer := fixture.blocks[0]
	copy(manufacturer[0:4], card.UID)
	manufacturer[4] = card.UID[0] ^ card.UID[1] ^ card.UID[2] ^ card.UID[3]
	fixture.blocks[0] = manufacturer
	if len(ndefSectors) == 0 {
		return fixture
	}
	mad := mifare.MAD{Present: true, Version: 1, MultiApplication: true, AIDs: make(map[int]uint16)}
	for _, sector := range ndefSectors {
		mad.AIDs[sector] = mifare.NDEFAID
	}
	encoded, err := mad.Blocks()
	if err != nil {
		t.Fatal(err)
	}
	fixture.blocks[1], fixture.blocks[2] = encoded.Sector0[0], encoded.Sector0[1]
	fixture.blocks[3] = [mifare.BlockSize]byte{0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x78, 0x77, 0x88, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	areaBlocks, _ := mifare.NDEFAreaBlocks(layout, ndefSectors)
	if message == nil {
		message, _ = type2.EncodeTextMessage("hello", "en")
	}
	area, err := type2.BuildNDEFArea(len(areaBlocks)*mifare.BlockSize, message)
	if err != nil {
		t.Fatal(err)
	}
	for index, block := range areaBlocks {
		value := fixture.blocks[block]
		copy(value[:], area[index*mifare.BlockSize:(index+1)*mifare.BlockSize])
		fixture.blocks[block] = value
	}
	secretB := nfc.Key{0x11, 0x12, 0x13, 0x14, 0x15, 0x16}
	for _, sector := range ndefSectors {
		fixture.keyA[sector] = nfc.Key{0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7}
		fixture.keyB[sector] = secretB
		trailer, _ := layout.TrailerBlock(sector)
		fixture.blocks[trailer] = [mifare.BlockSize]byte{0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7, 0x7F, 0x07, 0x88, 0x40, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	}
	return fixture
}

func (f *classicNDEFCard) sectorFor(block int) int {
	sector, _ := f.layout.SectorForBlock(block)
	return sector
}

func (f *classicNDEFCard) readBlockLocked(block int) ([mifare.BlockSize]byte, error) {
	if f.authed != f.sectorFor(block) {
		return [mifare.BlockSize]byte{}, nfc.NewError("read block", nfc.CodeAuthenticationFailed, "sector is not authenticated", nil)
	}
	value := f.blocks[block]
	trailer, _ := f.layout.TrailerBlock(f.sectorFor(block))
	if block == trailer {
		copy(value[0:6], make([]byte, 6))
		access, err := mifare.DecodeAccessBits([3]byte{value[6], value[7], value[8]})
		if err != nil || !access.Groups[3].KeyBReadable() {
			copy(value[10:16], make([]byte, 6))
		}
	}
	return value, nil
}

func (f *classicNDEFCard) reader() *mock.Reader {
	return mock.NewReaderWithFuncs(mock.ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			f.selects++
			if f.changeAt > 0 && f.selects >= f.changeAt {
				replacement := f.card.Clone()
				replacement.UID = []byte{0x04, 0x99, 0x88, 0x77}
				return replacement, nil
			}
			f.authed = -1
			return f.card.Clone(), nil
		},
		Authenticate: func(_ context.Context, block byte, kind nfc.KeyType, key nfc.Key) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			sector := f.sectorFor(int(block))
			expected := f.keyA[sector]
			if kind == nfc.KeyTypeB {
				expected = f.keyB[sector]
			}
			if key != expected {
				f.authed = -1
				return nfc.ErrAuthenticationFailed
			}
			f.authed, f.authType = sector, kind
			return nil
		},
		ReadBlock: func(_ context.Context, block byte) ([mifare.BlockSize]byte, error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			return f.readBlockLocked(int(block))
		},
		WriteBlock: func(_ context.Context, block byte, data [mifare.BlockSize]byte) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			if f.authed != f.sectorFor(int(block)) {
				return nfc.NewError("write block", nfc.CodeAuthenticationFailed, "sector is not authenticated", nil)
			}
			if int(block) == 0 {
				return nfc.NewError("write block", nfc.CodeInvalidArgument, "block 0 is read-only", nil)
			}
			trailer, _ := f.layout.TrailerBlock(f.sectorFor(int(block)))
			if int(block) == trailer {
				return nfc.NewError("write block", nfc.CodeInvalidArgument, "trailer requires the trailer capability", nil)
			}
			current := f.blocks[trailer]
			access, err := mifare.DecodeAccessBits([3]byte{current[6], current[7], current[8]})
			if err != nil {
				return err
			}
			group, _ := f.layout.AccessGroupForBlock(int(block))
			mask := access.Groups[group].DataWriteKeys()
			if f.authType == nfc.KeyTypeA && !mask.AllowsA() || f.authType == nfc.KeyTypeB && !mask.AllowsB() {
				return nfc.NewError("write block", nfc.CodePermission, "authenticated key cannot write this block", nil)
			}
			f.writes = append(f.writes, int(block))
			f.blocks[int(block)] = data
			return nil
		},
		WriteTrailer: func(_ context.Context, block byte, data [mifare.BlockSize]byte) error {
			f.mu.Lock()
			defer f.mu.Unlock()
			sector := f.sectorFor(int(block))
			if f.authed != sector {
				return nfc.NewError("write trailer", nfc.CodeAuthenticationFailed, "sector is not authenticated", nil)
			}
			trailer, _ := f.layout.TrailerBlock(sector)
			if int(block) != trailer {
				return nfc.NewError("write trailer", nfc.CodeInvalidArgument, "not a trailer block", nil)
			}
			current := f.blocks[trailer]
			access, err := mifare.DecodeAccessBits([3]byte{current[6], current[7], current[8]})
			if err != nil {
				return err
			}
			mask := access.Groups[3].FullTrailerWriteKeys()
			if f.authType == nfc.KeyTypeA && !mask.AllowsA() || f.authType == nfc.KeyTypeB && !mask.AllowsB() {
				return nfc.NewError("write trailer", nfc.CodePermission, "authenticated key cannot write the trailer", nil)
			}
			f.trailers = append(f.trailers, int(block))
			f.blocks[int(block)] = data
			var keyA, keyB nfc.Key
			copy(keyA[:], data[0:6])
			copy(keyB[:], data[10:16])
			f.keyA[sector] = keyA
			f.keyB[sector] = keyB
			return nil
		},
	})
}

func (f *classicNDEFCard) block(block int) [mifare.BlockSize]byte {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.blocks[block]
}

func (f *classicNDEFCard) trailer(block int) [mifare.BlockSize]byte {
	return f.block(block)
}

func classicNDEFService(t *testing.T, fixture *classicNDEFCard, backupRoot string) *workflow.ClassicNDEFService {
	t.Helper()
	reader := fixture.reader()
	if err := reader.Open(context.Background(), "mock:classic-ndef"); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reader.Close() })
	return workflow.NewClassicNDEFService(
		directExecutor{reader: reader},
		workflow.NewDumpService(directExecutor{reader: reader}),
		workflow.FileClassicNDEFBackupStore{Root: backupRoot},
	)
}

func TestClassicNDEFReadFormattedCard(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1, 2}, nil)
	service := classicNDEFService(t, fixture, t.TempDir())
	state, err := service.Read(context.Background(), workflow.ClassicNDEFRequest{Card: fixture.card})
	if err != nil {
		t.Fatal(err)
	}
	if !state.Application || len(state.Sectors) != 2 || state.Capacity != 96 {
		t.Fatalf("state = %+v", state)
	}
	if state.NDEF == nil || !state.NDEF.Present || state.NDEF.Text != "hello" {
		t.Fatalf("NDEF = %+v", state.NDEF)
	}
	if state.MAD.AIDs[1] != mifare.NDEFAID {
		t.Fatalf("AID = %04X", state.MAD.AIDs[1])
	}
}

func TestClassicNDEFReadWithoutMAD(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, nil, nil)
	service := classicNDEFService(t, fixture, t.TempDir())
	state, err := service.Read(context.Background(), workflow.ClassicNDEFRequest{Card: fixture.card})
	if err != nil {
		t.Fatal(err)
	}
	if state.Application || state.MAD.Present || state.NDEF.Present {
		t.Fatalf("state = %+v", state)
	}
}

func TestClassicNDEFWriteUpdatesOnlyMessageBlocks(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1}, nil)
	backupRoot := t.TempDir()
	service := classicNDEFService(t, fixture, backupRoot)
	message, err := type2.EncodeTextMessage("updated", "en")
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.Write(context.Background(), workflow.ClassicNDEFWriteRequest{
		Card: fixture.card, Message: message,
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.BackupPath == "" {
		t.Fatal("write did not report a backup")
	}
	if _, err := os.Stat(result.BackupPath); err != nil {
		t.Fatalf("backup file: %v", err)
	}
	if len(result.Verified) == 0 || len(result.Verified) > 3 {
		t.Fatalf("verified blocks = %v", result.Verified)
	}
	if _, err := os.Stat(filepath.Join(backupRoot, filepath.Base(result.BackupPath))); err != nil {
		t.Fatal(err)
	}
	state, err := service.Read(context.Background(), workflow.ClassicNDEFRequest{Card: fixture.card})
	if err != nil {
		t.Fatal(err)
	}
	if state.NDEF.Text != "updated" {
		t.Fatalf("NDEF text = %q", state.NDEF.Text)
	}
}

func TestClassicNDEFWriteRefusesReadOnlyMapping(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1}, nil)
	fixture.setTrailerByte(7, 9, 0x43)
	service := classicNDEFService(t, fixture, t.TempDir())
	message, _ := type2.EncodeTextMessage("nope", "en")
	_, err := service.Write(context.Background(), workflow.ClassicNDEFWriteRequest{Card: fixture.card, Message: message})
	if err == nil || !hasNFCCode(err, nfc.CodePermission) {
		t.Fatalf("write error = %v", err)
	}
	if len(fixture.writes) != 0 {
		t.Fatalf("writes = %v", fixture.writes)
	}
}

func TestClassicNDEFWriteAbortsOnCardChange(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1}, nil)
	fixture.changeAt = 8
	service := classicNDEFService(t, fixture, t.TempDir())
	message, _ := type2.EncodeTextMessage("changed", "en")
	_, err := service.Write(context.Background(), workflow.ClassicNDEFWriteRequest{Card: fixture.card, Message: message})
	if !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("write error = %v", err)
	}
	if len(fixture.writes) != 0 {
		t.Fatalf("writes = %v", fixture.writes)
	}
}

func TestClassicNDEFFormatBlankCard(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, nil, nil)
	service := classicNDEFService(t, fixture, t.TempDir())
	result, err := service.Format(context.Background(), workflow.ClassicNDEFFormatRequest{
		Card: fixture.card, Sectors: []int{1, 2},
	})
	if err != nil {
		t.Fatal(err)
	}
	if !result.Formatted || result.BackupPath == "" {
		t.Fatalf("result = %+v", result)
	}
	if _, err := os.Stat(result.BackupPath); err != nil {
		t.Fatal(err)
	}
	madBlock := fixture.block(1)
	if madBlock[2] != 0x03 || madBlock[3] != 0xE1 {
		t.Fatalf("MAD AID = %02X %02X", madBlock[2], madBlock[3])
	}
	sector0Trailer := fixture.trailer(3)
	wantMAD := [mifare.BlockSize]byte{0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x78, 0x77, 0x88, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	if !bytes.Equal(sector0Trailer[:], wantMAD[:]) {
		t.Fatalf("sector 0 trailer = %X", sector0Trailer)
	}
	wantNDEF := [mifare.BlockSize]byte{0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7, 0x7F, 0x07, 0x88, 0x40, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	for _, block := range []int{7, 11} {
		if got := fixture.trailer(block); !bytes.Equal(got[:], wantNDEF[:]) {
			t.Fatalf("trailer %d = %X", block, got)
		}
	}
	if area := fixture.block(4); area[0] != 0x03 || area[1] != 0x00 || area[2] != 0xFE {
		t.Fatalf("NDEF TLV = %X", area)
	}
	state, err := service.Read(context.Background(), workflow.ClassicNDEFRequest{Card: fixture.card})
	if err != nil {
		t.Fatal(err)
	}
	if !state.Application || len(state.Sectors) != 2 || state.NDEF.Present {
		t.Fatalf("formatted state = %+v", state)
	}
	second := classicNDEFService(t, fixture, t.TempDir())
	state2, err := second.Read(context.Background(), workflow.ClassicNDEFRequest{Card: fixture.card})
	if err != nil {
		t.Fatal(err)
	}
	if !state2.Application || len(state2.Sectors) != 2 || state2.NDEF.Present {
		t.Fatalf("second formatted state = %+v", state2)
	}
}

func TestClassicNDEFFormatRefusesOtherApplication(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, nil, nil)
	mad := mifare.MAD{Present: true, Version: 1, MultiApplication: true, AIDs: map[int]uint16{1: 0x1234}}
	encoded, _ := mad.Blocks()
	fixture.setBlock(1, encoded.Sector0[0])
	fixture.setBlock(2, encoded.Sector0[1])
	fixture.setBlock(3, [mifare.BlockSize]byte{0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x78, 0x77, 0x88, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF})
	service := classicNDEFService(t, fixture, t.TempDir())
	_, err := service.Format(context.Background(), workflow.ClassicNDEFFormatRequest{Card: fixture.card, Sectors: []int{1}})
	if err == nil || !strings.Contains(err.Error(), "another application") {
		t.Fatalf("format error = %v", err)
	}
	if len(fixture.writes)+len(fixture.trailers) != 0 {
		t.Fatalf("writes = %v trailers = %v", fixture.writes, fixture.trailers)
	}
}

func TestClassicNDEFFormatRefusesUnverifiedKeyB(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1}, nil)
	fixture.setTrailerByte(7, 9, 0x41) // proprietary write access, non-standard trailer
	service := classicNDEFService(t, fixture, t.TempDir())
	_, err := service.Format(context.Background(), workflow.ClassicNDEFFormatRequest{Card: fixture.card, Sectors: []int{1}})
	if err == nil || !hasNFCCode(err, nfc.CodePermission) {
		t.Fatalf("format error = %v", err)
	}
	if len(fixture.writes)+len(fixture.trailers) != 0 {
		t.Fatalf("writes = %v trailers = %v", fixture.writes, fixture.trailers)
	}
}

func TestClassicNDEFReadRejectsMalformedTLV(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1}, nil)
	fixture.setBlock(4, [mifare.BlockSize]byte{0x03, 0xFF, 0xFF, 0xFF})
	service := classicNDEFService(t, fixture, t.TempDir())
	if _, err := service.Read(context.Background(), workflow.ClassicNDEFRequest{Card: fixture.card}); err == nil {
		t.Fatal("malformed TLV was accepted")
	}
}

func TestFileClassicNDEFBackupStoreWritesRestrictedProject(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1}, nil)
	reader := fixture.reader()
	if err := reader.Open(context.Background(), "mock:classic-ndef"); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	dump, err := workflow.NewDumpService(directExecutor{reader: reader}).ReadCard(context.Background(), workflow.DumpRequest{
		Card: fixture.card, Keys: classicFixtureKeys(fixture.layout),
	})
	if err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(t.TempDir(), "classic-ndef-backups")
	path, err := workflow.FileClassicNDEFBackupStore{Root: root}.Save(context.Background(), dump)
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("backup mode = %v", info.Mode().Perm())
	}
	if _, err := workflow.LoadDumpProject(path); err != nil {
		t.Fatal(err)
	}
}

func classicFixtureKeys(layout mifare.Layout) []workflow.SectorKeys {
	sectors, _ := layout.SectorCount()
	result := make([]workflow.SectorKeys, sectors)
	key := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	for sector := range result {
		result[sector] = workflow.SectorKeys{Sector: sector, KeyA: &key, KeyB: &key}
	}
	return result
}

func (f *classicNDEFCard) setBlock(block int, value [mifare.BlockSize]byte) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.blocks[block] = value
}

func (f *classicNDEFCard) setTrailerByte(block, index int, value byte) {
	f.mu.Lock()
	defer f.mu.Unlock()
	updated := f.blocks[block]
	updated[index] = value
	f.blocks[block] = updated
}

func hasNFCCode(err error, want nfc.ErrorCode) bool {
	code, ok := nfc.ErrorCodeOf(err)
	return ok && code == want
}

func TestClassicNDEFFormatRejectsPartialApplication(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1, 2}, nil)
	service := classicNDEFService(t, fixture, t.TempDir())
	_, err := service.Format(context.Background(), workflow.ClassicNDEFFormatRequest{Card: fixture.card, Sectors: []int{1}})
	if err == nil || !strings.Contains(err.Error(), "complete application") {
		t.Fatalf("format error = %v", err)
	}
	if len(fixture.writes)+len(fixture.trailers) != 0 {
		t.Fatalf("writes = %v trailers = %v", fixture.writes, fixture.trailers)
	}
}

func TestClassicNDEFFormatExtendsApplication(t *testing.T) {
	fixture := newClassicNDEFCard(t, mifare.Classic1K, []int{1}, nil)
	service := classicNDEFService(t, fixture, t.TempDir())
	result, err := service.Format(context.Background(), workflow.ClassicNDEFFormatRequest{
		Card: fixture.card, Sectors: []int{1, 2},
	})
	if err != nil {
		t.Fatal(err)
	}
	if !result.Formatted || result.State.Application == false || len(result.State.Sectors) != 2 {
		t.Fatalf("format state = %+v", result.State)
	}
	if result.State.NDEF.Present {
		t.Fatalf("format left an NDEF message: %+v", result.State.NDEF)
	}
	state, err := service.Read(context.Background(), workflow.ClassicNDEFRequest{Card: fixture.card})
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sectors) != 2 || state.MAD.AIDs[2] != mifare.NDEFAID {
		t.Fatalf("final state = %+v", state)
	}
}
