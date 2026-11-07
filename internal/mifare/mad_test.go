package mifare_test

import (
	"encoding/hex"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
)

func madTrailer(keyA string, access [3]byte, gpb byte, keyB string) [mifare.BlockSize]byte {
	value := keyA + hex.EncodeToString(access[:]) + hex.EncodeToString([]byte{gpb}) + keyB
	decoded, _ := hex.DecodeString(value)
	var block [mifare.BlockSize]byte
	copy(block[:], decoded)
	return block
}

func mad1Input() mifare.MADInput {
	var input mifare.MADInput
	input.Sector0[1] = [mifare.BlockSize]byte{
		0xF3, 0x01, 0x03, 0xE1, 0x03, 0xE1, 0x00, 0x00,
		0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	}
	input.Sector0[3] = madTrailer("A0A1A2A3A4A5", [3]byte{0x78, 0x77, 0x88}, 0xC1, "FFFFFFFFFFFF")
	return input
}

func TestParseMAD1FindsNDEFSectors(t *testing.T) {
	mad, err := mifare.ParseMAD(mifare.Classic1K, mad1Input())
	if err != nil {
		t.Fatal(err)
	}
	if !mad.Present || mad.Version != 1 || !mad.MultiApplication || mad.GPB() != 0xC1 {
		t.Fatalf("MAD = %+v", mad)
	}
	sectors, err := mad.NDEFSectors()
	if err != nil {
		t.Fatal(err)
	}
	if len(sectors) != 2 || sectors[0] != 1 || sectors[1] != 2 {
		t.Fatalf("NDEF sectors = %v", sectors)
	}
	if got := mad.FreeSectors(); len(got) != 13 {
		t.Fatalf("free sectors = %v", got)
	}
}

func TestParseMADRejectsBadChecksumAndVersion(t *testing.T) {
	input := mad1Input()
	input.Sector0[1][0] = 0x00
	if _, err := mifare.ParseMAD(mifare.Classic1K, input); !errors.Is(err, mifare.ErrMADChecksum) {
		t.Fatalf("bad CRC error = %v", err)
	}

	input = mad1Input()
	input.Sector0[3][9] = 0xC7
	if _, err := mifare.ParseMAD(mifare.Classic1K, input); !errors.Is(err, mifare.ErrMADVersion) {
		t.Fatalf("bad version error = %v", err)
	}
}

func TestParseMADWithoutDirectoryIsNotAnError(t *testing.T) {
	input := mad1Input()
	input.Sector0[3][9] = 0x69
	mad, err := mifare.ParseMAD(mifare.Classic1K, input)
	if err != nil {
		t.Fatal(err)
	}
	if mad.Present {
		t.Fatal("MAD without the DA bit was reported present")
	}
	if _, err := mad.NDEFSectors(); !errors.Is(err, mifare.ErrNoMAD) {
		t.Fatalf("NDEFSectors() error = %v", err)
	}
}

func TestMADEncodeRoundTripAndAIDByteOrder(t *testing.T) {
	mad, err := mifare.ParseMAD(mifare.Classic1K, mad1Input())
	if err != nil {
		t.Fatal(err)
	}
	if err := mad.SetAID(3, mifare.NDEFAID); err != nil {
		t.Fatal(err)
	}
	blocks, err := mad.Blocks()
	if err != nil {
		t.Fatal(err)
	}
	if blocks.Sector0[0][2] != 0x03 || blocks.Sector0[0][3] != 0xE1 {
		t.Fatalf("sector 1 AID bytes = %02X %02X", blocks.Sector0[0][2], blocks.Sector0[0][3])
	}
	if blocks.Sector0[0][6] != 0x03 || blocks.Sector0[0][7] != 0xE1 {
		t.Fatalf("sector 3 AID bytes = %02X %02X", blocks.Sector0[0][6], blocks.Sector0[0][7])
	}
	input := mad1Input()
	input.Sector0[1], input.Sector0[2] = blocks.Sector0[0], blocks.Sector0[1]
	reparsed, err := mifare.ParseMAD(mifare.Classic1K, input)
	if err != nil {
		t.Fatal(err)
	}
	sectors, err := reparsed.NDEFSectors()
	if err != nil {
		t.Fatal(err)
	}
	if len(sectors) != 3 || sectors[2] != 3 {
		t.Fatalf("NDEF sectors = %v", sectors)
	}
}

func TestMAD2MapsSectorsAroundTheSecondDirectory(t *testing.T) {
	// Recompute CRCs through the exported encoder by parsing a scaffold MAD.
	scaffold := mifare.MAD{
		Present: true, Version: 2, AIDs: map[int]uint16{},
	}
	for sector := 1; sector <= 15; sector++ {
		scaffold.AIDs[sector] = mifare.NDEFAID
	}
	for sector := 17; sector <= 19; sector++ {
		scaffold.AIDs[sector] = mifare.NDEFAID
	}
	encoded, err := scaffold.Blocks()
	if err != nil {
		t.Fatal(err)
	}
	var input mifare.MADInput
	input.Sector0[1], input.Sector0[2] = encoded.Sector0[0], encoded.Sector0[1]
	input.Sector0[3][9] = 0xC2
	sector16 := [4][mifare.BlockSize]byte{}
	sector16[0], sector16[1], sector16[2] = encoded.Sector16[0], encoded.Sector16[1], encoded.Sector16[2]
	input.Sector16 = &sector16
	mad, err := mifare.ParseMAD(mifare.Classic4K, input)
	if err != nil {
		t.Fatal(err)
	}
	sectors, err := mad.NDEFSectors()
	if err != nil {
		t.Fatal(err)
	}
	want := []int{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19}
	if len(sectors) != len(want) {
		t.Fatalf("NDEF sectors = %v", sectors)
	}
	for index := range want {
		if sectors[index] != want[index] {
			t.Fatalf("NDEF sectors = %v", sectors)
		}
	}

	if err := mad.SetAID(16, mifare.NDEFAID); err == nil {
		t.Fatal("sector 16 accepted as a managed application sector")
	}
}

func TestNDEFAreaBlocksSkipTrailers(t *testing.T) {
	blocks, err := mifare.NDEFAreaBlocks(mifare.Classic1K, []int{1, 2})
	if err != nil {
		t.Fatal(err)
	}
	want := []int{4, 5, 6, 8, 9, 10}
	if len(blocks) != len(want) {
		t.Fatalf("blocks = %v", blocks)
	}
	for index := range want {
		if blocks[index] != want[index] {
			t.Fatalf("blocks = %v", blocks)
		}
	}
	if _, err := mifare.NDEFAreaBlocks(mifare.Classic1K, []int{0}); err == nil {
		t.Fatal("MAD sector accepted as an NDEF area")
	}
}

func TestValidateNDEFGPB(t *testing.T) {
	trailer := [mifare.BlockSize]byte{}
	trailer[9] = 0x40
	if readOnly, err := mifare.ValidateNDEFGPB(trailer); err != nil || readOnly {
		t.Fatalf("writeable GPB = %v, %v", readOnly, err)
	}
	trailer[9] = 0x43
	if readOnly, err := mifare.ValidateNDEFGPB(trailer); err != nil || !readOnly {
		t.Fatalf("read-only GPB = %v, %v", readOnly, err)
	}
	trailer[9] = 0x41
	if _, err := mifare.ValidateNDEFGPB(trailer); err == nil {
		t.Fatal("proprietary write access accepted")
	}
	trailer[9] = 0x80
	if _, err := mifare.ValidateNDEFGPB(trailer); err == nil {
		t.Fatal("mapping version 2.0 accepted")
	}
}
