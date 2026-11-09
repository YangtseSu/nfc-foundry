package workflow

import (
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// DumpDiffState classifies one block of a dump comparison. Unknown is a block
// with at least one unknown byte on either side; it is never treated as equal
// to a known zero block.
type DumpDiffState string

const (
	DumpDiffEqual   DumpDiffState = "equal"
	DumpDiffChanged DumpDiffState = "changed"
	DumpDiffUnknown DumpDiffState = "unknown"
)

type DumpDiffByte struct {
	Offset      int  `json:"offset"`
	Before      byte `json:"before"`
	After       byte `json:"after"`
	BeforeKnown bool `json:"beforeKnown"`
	AfterKnown  bool `json:"afterKnown"`
}

type DumpDiffTrailer struct {
	Valid         bool    `json:"valid"`
	Error         string  `json:"error,omitempty"`
	KeyAChanged   bool    `json:"keyAChanged"`
	KeyBChanged   bool    `json:"keyBChanged"`
	AccessChanged bool    `json:"accessChanged"`
	GPBChanged    bool    `json:"gpbChanged"`
	BeforeKeyA    string  `json:"beforeKeyA"`
	AfterKeyA     string  `json:"afterKeyA"`
	BeforeKeyB    string  `json:"beforeKeyB"`
	AfterKeyB     string  `json:"afterKeyB"`
	BeforeAccess  [3]byte `json:"beforeAccess"`
	AfterAccess   [3]byte `json:"afterAccess"`
	BeforeGPB     byte    `json:"beforeGpb"`
	AfterGPB      byte    `json:"afterGpb"`
}

type DumpDiffBlock struct {
	Block           int                 `json:"block"`
	Sector          int                 `json:"sector"`
	Kind            string              `json:"kind"`
	State           DumpDiffState       `json:"state"`
	Before          [nfc.BlockSize]byte `json:"before"`
	After           [nfc.BlockSize]byte `json:"after"`
	BeforeKnownMask uint16              `json:"beforeKnownMask"`
	AfterKnownMask  uint16              `json:"afterKnownMask"`
	Bytes           []DumpDiffByte      `json:"bytes"`
	Trailer         *DumpDiffTrailer    `json:"trailer,omitempty"`
}

type DumpDiffIdentity struct {
	Card           nfc.CardInfo  `json:"card"`
	Layout         mifare.Layout `json:"-"`
	LayoutName     string        `json:"layout"`
	Complete       bool          `json:"complete"`
	ReadableBlocks int           `json:"readableBlocks"`
	TotalBlocks    int           `json:"totalBlocks"`
}

type DumpDiff struct {
	Layout          mifare.Layout    `json:"-"`
	LayoutName      string           `json:"layout"`
	Before          DumpDiffIdentity `json:"before"`
	After           DumpDiffIdentity `json:"after"`
	IdentityChanged bool             `json:"identityChanged"`
	ChangedBlocks   int              `json:"changedBlocks"`
	EqualBlocks     int              `json:"equalBlocks"`
	UnknownBlocks   int              `json:"unknownBlocks"`
	TrailerChanges  int              `json:"trailerChanges"`
	Blocks          []DumpDiffBlock  `json:"blocks"`
}

// Readability counts fully known blocks and sectors. A sector is readable only
// when every one of its blocks is readable.
type Readability struct {
	ReadableBlocks  int   `json:"readableBlocks"`
	TotalBlocks     int   `json:"totalBlocks"`
	ReadableSectors int   `json:"readableSectors"`
	TotalSectors    int   `json:"totalSectors"`
	SkippedSectors  []int `json:"skippedSectors"`
}

func DumpReadability(image mifare.Dump) Readability {
	readability := Readability{TotalBlocks: len(image.Blocks), SkippedSectors: []int{}}
	readable := make([]bool, len(image.Blocks))
	for index, block := range image.Blocks {
		if block.Complete() {
			readability.ReadableBlocks++
			readable[index] = true
		}
	}
	sectors, err := image.Layout.SectorCount()
	if err != nil {
		return readability
	}
	readability.TotalSectors = sectors
	for sector := range sectors {
		first, err := image.Layout.FirstBlock(sector)
		if err != nil {
			continue
		}
		count, err := image.Layout.BlocksInSector(sector)
		if err != nil {
			continue
		}
		complete := first+count <= len(readable)
		for block := first; complete && block < first+count; block++ {
			complete = complete && readable[block]
		}
		if complete {
			readability.ReadableSectors++
		} else {
			readability.SkippedSectors = append(readability.SkippedSectors, sector)
		}
	}
	return readability
}

// DiffDumps compares two images block-wise without touching a reader.
func DiffDumps(before, after DumpResult) (DumpDiff, error) {
	if !before.Dump.Layout.Valid() || !after.Dump.Layout.Valid() {
		return DumpDiff{}, fmt.Errorf("both dumps must use a supported MIFARE Classic layout")
	}
	if before.Dump.Layout != after.Dump.Layout {
		return DumpDiff{}, fmt.Errorf("dump layouts do not match: %d and %d", before.Dump.Layout, after.Dump.Layout)
	}
	if len(before.Dump.Blocks) != len(after.Dump.Blocks) {
		return DumpDiff{}, fmt.Errorf("dump block counts do not match: %d and %d", len(before.Dump.Blocks), len(after.Dump.Blocks))
	}
	readabilityBefore, readabilityAfter := DumpReadability(before.Dump), DumpReadability(after.Dump)
	diff := DumpDiff{
		Layout: before.Dump.Layout, LayoutName: layoutName(before.Dump.Layout),
		Before: DumpDiffIdentity{
			Card: before.Card.Clone(), Layout: before.Dump.Layout, LayoutName: layoutName(before.Dump.Layout),
			Complete: before.Complete(), ReadableBlocks: readabilityBefore.ReadableBlocks, TotalBlocks: readabilityBefore.TotalBlocks,
		},
		After: DumpDiffIdentity{
			Card: after.Card.Clone(), Layout: after.Dump.Layout, LayoutName: layoutName(after.Dump.Layout),
			Complete: after.Complete(), ReadableBlocks: readabilityAfter.ReadableBlocks, TotalBlocks: readabilityAfter.TotalBlocks,
		},
		Blocks: make([]DumpDiffBlock, 0),
	}
	diff.IdentityChanged = !sameIdentity(before.Card, after.Card)

	for block := range before.Dump.Blocks {
		left, right := before.Dump.Blocks[block], after.Dump.Blocks[block]
		entry := DumpDiffBlock{
			Block: block, Sector: left.Sector, Kind: diffBlockKind(before.Dump.Layout, block),
			Before: left.Data, After: right.Data, BeforeKnownMask: left.KnownMask, AfterKnownMask: right.KnownMask,
			Bytes: []DumpDiffByte{},
		}
		allKnown := left.Complete() && right.Complete()
		equal := allKnown
		for offset := range nfc.BlockSize {
			mask := uint16(1 << offset)
			beforeKnown, afterKnown := left.KnownMask&mask != 0, right.KnownMask&mask != 0
			if left.Data[offset] != right.Data[offset] || beforeKnown != afterKnown {
				entry.Bytes = append(entry.Bytes, DumpDiffByte{
					Offset: offset, Before: left.Data[offset], After: right.Data[offset],
					BeforeKnown: beforeKnown, AfterKnown: afterKnown,
				})
				equal = false
			}
			allKnown = allKnown && beforeKnown && afterKnown
		}
		switch {
		case !allKnown:
			entry.State = DumpDiffUnknown
			diff.UnknownBlocks++
		case equal:
			entry.State = DumpDiffEqual
			diff.EqualBlocks++
		default:
			entry.State = DumpDiffChanged
			diff.ChangedBlocks++
		}
		if entry.Kind == "trailer" {
			entry.Trailer = diffTrailer(left, right)
			if entry.Trailer != nil && (entry.Trailer.KeyAChanged || entry.Trailer.KeyBChanged || entry.Trailer.AccessChanged || entry.Trailer.GPBChanged) {
				diff.TrailerChanges++
			}
		}
		diff.Blocks = append(diff.Blocks, entry)
	}
	return diff, nil
}

func sameIdentity(left, right nfc.CardInfo) bool {
	return nfc.SameCard(left, right)
}

func diffBlockKind(layout mifare.Layout, block int) string {
	if block == 0 {
		return "manufacturer"
	}
	if trailer, err := layout.IsTrailer(block); err == nil && trailer {
		return "trailer"
	}
	return "data"
}

func diffTrailer(before, after mifare.Block) *DumpDiffTrailer {
	trailer := &DumpDiffTrailer{
		Valid:      true,
		BeforeKeyA: fmt.Sprintf("%X", before.Data[0:6]), AfterKeyA: fmt.Sprintf("%X", after.Data[0:6]),
		BeforeKeyB: fmt.Sprintf("%X", before.Data[10:16]), AfterKeyB: fmt.Sprintf("%X", after.Data[10:16]),
		BeforeAccess: [3]byte{before.Data[6], before.Data[7], before.Data[8]},
		AfterAccess:  [3]byte{after.Data[6], after.Data[7], after.Data[8]},
		BeforeGPB:    before.Data[9], AfterGPB: after.Data[9],
	}
	keyACompared := before.KnownMask&0x003f == 0x003f && after.KnownMask&0x003f == 0x003f
	keyBCompared := before.KnownMask&0xfc00 == 0xfc00 && after.KnownMask&0xfc00 == 0xfc00
	trailer.KeyAChanged = keyACompared && trailer.BeforeKeyA != trailer.AfterKeyA
	trailer.KeyBChanged = keyBCompared && trailer.BeforeKeyB != trailer.AfterKeyB
	trailer.AccessChanged = trailer.BeforeAccess != trailer.AfterAccess
	trailer.GPBChanged = trailer.BeforeGPB != trailer.AfterGPB
	if _, err := mifare.DecodeAccessBits(trailer.BeforeAccess); err != nil {
		trailer.Valid = false
		trailer.Error = "before: " + err.Error()
	}
	if _, err := mifare.DecodeAccessBits(trailer.AfterAccess); err != nil {
		trailer.Valid = false
		if trailer.Error != "" {
			trailer.Error += "; after: " + err.Error()
		} else {
			trailer.Error = "after: " + err.Error()
		}
	}
	return trailer
}
