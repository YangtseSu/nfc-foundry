package cli

import (
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/app"
)

func TestReaderCapabilitySummary(t *testing.T) {
	tests := []struct {
		name   string
		device app.DeviceDTO
		want   []string
	}{
		{
			name: "pn532",
			device: app.DeviceDTO{
				Known: true, ClassicRead: true, ClassicWrite: true, Type2Read: true, Type2Write: true,
				APDU: true, RawBits: true, Nested: true, Darkside: true, Hardnested: true, Validation: "partial",
			},
			want: []string{"classic read/write", "type2 read/write", "iso-dep", "raw bits", "recovery nested/darkside/hardnested", "partially hardware-validated"},
		},
		{
			name: "pcsc",
			device: app.DeviceDTO{
				Known: true, Validation: "none",
			},
			want: []string{"not hardware-validated"},
		},
		{
			name: "unclassified",
			device: app.DeviceDTO{
				Known: false, ClassicRead: true, ClassicWrite: true, Validation: "unverified",
			},
			want: []string{"classic read/write", "unverified profile"},
		},
	}
	for _, test := range tests {
		got := readerCapabilitySummary(test.device)
		for _, want := range test.want {
			if !strings.Contains(got, want) {
				t.Errorf("%s: summary %q does not contain %q", test.name, got, want)
			}
		}
	}
}
