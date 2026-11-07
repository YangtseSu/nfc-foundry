package isodep_test

import (
	"bytes"
	"context"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/isodep"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func atsFrame(t *testing.T, body []byte) []byte {
	t.Helper()
	return iso14443a.WithCRCA(body)
}

func TestParseATSInterfaceBytes(t *testing.T) {
	// TL, T0 = TA+TB+FSCI 8, TA(1), TB(1) with FWI 4.
	frame := atsFrame(t, []byte{0x04, 0x38, 0x80, 0x40})
	ats, err := isodep.ParseATS(frame)
	if err != nil {
		t.Fatal(err)
	}
	if ats.FSCI != 8 || ats.FSC != 256 || ats.FWI != 4 || ats.TA == nil || ats.TB == nil || ats.TC != nil {
		t.Fatalf("ATS = %+v", ats)
	}
}

func TestParseATSRejectsBadCRCAndLength(t *testing.T) {
	frame := atsFrame(t, []byte{0x04, 0x38, 0x80, 0x40})
	frame[len(frame)-1] ^= 0xFF
	if _, err := isodep.ParseATS(frame); err == nil {
		t.Fatal("bad CRC accepted")
	}
	if _, err := isodep.ParseATS([]byte{0x05, 0x38, 0x80, 0x40, 0x00}); err == nil {
		t.Fatal("length mismatch accepted")
	}
}

func TestBuildShortAPDUs(t *testing.T) {
	for _, test := range []struct {
		name string
		data []byte
		le   int
		want []byte
	}{
		{"case1", nil, 0, []byte{0x00, 0xA4, 0x04, 0x00}},
		{"case2", nil, 15, []byte{0x00, 0xB0, 0x00, 0x00, 0x0F}},
		{"case2_256", nil, 256, []byte{0x00, 0xB0, 0x00, 0x00, 0x00}},
		{"case3", []byte{0x01, 0x02}, 0, []byte{0x00, 0xA4, 0x04, 0x00, 0x02, 0x01, 0x02}},
		{"case4", []byte{0x01}, 1, []byte{0x00, 0xA4, 0x04, 0x00, 0x01, 0x01, 0x01}},
	} {
		t.Run(test.name, func(t *testing.T) {
			got, err := isodep.BuildShort(0x00, 0xA4, 0x04, 0x00, test.data, test.le)
			if test.name == "case2" || test.name == "case2_256" {
				got, err = isodep.BuildShort(0x00, 0xB0, 0x00, 0x00, test.data, test.le)
			}
			if err != nil || !bytes.Equal(got, test.want) {
				t.Fatalf("APDU = % X, %v; want % X", got, err, test.want)
			}
		})
	}
}

type fakeTransport struct {
	ats     []byte
	handler func(body []byte) []byte
	rawMode bool
}

func (f *fakeTransport) SetRawMode(_ context.Context, enabled bool) error {
	f.rawMode = enabled
	return nil
}

func (f *fakeTransport) TransceiveBytes(_ context.Context, tx []byte) ([]byte, error) {
	if !f.rawMode || len(tx) < 3 {
		return nil, nfc.ErrInvalidArgument
	}
	body := tx[:len(tx)-2]
	crc := iso14443a.CRCA(body)
	if tx[len(tx)-2] != crc[0] || tx[len(tx)-1] != crc[1] {
		return nil, nfc.ErrInvalidArgument
	}
	if len(body) == 2 && body[0] == 0xE0 {
		return f.ats, nil
	}
	return iso14443a.WithCRCA(f.handler(body)), nil
}

func (f *fakeTransport) TransceiveBits(context.Context, []byte, int) ([]byte, int, error) {
	return nil, 0, nfc.ErrUnsupported
}

func openFake(t *testing.T, handler func([]byte) []byte) *isodep.Session {
	t.Helper()
	transport := &fakeTransport{ats: atsFrame(t, []byte{0x01}), handler: handler}
	card := nfc.CardInfo{UID: []byte{1, 2, 3, 4}, SAK: 0x20}
	session, err := isodep.OpenSession(context.Background(), transport, card)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = session.Close(context.Background()) })
	return session
}

