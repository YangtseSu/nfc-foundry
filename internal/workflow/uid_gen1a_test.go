package workflow

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
)

type uidTestExecutor struct{ reader nfc.Reader }

func (e uidTestExecutor) WithReader(ctx context.Context, operation func(context.Context, nfc.Reader) error) error {
	return operation(ctx, e.reader)
}

type uidTestBackups struct{}

func (uidTestBackups) Save(context.Context, UIDBackup) (string, error) {
	return "/backup.json", nil
}

func openUIDMockReader(t *testing.T, funcs mock.ReaderFuncs) *mock.Reader {
	t.Helper()
	reader := mock.NewReaderWithFuncs(funcs)
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reader.Close() })
	return reader
}

func gen1AWorkflowFixture(t *testing.T, dataFrameTimeout, normalCard bool) (*mock.Reader, nfc.CardInfo, *[nfc.BlockSize]byte, *[]bool) {
	t.Helper()
	card := nfc.CardInfo{UID: []byte{1, 2, 3, 4}, ATQA: [2]byte{4, 0}, SAK: 0x08}
	block0 := &[nfc.BlockSize]byte{1, 2, 3, 4, 4, 0x08, 0x04, 0x00, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69}
	trailer := [nfc.BlockSize]byte{6: 0xff, 7: 0x07, 8: 0x80, 9: 0x69}
	rawModeCalls := &[]bool{}
	reader := openUIDMockReader(t, mock.ReaderFuncs{
		CardInfo:     func(context.Context) (nfc.CardInfo, error) { return card.Clone(), nil },
		Authenticate: func(context.Context, byte, nfc.KeyType, nfc.Key) error { return nil },
		ReadBlock: func(_ context.Context, block byte) ([nfc.BlockSize]byte, error) {
			if block == 3 {
				return trailer, nil
			}
			return *block0, nil
		},
		WriteManufacturer: func(context.Context, [nfc.BlockSize]byte) error {
			return nfc.NewError("write manufacturer block", nfc.CodeTimeout, "RF Transmission Error", nil)
		},
		SetRawMode: func(_ context.Context, enabled bool) error {
			*rawModeCalls = append(*rawModeCalls, enabled)
			return nil
		},
		TransceiveBits: func(_ context.Context, tx []byte, bits int) ([]byte, int, error) {
			if normalCard {
				return nil, 0, nfc.NewError("transceive bits", nfc.CodeTimeout, "no response", nil)
			}
			if !bytes.Equal(tx, []byte{0x40}) || bits != 7 {
				t.Fatalf("unexpected unlock frame: %x/%d", tx, bits)
			}
			return []byte{0x0a}, 4, nil
		},
		TransceiveBytes: func(_ context.Context, tx []byte) ([]byte, error) {
			switch {
			case bytes.Equal(tx, []byte{0x50, 0x00, 0x57, 0xcd}):
				return nil, nfc.NewError("transceive bytes", nfc.CodeTimeout, "no response", nil)
			case bytes.Equal(tx, []byte{0x43}), bytes.Equal(tx, []byte{0xa0, 0x00, 0x5f, 0xb1}):
				return []byte{0x0a}, nil
			case len(tx) == nfc.BlockSize+2:
				copy(block0[:], tx[:nfc.BlockSize])
				card.UID = append([]byte(nil), tx[0:4]...)
				if dataFrameTimeout {
					return nil, nfc.NewError("transceive bytes", nfc.CodeTimeout, "no response", nil)
				}
				return []byte{0x0a}, nil
			default:
				t.Fatalf("unexpected Gen1A frame: %x", tx)
				return nil, nil
			}
		},
	})
	return reader, card, block0, rawModeCalls
}

