package type2

import (
	"bytes"
	"errors"
	"testing"
)

func TestTextTemplateLanguage(t *testing.T) {
	message, err := EncodeTemplate("text", map[string]string{"text": "你好", "language": "zh-Hans"})
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Text != "你好" {
		t.Fatalf("text = %q", record.Text)
	}
	payload := record.Payload
	if len(payload) < 2 || payload[0:2] != "07" {
		t.Fatalf("language length in payload = %q", payload)
	}
}

func TestProviderPresets(t *testing.T) {
	message, err := EncodeTemplate("search", map[string]string{"search_provider": "google", "query": "nfc tag"})
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.URI != "https://www.google.com/search?q=nfc+tag" {
		t.Fatalf("search URI = %q", record.URI)
	}
	message, err = EncodeTemplate("social", map[string]string{"social_provider": "github", "handle": "foundry"})
	if err != nil {
		t.Fatal(err)
	}
	if uri := parseSingle(t, message).URI; uri != "https://github.com/foundry" {
		t.Fatalf("social URI = %q", uri)
	}
	message, err = EncodeTemplate("payment", map[string]string{"payment_provider": "paypal", "handle": "foundry"})
	if err != nil {
		t.Fatal(err)
	}
	if uri := parseSingle(t, message).URI; uri != "https://paypal.me/foundry" {
		t.Fatalf("payment URI = %q", uri)
	}
	if _, err := EncodeTemplate("social", map[string]string{"social_provider": "github"}); !errors.Is(err, ErrMalformedNDEF) {
		t.Fatalf("missing handle error = %v", err)
	}
}

func TestBluetoothLERoundTrip(t *testing.T) {
	message, err := EncodeTemplate("bluetooth-le", map[string]string{
		"address": "01:02:03:04:05:06", "address_type": "random", "name": "Foundry LE", "role": "both",
	})
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Kind != KindBluetoothLE || record.Summary != "Foundry LE" {
		t.Fatalf("record = %+v", record)
	}
	values := fieldMap(record.Fields)
	if values["bt_address"] != "01:02:03:04:05:06" || values["bt_address_type"] != "random" ||
		values["bt_name"] != "Foundry LE" || values["bt_role"] != "both" {
		t.Fatalf("BLE fields = %+v", record.Fields)
	}
}

func TestSmartPosterMetadata(t *testing.T) {
	message, err := EncodeTemplate("smartposter", map[string]string{
		"uri": "https://example.com/poster", "title": "Poster", "language": "de",
		"action": "save", "resource_type": "text/html", "resource_size": "2048",
	})
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Kind != KindSmartPoster || record.Summary != "Poster" {
		t.Fatalf("record = %+v", record)
	}
	values := fieldMap(record.Fields)
	if values["action"] != "save" || values["resource_type"] != "text/html" || values["resource_size"] != "2048" {
		t.Fatalf("Smart Poster fields = %+v", record.Fields)
	}
}

func TestEncodeInputsMultiRecord(t *testing.T) {
	message, err := EncodeInputs([]RecordInput{
		{Kind: "text", Fields: map[string]string{"text": "first", "language": "en"}},
		{Kind: "uri", Fields: map[string]string{"uri": "https://example.com"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	records, err := ParseMessage(message)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 2 || records[0].Text != "first" || records[1].URI != "https://example.com" {
		t.Fatalf("records = %+v", records)
	}
	if message[0]&0x80 == 0 || message[0]&0x40 != 0 {
		t.Fatalf("first record flags = % X", message[0])
	}
	if _, err := EncodeInputs(nil); !errors.Is(err, ErrMalformedNDEF) {
		t.Fatalf("empty inputs error = %v", err)
	}
}

func TestBinaryDumpRoundTrip(t *testing.T) {
	document := testDocument(t, ProductNTAG215)
	data, err := document.MarshalBinary()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.HasPrefix(data, BinaryDumpMagic[:]) {
		t.Fatalf("binary magic = % X", data[:8])
	}
	var decoded Document
	if err := decoded.UnmarshalBinary(data); err != nil {
		t.Fatal(err)
	}
	if decoded.UID != document.UID || decoded.Product != document.Product ||
		decoded.TotalPages != document.TotalPages || len(decoded.Pages) != len(document.Pages) {
		t.Fatalf("decoded = %+v", decoded)
	}
	for index := range document.Pages {
		if decoded.Pages[index] != document.Pages[index] {
			t.Fatalf("page %d = %s; want %s", index, decoded.Pages[index], document.Pages[index])
		}
	}
	if decoded.CC.Valid != document.CC.Valid || decoded.Protection.Auth0 != document.Protection.Auth0 {
		t.Fatalf("derived data = %+v", decoded)
	}
}

func TestBinaryDumpRejectsCorruption(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	data, err := document.MarshalBinary()
	if err != nil {
		t.Fatal(err)
	}
	var decoded Document
	if err := decoded.UnmarshalBinary(data[:len(data)-1]); err == nil {
		t.Fatal("truncated dump was accepted")
	}
	bad := append([]byte(nil), data...)
	bad[0] = 'X'
	if err := decoded.UnmarshalBinary(bad); err == nil {
		t.Fatal("bad magic was accepted")
	}
	bad = append([]byte(nil), data...)
	bad[9] = 99
	if err := decoded.UnmarshalBinary(bad); err == nil {
		t.Fatal("unknown product was accepted")
	}
}
