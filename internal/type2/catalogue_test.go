package type2

import (
	"errors"
	"strings"
	"testing"
)

func parseSingle(t *testing.T, message []byte) Record {
	t.Helper()
	records, err := ParseMessage(message)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 {
		t.Fatalf("record count = %d", len(records))
	}
	return records[0]
}

func TestSmartPosterRoundTrip(t *testing.T) {
	message, err := EncodeSmartPosterMessage("https://example.com/poster", "Example poster")
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Kind != KindSmartPoster || record.Summary != "Example poster" ||
		record.URI != "https://example.com/poster" || record.Text != "Example poster" {
		t.Fatalf("record = %+v", record)
	}
}

func TestVCardRoundTrip(t *testing.T) {
	message, err := EncodeVCardMessage(map[string]string{
		"name": "Ada Lovelace", "organization": "Analytical Engines",
		"job_title": "Mathematician", "phone": "+44 20 1234", "email": "ada@example.com",
		"address": "12 St James's Square", "url": "https://example.com", "note": "Met; once",
	})
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Kind != KindVCard || record.Summary != "Ada Lovelace" {
		t.Fatalf("record = %+v", record)
	}
	values := fieldMap(record.Fields)
	for key, want := range map[string]string{
		"name": "Ada Lovelace", "organization": "Analytical Engines", "job_title": "Mathematician",
		"phone": "+44 20 1234", "email": "ada@example.com", "address": "12 St James's Square",
		"url": "https://example.com", "note": "Met; once",
	} {
		if values[key] != want {
			t.Fatalf("vCard field %s = %q; want %q", key, values[key], want)
		}
	}
}

func TestParseFoldedVCard(t *testing.T) {
	payload := []byte("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada\r\n Lovelace\r\nEMAIL:ada@example.com\r\nEND:VCARD\r\n")
	name, fields := parseVCard(payload)
	if name != "AdaLovelace" && name != "Ada Lovelace" {
		t.Fatalf("folded name = %q", name)
	}
	if fieldMap(fields)["email"] != "ada@example.com" {
		t.Fatalf("fields = %+v", fields)
	}
}

func TestWiFiRoundTrip(t *testing.T) {
	message, err := EncodeWiFiMessage("Foundry Lab", "correct horse battery staple", "wpa2")
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Kind != KindWiFi || record.Summary != "Foundry Lab" {
		t.Fatalf("record = %+v", record)
	}
	values := fieldMap(record.Fields)
	if values["ssid"] != "Foundry Lab" || values["auth"] != "WPA2-Personal" ||
		values["encryption"] != "AES" || values["network_key"] != "••••••" {
		t.Fatalf("Wi-Fi fields = %+v", record.Fields)
	}
	if _, err := EncodeWiFiMessage("Open", "secret", "open"); !errors.Is(err, ErrMalformedNDEF) {
		t.Fatalf("open network with password error = %v", err)
	}
}

func TestBluetoothRoundTrip(t *testing.T) {
	message, err := EncodeBluetoothMessage("01:02:03:04:05:06", "Foundry speaker")
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Kind != KindBluetooth || record.Summary != "Foundry speaker" {
		t.Fatalf("record = %+v", record)
	}
	values := fieldMap(record.Fields)
	if values["bt_address"] != "01:02:03:04:05:06" || values["bt_name"] != "Foundry speaker" {
		t.Fatalf("Bluetooth fields = %+v", record.Fields)
	}
}

func TestAndroidApplicationRecord(t *testing.T) {
	message, err := EncodeExternalMessage("android.com:pkg", []byte("com.example.app"))
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if record.Kind != KindApp || record.Summary != "com.example.app" || fieldMap(record.Fields)["package"] != "com.example.app" {
		t.Fatalf("record = %+v", record)
	}
}

func TestTemplateEncoders(t *testing.T) {
	tests := []struct {
		template string
		fields   map[string]string
		kind     string
	}{
		{"text", map[string]string{"text": "Hello"}, KindText},
		{"uri", map[string]string{"uri": "https://example.com"}, KindURL},
		{"custom", map[string]string{"uri": "app://open/1"}, KindURI},
		{"search", map[string]string{"search_prefix": "https://duckduckgo.com/?q=", "query": "nfc foundry"}, KindURL},
		{"payment", map[string]string{"uri": "https://pay.example.com/1"}, KindURL},
		{"social", map[string]string{"uri": "https://social.example.com/@foundry"}, KindURL},
		{"app", map[string]string{"package": "com.example.app"}, KindApp},
		{"email", map[string]string{"to": "test@example.com", "subject": "NFC"}, KindEmail},
		{"contact", map[string]string{"name": "Test Person", "email": "test@example.com"}, KindVCard},
		{"phone", map[string]string{"phone": "+1 555 0100"}, KindPhone},
		{"sms", map[string]string{"number": "+1 555 0100", "body": "Hello"}, KindSMS},
		{"location", map[string]string{"latitude": "37.7749", "longitude": "-122.4194", "label": "Office"}, KindLocation},
		{"address", map[string]string{"address": "1 Market Street"}, KindLocation},
		{"bluetooth", map[string]string{"address": "01:02:03:04:05:06", "name": "Speaker"}, KindBluetooth},
		{"wifi", map[string]string{"ssid": "Foundry", "password": "password123", "security": "wpa2"}, KindWiFi},
		{"smartposter", map[string]string{"uri": "https://example.com", "title": "Example"}, KindSmartPoster},
	}
	for _, test := range tests {
		message, err := EncodeTemplate(test.template, test.fields)
		if err != nil {
			t.Fatalf("%s: %v", test.template, err)
		}
		record := parseSingle(t, message)
		if record.Kind != test.kind {
			t.Fatalf("%s kind = %q; want %q", test.template, record.Kind, test.kind)
		}
	}
}

func TestTemplateRequiredFields(t *testing.T) {
	for _, test := range []struct {
		template string
		fields   map[string]string
	}{
		{"text", map[string]string{}},
		{"uri", map[string]string{}},
		{"contact", map[string]string{}},
		{"wifi", map[string]string{"ssid": "Foundry", "security": "wpa2"}},
		{"location", map[string]string{"latitude": "200", "longitude": "0"}},
		{"sms", map[string]string{}},
		{"bluetooth", map[string]string{"address": "bad"}},
	} {
		if _, err := EncodeTemplate(test.template, test.fields); !errors.Is(err, ErrMalformedNDEF) {
			t.Fatalf("%s error = %v", test.template, err)
		}
	}
}

func TestTemplateSearchEncodesQuery(t *testing.T) {
	message, err := EncodeTemplate("search", map[string]string{
		"search_prefix": "https://search.example.com/?q=",
		"query":         "nfc & ndef",
	})
	if err != nil {
		t.Fatal(err)
	}
	record := parseSingle(t, message)
	if !strings.Contains(record.URI, "nfc+%26+ndef") {
		t.Fatalf("search URI = %q", record.URI)
	}
}

func fieldMap(fields []Field) map[string]string {
	values := make(map[string]string, len(fields))
	for _, field := range fields {
		values[field.Key] = field.Value
	}
	return values
}