func TestUIDWriteUsesInProcessGen1AAndVerifiesFullBlock(t *testing.T) {
	reader, card, block0, rawModeCalls := gen1AWorkflowFixture(t, false, false)
	service := NewUIDService(uidTestExecutor{reader: reader}, uidTestBackups{})
	service.reselectLimit = 100 * time.Millisecond
	service.retryDelay = time.Millisecond
	key := nfc.Key{0xff, 0xff, 0xff, 0xff, 0xff, 0xff}
	result, err := service.Write(context.Background(), UIDWriteRequest{
		TaskID: "gen1a", Card: card.Clone(), Device: nfc.DeviceInfo{ConnString: "mock:gen1a"},
		NewUID: []byte{5, 6, 7, 8}, Sector0Keys: SectorKeys{Sector: 0, KeyA: &key},
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !result.Verified || result.NewBlock0 != *block0 || result.ActualBlock0 != *block0 {
		t.Fatalf("result = %+v; block0 = %x", result, *block0)
	}
	open, _, openCount, closeCount := reader.State()
	if !open || openCount != 1 || closeCount != 0 {
		t.Fatalf("reader was reopened: open=%v opens=%d closes=%d", open, openCount, closeCount)
	}
	if got := fmt.Sprint(*rawModeCalls); got != "[true false]" {
		t.Fatalf("raw mode calls = %s", got)
	}
}

func TestUIDWriteGen1AUncertainDataFrameStillUsesReadback(t *testing.T) {
	reader, card, block0, rawModeCalls := gen1AWorkflowFixture(t, true, false)
	service := NewUIDService(uidTestExecutor{reader: reader}, uidTestBackups{})
	service.reselectLimit = 100 * time.Millisecond
	service.retryDelay = time.Millisecond
	key := nfc.Key{}
	result, err := service.Write(context.Background(), UIDWriteRequest{
		Card: card.Clone(), NewUID: []byte{5, 6, 7, 8}, Sector0Keys: SectorKeys{Sector: 0, KeyA: &key},
	}, nil)
	if err != nil || !result.Verified || result.NewBlock0 != *block0 {
		t.Fatalf("result=%+v error=%v block0=%x", result, err, *block0)
	}
	if got := fmt.Sprint(*rawModeCalls); got != "[true false]" {
		t.Fatalf("raw mode calls = %s", got)
	}
}

func TestUIDWriteNormalCardReportsGen1ANotSupported(t *testing.T) {
	reader, card, _, rawModeCalls := gen1AWorkflowFixture(t, false, true)
	service := NewUIDService(uidTestExecutor{reader: reader}, uidTestBackups{})
	service.reselectLimit = 100 * time.Millisecond
	service.retryDelay = time.Millisecond
	key := nfc.Key{}
	result, err := service.Write(context.Background(), UIDWriteRequest{
		Card: card.Clone(), NewUID: []byte{5, 6, 7, 8}, Sector0Keys: SectorKeys{Sector: 0, KeyA: &key},
	}, nil)
	if !errors.Is(err, mifare.ErrGen1ANotSupported) || !errors.Is(err, nfc.ErrUnsupported) {
		t.Fatalf("error = %v; want ErrGen1ANotSupported and ErrUnsupported", err)
	}
	if result.Verified || result.BackupPath == "" {
		t.Fatalf("result = %+v", result)
	}
	if got := fmt.Sprint(*rawModeCalls); got != "[true false]" {
		t.Fatalf("raw mode calls = %s", got)
	}
}

func TestUIDWriteDoesNotUseGen1AWhenCardDisappears(t *testing.T) {
	card := nfc.CardInfo{UID: []byte{1, 2, 3, 4}, ATQA: [2]byte{4, 0}, SAK: 0x08}
	block0 := [nfc.BlockSize]byte{1, 2, 3, 4, 4}
	trailer := [nfc.BlockSize]byte{6: 0xff, 7: 0x07, 8: 0x80, 9: 0x69}
	written := false
	rawEntered := false
	reader := openUIDMockReader(t, mock.ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) {
			if written {
				return nfc.CardInfo{}, nfc.NewError("card info", nfc.CodeNoCard, "no card", nil)
			}
			return card.Clone(), nil
		},
		Authenticate: func(context.Context, byte, nfc.KeyType, nfc.Key) error { return nil },
		ReadBlock: func(_ context.Context, block byte) ([nfc.BlockSize]byte, error) {
			if block == 3 {
				return trailer, nil
			}
			return block0, nil
		},
		WriteManufacturer: func(context.Context, [nfc.BlockSize]byte) error {
			written = true
			return nil
		},
		SetRawMode: func(context.Context, bool) error {
			rawEntered = true
			return nil
		},
	})
	service := NewUIDService(uidTestExecutor{reader: reader}, uidTestBackups{})
	service.reselectLimit = 10 * time.Millisecond
	service.retryDelay = time.Millisecond
	key := nfc.Key{}
	_, err := service.Write(context.Background(), UIDWriteRequest{
		Card: card, NewUID: []byte{5, 6, 7, 8}, Sector0Keys: SectorKeys{Sector: 0, KeyA: &key},
	}, nil)
	if !errors.Is(err, nfc.ErrVerificationFailed) || rawEntered {
		t.Fatalf("error=%v rawEntered=%v", err, rawEntered)
	}
}
