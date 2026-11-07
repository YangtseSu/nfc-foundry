package type4_test

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/isodep"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/type4"
)

type fakeTag struct {
	t        *testing.T
	cc       []byte
	file     []byte
	selected bool
	fileID   uint16
}

func newFakeTag(t *testing.T, message []byte, writeAccess byte) *fakeTag {
	t.Helper()
	cc := make([]byte, 15)
	binary.BigEndian.PutUint16(cc[0:2], 15)
	cc[2] = 0x20
	binary.BigEndian.PutUint16(cc[3:5], 0x3B)
	binary.BigEndian.PutUint16(cc[5:7], 0x3B)
	cc[7], cc[8] = 0x04, 0x06
	binary.BigEndian.PutUint16(cc[9:11], 0xE104)
	binary.BigEndian.PutUint16(cc[11:13], 100)
	cc[13], cc[14] = 0x00, writeAccess
	file := make([]byte, 2+len(message))
	binary.BigEndian.PutUint16(file[0:2], uint16(len(message)))
	copy(file[2:], message)
	return &fakeTag{t: t, cc: cc, file: file}
}

func (f *fakeTag) Transmit(_ context.Context, apdu []byte) (isodep.Response, error) {
	if len(apdu) < 4 {
		f.t.Fatalf("short APDU % X", apdu)
	}
	cla, ins, p1, p2 := apdu[0], apdu[1], apdu[2], apdu[3]
	switch {
	case cla == 0x00 && ins == 0xA4 && p1 == 0x04:
		if !bytes.HasPrefix(apdu[5:], []byte{0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01}) {
			return isodep.Response{SW1: 0x6A, SW2: 0x82}, nil
		}
		f.selected = true
		return isodep.Response{SW1: 0x90, SW2: 0x00}, nil
	case cla == 0x00 && ins == 0xA4 && p1 == 0x00:
		if len(apdu) < 7 {
			return isodep.Response{SW1: 0x67, SW2: 0x00}, nil
		}
		f.fileID = uint16(apdu[5])<<8 | uint16(apdu[6])
		switch f.fileID {
		case 0xE103:
			return isodep.Response{SW1: 0x90, SW2: 0x00}, nil
		default:
			return isodep.Response{SW1: 0x90, SW2: 0x00}, nil
		}
	case cla == 0x00 && ins == 0xB0:
		offset := int(p1)<<8 | int(p2)
		le := int(apdu[4])
		if le == 0 {
			le = 256
		}
		source := f.file
		if f.fileID == 0xE103 {
			source = f.cc
		}
		if offset >= len(source) {
			return isodep.Response{SW1: 0x62, SW2: 0x82}, nil
		}
		end := min(offset+le, len(source))
		return isodep.Response{Data: append([]byte(nil), source[offset:end]...), SW1: 0x90, SW2: 0x00}, nil
	case cla == 0x00 && ins == 0xD6:
		if f.fileID == 0xE103 {
			return isodep.Response{SW1: 0x69, SW2: 0x82}, nil
		}
		offset := int(p1)<<8 | int(p2)
		lc := int(apdu[4])
		data := apdu[5 : 5+lc]
		if offset+len(data) > len(f.file) {
			f.file = append(f.file, make([]byte, offset+len(data)-len(f.file))...)
		}
		copy(f.file[offset:], data)
		return isodep.Response{SW1: 0x90, SW2: 0x00}, nil
	default:
		f.t.Fatalf("unexpected APDU % X", apdu)
		return isodep.Response{SW1: 0x6D, SW2: 0x00}, nil
	}
}

func TestReadNDEF(t *testing.T) {
	message, _ := type2.EncodeTextMessage("hello", "en")
	tag := newFakeTag(t, message, 0x00)
	document, err := type4.New(tag).Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if document.NDEF == nil || !document.NDEF.Present || document.NDEF.Text != "hello" {
		t.Fatalf("NDEF = %+v", document.NDEF)
	}
	if document.CC.FileID != 0xE104 || document.CC.MaxMessage != 100 {
		t.Fatalf("CC = %+v", document.CC)
	}
}

func TestWriteNDEFBacksUpPreviousAndVerifies(t *testing.T) {
	previous, _ := type2.EncodeTextMessage("hello", "en")
	next, _ := type2.EncodeTextMessage("updated", "en")
	tag := newFakeTag(t, previous, 0x00)
	result, err := type4.New(tag).Write(context.Background(), next)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(result.Previous, previous) {
		t.Fatalf("previous = % X", result.Previous)
	}
	if result.Final.NDEF.Text != "updated" {
		t.Fatalf("final = %+v", result.Final.NDEF)
	}
	if got := int(binary.BigEndian.Uint16(tag.file[0:2])); got != len(next) {
		t.Fatalf("NLEN = %d", got)
	}
}

func TestWriteRefusesReadOnlyCC(t *testing.T) {
	message, _ := type2.EncodeTextMessage("hello", "en")
	tag := newFakeTag(t, message, 0xFF)
	if _, err := type4.New(tag).Write(context.Background(), message); err == nil {
		t.Fatal("read-only NDEF file was written")
	}
}

func TestReadRejectsNLENBeyondCapacity(t *testing.T) {
	message, _ := type2.EncodeTextMessage("hello", "en")
	tag := newFakeTag(t, message, 0x00)
	binary.BigEndian.PutUint16(tag.file[0:2], 101)
	if _, err := type4.New(tag).Read(context.Background()); err == nil {
		t.Fatal("oversized NLEN accepted")
	}
}

func TestSelectErrorIsStatus(t *testing.T) {
	tag := &fakeTag{t: t}
	_, err := type4.New(tag).Read(context.Background())
	var status *type4.StatusError
	if !errors.As(err, &status) {
		t.Fatalf("error = %v", err)
	}
}

func TestMessageDecodeKeepsRecords(t *testing.T) {
	message, _ := hex.DecodeString("D101085402656E68656C6C6F")
	tag := newFakeTag(t, message, 0x00)
	document, err := type4.New(tag).Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(document.NDEF.Records) != 1 || document.NDEF.Records[0].Kind != "text" {
		t.Fatalf("records = %+v", document.NDEF.Records)
	}
}
