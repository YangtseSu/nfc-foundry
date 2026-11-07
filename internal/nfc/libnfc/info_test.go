package libnfc_test

import (
	"slices"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc/libnfc"
)

func TestInfoReportsCompiledDrivers(t *testing.T) {
	info := libnfc.Info()
	if !info.Available {
		if info.Version != "" || len(info.Drivers) != 0 || info.Error == "" {
			t.Fatalf("unavailable runtime reported stale metadata: %#v", info)
		}
		return
	}
	if info.Version == "" {
		t.Fatal("available runtime reported no version")
	}
	if len(info.Drivers) == 0 {
		t.Fatal("available runtime reported no compiled drivers")
	}
	for _, driver := range info.Drivers {
		if driver == "" {
			t.Fatalf("compiled driver list contains an empty name: %#v", info.Drivers)
		}
	}
	if !slices.Contains(info.Drivers, "pn532_uart") {
		t.Fatalf("compiled driver list omits the baseline pn532_uart driver: %#v", info.Drivers)
	}
}