func TestSessionTransmitAndResponseChaining(t *testing.T) {
	command := []byte{0x00, 0xA4, 0x04, 0x00, 0x07, 0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01, 0x00}
	step := 0
	session := openFake(t, func(body []byte) []byte {
		switch step {
		case 0:
			if body[0] != 0x02 || !bytes.Equal(body[1:], command) {
				t.Fatalf("first command block = % X", body)
			}
			step++
			return []byte{0x12, 0xAA, 0xBB}
		case 1:
			if body[0] != 0xA2 {
				t.Fatalf("expected ACK, got %02X", body[0])
			}
			step++
			return []byte{0x03, 0xCC, 0xDD, 0x90, 0x00}
		default:
			t.Fatalf("unexpected frame % X", body)
			return nil
		}
	})
	response, err := session.Transmit(context.Background(), command)
	if err != nil {
		t.Fatal(err)
	}
	if !response.OK() || !bytes.Equal(response.Data, []byte{0xAA, 0xBB, 0xCC, 0xDD}) {
		t.Fatalf("response = %+v", response)
	}
}

func TestSessionCommandChaining(t *testing.T) {
	command := make([]byte, 40)
	copy(command, []byte{0x00, 0xD6, 0x00, 0x00})
	// FSCI 2 => FSC 32 => 29-byte command chunks.
	step := 0
	transport := &fakeTransport{ats: atsFrame(t, []byte{0x02, 0x02}), handler: func(body []byte) []byte {
		switch step {
		case 0:
			if body[0] != 0x12 || len(body)-1 != 29 {
				t.Fatalf("first chained block = %02X len %d", body[0], len(body)-1)
			}
			step++
			return []byte{0xA2}
		case 1:
			if body[0] != 0x03 || len(body)-1 != 11 {
				t.Fatalf("last block = %02X len %d", body[0], len(body)-1)
			}
			step++
			return []byte{0x03, 0x90, 0x00}
		default:
			t.Fatalf("unexpected frame % X", body)
			return nil
		}
	}}
	card := nfc.CardInfo{UID: []byte{1, 2, 3, 4}, SAK: 0x20}
	session, err := isodep.OpenSession(context.Background(), transport, card)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close(context.Background())
	response, err := session.Transmit(context.Background(), command)
	if err != nil {
		t.Fatal(err)
	}
	if !response.OK() {
		t.Fatalf("response = %+v", response)
	}
}

func TestSessionHandlesWTX(t *testing.T) {
	step := 0
	transport := &fakeTransport{ats: atsFrame(t, []byte{0x01}), handler: func(body []byte) []byte {
		switch step {
		case 0:
			step++
			return []byte{0xF2, 0x01}
		case 1:
			if !bytes.Equal(body, []byte{0xF2, 0x01}) {
				t.Fatalf("WTX response = % X", body)
			}
			step++
			return []byte{0x02, 0x90, 0x00}
		default:
			t.Fatalf("unexpected frame % X", body)
			return nil
		}
	}}
	card := nfc.CardInfo{UID: []byte{1, 2, 3, 4}, SAK: 0x20}
	session, err := isodep.OpenSession(context.Background(), transport, card)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close(context.Background())
	response, err := session.Transmit(context.Background(), []byte{0x00, 0xB0, 0x00, 0x00})
	if err != nil {
		t.Fatal(err)
	}
	if !response.OK() {
		t.Fatalf("response = %+v", response)
	}
}

func TestSessionRejectsWrongBlockNumber(t *testing.T) {
	transport := &fakeTransport{ats: atsFrame(t, []byte{0x01}), handler: func(body []byte) []byte {
		return []byte{0x03, 0x90, 0x00}
	}}
	card := nfc.CardInfo{UID: []byte{1, 2, 3, 4}, SAK: 0x20}
	session, err := isodep.OpenSession(context.Background(), transport, card)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close(context.Background())
	if _, err := session.Transmit(context.Background(), []byte{0x00, 0xB0, 0x00, 0x00}); err == nil {
		t.Fatal("wrong block number accepted")
	}
}
