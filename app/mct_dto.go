package app

import (
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/workbench"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

type ValueBlockDTO struct {
	Valid    bool   `json:"valid"`
	Error    string `json:"error,omitempty"`
	Value    int64  `json:"value"`
	Address  int    `json:"address"`
	ValueHex string `json:"valueHex,omitempty"`
}

type BCCBlockDTO struct {
	Applicable bool   `json:"applicable"`
	Valid      bool   `json:"valid"`
	Expected   string `json:"expected,omitempty"`
	Actual     string `json:"actual,omitempty"`
	Error      string `json:"error,omitempty"`
}

type ValueBlockPreviewRequestDTO struct {
	Block   int    `json:"block"`
	Value   string `json:"value"`
	Address int    `json:"address"`
}

type ValueBlockPreviewDTO struct {
	Block   int           `json:"block"`
	Sector  int           `json:"sector"`
	Hex     string        `json:"hex"`
	Decoded ValueBlockDTO `json:"decoded"`
	Warning string        `json:"warning,omitempty"`
}

type AccessConditionInputDTO struct {
	Group int  `json:"group"`
	C1    bool `json:"c1"`
	C2    bool `json:"c2"`
	C3    bool `json:"c3"`
}

type TrailerPreviewRequestDTO struct {
	Block  int                       `json:"block"`
	KeyA   string                    `json:"keyA"`
	KeyB   string                    `json:"keyB"`
	GPB    string                    `json:"gpb,omitempty"`
	Groups []AccessConditionInputDTO `json:"groups"`
}

type TrailerPreviewDTO struct {
	Block  int              `json:"block"`
	Sector int              `json:"sector"`
	OldHex string           `json:"oldHex"`
	Hex    string           `json:"hex"`
	KeyA   string           `json:"keyA"`
	KeyB   string           `json:"keyB"`
	GPB    string           `json:"gpb"`
	Access AccessSummaryDTO `json:"access"`
}

type DumpIdentityDTO struct {
	UID            string `json:"uid,omitempty"`
	ATQA           string `json:"atqa,omitempty"`
	SAK            string `json:"sak,omitempty"`
	Layout         string `json:"layout"`
	Complete       bool   `json:"complete"`
	ReadableBlocks int    `json:"readableBlocks"`
	TotalBlocks    int    `json:"totalBlocks"`
}

type DumpTrailerDiffDTO struct {
	Valid         bool   `json:"valid"`
	Error         string `json:"error,omitempty"`
	KeyAChanged   bool   `json:"keyAChanged"`
	KeyBChanged   bool   `json:"keyBChanged"`
	AccessChanged bool   `json:"accessChanged"`
	GPBChanged    bool   `json:"gpbChanged"`
	BeforeKeyA    string `json:"beforeKeyA"`
	AfterKeyA     string `json:"afterKeyA"`
	BeforeKeyB    string `json:"beforeKeyB"`
	AfterKeyB     string `json:"afterKeyB"`
	BeforeAccess  string `json:"beforeAccess"`
	AfterAccess   string `json:"afterAccess"`
	BeforeGPB     string `json:"beforeGpb"`
	AfterGPB      string `json:"afterGpb"`
}

type DumpBlockDiffDTO struct {
	Block           int                 `json:"block"`
	Sector          int                 `json:"sector"`
	Kind            string              `json:"kind"`
	State           string              `json:"state"`
	BeforeHex       string              `json:"beforeHex"`
	AfterHex        string              `json:"afterHex"`
	BeforeASCII     string              `json:"beforeAscii"`
	AfterASCII      string              `json:"afterAscii"`
	BeforeKnownMask string              `json:"beforeKnownMask"`
	AfterKnownMask  string              `json:"afterKnownMask"`
	Bytes           []ByteDiffDTO       `json:"bytes"`
	Trailer         *DumpTrailerDiffDTO `json:"trailer,omitempty"`
}

type DumpDiffDTO struct {
	Layout          string             `json:"layout"`
	Cancelled       bool               `json:"cancelled,omitempty"`
	Before          DumpIdentityDTO    `json:"before"`
	After           DumpIdentityDTO    `json:"after"`
	IdentityChanged bool               `json:"identityChanged"`
	ChangedBlocks   int                `json:"changedBlocks"`
	EqualBlocks     int                `json:"equalBlocks"`
	UnknownBlocks   int                `json:"unknownBlocks"`
	TrailerChanges  int                `json:"trailerChanges"`
	Blocks          []DumpBlockDiffDTO `json:"blocks"`
}

type ClonePreflightDTO struct {
	Token         string          `json:"token,omitempty"`
	Valid         bool            `json:"valid"`
	Error         string          `json:"error,omitempty"`
	Source        DumpIdentityDTO `json:"source"`
	Target        CardDTO         `json:"target"`
	Block0Write   bool            `json:"block0Write"`
	OldBlock0     string          `json:"oldBlock0"`
	NewBlock0     string          `json:"newBlock0"`
	Blocks        []int           `json:"blocks"`
	TrailerBlocks []int           `json:"trailerBlocks"`
	BackupPath    string          `json:"backupPath,omitempty"`
	Warnings      []string        `json:"warnings"`
}

type CloneStartRequestDTO struct {
	Token        string `json:"token"`
	Confirmation string `json:"confirmation"`
}

type CloneResultDTO struct {
	Phase      string             `json:"phase"`
	SourcePath string             `json:"sourcePath,omitempty"`
	BackupPath string             `json:"backupPath,omitempty"`
	Restore    *RestoreResultDTO  `json:"restore,omitempty"`
	UID        *UIDWriteResultDTO `json:"uid,omitempty"`
	Diff       *DumpDiffDTO       `json:"diff,omitempty"`
	Verified   bool               `json:"verified"`
}

type FactoryPreflightDTO struct {
	Token         string   `json:"token,omitempty"`
	Valid         bool     `json:"valid"`
	Error         string   `json:"error,omitempty"`
	Card          CardDTO  `json:"card"`
	Blocks        []int    `json:"blocks"`
	TrailerBlocks []int    `json:"trailerBlocks"`
	TrailerBytes  string   `json:"trailerBytes"`
	KeepData      bool     `json:"keepData"`
	BackupPath    string   `json:"backupPath,omitempty"`
	Warnings      []string `json:"warnings"`
}

type FactoryStartRequestDTO struct {
	Token        string `json:"token"`
	Confirmation string `json:"confirmation"`
}

type FactoryResultDTO struct {
	Phase      string            `json:"phase"`
	BackupPath string            `json:"backupPath,omitempty"`
	Restore    *RestoreResultDTO `json:"restore,omitempty"`
	Diff       *DumpDiffDTO      `json:"diff,omitempty"`
	Verified   bool              `json:"verified"`
}

type KeyFileLineDTO struct {
	Number    int    `json:"number"`
	Text      string `json:"text"`
	Key       string `json:"key,omitempty"`
	Problem   string `json:"problem,omitempty"`
	Duplicate bool   `json:"duplicate"`
	Comment   bool   `json:"comment"`
	Ignored   bool   `json:"ignored"`
}

type KeyFilePreviewDTO struct {
	Valid      int              `json:"valid"`
	Duplicates int              `json:"duplicates"`
	Ignored    int              `json:"ignored"`
	Issues     []ImportIssueDTO `json:"issues"`
	Lines      []KeyFileLineDTO `json:"lines"`
}

type KeyFileContentDTO struct {
	Path    string `json:"path"`
	Content string `json:"content"`
}

func valueBlockDTO(data [16]byte) *ValueBlockDTO {
	decoded, err := mifare.DecodeValueBlock(data)
	if err != nil {
		return nil
	}
	return &ValueBlockDTO{
		Valid: true, Value: int64(decoded.Value), Address: int(decoded.Address),
		ValueHex: fmt.Sprintf("%08X", uint32(decoded.Value)),
	}
}

func bccBlockDTO(block mifare.Block) *BCCBlockDTO {
	if block.Number != 0 {
		return nil
	}
	dto := &BCCBlockDTO{Applicable: true}
	if block.KnownMask&0x1f != 0x1f {
		dto.Error = "UID and BCC bytes are incomplete"
		return dto
	}
	expected, err := mifare.BCCFor4ByteUID(block.Data[0:4])
	if err != nil {
		dto.Applicable = false
		dto.Error = err.Error()
		return dto
	}
	dto.Expected = fmt.Sprintf("%02X", expected)
	dto.Actual = fmt.Sprintf("%02X", block.Data[4])
	dto.Valid = expected == block.Data[4]
	if !dto.Valid {
		dto.Error = fmt.Sprintf("BCC is %02X, expected %02X", block.Data[4], expected)
	}
	return dto
}

func dumpIdentityDTO(result workflow.DumpResult) DumpIdentityDTO {
	readability := workflow.DumpReadability(result.Dump)
	return DumpIdentityDTO{
		UID: formatHex(result.Card.UID), ATQA: formatHex(result.Card.ATQA[:]), SAK: fmt.Sprintf("%02X", result.Card.SAK),
		Layout: dumpLayoutName(result.Dump.Layout), Complete: result.Complete(),
		ReadableBlocks: readability.ReadableBlocks, TotalBlocks: readability.TotalBlocks,
	}
}

func dumpDiffDTO(diff workflow.DumpDiff) DumpDiffDTO {
	identity := func(value workflow.DumpDiffIdentity) DumpIdentityDTO {
		return DumpIdentityDTO{
			UID: formatHex(value.Card.UID), ATQA: formatHex(value.Card.ATQA[:]), SAK: fmt.Sprintf("%02X", value.Card.SAK),
			Layout: value.LayoutName, Complete: value.Complete,
			ReadableBlocks: value.ReadableBlocks, TotalBlocks: value.TotalBlocks,
		}
	}
	dto := DumpDiffDTO{
		Layout: diff.LayoutName, Before: identity(diff.Before), After: identity(diff.After),
		IdentityChanged: diff.IdentityChanged, ChangedBlocks: diff.ChangedBlocks, EqualBlocks: diff.EqualBlocks,
		UnknownBlocks: diff.UnknownBlocks, TrailerChanges: diff.TrailerChanges, Blocks: make([]DumpBlockDiffDTO, len(diff.Blocks)),
	}
	for index, block := range diff.Blocks {
		entry := DumpBlockDiffDTO{
			Block: block.Block, Sector: block.Sector, Kind: block.Kind, State: string(block.State),
			BeforeHex: strings.ToUpper(fmt.Sprintf("%X", block.Before)), AfterHex: strings.ToUpper(fmt.Sprintf("%X", block.After)),
			BeforeASCII: workbench.ASCII(block.Before), AfterASCII: workbench.ASCII(block.After),
			BeforeKnownMask: fmt.Sprintf("%04X", block.BeforeKnownMask), AfterKnownMask: fmt.Sprintf("%04X", block.AfterKnownMask),
			Bytes: make([]ByteDiffDTO, len(block.Bytes)),
		}
		for byteIndex, item := range block.Bytes {
			entry.Bytes[byteIndex] = ByteDiffDTO{
				Offset: item.Offset, Before: fmt.Sprintf("%02X", item.Before), After: fmt.Sprintf("%02X", item.After),
				WasKnown: item.BeforeKnown, NowKnown: item.AfterKnown,
			}
		}
		if block.Trailer != nil {
			trailer := &DumpTrailerDiffDTO{
				Valid: block.Trailer.Valid, Error: block.Trailer.Error,
				KeyAChanged: block.Trailer.KeyAChanged, KeyBChanged: block.Trailer.KeyBChanged,
				AccessChanged: block.Trailer.AccessChanged, GPBChanged: block.Trailer.GPBChanged,
				BeforeKeyA: block.Trailer.BeforeKeyA, AfterKeyA: block.Trailer.AfterKeyA,
				BeforeKeyB: block.Trailer.BeforeKeyB, AfterKeyB: block.Trailer.AfterKeyB,
				BeforeAccess: fmt.Sprintf("%02X%02X%02X", block.Trailer.BeforeAccess[0], block.Trailer.BeforeAccess[1], block.Trailer.BeforeAccess[2]),
				AfterAccess:  fmt.Sprintf("%02X%02X%02X", block.Trailer.AfterAccess[0], block.Trailer.AfterAccess[1], block.Trailer.AfterAccess[2]),
				BeforeGPB:    fmt.Sprintf("%02X", block.Trailer.BeforeGPB), AfterGPB: fmt.Sprintf("%02X", block.Trailer.AfterGPB),
			}
			entry.Trailer = trailer
		}
		dto.Blocks[index] = entry
	}
	return dto
}

func keyFilePreviewDTO(analysis keys.DictionaryAnalysis) KeyFilePreviewDTO {
	dto := KeyFilePreviewDTO{
		Valid: analysis.Report.Valid, Duplicates: analysis.Report.Duplicates, Ignored: analysis.Report.Ignored,
		Issues: make([]ImportIssueDTO, len(analysis.Report.Issues)), Lines: make([]KeyFileLineDTO, len(analysis.Lines)),
	}
	for index, issue := range analysis.Report.Issues {
		dto.Issues[index] = ImportIssueDTO{Line: issue.Line, Message: issue.Message}
	}
	for index, line := range analysis.Lines {
		entry := KeyFileLineDTO{
			Number: line.Number, Text: line.Text, Problem: line.Problem,
			Duplicate: line.Duplicate, Comment: line.Comment, Ignored: line.Ignored,
		}
		if line.Key != nil {
			entry.Key = strings.ToUpper(fmt.Sprintf("%X", line.Key[:]))
		}
		dto.Lines[index] = entry
	}
	return dto
}
