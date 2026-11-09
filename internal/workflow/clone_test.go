package workflow_test

import (
	"context"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

type cloneHarness struct {
	service *workflow.CloneService
	request workflow.CloneRequest
	blocks  map[byte][nfc.BlockSize]byte
	card    *nfc.CardInfo
	writes  *[]string
}

func newCloneHarness(t *testing.T, writeManufacturer func(*[nfc.BlockSize]byte, *nfc.CardInfo) error) *cloneHarness {
	t.Helper()
	source := completeDumpResult(t)
	targetCard := classic1KCard(9)
	target := completeDumpResult(t)
	target.Card = targetCard
	target.Dump.Blocks[0] = mifare.Block{Number: 0, Sector: 0, Data: [nfc.BlockSize]byte{9, 2, 3, 4, 0x0c}, KnownMask: mifare.AllBytesKnown, Status: mifare.BlockRead}
	for index := 1; index < len(target.Dump.Blocks); index++ {
		if index%4 != 3 {
			target.Dump.Blocks[index].Data = [nfc.BlockSize]byte{0xaa}
		}
	}
	blocks := make(map[byte][nfc.BlockSize]byte, len(target.Dump.Blocks))
	for index := range target.Dump.Blocks {
		blocks[byte(index)] = target.Dump.Blocks[index].Data
	}
	card := targetCard.Clone()
	writes := make([]string, 0)
	reader := openMockReader(t, mock.ReaderFuncs{
		CardInfo:     func(context.Context) (nfc.CardInfo, error) { return card.Clone(), nil },
		Authenticate: func(context.Context, byte, nfc.KeyType, nfc.Key) error { return nil },
		ReadBlock: func(_ context.Context, block byte) ([nfc.BlockSize]byte, error) {
			return blocks[block], nil
		},
		WriteBlock: func(_ context.Context, block byte, data [nfc.BlockSize]byte) error {
			writes = append(writes, "data")
			blocks[block] = data
			return nil
		},
		WriteTrailer: func(_ context.Context, block byte, data [nfc.BlockSize]byte) error {
			writes = append(writes, "trailer")
			blocks[block] = data
			return nil
		},
		WriteManufacturer: func(_ context.Context, data [nfc.BlockSize]byte) error {
			writes = append(writes, "block0")
			if writeManufacturer != nil {
				return writeManufacturer(&data, &card)
			}
			blocks[0] = data
			card.UID = append([]byte(nil), data[0:4]...)
			return nil
		},
		SetRawMode: func(context.Context, bool) error { return nil },
		TransceiveBytes: func(context.Context, []byte) ([]byte, error) {
			return nil, nfc.NewError("transceive bytes", nfc.CodeTimeout, "no response", nil)
		},
		TransceiveBits: func(context.Context, []byte, int) ([]byte, int, error) {
			return nil, 0, nfc.NewError("transceive bits", nfc.CodeTimeout, "no response", nil)
		},
	})
	key := nfc.Key{0xff, 0xff, 0xff, 0xff, 0xff, 0xff}
	keys := make([]workflow.SectorKeys, mifare.Classic1KSectors)
	for sector := range keys {
		keys[sector] = workflow.SectorKeys{Sector: sector, KeyA: &key}
	}
	request := workflow.CloneRequest{
		Source: source, Card: targetCard, Device: nfc.DeviceInfo{ConnString: "mock:test"}, TargetKeys: keys,
	}
	dumps := workflow.NewDumpService(directExecutor{reader: reader})
	uid := workflow.NewUIDService(directExecutor{reader: reader}, nil)
	return &cloneHarness{
		service: workflow.NewCloneService(directExecutor{reader: reader}, dumps, uid),
		request: request, blocks: blocks, card: &card, writes: &writes,
	}
}

func TestClonePreflightReportsBlockZeroRequirement(t *testing.T) {
	harness := newCloneHarness(t, nil)
	plan, err := harness.service.Preflight(context.Background(), harness.request)
	if err != nil {
		t.Fatal(err)
	}
	if !plan.Block0Write || plan.Block0 == nil {
		t.Fatalf("plan block 0 = %+v", plan)
	}
	if len(plan.Restore.Operations) != 63 {
		t.Fatalf("restore operations = %d", len(plan.Restore.Operations))
	}
}

func TestCloneWritesSourceImageAndVerifiesReadback(t *testing.T) {
	harness := newCloneHarness(t, nil)
	result, err := harness.service.Clone(context.Background(), harness.request, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Verified || result.Phase != workflow.CloneComplete {
		t.Fatalf("clone result = %+v", result)
	}
	if result.UID == nil || !result.UID.Verified {
		t.Fatal("block 0 was not verified")
	}
	if result.Diff.ChangedBlocks != 0 || result.Diff.UnknownBlocks != 0 || result.Diff.IdentityChanged {
		t.Fatalf("diff = %+v", result.Diff)
	}
	if harness.card.UID[0] != harness.request.Source.Card.UID[0] {
		t.Fatalf("target UID = %x", harness.card.UID)
	}
	if got := (*harness.writes)[len(*harness.writes)-1]; got != "block0" {
		t.Fatalf("final write = %s; block 0 must come after data and trailers", got)
	}
}

func TestCloneFailsWhenBlockZeroIsReadOnly(t *testing.T) {
	harness := newCloneHarness(t, func(_ *[nfc.BlockSize]byte, _ *nfc.CardInfo) error {
		return nfc.NewError("write manufacturer block", nfc.CodeUnsupported, "genuine card block 0 is read-only", nil)
	})
	result, err := harness.service.Clone(context.Background(), harness.request, nil)
	if err == nil {
		t.Fatal("read-only block 0 was reported as a successful clone")
	}
	if result.Verified {
		t.Fatal("failed clone was marked verified")
	}
	if result.Phase != workflow.CloneVerify && result.Phase != workflow.CloneBlock0 {
		t.Fatalf("phase = %s", result.Phase)
	}
}

func TestCloneReplacesManufacturerBytesWithoutChangingUID(t *testing.T) {
	harness := newCloneHarness(t, nil)
	harness.card.UID = append([]byte(nil), 1, 2, 3, 4)
	harness.blocks[0] = [nfc.BlockSize]byte{1, 2, 3, 4, 4, 0xaa}
	harness.request.Card = harness.card.Clone()

	plan, err := harness.service.Preflight(context.Background(), harness.request)
	if err != nil {
		t.Fatal(err)
	}
	if !plan.Block0Write {
		t.Fatal("same-UID manufacturer byte change did not require a block 0 write")
	}
	result, err := harness.service.Clone(context.Background(), harness.request, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Verified {
		t.Fatalf("clone result = %+v", result)
	}
	if got := harness.blocks[0][5]; got != 0 {
		t.Fatalf("manufacturer byte 5 = %02X, want 00", got)
	}
}

func TestCloneRejectsPartialSourceAndLayoutMismatch(t *testing.T) {
	harness := newCloneHarness(t, nil)
	harness.request.Source.Dump.Blocks[8].KnownMask = 0
	if _, err := harness.service.Preflight(context.Background(), harness.request); err == nil {
		t.Fatal("partial source image was accepted")
	}

	harness = newCloneHarness(t, nil)
	harness.request.Source.Dump = complete4KDumpResult(t).Dump
	if _, err := harness.service.Preflight(context.Background(), harness.request); err == nil {
		t.Fatal("layout mismatch was accepted")
	}
}

func TestFactoryImageResetsTrailersAndOptionallyData(t *testing.T) {
	backup := completeDumpResult(t)
	backup.Dump.Blocks[1].Data = [nfc.BlockSize]byte{0xde, 0xad}
	image, err := workflow.BuildFactoryImage(backup, false)
	if err != nil {
		t.Fatal(err)
	}
	if image.Dump.Blocks[0] != backup.Dump.Blocks[0] {
		t.Fatal("factory image changed block 0")
	}
	if image.Dump.Blocks[1].Data != [nfc.BlockSize]byte{} {
		t.Fatalf("data block = %x", image.Dump.Blocks[1].Data)
	}
	wantTrailer := [nfc.BlockSize]byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x07, 0x80, 0x69, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff}
	if image.Dump.Blocks[3].Data != wantTrailer {
		t.Fatalf("trailer = %x", image.Dump.Blocks[3].Data)
	}
	if !image.Complete() {
		t.Fatal("factory image is incomplete")
	}

	keep, err := workflow.BuildFactoryImage(backup, true)
	if err != nil {
		t.Fatal(err)
	}
	if keep.Dump.Blocks[1].Data != backup.Dump.Blocks[1].Data {
		t.Fatalf("keep-data factory image changed block 1: %x", keep.Dump.Blocks[1].Data)
	}
	if keep.Dump.Blocks[3].Data != wantTrailer {
		t.Fatal("keep-data factory image did not reset the trailer")
	}
}

func TestFactoryImageUsesLargeSectorGeometry(t *testing.T) {
	backup := complete4KDumpResult(t)
	image, err := workflow.BuildFactoryImage(backup, false)
	if err != nil {
		t.Fatal(err)
	}
	if image.Dump.Layout != mifare.Classic4K {
		t.Fatalf("layout = %d", image.Dump.Layout)
	}
	lastTrailer := len(image.Dump.Blocks) - 1
	if image.Dump.Blocks[lastTrailer].Data != [nfc.BlockSize]byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x07, 0x80, 0x69, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff} {
		t.Fatalf("large-sector trailer = %x", image.Dump.Blocks[lastTrailer].Data)
	}
	if image.Dump.Blocks[lastTrailer-1].Data != [nfc.BlockSize]byte{} {
		t.Fatalf("large-sector data block = %x", image.Dump.Blocks[lastTrailer-1].Data)
	}
}

func TestSectorKeysFromDumpUsesKnownTrailerBytes(t *testing.T) {
	result := completeDumpResult(t)
	keys := workflow.SectorKeysFromDump(result.Dump)
	if len(keys) != mifare.Classic1KSectors || keys[0].KeyA == nil || keys[0].KeyB == nil {
		t.Fatalf("sector keys = %+v", keys)
	}
	result.Dump.Blocks[3].KnownMask = 0x03c0
	keys = workflow.SectorKeysFromDump(result.Dump)
	if keys[0].KeyA != nil || keys[0].KeyB != nil {
		t.Fatalf("masked trailer produced keys: %+v", keys[0])
	}
}

func TestCloneServiceRequiresReader(t *testing.T) {
	service := workflow.NewCloneService(nil, nil, nil)
	_, err := service.Preflight(context.Background(), workflow.CloneRequest{})
	if !errors.Is(err, nfc.ErrNotOpen) {
		t.Fatalf("Preflight error = %v", err)
	}
}
