package mock

import (
	"context"
	"fmt"
	"sync"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

// Type2Tag is a deterministic NFC Forum Type 2 memory image. It answers the
// commands used by the Type 2 module so workflow and CLI tests do not need
// hardware.
type Type2Tag struct {
	mu sync.Mutex

	product  type2.Product
	geometry type2.Geometry
	uid      []byte
	pages    [][4]byte
	password [4]byte
	pack     [2]byte

	// FailWritePage makes WRITE return a NAK for that page.
	FailWritePage int
	// CorruptWritePage stores a modified value for that page after ACK.
	CorruptWritePage int
	writeCalls       []int
}

// NewType2Tag creates an unformatted tag with the product's factory memory
// content. A zero UID gets a deterministic seven-byte test UID.
func NewType2Tag(product type2.Product, uid []byte) *Type2Tag {
	geometry, ok := type2.GeometryFor(product)
	if !ok {
		panic(fmt.Sprintf("unsupported mock Type 2 product %q", product))
	}
	if len(uid) == 0 {
		uid = []byte{0x04, 0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6}
	}
	if len(uid) != 7 {
		panic("mock Type 2 tags require a seven-byte UID")
	}
	tag := &Type2Tag{product: product, geometry: geometry, uid: append([]byte(nil), uid...), password: [4]byte{0xFF, 0xFF, 0xFF, 0xFF}, FailWritePage: -1, CorruptWritePage: -1}
	tag.pages = make([][4]byte, geometry.TotalPages)
	bcc0 := byte(0x88) ^ uid[0] ^ uid[1] ^ uid[2]
	copy(tag.pages[0][:], []byte{uid[0], uid[1], uid[2], bcc0})
	copy(tag.pages[1][:], uid[3:7])
	tag.pages[2][0] = uid[3] ^ uid[4] ^ uid[5] ^ uid[6]
	if geometry.Product != type2.ProductUltralight {
		cc := byte(0x12)
		switch geometry.Product {
		case type2.ProductNTAG215:
			cc = 0x3E
		case type2.ProductNTAG216:
			cc = 0x6D
		}
		copy(tag.pages[3][:], []byte{0xE1, 0x10, cc, 0x00})
		if geometry.ConfigStart >= 0 {
			copy(tag.pages[geometry.ConfigStart][:], []byte{0x04, 0x00, 0x00, 0xFF})
		}
		copy(tag.pages[geometry.PasswordPage][:], tag.password[:])
	}
	return tag
}

// Format writes a valid capability container for an original Ultralight tag.
func (t *Type2Tag) Format() {
	t.mu.Lock()
	defer t.mu.Unlock()
	copy(t.pages[3][:], []byte{0xE1, 0x10, 0x06, 0x00})
}

// CardInfo returns the selection data for the tag.
func (t *Type2Tag) CardInfo() nfc.CardInfo {
	t.mu.Lock()
	defer t.mu.Unlock()
	return nfc.CardInfo{UID: append([]byte(nil), t.uid...), ATQA: [2]byte{0x00, 0x44}, SAK: 0x00}
}

// Page returns a copy of one page.
func (t *Type2Tag) Page(page int) ([4]byte, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if page < 0 || page >= len(t.pages) {
		return [4]byte{}, fmt.Errorf("mock page %d is outside the tag", page)
	}
	return t.pages[page], nil
}

// SetPage updates one page without exercising the command path.
func (t *Type2Tag) SetPage(page int, data [4]byte) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if page >= 0 && page < len(t.pages) {
		t.pages[page] = data
	}
}

// SetPassword changes the password used by PWD_AUTH.
func (t *Type2Tag) SetPassword(password [4]byte, pack [2]byte) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.password = password
	t.pack = pack
	if t.geometry.PasswordPage >= 0 {
		t.pages[t.geometry.PasswordPage] = password
		t.pages[t.geometry.PackPage][0] = pack[0]
		t.pages[t.geometry.PackPage][1] = pack[1]
	}
}

// SetRawMode enters or leaves the raw-frame transport. The mock tag always
// accepts the transition.
func (t *Type2Tag) SetRawMode(context.Context, bool) error { return nil }

// TransceiveBytes validates the CRC-A of a raw command, executes it, and
// frames data responses with CRC-A. ACK/NAK frames stay one byte.
func (t *Type2Tag) TransceiveBytes(ctx context.Context, tx []byte) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if len(tx) < 3 {
		return nil, nfc.NewError("mock Type 2 command", nfc.CodeInvalidArgument, "frame is too short", nil)
	}
	body := tx[:len(tx)-2]
	crc := iso14443a.CRCA(body)
	if tx[len(tx)-2] != crc[0] || tx[len(tx)-1] != crc[1] {
		return nil, nfc.NewError("mock Type 2 command", nfc.CodeIO, "command CRC-A is invalid", type2.ErrNAK)
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	response := t.handle(body)
	if len(response) == 1 {
		return response, nil
	}
	return iso14443a.WithCRCA(response), nil
}

func (t *Type2Tag) handle(tx []byte) []byte {
	if len(tx) == 0 {
		return t.nak()
	}
	switch tx[0] {
	case 0x60:
		version := type2.VersionForProduct(t.product)
		if version == nil {
			return t.nak()
		}
		return append([]byte(nil), version...)
	case 0x30:
		if len(tx) != 2 {
			return t.nak()
		}
		start := int(tx[1])
		if start+3 >= len(t.pages) {
			return t.nak()
		}
		response := make([]byte, 0, 16)
		for page := start; page < start+4; page++ {
			response = append(response, t.pages[page][:]...)
		}
		return response
	case 0x3A:
		if len(tx) != 3 {
			return t.nak()
		}
		start, end := int(tx[1]), int(tx[2])
		if start > end || end >= len(t.pages) {
			return t.nak()
		}
		response := make([]byte, 0, (end-start+1)*4)
		for page := start; page <= end; page++ {
			response = append(response, t.pages[page][:]...)
		}
		return response
	case 0xA2:
		if len(tx) != 6 {
			return t.nak()
		}
		page := int(tx[1])
		if page < 2 || page >= len(t.pages) {
			return t.nak()
		}
		if page == t.FailWritePage {
			return t.nak()
		}
		t.writeCalls = append(t.writeCalls, page)
		var data [4]byte
		copy(data[:], tx[2:6])
		if page == t.CorruptWritePage {
			data[0] ^= 0xFF
		}
		t.pages[page] = data
		return []byte{0x0A}
	case 0x1B:
		if len(tx) != 5 {
			return t.nak()
		}
		var password [4]byte
		copy(password[:], tx[1:5])
		if password != t.password {
			return t.nak()
		}
		return []byte{t.pack[0], t.pack[1]}
	default:
		return t.nak()
	}
}

// WriteCalls returns the pages accepted by WRITE in order.
func (t *Type2Tag) WriteCalls() []int {
	t.mu.Lock()
	defer t.mu.Unlock()
	return append([]int(nil), t.writeCalls...)
}

func (t *Type2Tag) nak() []byte { return []byte{0x00} }

// NewType2Reader returns a closed mock Reader wired to the tag. Callers open it
// through the device manager or directly.
func NewType2Reader(tag *Type2Tag) *Reader {
	return NewReaderWithFuncs(ReaderFuncs{
		CardInfo:        func(context.Context) (nfc.CardInfo, error) { return tag.CardInfo(), nil },
		SetRawMode:      tag.SetRawMode,
		TransceiveBytes: tag.TransceiveBytes,
	})
}
