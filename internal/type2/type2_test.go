package type2

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

type scriptedTransport struct {
	calls     [][]byte
	responses [][]byte
	failures  []error
}

func (s *scriptedTransport) SetRawMode(context.Context, bool) error { return nil }

func (s *scriptedTransport) TransceiveBytes(_ context.Context, tx []byte) ([]byte, error) {
	s.calls = append(s.calls, append([]byte(nil), tx...))
	index := len(s.calls) - 1
	if index < len(s.failures) && s.failures[index] != nil {
		return nil, s.failures[index]
	}
	if index < len(s.responses) {
		return append([]byte(nil), s.responses[index]...), nil
	}
	return nil, errors.New("unexpected command")
}

func framed(data []byte) []byte {
	return iso14443a.WithCRCA(data)
}

func TestCommandFramesAndResponses(t *testing.T) {
	version := []byte{0x00, 0x04, 0x04, 0x02, 0x01, 0x00, 0x0F, 0x03}
	transport := &scriptedTransport{responses: [][]byte{framed(version), framed(make([]byte, 4)), {0x0A}, framed([]byte{0x12, 0x34})}}
	session, err := OpenSession(context.Background(), transport)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close(context.Background())
	if got, err := session.GetVersion(context.Background()); err != nil || !bytes.Equal(got, version) {
		t.Fatalf("GetVersion = % X, %v", got, err)
	}
	if got, err := session.FastRead(context.Background(), 4, 4); err != nil || len(got) != 4 {
		t.Fatalf("FastRead = % X, %v", got, err)
	}
	if err := session.WritePage(context.Background(), 7, [4]byte{1, 2, 3, 4}); err != nil {
		t.Fatal(err)
	}
	pack, err := session.PwdAuth(context.Background(), [4]byte{0xAA, 0xBB, 0xCC, 0xDD})
	if err != nil || pack != [2]byte{0x12, 0x34} {
		t.Fatalf("PwdAuth = % X, %v", pack, err)
	}
	want := [][]byte{
		framed([]byte{0x60}), framed([]byte{0x3A, 0x04, 0x04}),
		framed([]byte{0xA2, 0x07, 1, 2, 3, 4}), framed([]byte{0x1B, 0xAA, 0xBB, 0xCC, 0xDD}),
	}
	for index := range want {
		if !bytes.Equal(transport.calls[index], want[index]) {
			t.Fatalf("frame %d = % X; want % X", index, transport.calls[index], want[index])
		}
	}
}

func TestCommandRejectsNAKAndUnexpectedLengths(t *testing.T) {
	transport := &scriptedTransport{responses: [][]byte{{0x00}, {0x0A}, {0x01, 0x02, 0x03}}}
	session, err := OpenSession(context.Background(), transport)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close(context.Background())
	if _, err := session.GetVersion(context.Background()); !errors.Is(err, ErrNAK) {
		t.Fatalf("NAK error = %v", err)
	}
	if _, err := session.GetVersion(context.Background()); err == nil || !strings.Contains(err.Error(), "ACK") {
		t.Fatalf("ACK for data command error = %v", err)
	}
	if _, err := session.PwdAuth(context.Background(), [4]byte{}); err == nil || !strings.Contains(err.Error(), "CRC-A") {
		t.Fatalf("short PWD_AUTH response error = %v", err)
	}
}

func TestCommandRejectsBadCRC(t *testing.T) {
	response := framed(make([]byte, 4))
	response[0] ^= 0xFF
	transport := &scriptedTransport{responses: [][]byte{response}}
	session, err := OpenSession(context.Background(), transport)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close(context.Background())
	if _, err := session.FastRead(context.Background(), 0, 0); !errors.Is(err, nfc.ErrIO) {
		t.Fatalf("bad CRC error = %v", err)
	}
}

func TestParseCC(t *testing.T) {
	geometry, _ := GeometryFor(ProductNTAG213)
	for _, test := range []struct {
		page  [4]byte
		valid bool
		err   bool
	}{
		{[4]byte{0xE1, 0x10, 0x12, 0x00}, true, false},
		{[4]byte{0xE1, 0x10, 0x12, 0x0F}, true, false},
		{[4]byte{0x00, 0x00, 0x00, 0x00}, false, false},
		{[4]byte{0xE2, 0x10, 0x12, 0x00}, false, true},
		{[4]byte{0xE1, 0x20, 0x12, 0x00}, false, true},
		{[4]byte{0xE1, 0x10, 0x12, 0x01}, false, true},
		{[4]byte{0xE1, 0x10, 0xFF, 0x00}, false, true},
	} {
		cc, err := ParseCC(test.page, geometry)
		if (err != nil) != test.err {
			t.Fatalf("ParseCC(% X) error = %v; want error %v", test.page, err, test.err)
		}
		if cc.Valid != test.valid {
			t.Fatalf("ParseCC(% X) valid = %v; want %v", test.page, cc.Valid, test.valid)
		}
	}
}

