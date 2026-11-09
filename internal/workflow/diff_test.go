package workflow_test

import (
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func TestDiffDumpsClassifiesChangedEqualAndUnknownBlocks(t *testing.T) {
	before := completeDumpResult(t)

	changed := before.Dump.Clone()
	changed.Blocks[1].Data[3] ^= 0x20
	afterChanged := before
	afterChanged.Dump = changed

	diff, err := workflow.DiffDumps(before, afterChanged)
	if err != nil {
		t.Fatal(err)
	}
	if diff.ChangedBlocks != 1 || diff.EqualBlocks != len(before.Dump.Blocks)-1 || diff.UnknownBlocks != 0 {
		t.Fatalf("counts = changed %d, equal %d, unknown %d", diff.ChangedBlocks, diff.EqualBlocks, diff.UnknownBlocks)
	}
	block := diff.Blocks[1]
	if block.State != workflow.DumpDiffChanged || len(block.Bytes) != 1 || block.Bytes[0].Offset != 3 {
		t.Fatalf("changed block = %+v", block)
	}

	unknown := before.Dump.Clone()
	unknown.Blocks[1].KnownMask &^= 1 << 3
	afterUnknown := before
	afterUnknown.Dump = unknown
	diff, err = workflow.DiffDumps(before, afterUnknown)
	if err != nil {
		t.Fatal(err)
	}
	if diff.UnknownBlocks != 1 || diff.ChangedBlocks != 0 {
		t.Fatalf("unknown counts = changed %d, unknown %d", diff.ChangedBlocks, diff.UnknownBlocks)
	}
	if diff.Blocks[1].State != workflow.DumpDiffUnknown {
		t.Fatalf("block state = %s", diff.Blocks[1].State)
	}
}

func TestDiffDumpsDistinguishesUnknownFromKnownZero(t *testing.T) {
	before := completeDumpResult(t)
	zero := before
	zero.Dump = before.Dump.Clone()
	zero.Dump.Blocks[5].Data = [16]byte{}
	zero.Dump.Blocks[5].KnownMask = mifare.AllBytesKnown

	unknown := zero
	unknown.Dump = zero.Dump.Clone()
	unknown.Dump.Blocks[5].KnownMask = 0

	diff, err := workflow.DiffDumps(zero, unknown)
	if err != nil {
		t.Fatal(err)
	}
	if diff.Blocks[5].State != workflow.DumpDiffUnknown || diff.UnknownBlocks != 1 {
		t.Fatalf("unknown zero block = %+v", diff.Blocks[5])
	}
}

func TestDiffDumpsReportsTrailerAndIdentityChanges(t *testing.T) {
	before := completeDumpResult(t)
	after := before
	after.Dump = before.Dump.Clone()
	after.Dump.Blocks[3].Data[0] = 0x11
	after.Dump.Blocks[3].Data[6] = 0x7f
	after.Card = before.Card.Clone()
	after.Card.UID[0] = 9

	diff, err := workflow.DiffDumps(before, after)
	if err != nil {
		t.Fatal(err)
	}
	if !diff.IdentityChanged {
		t.Fatal("identity change was not reported")
	}
	if diff.TrailerChanges != 1 {
		t.Fatalf("trailer changes = %d", diff.TrailerChanges)
	}
	trailer := diff.Blocks[3].Trailer
	if trailer == nil || !trailer.KeyAChanged || !trailer.AccessChanged || trailer.KeyBChanged {
		t.Fatalf("trailer diff = %+v", trailer)
	}
	if trailer.Valid {
		t.Fatal("invalid access bytes were reported as valid")
	}
}

func TestDiffDumpsRejectsLayoutMismatch(t *testing.T) {
	before := completeDumpResult(t)
	after := complete4KDumpResult(t)
	if _, err := workflow.DiffDumps(before, after); err == nil {
		t.Fatal("layout mismatch was accepted")
	}
}

func TestDumpReadabilityCountsSkippedSectors(t *testing.T) {
	result := completeDumpResult(t)
	result.Dump.Blocks[4].KnownMask &^= 0xff
	readability := workflow.DumpReadability(result.Dump)
	if readability.TotalBlocks != 64 || readability.ReadableBlocks != 63 {
		t.Fatalf("block counts = %d/%d", readability.ReadableBlocks, readability.TotalBlocks)
	}
	if readability.TotalSectors != 16 || readability.ReadableSectors != 15 {
		t.Fatalf("sector counts = %d/%d", readability.ReadableSectors, readability.TotalSectors)
	}
	if len(readability.SkippedSectors) != 1 || readability.SkippedSectors[0] != 1 {
		t.Fatalf("skipped sectors = %v", readability.SkippedSectors)
	}
}
