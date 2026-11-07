package device_test

import (
	"reflect"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestCollapseReaderDuplicatesPrefersDirectACR122USB(t *testing.T) {
	devices := []nfc.DeviceInfo{
		{ConnString: "acr122_usb:001:004"},
		{ConnString: "acr122_pcsc:ACS ACR122U PICC Interface 00 00"},
		{ConnString: "pcsc:ACS ACR122U PICC Interface 00 00"},
		{ConnString: "pn532_uart:/dev/ttyUSB0"},
		{ConnString: "pcsc:Identiv uTrust 3700 F CL Reader 00 00"},
	}
	got := device.CollapseReaderDuplicates(devices)
	want := []nfc.DeviceInfo{
		{ConnString: "acr122_usb:001:004"},
		{ConnString: "pn532_uart:/dev/ttyUSB0"},
		{ConnString: "pcsc:Identiv uTrust 3700 F CL Reader 00 00"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("collapsed devices = %#v; want %#v", got, want)
	}
}

func TestCollapseReaderDuplicatesPrefersACR122PCSCDriver(t *testing.T) {
	devices := []nfc.DeviceInfo{
		{ConnString: "acr122_pcsc:ACS ACR122U PICC Interface 00 00"},
		{ConnString: "pcsc:ACS ACR122U PICC Interface 00 00"},
		{ConnString: "pcsc:ACS ACR1222 00 00"},
	}
	got := device.CollapseReaderDuplicates(devices)
	want := []nfc.DeviceInfo{
		{ConnString: "acr122_pcsc:ACS ACR122U PICC Interface 00 00"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("collapsed devices = %#v; want %#v", got, want)
	}
}

func TestCollapseReaderDuplicatesLeavesUnrelatedReadersAlone(t *testing.T) {
	devices := []nfc.DeviceInfo{
		{ConnString: "pn53x_usb:001:003"},
		{ConnString: "pcsc:Identiv uTrust 3700 F CL Reader 00 00"},
		{ConnString: "pn532_uart:/dev/ttyUSB0"},
	}
	got := device.CollapseReaderDuplicates(devices)
	if !reflect.DeepEqual(got, devices) {
		t.Fatalf("collapsed devices = %#v; want unchanged %#v", got, devices)
	}
}
