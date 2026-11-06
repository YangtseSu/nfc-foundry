package mifare

import (
	"bytes"
	"context"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

type gen1ATransport struct {
	rawModeCalls    []bool
	bitsCalls       []gen1ABitsCall
	bytesCalls      [][]byte
	transceiveBits  func([]byte, int) ([]byte, int, error)
	transceiveBytes func([]byte) ([]byte, error)
}

type gen1ABitsCall struct {
	data []byte
	bits int
}

func (t *gen1ATransport) SetRawMode(_ context.Context, enabled bool) error {
	t.rawModeCalls = append(t.rawModeCalls, enabled)
	return nil
}

func (t *gen1ATransport) TransceiveBytes(_ context.Context, tx []byte) ([]byte, error) {
	t.bytesCalls = append(t.bytesCalls, append([]byte(nil), tx...))
	if t.transceiveBytes != nil {
		return t.transceiveBytes(tx)
	}
	return []byte{mifareACK}, nil
}

func (t *gen1ATransport) TransceiveBits(_ context.Context, tx []byte, txBits int) ([]byte, int, error) {
	t.bitsCalls = append(t.bitsCalls, gen1ABitsCall{data: append([]byte(nil), tx...), bits: txBits})
	if t.transceiveBits != nil {
		return t.transceiveBits(tx, txBits)
	}
	return []byte{mifareACK}, 4, nil
}

func gen1ATestCard() nfc.CardInfo {
	return nfc.CardInfo{UID: []byte{1, 2, 3, 4}, ATQA: [2]byte{0, 4}, SAK: 0x08}
}

func gen1ATestBlock0() [nfc.BlockSize]byte {
	return [nfc.BlockSize]byte{5, 6, 7, 8, 0x0c, 0x08, 0x04, 0x00, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69}
}

func TestCRCAMatchesUpstreamFrames(t *testing.T) {
	halt := iso14443a.CRCA([]byte{mifareHalt, 0x00})
	write := iso14443a.CRCA([]byte{mifareWrite, 0x00})
	if halt != [2]byte{0x57, 0xcd} {
		t.Fatalf("HALT CRC = %x; want 57cd", halt)
	}
	if write != [2]byte{0x5f, 0xb1} {
		t.Fatalf("write CRC = %x; want 5fb1", write)
	}
	if frame := iso14443a.WithCRCA([]byte{mifareHalt, 0x00}); !bytes.Equal(frame, []byte{0x50, 0x00, 0x57, 0xcd}) {
		t.Fatalf("HALT frame = %x", frame)
	}
}

func TestGen1AACKAcceptsBothNibbleAlignments(t *testing.T) {
	if !isGen1AACK([]byte{0x0a}) || !isGen1AACK([]byte{0xa0}) {
		t.Fatal("valid ACK alignment was rejected")
	}
	for _, response := range [][]byte{nil, {}, {0x00}, {0x0a, 0x00}, {0x0b}} {
		if isGen1AACK(response) {
			t.Fatalf("unexpected ACK accepted: %x", response)
		}
	}
}

func TestWriteGen1ABlock0ReplaysUpstreamSequence(t *testing.T) {
	transport := &gen1ATransport{}
	if err := WriteGen1ABlock0(context.Background(), transport, gen1ATestCard(), gen1ATestBlock0()); err != nil {
		t.Fatal(err)
	}
	if len(transport.rawModeCalls) != 2 || !transport.rawModeCalls[0] || transport.rawModeCalls[1] {
		t.Fatalf("raw mode calls = %v; want [true false]", transport.rawModeCalls)
	}
	if len(transport.bitsCalls) != 1 || transport.bitsCalls[0].bits != 7 || !bytes.Equal(transport.bitsCalls[0].data, []byte{0x40}) {
		t.Fatalf("bit calls = %+v", transport.bitsCalls)
	}
	block0 := gen1ATestBlock0()
	wantBytes := [][]byte{
		{0x50, 0x00, 0x57, 0xcd},
		{0x43},
		{0xa0, 0x00, 0x5f, 0xb1},
		iso14443a.WithCRCA(block0[:]),
	}
	if len(transport.bytesCalls) != len(wantBytes) {
		t.Fatalf("byte calls = %x", transport.bytesCalls)
	}
	for index := range wantBytes {
		if !bytes.Equal(transport.bytesCalls[index], wantBytes[index]) {
			t.Fatalf("byte frame %d = %x; want %x", index, transport.bytesCalls[index], wantBytes[index])
		}
	}
}

func TestWriteGen1ABlock0NormalCardFailsAtFirstUnlock(t *testing.T) {
	transport := &gen1ATransport{
		transceiveBits: func([]byte, int) ([]byte, int, error) {
			return nil, 0, nfc.NewError("transceive bits", nfc.CodeTimeout, "no response", nil)
		},
	}
	err := WriteGen1ABlock0(context.Background(), transport, gen1ATestCard(), gen1ATestBlock0())
	if !errors.Is(err, ErrGen1ANotSupported) || !errors.Is(err, nfc.ErrUnsupported) {
		t.Fatalf("error = %v; want ErrGen1ANotSupported with CodeUnsupported", err)
	}
	if len(transport.rawModeCalls) != 2 || transport.rawModeCalls[1] {
		t.Fatalf("raw mode was not restored: %v", transport.rawModeCalls)
	}
	if len(transport.bytesCalls) != 1 {
		t.Fatalf("wrote after a failed unlock: %x", transport.bytesCalls)
	}
}

func TestWriteGen1ABlock0SecondUnlockStepRejected(t *testing.T) {
	transport := &gen1ATransport{
		transceiveBytes: func(frame []byte) ([]byte, error) {
			if bytes.Equal(frame, []byte{0x43}) {
				return nil, nfc.NewError("transceive bytes", nfc.CodeTimeout, "no response", nil)
			}
			return []byte{mifareACK}, nil
		},
	}
	err := WriteGen1ABlock0(context.Background(), transport, gen1ATestCard(), gen1ATestBlock0())
	if !errors.Is(err, ErrGen1ANotSupported) {
		t.Fatalf("error = %v; want ErrGen1ANotSupported", err)
	}
	if len(transport.rawModeCalls) != 2 {
		t.Fatalf("raw mode calls = %v", transport.rawModeCalls)
	}
}

func TestWriteGen1ABlock0UncertainDataFrameCanBeVerifiedByCaller(t *testing.T) {
	dataCalls := 0
	transport := &gen1ATransport{
		transceiveBytes: func(frame []byte) ([]byte, error) {
			dataCalls++
			if dataCalls == 4 {
				return nil, nfc.NewError("transceive bytes", nfc.CodeTimeout, "no response", nil)
			}
			return []byte{mifareACK}, nil
		},
	}
	err := WriteGen1ABlock0(context.Background(), transport, gen1ATestCard(), gen1ATestBlock0())
	if !errors.Is(err, ErrGen1AWriteUncertain) {
		t.Fatalf("error = %v; want ErrGen1AWriteUncertain", err)
	}
	if len(transport.rawModeCalls) != 2 {
		t.Fatalf("raw mode calls = %v", transport.rawModeCalls)
	}
}

func TestWriteGen1ABlock0HaltTimeoutIsExpected(t *testing.T) {
	transport := &gen1ATransport{
		transceiveBytes: func(frame []byte) ([]byte, error) {
			if bytes.Equal(frame, []byte{0x50, 0x00, 0x57, 0xcd}) {
				return nil, nfc.NewError("transceive bytes", nfc.CodeTimeout, "no response", nil)
			}
			return []byte{mifareACK}, nil
		},
	}
	if err := WriteGen1ABlock0(context.Background(), transport, gen1ATestCard(), gen1ATestBlock0()); err != nil {
		t.Fatal(err)
	}
}

func TestWriteGen1ABlock0RejectsInvalidInputBeforeRawMode(t *testing.T) {
	transport := &gen1ATransport{}
	badBCC := gen1ATestBlock0()
	badBCC[4] = 0xff
	if err := WriteGen1ABlock0(context.Background(), transport, gen1ATestCard(), badBCC); !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("bad BCC error = %v", err)
	}
	if err := WriteGen1ABlock0(context.Background(), transport, nfc.CardInfo{UID: []byte{1, 2, 3, 4}, SAK: 0x18}, gen1ATestBlock0()); !errors.Is(err, nfc.ErrUnsupported) {
		t.Fatalf("Classic 4K error = %v", err)
	}
	if err := WriteGen1ABlock0(context.Background(), transport, nfc.CardInfo{UID: []byte{1, 2, 3, 4, 5, 6, 7}, SAK: 0x08}, gen1ATestBlock0()); !errors.Is(err, nfc.ErrUnsupported) {
		t.Fatalf("7-byte UID error = %v", err)
	}
	if len(transport.rawModeCalls) != 0 {
		t.Fatalf("raw mode entered for invalid input: %v", transport.rawModeCalls)
	}
}

func TestWriteGen1ABlock0RestoresRawModeAfterCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	transport := &gen1ATransport{
		transceiveBits: func([]byte, int) ([]byte, int, error) {
			cancel()
			return nil, 0, nfc.NewError("transceive bits", nfc.CodeCanceled, "canceled", context.Canceled)
		},
	}
	err := WriteGen1ABlock0(ctx, transport, gen1ATestCard(), gen1ATestBlock0())
	if err == nil || !errors.Is(err, nfc.ErrCanceled) {
		t.Fatalf("error = %v; want cancellation", err)
	}
	if len(transport.rawModeCalls) != 2 || transport.rawModeCalls[1] {
		t.Fatalf("raw mode was not restored after cancellation: %v", transport.rawModeCalls)
	}
}
