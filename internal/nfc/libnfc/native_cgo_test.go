//go:build libnfc && cgo

package libnfc

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestCGOInitializeAndExit(t *testing.T) {
	before := nativeResources()
	if result := nativeSmoke(); result.code != nativeOK {
		t.Fatalf("nativeSmoke() = %+v", result)
	}
	if after := nativeResources(); after != before {
		t.Fatalf("resources after smoke = %+v; want %+v", after, before)
	}
}

func TestCGODiscoveryDefaults(t *testing.T) {
	if result := nativeConfigureDiscoveryDefaults(); result.code != nativeOK {
		t.Fatalf("nativeConfigureDiscoveryDefaults() = %+v", result)
	}
}

func TestCGOOpenFailureReleasesResources(t *testing.T) {
	reader := NewReader()
	before := nativeResources()
	connString := strings.Repeat("x", 1024)
	err := reader.Open(context.Background(), connString)
	if !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("Open() error = %v; want ErrInvalidArgument", err)
	}
	if after := nativeResources(); after != before {
		t.Fatalf("resources after failed open = %+v; want %+v", after, before)
	}

	err = reader.Open(context.Background(), "not_a_real_libnfc_driver:test")
	if !errors.Is(err, nfc.ErrDeviceDisconnected) {
		t.Fatalf("Open() unknown driver error = %v; want ErrDeviceDisconnected", err)
	}
	if after := nativeResources(); after != before {
		t.Fatalf("resources after native open failure = %+v; want %+v", after, before)
	}
}

func TestCGOListDevicesUsesEnvironmentSelection(t *testing.T) {
	const connString = "pn532_uart:/dev/nfc-foundry-smoke-not-opened"
	t.Setenv("LIBNFC_DEVICE", connString)
	t.Setenv("LIBNFC_AUTO_SCAN", "false")
	t.Setenv("LIBNFC_INTRUSIVE_SCAN", "false")
	devices, err := NewBackend().ListDevices(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].ConnString != connString {
		t.Fatalf("devices = %+v; want exactly %q", devices, connString)
	}
}
