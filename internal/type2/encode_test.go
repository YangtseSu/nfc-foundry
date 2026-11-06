package type2

import (
	"bytes"
	"errors"
	"strings"
	"testing"
)

func TestEncodeTextMessageRoundTrip(t *testing.T) {
	message, err := EncodeTextMessage("hello", "en")
	if err != nil {
		t.Fatal(err)
	}
	records, err := ParseMessage(message)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 || records[0].Text != "hello" || records[0].Type != "T" {
		t.Fatalf("records = %+v", records)
	}
}

func TestEncodeURIMessageUsesLongestPrefix(t *testing.T) {
	message, err := EncodeURIMessage("https://example.com/path")
	if err != nil {
		t.Fatal(err)
	}
	records, err := ParseMessage(message)
	if err != nil {
		t.Fatal(err)
	}
	if records[0].URI != "https://example.com/path" {
		t.Fatalf("URI = %q", records[0].URI)
	}
	if !bytes.HasPrefix(message, []byte{0xD1, 0x01}) {
		t.Fatalf("message header = % X", message[:2])
	}
}

func TestEncodeLongTextUsesLongRecord(t *testing.T) {
	text := strings.Repeat("A", 300)
	message, err := EncodeTextMessage(text, "en")
	if err != nil {
		t.Fatal(err)
	}
	if message[0]&0x10 != 0 {
		t.Fatal("long payload used the short-record flag")
	}
	records, err := ParseMessage(message)
	if err != nil {
		t.Fatal(err)
	}
	if records[0].Text != text {
		t.Fatalf("decoded text length = %d", len(records[0].Text))
	}
}

func TestEncodeRejectsEmptyURI(t *testing.T) {
	if _, err := EncodeURIMessage(""); !errors.Is(err, ErrMalformedNDEF) {
		t.Fatalf("empty URI error = %v", err)
	}
}
