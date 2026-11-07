package libnfc

import (
	"context"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestGenericPCSCReaderRejectsUnsupportedNativePaths(t *testing.T) {
	native := &fakeNative{}
	reader := &Reader{native: native}
	if err := reader.Open(context.Background(), "pcsc:ACS ACR122U PICC Interface 00 00"); err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { _ = reader.Close() })

	ctx := context.Background()
	checks := []struct {
		name string
		call func() error
	}{
		{"authenticate", func() error {
			return reader.Authenticate(ctx, 1, nfc.KeyTypeA, nfc.Key{})
		}},
		{"read block", func() error {
			_, err := reader.ReadBlock(ctx, 1)
			return err
		}},
		{"write block", func() error {
			return reader.WriteBlock(ctx, 1, [16]byte{})
		}},
		{"manufacturer write", func() error {
			return reader.WriteManufacturerBlock(ctx, [16]byte{})
		}},
		{"trailer write", func() error {
			return reader.WriteSectorTrailer(ctx, 3, [16]byte{})
		}},
		{"raw mode", func() error {
			return reader.SetRawMode(ctx, true)
		}},
		{"raw bytes", func() error {
			_, err := reader.TransceiveBytes(ctx, []byte{0x30, 0x00})
			return err
		}},
		{"raw bits", func() error {
			_, _, err := reader.TransceiveBits(ctx, []byte{0x26}, 7)
			return err
		}},
	}
	for _, check := range checks {
		err := check.call()
		if !errors.Is(err, nfc.ErrUnsupported) {
			t.Fatalf("%s error = %v; want ErrUnsupported", check.name, err)
		}
	}
	if native.opens != 1 || native.closes != 0 {
		t.Fatalf("unsupported operations did not stay local: opens=%d closes=%d", native.opens, native.closes)
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.authCalls != 0 || len(native.readBlocks) != 0 || native.writeCalls != 0 ||
		len(native.transceiveFrames) != 0 || len(native.transceiveBitFrames) != 0 ||
		len(native.rawModeCalls) != 0 || native.manufacturerWritten != [16]byte{} {
		t.Fatalf("unsupported operations reached the native driver: %+v", native)
	}
}

func TestConnectionHintsCoverPCSCAndUSBDrivers(t *testing.T) {
	tests := map[string]string{
		"pcsc:reader":             "pcscd",
		"acr122_pcsc:reader":      "pcscd",
		"acr122_usb:001:004":      "pcscd",
		"pn53x_usb:001:004":       "pcscd",
		"pn532_uart:/dev/ttyUSB0": "",
	}
	for connString, want := range tests {
		hint := connectionHint(connString)
		if want == "" {
			if hint != "" {
				t.Errorf("connectionHint(%q) = %q; want empty", connString, hint)
			}
			continue
		}
		if hint == "" {
			t.Errorf("connectionHint(%q) is empty; want an actionable hint", connString)
		}
	}
}
