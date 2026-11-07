package desfire_test

import (
	"bytes"
	"context"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/desfire"
	"github.com/YangtseSu/nfc-foundry/internal/isodep"
)

type fakeExchanger struct {
	t       *testing.T
	handler func([]byte) isodep.Response
}

func (f *fakeExchanger) Transmit(_ context.Context, apdu []byte) (isodep.Response, error) {
	return f.handler(apdu), nil
}

func response(data []byte, sw1, sw2 byte) isodep.Response {
	return isodep.Response{Data: data, SW1: sw1, SW2: sw2}
}

func TestIdentifyAndApplications(t *testing.T) {
	card := desfire.New(&fakeExchanger{t: t, handler: func(apdu []byte) isodep.Response {
		if apdu[1] == 0x60 {
			return response([]byte{0x04, 0x01, 0x01, 0x12, 0x34, 0x18, 0x05}, 0x91, 0x00)
		}
		if apdu[1] == 0x6A {
			return response([]byte{0x01, 0x00, 0x00, 0x02, 0x00, 0x00}, 0x91, 0x00)
		}
		t.Fatalf("unexpected APDU % X", apdu)
		return response(nil, 0x91, 0x1C)
	}})
	version, err := card.Identify(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if version.Vendor != 0x04 || version.Type != 0x01 || version.Storage != 0x18 || version.Protocol != 0x05 {
		t.Fatalf("version = %+v", version)
	}
	apps, err := card.Applications(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(apps) != 2 || apps[0] != 1 || apps[1] != 2 {
		t.Fatalf("apps = %v", apps)
	}
}

func TestFilesAndReadStandardFile(t *testing.T) {
	selectData := []byte{0x01, 0x00, 0x00}
	readData := []byte{0x01, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00}
	card := desfire.New(&fakeExchanger{t: t, handler: func(apdu []byte) isodep.Response {
		switch apdu[1] {
		case 0x5A:
			if !bytes.Equal(apdu[5:8], selectData) {
				t.Fatalf("select data = % X", apdu[5:8])
			}
			return response(nil, 0x91, 0x00)
		case 0x6F:
			return response([]byte{0x01, 0x02}, 0x91, 0x00)
		case 0xF5:
			switch apdu[5] {
			case 0x01:
				// standard, plain, access 0x0000, size 4 (LSB first)
				return response([]byte{0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00}, 0x91, 0x00)
			case 0x02:
				// value file, plain, access 0x0000, lower 0, upper 100, limited 10, flag 1
				return response([]byte{0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x01}, 0x91, 0x00)
			}
		case 0xBD:
			if !bytes.Equal(apdu[5:12], readData) {
				t.Fatalf("read data = % X", apdu[5:12])
			}
			return response([]byte{0xDE, 0xAD, 0xBE, 0xEF}, 0x91, 0x00)
		}
		t.Fatalf("unexpected APDU % X", apdu)
		return response(nil, 0x91, 0x1C)
	}})
	files, err := card.Files(context.Background(), 1)
	if err != nil {
		t.Fatal(err)
	}
	if len(files) != 2 || files[0].Type != desfire.FileStandard || files[0].Size != 4 {
		t.Fatalf("files = %+v", files)
	}
	if files[1].Type != desfire.FileValue || files[1].ValueUpper != 100 || !files[1].LimitedCreditOn {
		t.Fatalf("value file = %+v", files[1])
	}
	data, err := card.ReadFile(context.Background(), 1, 1)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(data, []byte{0xDE, 0xAD, 0xBE, 0xEF}) {
		t.Fatalf("data = % X", data)
	}
}

func TestReadFileCollectsAdditionalFrames(t *testing.T) {
	payload := bytes.Repeat([]byte{0x5A}, 4)
	step := 0
	card := desfire.New(&fakeExchanger{t: t, handler: func(apdu []byte) isodep.Response {
		switch apdu[1] {
		case 0x5A:
			return response(nil, 0x91, 0x00)
		case 0xF5:
			return response([]byte{0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00}, 0x91, 0x00)
		case 0xBD:
			if step == 0 {
				step++
				return response(payload[:2], 0x91, 0xAF)
			}
			step++
			return response(payload[2:], 0x91, 0x00)
		case 0xAF:
			return response(payload[2:], 0x91, 0x00)
		}
		t.Fatalf("unexpected APDU % X", apdu)
		return response(nil, 0x91, 0x1C)
	}})
	data, err := card.ReadFile(context.Background(), 1, 1)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(data, payload) {
		t.Fatalf("data = % X", data)
	}
}

func TestStatusErrorIsReported(t *testing.T) {
	card := desfire.New(&fakeExchanger{t: t, handler: func(apdu []byte) isodep.Response {
		return response(nil, 0x91, 0xA0)
	}})
	_, err := card.Identify(context.Background())
	var status *desfire.StatusError
	if !errors.As(err, &status) || status.SW2 != 0xA0 {
		t.Fatalf("error = %v", err)
	}
}