func TestDetectProduct(t *testing.T) {
	for product, size := range map[Product]byte{ProductNTAG213: 0x0F, ProductNTAG215: 0x11, ProductNTAG216: 0x13} {
		version := []byte{0x00, 0x04, 0x04, 0x02, 0x01, 0x00, size, 0x03}
		got, ok := DetectProduct(version)
		if !ok || got != product {
			t.Fatalf("DetectProduct(% X) = %q, %v; want %q", version, got, ok, product)
		}
		if !bytes.Equal(VersionForProduct(product), version) {
			t.Fatalf("VersionForProduct(%q) = % X; want % X", product, VersionForProduct(product), version)
		}
	}
	if _, ok := DetectProduct([]byte{0, 4, 3, 1, 1, 0, 0x0B, 3}); ok {
		t.Fatal("unsupported Ultralight EV1 version was accepted")
	}
}

func TestValidateCardPages(t *testing.T) {
	uid := []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66}
	pages := make([][4]byte, 4)
	pages[0] = [4]byte{uid[0], uid[1], uid[2], 0x88 ^ uid[0] ^ uid[1] ^ uid[2]}
	copy(pages[1][:], uid[3:7])
	pages[2][0] = uid[3] ^ uid[4] ^ uid[5] ^ uid[6]
	card := nfc.CardInfo{UID: uid, ATQA: [2]byte{0x00, 0x44}, SAK: 0x00}
	if err := ValidateCardPages(card, pages); err != nil {
		t.Fatal(err)
	}
	pages[0][3] ^= 0xFF
	if err := ValidateCardPages(card, pages); !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("bad BCC error = %v", err)
	}
}

func textRecord(text string) []byte {
	payload := append([]byte{0x02, 'e', 'n'}, []byte(text)...)
	return append([]byte{0xD1, 0x01, byte(len(payload)), 'T'}, payload...)
}

func uriRecord(uri string) []byte {
	payload := append([]byte{0x04}, []byte(uri)...)
	return append([]byte{0xD1, 0x01, byte(len(payload)), 'U'}, payload...)
}

func TestParseMessageTextAndURI(t *testing.T) {
	records, err := ParseMessage(textRecord("hello"))
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 || records[0].Text != "hello" || records[0].TNF != 0x01 {
		t.Fatalf("text records = %+v", records)
	}
	records, err = ParseMessage(uriRecord("example.com"))
	if err != nil {
		t.Fatal(err)
	}
	if records[0].URI != "https://example.com" {
		t.Fatalf("URI = %q", records[0].URI)
	}
}

func TestParseMessageLongRecord(t *testing.T) {
	payload := append([]byte{0x00}, bytes.Repeat([]byte{'A'}, 300)...)
	message := []byte{0xC1, 0x01, 0x00, 0x00, 0x01, 0x2D, 'T'}
	message = append(message, payload...)
	records, err := ParseMessage(message)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 || len(records[0].Text) != 300 {
		t.Fatalf("long text length = %d", len(records[0].Text))
	}
}

func TestParseMessageRejectsMalformedLayouts(t *testing.T) {
	valid := textRecord("hello")
	for name, message := range map[string][]byte{
		"missing MB":        append([]byte{valid[0] &^ 0x80}, valid[1:]...),
		"missing ME":        append([]byte{valid[0] &^ 0x40}, valid[1:]...),
		"reserved TNF":      append([]byte{valid[0]&0xF8 | 0x07}, valid[1:]...),
		"trailing bytes":    append(append([]byte(nil), valid...), 0x00),
		"truncated type":    {0xD1, 0x10},
		"truncated payload": {0xD1, 0x01, 0x05, 'T'},
		"empty message":     {},
	} {
		if _, err := ParseMessage(message); err == nil {
			t.Fatalf("%s was accepted", name)
		}
	}
}

func TestParseNDEFTLVs(t *testing.T) {
	message := textRecord("hi")
	area := append([]byte{0x00, 0x03, byte(len(message))}, message...)
	area = append(area, 0xFE, 0x00, 0x00)
	ndef, err := ParseNDEF(area)
	if err != nil {
		t.Fatal(err)
	}
	if !ndef.Present || ndef.Text != "hi" {
		t.Fatalf("NDEF = %+v", ndef)
	}
	empty, err := ParseNDEF([]byte{0x00, 0x00, 0x00, 0x00})
	if err != nil || empty.Present {
		t.Fatalf("empty NDEF = %+v, %v", empty, err)
	}
	if _, err := ParseNDEF([]byte{0x03, 0x05, 0x00}); err == nil {
		t.Fatal("truncated NDEF TLV was accepted")
	}
}

