package mifare

import (
	"errors"
	"fmt"
	"sort"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

const (
	// MADSector contains the MAD1 directory and, on 4K cards, the first half
	// of MAD2.
	MADSector = 0
	// MAD2Sector contains the second half of a MAD2 directory.
	MAD2Sector = 16
	// NDEFAID identifies sectors that store the NDEF application.
	NDEFAID uint16 = 0x03E1
)

var (
	// NDEFPublicKey is the standardized public Key A of an NDEF sector.
	NDEFPublicKey = nfc.Key{0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7}
	// MADPublicKey is the standardized public Key A of a MAD sector.
	MADPublicKey = nfc.Key{0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5}
)

var (
	// ErrNoMAD means the card does not advertise a MIFARE Application
	// Directory. It is not a corruption error.
	ErrNoMAD = errors.New("card has no MIFARE Application Directory")
	// ErrMADChecksum means the directory advertises itself but its CRC is
	// wrong. The mapping must not be trusted or modified.
	ErrMADChecksum = errors.New("MIFARE Application Directory checksum does not match")
	// ErrMADVersion means the directory uses a mapping this product does not
	// implement.
	ErrMADVersion = errors.New("unsupported MIFARE Application Directory version")
	// ErrNoNDEFApplication means a valid MAD has no sector marked as NDEF.
	ErrNoNDEFApplication = errors.New("MAD has no NDEF application")
	// ErrNDEFNotContiguous means NDEF sectors are not the contiguous run the
	// mapping requires.
	ErrNDEFNotContiguous = errors.New("NDEF sectors are not contiguous")
)

const (
	madDA       byte = 0x80
	madMA       byte = 0x40
	madVersion1      = 1
	madVersion2      = 2
)

// MADInput is the raw MAD and trailer content read from a card.
type MADInput struct {
	// Sector0 holds blocks 0..3 of sector 0. Blocks 1 and 2 carry MAD1;
	// block 3 is the trailer whose GPB selects the mapping version.
	Sector0 [4][BlockSize]byte
	// Sector16 holds blocks 0..3 of sector 16 for MAD2. It must be nil when
	// the card does not use MAD2.
	Sector16 *[4][BlockSize]byte
}

// MAD is a validated MIFARE Application Directory. AIDs keeps administration
// codes such as free (0x0000) so callers can distinguish free sectors from
// unknown applications.
type MAD struct {
	Present          bool
	Version          int
	MultiApplication bool
	Info             byte
	Info16           byte
	AIDs             map[int]uint16

	gpb   byte
	gpb16 byte
}

// MADBlocks is the encoded directory content, excluding sector trailers.
type MADBlocks struct {
	Sector0  [2][BlockSize]byte
	Sector16 *[3][BlockSize]byte
}

// ParseMAD validates a MAD read from a card. A clear DA bit returns a
// non-present MAD without an error. A set DA bit with a bad CRC, an
// unsupported version, or an inconsistent layout is an error.
func ParseMAD(layout Layout, input MADInput) (MAD, error) {
	if !layout.Valid() {
		return MAD{}, invalidLayout(layout)
	}
	mad := MAD{Present: false, AIDs: make(map[int]uint16)}
	gpb := input.Sector0[3][9]
	if gpb&madDA == 0 {
		return mad, nil
	}
	version := int(gpb & 0x03)
	switch version {
	case madVersion1:
	case madVersion2:
		if layout != Classic4K {
			return MAD{}, fmt.Errorf("%w: MAD2 requires a 4K card", ErrMADVersion)
		}
		if input.Sector16 == nil {
			return MAD{}, fmt.Errorf("%w: MAD2 sector 16 is missing", ErrMADVersion)
		}
	default:
		return MAD{}, fmt.Errorf("%w: version code %d", ErrMADVersion, version)
	}

	if got, want := input.Sector0[1][0], madSectorCRC(input.Sector0[1], input.Sector0[2]); got != want {
		return MAD{}, fmt.Errorf("%w: sector 0 CRC is %02X, expected %02X", ErrMADChecksum, got, want)
	}
	if version == madVersion2 {
		if got, want := input.Sector16[0][0], madSectorCRC(input.Sector16[0], input.Sector16[1], input.Sector16[2]); got != want {
			return MAD{}, fmt.Errorf("%w: sector 16 CRC is %02X, expected %02X", ErrMADChecksum, got, want)
		}
		if input.Sector16[3][9]&madDA != 0 {
			return MAD{}, fmt.Errorf("%w: sector 16 trailer advertises a second MAD", ErrMADVersion)
		}
	}

	mad.Present = true
	mad.Version = version
	mad.MultiApplication = gpb&madMA != 0
	mad.gpb = gpb
	mad.Info = input.Sector0[1][1]
	for index := range 7 {
		mad.AIDs[index+1] = aidAt(input.Sector0[1][2+index*2:])
	}
	for index := range 8 {
		mad.AIDs[index+8] = aidAt(input.Sector0[2][index*2:])
	}
	if version == madVersion2 {
		mad.gpb16 = input.Sector16[3][9]
		mad.Info16 = input.Sector16[0][1]
		for index := range 7 {
			mad.AIDs[index+17] = aidAt(input.Sector16[0][2+index*2:])
		}
		for index := range 8 {
			mad.AIDs[index+24] = aidAt(input.Sector16[1][index*2:])
		}
		for index := range 8 {
			mad.AIDs[index+32] = aidAt(input.Sector16[2][index*2:])
		}
	}
	return mad, nil
}

// GPB returns the sector 0 trailer General Purpose Byte.
func (m MAD) GPB() byte { return m.gpb }

// GPB16 returns the sector 16 trailer General Purpose Byte for MAD2.
func (m MAD) GPB16() byte { return m.gpb16 }

// PublisherSector returns the card publisher pointer from the MAD1 info byte.
func (m MAD) PublisherSector() int {
	if m.Version == madVersion2 {
		return int(m.Info & 0x3F)
	}
	return int(m.Info & 0x0F)
}

// ManagedSectors returns every sector addressable by this directory in
// ascending order. Sector 16 is a MAD sector and is never included.
func (m MAD) ManagedSectors() []int {
	if !m.Present {
		return nil
	}
	sectors := make([]int, 0, len(m.AIDs))
	for sector := range m.AIDs {
		sectors = append(sectors, sector)
	}
	sort.Ints(sectors)
	return sectors
}

// FreeSectors returns managed sectors whose AID marks them free.
func (m MAD) FreeSectors() []int {
	result := make([]int, 0)
	for _, sector := range m.ManagedSectors() {
		if m.AIDs[sector] == 0x0000 {
			result = append(result, sector)
		}
	}
	return result
}

// NDEFSectors returns the sorted, validated NDEF sector run.
func (m MAD) NDEFSectors() ([]int, error) {
	if !m.Present {
		return nil, ErrNoMAD
	}
	var sectors []int
	for sector, aid := range m.AIDs {
		if aid == NDEFAID {
			sectors = append(sectors, sector)
		}
	}
	if len(sectors) == 0 {
		return nil, ErrNoNDEFApplication
	}
	sort.Ints(sectors)
	for index := 1; index < len(sectors); index++ {
		previous, current := sectors[index-1], sectors[index]
		if current == previous+1 {
			continue
		}
		if m.Version == madVersion2 && previous < MAD2Sector && current > MAD2Sector {
			continue
		}
		return nil, fmt.Errorf("%w: sectors %d and %d", ErrNDEFNotContiguous, previous, current)
	}
	return sectors, nil
}

// SetAID assigns one managed sector an application identifier.
func (m *MAD) SetAID(sector int, aid uint16) error {
	if m == nil || !m.Present {
		return ErrNoMAD
	}
	if err := m.validateManagedSector(sector); err != nil {
		return err
	}
	if m.AIDs == nil {
		m.AIDs = make(map[int]uint16)
	}
	m.AIDs[sector] = aid
	return nil
}

// Blocks encodes the current directory with recomputed CRCs.
func (m MAD) Blocks() (MADBlocks, error) {
	if !m.Present {
		return MADBlocks{}, ErrNoMAD
	}
	if m.Version != madVersion1 && m.Version != madVersion2 {
		return MADBlocks{}, fmt.Errorf("%w: version code %d", ErrMADVersion, m.Version)
	}
	blocks := MADBlocks{}
	blocks.Sector0[0] = [BlockSize]byte{0, m.Info}
	for index := range 7 {
		putAID(blocks.Sector0[0][2+index*2:], m.AIDs[index+1])
	}
	for index := range 8 {
		putAID(blocks.Sector0[1][index*2:], m.AIDs[index+8])
	}
	blocks.Sector0[0][0] = madSectorCRC(blocks.Sector0[0], blocks.Sector0[1])
	if m.Version == madVersion1 {
		return blocks, nil
	}
	sector16 := &[3][BlockSize]byte{}
	sector16[0] = [BlockSize]byte{0, m.Info16}
	for index := range 7 {
		putAID(sector16[0][2+index*2:], m.AIDs[index+17])
	}
	for index := range 8 {
		putAID(sector16[1][index*2:], m.AIDs[index+24])
	}
	for index := range 8 {
		putAID(sector16[2][index*2:], m.AIDs[index+32])
	}
	sector16[0][0] = madSectorCRC(sector16[0], sector16[1], sector16[2])
	blocks.Sector16 = sector16
	return blocks, nil
}

// NDEFAreaBlocks returns the data blocks that form an NDEF byte area. Sector
// trailers are excluded because they are not part of the TLV stream.
func NDEFAreaBlocks(layout Layout, sectors []int) ([]int, error) {
	if !layout.Valid() {
		return nil, invalidLayout(layout)
	}
	var blocks []int
	for _, sector := range sectors {
		if sector == MADSector || sector == MAD2Sector {
			return nil, fmt.Errorf("sector %d is a MAD sector", sector)
		}
		first, err := layout.FirstBlock(sector)
		if err != nil {
			return nil, err
		}
		trailer, err := layout.TrailerBlock(sector)
		if err != nil {
			return nil, err
		}
		for block := first; block < trailer; block++ {
			blocks = append(blocks, block)
		}
	}
	return blocks, nil
}

// ValidateNDEFGPB checks the mapping version and access fields of an NDEF
// sector trailer. It reports whether the mapping marks the sector read-only.
func ValidateNDEFGPB(trailer [BlockSize]byte) (readOnly bool, err error) {
	gpb := trailer[9]
	major := gpb >> 6
	if major != 0x01 {
		return false, fmt.Errorf("%w: NDEF mapping major version %d", ErrMADVersion, major)
	}
	if read := (gpb >> 2) & 0x03; read != 0 {
		return false, fmt.Errorf("NDEF read access is not public")
	}
	switch write := gpb & 0x03; write {
	case 0x00:
		return false, nil
	case 0x03:
		return true, nil
	default:
		return false, fmt.Errorf("NDEF write access is proprietary")
	}
}

// ValidateMADGPB checks the directory version and MAD available bit.
func ValidateMADGPB(trailer [BlockSize]byte) (version int, err error) {
	gpb := trailer[9]
	if gpb&madDA == 0 {
		return 0, ErrNoMAD
	}
	switch int(gpb & 0x03) {
	case madVersion1:
		return madVersion1, nil
	case madVersion2:
		return madVersion2, nil
	default:
		return 0, fmt.Errorf("%w: version code %d", ErrMADVersion, gpb&0x03)
	}
}

// MADGPB returns the standardized trailing General Purpose Byte.
func MADGPB(version int) byte {
	switch version {
	case madVersion2:
		return madDA | madMA | 0x02
	default:
		return madDA | madMA | 0x01
	}
}

func (m MAD) validateManagedSector(sector int) error {
	if sector < 1 {
		return fmt.Errorf("sector %d is not managed by the MAD", sector)
	}
	if m.Version == madVersion2 {
		if sector == MAD2Sector || sector > 39 {
			return fmt.Errorf("sector %d is not managed by the MAD", sector)
		}
		return nil
	}
	if sector > 15 {
		return fmt.Errorf("sector %d is not managed by MAD1", sector)
	}
	return nil
}

func aidAt(data []byte) uint16 {
	if len(data) < 2 {
		return 0
	}
	return uint16(data[0])<<8 | uint16(data[1])
}

func putAID(data []byte, aid uint16) {
	if len(data) < 2 {
		return
	}
	data[0] = byte(aid >> 8)
	data[1] = byte(aid)
}

func madSectorCRC(blocks ...[BlockSize]byte) byte {
	crc := byte(0xC7)
	for index, block := range blocks {
		if index == 0 {
			crc = crc8Update(crc, block[1:]...)
			continue
		}
		crc = crc8Update(crc, block[:]...)
	}
	return crc
}

func crc8Update(crc byte, data ...byte) byte {
	const polynomial = 0x1D
	for _, value := range data {
		crc ^= value
		for bit := 0; bit < 8; bit++ {
			if crc&0x80 != 0 {
				crc = crc<<1 ^ polynomial
			} else {
				crc <<= 1
			}
		}
	}
	return crc
}
