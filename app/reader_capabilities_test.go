package app

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
)

func TestGenericPCSCReaderRejectsUnsupportedWorkflowsBeforeHardware(t *testing.T) {
	connString := "pcsc:ACS ACR122U PICC Interface 00 00"
	enumerator := mock.NewEnumerator([]nfc.DeviceInfo{{Name: "PC/SC", ConnString: connString}}, nil)
	service := NewServiceWithOptions(Options{
		Enumerator: enumerator,
		NewReader:  func() nfc.Reader { return mock.NewReader() },
		DeviceConfig: device.Config{
			PollInterval:      time.Hour,
			PollTimeout:       20 * time.Millisecond,
			OpenTimeout:       100 * time.Millisecond,
			RemovalMisses:     2,
			ReconnectBackoffs: []time.Duration{time.Millisecond},
			LockDir:           t.TempDir(),
		},
		KeyStorePath:      t.TempDir() + "/keys.json",
		UIDBackupRoot:     t.TempDir(),
		Type2BackupRoot:   t.TempDir(),
		ClassicBackupRoot: t.TempDir(),
		Type4BackupRoot:   t.TempDir(),
	})
	t.Cleanup(service.Shutdown)
	if err := service.ConnectDevice(connString); err != nil {
		t.Fatalf("connect: %v", err)
	}

	checks := []struct {
		name string
		call func() error
	}{
		{"classic read", func() error {
			_, err := service.ReadClassicBlock(context.Background(), 1, nil)
			return err
		}},
		{"classic write", func() error {
			_, err := service.WriteClassicBlock(context.Background(), 1, [16]byte{}, nil)
			return err
		}},
		{"classic key scan", func() error {
			_, err := service.StartKeyScan(false)
			return err
		}},
		{"classic card read", func() error {
			_, err := service.StartReadCard(ReadCardStartRequestDTO{})
			return err
		}},
		{"type2 read", func() error {
			_, err := service.ReadType2Card(context.Background(), "")
			return err
		}},
		{"type2 write", func() error {
			_, err := service.WriteType2Pages(context.Background(), Type2WriteRequestDTO{
				Writes: []Type2PageWriteDTO{{Page: 4, Hex: "00000000"}},
			})
			return err
		}},
		{"apdu", func() error {
			_, err := service.TransmitAPDU(context.Background(), "00A40400")
			return err
		}},
	}
	for _, check := range checks {
		err := check.call()
		if err == nil {
			t.Fatalf("%s unexpectedly succeeded", check.name)
		}
		if code, ok := nfc.ErrorCodeOf(err); !ok || code != nfc.CodeUnsupported {
			t.Fatalf("%s error = %v (code=%v); want CodeUnsupported", check.name, err, code)
		}
		var opErr *nfc.OpError
		if !errors.As(err, &opErr) || opErr.Detail == "" {
			t.Fatalf("%s error lacks actionable detail: %v", check.name, err)
		}
	}
}
