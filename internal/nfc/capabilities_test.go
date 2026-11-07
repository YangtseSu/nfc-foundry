package nfc_test

import (
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestDriverOf(t *testing.T) {
	tests := []struct {
		connString string
		want       nfc.Driver
	}{
		{"pn532_uart:/dev/ttyUSB0", nfc.DriverPN532UART},
		{"  pn53x_usb:001:004  ", nfc.DriverPN53XUSB},
		{"acr122_usb:001:004", nfc.DriverACR122USB},
		{"acr122_pcsc:ACS ACR122U 00 00", nfc.DriverACR122PCSC},
		{"pcsc:ACS ACR122U 00 00", nfc.DriverPCSC},
		{"mock:test", nfc.Driver("mock")},
		{"", nfc.Driver("")},
	}
	for _, test := range tests {
		if got := nfc.DriverOf(test.connString); got != test.want {
			t.Errorf("DriverOf(%q) = %q; want %q", test.connString, got, test.want)
		}
	}
}

func TestCapabilitiesForKnownDrivers(t *testing.T) {
	tests := []struct {
		driver         nfc.Driver
		classic        bool
		type2          bool
		rawBits        bool
		apdu           bool
		nested         bool
		known          bool
		validation     nfc.ValidationLevel
		wantAdvertised bool
	}{
		{nfc.DriverPN532UART, true, true, true, true, true, true, nfc.ValidationPartial, true},
		{nfc.DriverPN53XUSB, true, true, true, true, false, true, nfc.ValidationNone, false},
		{nfc.DriverACR122USB, true, true, true, true, false, true, nfc.ValidationPartial, true},
		{nfc.DriverACR122PCSC, true, true, true, true, false, true, nfc.ValidationNone, false},
		{nfc.DriverPCSC, false, false, false, false, false, true, nfc.ValidationNone, false},
	}
	for _, test := range tests {
		caps := nfc.CapabilitiesFor(string(test.driver) + ":test")
		if caps.Driver != test.driver {
			t.Fatalf("%s: driver = %q", test.driver, caps.Driver)
		}
		if caps.Known != test.known {
			t.Errorf("%s: known = %t", test.driver, caps.Known)
		}
		if caps.SupportsClassic() != test.classic {
			t.Errorf("%s: classic = %t", test.driver, caps.SupportsClassic())
		}
		if caps.SupportsType2() != test.type2 {
			t.Errorf("%s: type2 = %t", test.driver, caps.SupportsType2())
		}
		if caps.RawBits != test.rawBits {
			t.Errorf("%s: raw bits = %t", test.driver, caps.RawBits)
		}
		if caps.APDU != test.apdu {
			t.Errorf("%s: apdu = %t", test.driver, caps.APDU)
		}
		if caps.Nested != test.nested {
			t.Errorf("%s: nested = %t", test.driver, caps.Nested)
		}
		if caps.Validation != test.validation {
			t.Errorf("%s: validation = %q", test.driver, caps.Validation)
		}
		if caps.Advertised() != test.wantAdvertised {
			t.Errorf("%s: advertised = %t", test.driver, caps.Advertised())
		}
	}
}

func TestCapabilitiesForUnknownDriverStayUsableButUnverified(t *testing.T) {
	caps := nfc.CapabilitiesFor("mock:test")
	if caps.Driver != "mock" || caps.Known {
		t.Fatalf("unexpected unknown profile: %#v", caps)
	}
	if !caps.Enumerate || !caps.SelectISO14443A || !caps.SupportsClassic() ||
		!caps.SupportsType2() || !caps.RawBytes || !caps.RawBits || !caps.APDU {
		t.Fatalf("unknown profile lost implementation capabilities: %#v", caps)
	}
	if caps.Nested || caps.Darkside || caps.Hardnested {
		t.Fatalf("unknown profile must not advertise recovery: %#v", caps)
	}
	if caps.Validation != nfc.ValidationUnverified || caps.Advertised() {
		t.Fatalf("unknown profile must stay unverified: %#v", caps)
	}
}

func TestReaderProfilesAreIndependentCopies(t *testing.T) {
	profiles := nfc.ReaderProfiles()
	profiles[0].ClassicWrite = false
	if !nfc.CapabilitiesFor("pn532_uart:/dev/test").ClassicWrite {
		t.Fatal("ReaderProfiles returned mutable shared state")
	}
}
