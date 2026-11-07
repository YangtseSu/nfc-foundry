package mock

import (
	"context"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// ISODEPHandler handles one command APDU and returns response data followed by
// the two status bytes.
type ISODEPHandler func(apdu []byte) []byte

// NewISODEPReader creates a mock ISO/IEC 14443-4 Type A card. atsBody is the
// ATS without CRC. The handler must be deterministic and must not retain the
// APDU slice.
func NewISODEPReader(card nfc.CardInfo, atsBody []byte, handler ISODEPHandler) *Reader {
	rawMode := false
	pni := byte(0)
	ats := iso14443a.WithCRCA(atsBody)
	return NewReaderWithFuncs(ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) { return card.Clone(), nil },
		SetRawMode: func(_ context.Context, enabled bool) error {
			rawMode = enabled
			if !enabled {
				pni = 0
			}
			return nil
		},
		TransceiveBytes: func(_ context.Context, tx []byte) ([]byte, error) {
			if !rawMode || len(tx) < 3 {
				return nil, nfc.ErrInvalidArgument
			}
			body := tx[:len(tx)-2]
			crc := iso14443a.CRCA(body)
			if tx[len(tx)-2] != crc[0] || tx[len(tx)-1] != crc[1] {
				return nil, nfc.ErrInvalidArgument
			}
			if len(body) == 2 && body[0] == 0xE0 {
				return append([]byte(nil), ats...), nil
			}
			if body[0]&0xC0 != 0 || body[0]&0x10 != 0 {
				return nil, nfc.ErrUnsupported
			}
			if body[0]&0x01 != pni {
				return nil, nfc.ErrInvalidArgument
			}
			response := handler(append([]byte(nil), body[1:]...))
			frame := append([]byte{0x02 | pni}, response...)
			pni ^= 1
			return iso14443a.WithCRCA(frame), nil
		},
		TransceiveBits: func(context.Context, []byte, int) ([]byte, int, error) {
			return nil, 0, nfc.ErrUnsupported
		},
	})
}