func TestBuildNDEFArea(t *testing.T) {
	message := textRecord("hello")
	area, err := BuildNDEFArea(144, message)
	if err != nil {
		t.Fatal(err)
	}
	if area[0] != 0x03 || int(area[1]) != len(message) || area[2+len(message)] != 0xFE {
		t.Fatalf("area prefix = % X", area[:4+len(message)])
	}
	if len(area) != 144 || !bytes.Equal(area[2:2+len(message)], message) {
		t.Fatal("area does not contain the message at the expected offset")
	}
	if _, err := BuildNDEFArea(10, message); !errors.Is(err, ErrCapacity) {
		t.Fatalf("capacity error = %v", err)
	}
	if _, err := BuildNDEFArea(144, []byte{0xD1}); !errors.Is(err, ErrMalformedNDEF) {
		t.Fatalf("malformed message error = %v", err)
	}
}

func testDocument(t *testing.T, product Product) Document {
	t.Helper()
	geometry, _ := GeometryFor(product)
	uid := []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66}
	pages := make([][4]byte, geometry.TotalPages)
	pages[0] = [4]byte{uid[0], uid[1], uid[2], 0x88 ^ uid[0] ^ uid[1] ^ uid[2]}
	copy(pages[1][:], uid[3:])
	pages[2][0] = uid[3] ^ uid[4] ^ uid[5] ^ uid[6]
	cc := CapabilityContainer{Valid: true, Magic: 0xE1, Version: 0x10, Access: 0x00}
	switch product {
	case ProductNTAG213:
		pages[3] = [4]byte{0xE1, 0x10, 0x12, 0x00}
		cc.Size, cc.Capacity = 0x12, 144
	case ProductNTAG215:
		pages[3] = [4]byte{0xE1, 0x10, 0x3E, 0x00}
		cc.Size, cc.Capacity = 0x3E, 496
	case ProductNTAG216:
		pages[3] = [4]byte{0xE1, 0x10, 0x6D, 0x00}
		cc.Size, cc.Capacity = 0x6D, 872
	}
	card := nfc.CardInfo{UID: uid, ATQA: [2]byte{0x00, 0x44}, SAK: 0x00}
	identity := Identity{Product: product, Geometry: geometry}
	return NewDocument(card, identity, pages, cc)
}

func TestPlanNDEFWrites(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	message := textRecord("hello")
	writes, err := PlanNDEFWrites(document, message)
	if err != nil {
		t.Fatal(err)
	}
	if len(writes) == 0 || writes[0].Page != 4 {
		t.Fatalf("writes = %+v", writes)
	}
	area, _ := BuildNDEFArea(document.CC.Capacity, message)
	for _, write := range writes {
		data, err := write.Data()
		if err != nil {
			t.Fatal(err)
		}
		index := (write.Page - document.UserPages.Start) * PageSize
		if !bytes.Equal(data[:], area[index:index+PageSize]) {
			t.Fatalf("page %d data = % X; want % X", write.Page, data, area[index:index+PageSize])
		}
	}
}

func TestPlanNDEFWritesRefusesControlTLV(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	document.Pages[4] = "01020000"
	if _, err := PlanNDEFWrites(document, textRecord("hello")); !errors.Is(err, ErrUnsupportedTLV) {
		t.Fatalf("control TLV error = %v", err)
	}
}

func TestRestorePlanProtectsSystemPages(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	writes, err := RestorePlan(document, true)
	if err != nil {
		t.Fatal(err)
	}
	seen := map[int]string{}
	for _, write := range writes {
		seen[write.Page] = write.Region
	}
	for page := 0; page <= 3; page++ {
		if _, ok := seen[page]; ok {
			t.Fatalf("restore plan writes page %d", page)
		}
	}
	for page := 4; page <= 39; page++ {
		if seen[page] != "user" {
			t.Fatalf("page %d region = %q", page, seen[page])
		}
	}
	if seen[43] != "password" || seen[44] != "pack" || seen[41] != "config" || seen[40] != "dynamic-lock" {
		t.Fatalf("protected regions = %+v", seen)
	}
	if writes[len(writes)-1].Page != 40 {
		t.Fatalf("last restore page = %d; want dynamic-lock 40", writes[len(writes)-1].Page)
	}
}

func TestDocumentJSONRoundTrip(t *testing.T) {
	document := testDocument(t, ProductNTAG215)
	document.NDEF = &NDEF{Present: true, Length: 1, Raw: "D0", Records: []Record{{TNF: 1, Type: "T"}}}
	encoded, err := json.Marshal(document)
	if err != nil {
		t.Fatal(err)
	}
	var decoded Document
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if err := decoded.ValidateForCard(nfc.CardInfo{UID: []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66}, ATQA: [2]byte{0x00, 0x44}, SAK: 0x00}); err != nil {
		t.Fatal(err)
	}
	if decoded.Pages[3] != strings.ToUpper(hex.EncodeToString([]byte{0xE1, 0x10, 0x3E, 0x00})) {
		t.Fatalf("page 3 = %q", decoded.Pages[3])
	}
}
