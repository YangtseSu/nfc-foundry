package app

import (
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestValueBlockPreviewRoundTripsSignedValues(t *testing.T) {
	_, bindings, _, _, _ := newClassicWriteAppService(t, "")
	preview, err := bindings.PreviewValueBlock(ValueBlockPreviewRequestDTO{Block: 1, Value: "-1", Address: 1})
	if err != nil {
		t.Fatal(err)
	}
	if preview.Hex != "FFFFFFFF00000000FFFFFFFF01FE01FE" || preview.Decoded.Value != -1 || preview.Decoded.Address != 1 {
		t.Fatalf("preview = %+v", preview)
	}
	hexValue, err := bindings.PreviewValueBlock(ValueBlockPreviewRequestDTO{Block: 1, Value: "0x64", Address: -1})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(hexValue.Hex, "64000000") || hexValue.Decoded.Address != 1 {
		t.Fatalf("hex preview = %+v", hexValue)
	}
	if _, err := bindings.PreviewValueBlock(ValueBlockPreviewRequestDTO{Block: 3, Value: "1", Address: 3}); err == nil {
		t.Fatal("trailer block was accepted as a value block")
	}
}

func TestTrailerPreviewAndConfirmedTrailerWrite(t *testing.T) {
	_, bindings, fixture, recorder, _ := newClassicWriteAppService(t, "")
	preview, err := bindings.PreviewTrailer(TrailerPreviewRequestDTO{
		Block: 3, KeyA: "FFFFFFFFFFFF", KeyB: "A0A1A2A3A4A5", GPB: "69",
		Groups: []AccessConditionInputDTO{
			{Group: 0}, {Group: 1}, {Group: 2}, {Group: 3, C3: true},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if preview.Hex != "FFFFFFFFFFFFFF078069A0A1A2A3A4A5" || !preview.Access.Valid {
		t.Fatalf("preview = %+v", preview)
	}
	if _, err := bindings.EditBlockHex(3, preview.Hex); err != nil {
		t.Fatal(err)
	}
	preflight, err := bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "trailer", Blocks: []int{3}})
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.Valid || preflight.BackupPath == "" || len(preflight.TrailerBlocks) != 1 || preflight.TrailerBlocks[0] != 3 {
		t.Fatalf("preflight = %+v", preflight)
	}
	if _, err := bindings.StartWrite(WriteStartRequestDTO{Token: preflight.Token, ConfirmTrailers: true}); err == nil {
		t.Fatal("trailer write started without typed confirmation")
	}
	preflight, err = bindings.PreflightWrite(WritePreflightRequestDTO{Mode: "trailer", Blocks: []int{3}})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := bindings.StartWrite(WriteStartRequestDTO{Token: preflight.Token, ConfirmTrailers: true, Confirmation: "3"}); err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	fixture.mu.Lock()
	defer fixture.mu.Unlock()
	if len(fixture.trailers) != 1 || fixture.trailers[0] != 3 {
		t.Fatalf("trailer writes = %v", fixture.trailers)
	}
	if fixture.blocks[3][10] != 0xA0 || fixture.blocks[3][15] != 0xA5 {
		t.Fatalf("trailer bytes = %x", fixture.blocks[3])
	}
}

func TestComparisonDiffAgainstLoadedDump(t *testing.T) {
	_, bindings, _, _, path := newClassicWriteAppService(t, "")
	diff, err := bindings.LoadComparisonDumpFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if diff.ChangedBlocks != 0 || diff.UnknownBlocks != 0 || diff.IdentityChanged {
		t.Fatalf("identical comparison = %+v", diff)
	}
	if bindings.ComparisonPath() == "" {
		t.Fatal("comparison path is empty")
	}
	if _, err := bindings.EditBlockHex(1, "DEADBEEF00112233445566778899AABB"); err != nil {
		t.Fatal(err)
	}
	diff, err = bindings.ComparisonDiff()
	if err != nil {
		t.Fatal(err)
	}
	if diff.ChangedBlocks != 1 || diff.Blocks[1].State != "changed" {
		t.Fatalf("changed comparison = %+v", diff.Blocks[1])
	}
	bindings.ClearComparisonDump()
	if _, err := bindings.ComparisonDiff(); err == nil {
		t.Fatal("cleared comparison still produced a diff")
	}
}

func TestKeyFilePreviewAndImport(t *testing.T) {
	service, bindings, _, _, _ := newClassicWriteAppService(t, "")
	content := "# keys\nFFFFFFFFFFFF\nFF FF FF FF FF FF\nnot-a-key\n"
	preview := bindings.PreviewKeyFile(content)
	if preview.Valid != 1 || preview.Duplicates != 1 || len(preview.Issues) != 1 || len(preview.Lines) != 4 {
		t.Fatalf("preview = %+v", preview)
	}
	report, err := bindings.ImportKeyTextWithSet(content, "editor-set")
	if err != nil {
		t.Fatal(err)
	}
	if report.Valid != 1 || report.Duplicates != 2 || report.KeySet == nil || report.KeySet.KeyCount != 1 {
		t.Fatalf("import report = %+v", report)
	}
	if len(service.KeyCatalog()) == 0 {
		t.Fatal("key catalog is empty after the import")
	}
}

func TestFactoryFormatPreflightAndWrite(t *testing.T) {
	_, bindings, fixture, recorder, _ := newClassicWriteAppService(t, "")
	if _, err := bindings.EditBlockHex(1, "DEADBEEF00112233445566778899AABB"); err != nil {
		t.Fatal(err)
	}
	preflight, err := bindings.PreflightFactoryFormat(false)
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.Valid || preflight.BackupPath == "" || len(preflight.TrailerBlocks) != 16 {
		t.Fatalf("factory preflight = %+v", preflight)
	}
	if _, err := bindings.StartFactoryFormat(FactoryStartRequestDTO{Token: preflight.Token, Confirmation: "nope"}); err == nil {
		t.Fatal("factory format started with the wrong confirmation")
	}
	preflight, err = bindings.PreflightFactoryFormat(false)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := bindings.StartFactoryFormat(FactoryStartRequestDTO{Token: preflight.Token, Confirmation: "FORMAT"}); err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	fixture.mu.Lock()
	defer fixture.mu.Unlock()
	if fixture.blocks[1] != [nfc.BlockSize]byte{} {
		t.Fatalf("data block = %x", fixture.blocks[1])
	}
	want := [nfc.BlockSize]byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x07, 0x80, 0x69, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff}
	if fixture.blocks[3] != want {
		t.Fatalf("trailer = %x", fixture.blocks[3])
	}
	if len(fixture.writes) != 47 || len(fixture.trailers) != 16 {
		t.Fatalf("writes = %d trailers = %d", len(fixture.writes), len(fixture.trailers))
	}
}

func TestClonePreflightRequiresSourceIdentityAndRuns(t *testing.T) {
	_, bindings, _, recorder, _ := newClassicWriteAppService(t, "")
	preflight, err := bindings.PreflightClone()
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.Valid {
		t.Fatalf("clone preflight = %+v", preflight)
	}
	if preflight.Block0Write {
		t.Fatal("identical target block 0 should not be written")
	}
	if preflight.Blocks == nil || len(preflight.Blocks) != 63 {
		t.Fatalf("clone blocks = %v", preflight.Blocks)
	}
	if _, err := bindings.StartClone(CloneStartRequestDTO{Token: preflight.Token, Confirmation: "00000000"}); err == nil {
		t.Fatal("clone started with the wrong confirmation")
	}
	preflight, err = bindings.PreflightClone()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := bindings.StartClone(CloneStartRequestDTO{Token: preflight.Token, Confirmation: "04112233"}); err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
}

func TestValueBlockDTORejectsOrdinaryData(t *testing.T) {
	var data [nfc.BlockSize]byte
	copy(data[:], "ordinary data...")
	if dto := valueBlockDTO(data); dto != nil {
		t.Fatalf("ordinary data decoded as a value block: %+v", dto)
	}
	encoded := mifare.EncodeValueBlock(42, 7)
	dto := valueBlockDTO(encoded)
	if dto == nil || dto.Value != 42 || dto.Address != 7 {
		t.Fatalf("encoded value block = %+v", dto)
	}
}
