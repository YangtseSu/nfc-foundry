package workflow

import (
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// KeyArray is the Proxmark3 raw key array: every sector's Key A followed by
// every sector's Key B, without sector headers or text.
type KeyArray struct {
	Layout  mifare.Layout
	Sectors []VerifiedSectorKeys
}

func keyArraySize(layout mifare.Layout) (int, error) {
	sectors, err := layout.SectorCount()
	if err != nil {
		return 0, err
	}
	return sectors * 2 * nfc.KeySize, nil
}

// ParseKeyArray reads a Proxmark3 1K (192-byte) or 4K (480-byte) raw key
// array. The first half holds Key A for each sector; the second half holds
// Key B.
func ParseKeyArray(data []byte) (KeyArray, error) {
	var layout mifare.Layout
	switch len(data) {
	case mifare.Classic1KSectors * 2 * nfc.KeySize:
		layout = mifare.Classic1K
	case mifare.Classic4KSectors * 2 * nfc.KeySize:
		layout = mifare.Classic4K
	default:
		return KeyArray{}, fmt.Errorf("raw key array must be %d or %d bytes, got %d",
			mifare.Classic1KSectors*2*nfc.KeySize, mifare.Classic4KSectors*2*nfc.KeySize, len(data))
	}
	sectors, _ := layout.SectorCount()
	half := sectors * nfc.KeySize
	array := KeyArray{Layout: layout, Sectors: make([]VerifiedSectorKeys, sectors)}
	for sector := range sectors {
		keyA, err := nfc.KeyFromBytes(data[sector*nfc.KeySize : (sector+1)*nfc.KeySize])
		if err != nil {
			return KeyArray{}, fmt.Errorf("sector %d Key A: %w", sector, err)
		}
		keyB, err := nfc.KeyFromBytes(data[half+sector*nfc.KeySize : half+(sector+1)*nfc.KeySize])
		if err != nil {
			return KeyArray{}, fmt.Errorf("sector %d Key B: %w", sector, err)
		}
		array.Sectors[sector] = VerifiedSectorKeys{Sector: sector, KeyA: &keyA, KeyB: &keyB}
	}
	return array, nil
}

// EncodeKeyArray extracts the trailer keys from a complete dump in the
// Proxmark3 raw key-array layout.
func EncodeKeyArray(result DumpResult) ([]byte, error) {
	if err := validateCompleteDump(result); err != nil {
		return nil, err
	}
	size, err := keyArraySize(result.Dump.Layout)
	if err != nil {
		return nil, err
	}
	sectors, _ := result.Dump.Layout.SectorCount()
	half := sectors * nfc.KeySize
	data := make([]byte, size)
	for sector := range sectors {
		trailerBlock, err := result.Dump.Layout.TrailerBlock(sector)
		if err != nil {
			return nil, err
		}
		trailer := result.Dump.Blocks[trailerBlock]
		if !trailer.Complete() {
			return nil, fmt.Errorf("sector %d trailer is incomplete; a raw key array was not written", sector)
		}
		copy(data[sector*nfc.KeySize:], trailer.Data[0:nfc.KeySize])
		copy(data[half+sector*nfc.KeySize:], trailer.Data[10:10+nfc.KeySize])
	}
	return data, nil
}
