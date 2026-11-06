package device_test

import (
	"bufio"
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
)

func TestReaderLockRejectsSecondManager(t *testing.T) {
	config := testConfig(t)
	reader := mock.NewReader()
	first := device.NewManager(mock.NewEnumerator(nil, nil), func() nfc.Reader { return reader }, nil, config)
	if err := first.Connect(context.Background(), "pn532_uart:/dev/locked"); err != nil {
		t.Fatalf("first connect: %v", err)
	}

	second := device.NewManager(mock.NewEnumerator(nil, nil), func() nfc.Reader { return mock.NewReader() }, nil, config)
	err := second.Connect(context.Background(), "pn532_uart:/dev/locked")
	if !errors.Is(err, device.ErrReaderLocked) {
		t.Fatalf("second connect error = %v; want ErrReaderLocked", err)
	}
	if code, ok := nfc.ErrorCodeOf(err); !ok || code != nfc.CodeBusy {
		t.Fatalf("second connect code = %v; want busy", code)
	}

	different := device.NewManager(mock.NewEnumerator(nil, nil), func() nfc.Reader { return mock.NewReader() }, nil, config)
	if err := different.Connect(context.Background(), "pn532_uart:/dev/other"); err != nil {
		t.Fatalf("different reader connect: %v", err)
	}
	t.Cleanup(different.Shutdown)

	if err := first.Disconnect(); err != nil {
		t.Fatalf("first disconnect: %v", err)
	}
	if err := second.Connect(context.Background(), "pn532_uart:/dev/locked"); err != nil {
		t.Fatalf("connect after release: %v", err)
	}
	t.Cleanup(second.Shutdown)
}

func TestReaderLockFileIsPrivate(t *testing.T) {
	dir := t.TempDir()
	manager := device.NewManager(mock.NewEnumerator(nil, nil), func() nfc.Reader { return mock.NewReader() }, nil, device.Config{
		PollInterval: time.Millisecond, PollTimeout: 20 * time.Millisecond, OpenTimeout: 20 * time.Millisecond,
		RemovalMisses: 2, ReconnectBackoffs: []time.Duration{time.Millisecond}, LockDir: dir,
	})
	if err := manager.Connect(context.Background(), "pn532_uart:/dev/private"); err != nil {
		t.Fatalf("connect: %v", err)
	}
	t.Cleanup(manager.Shutdown)

	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatalf("read lock dir: %v", err)
	}
	if len(entries) != 1 {
		t.Fatalf("lock files = %d; want 1", len(entries))
	}
	info, err := entries[0].Info()
	if err != nil {
		t.Fatalf("stat lock file: %v", err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("lock file mode = %o; want 600", info.Mode().Perm())
	}
	if filepath.Ext(info.Name()) != ".lock" {
		t.Fatalf("lock file name = %q; want .lock suffix", info.Name())
	}
}

// TestReaderLockHelperProcess is launched by TestReaderLockRejectsOtherProcess.
// It holds a reader lock until the parent kills it.
func TestReaderLockHelperProcess(t *testing.T) {
	if os.Getenv("NFC_FOUNDRY_LOCK_HELPER") != "1" {
		t.Skip("helper process only")
	}
	manager := device.NewManager(mock.NewEnumerator(nil, nil), func() nfc.Reader { return mock.NewReader() }, nil, device.Config{
		PollInterval: time.Hour, PollTimeout: time.Second, OpenTimeout: time.Second,
		RemovalMisses: 2, ReconnectBackoffs: []time.Duration{time.Second},
		LockDir: os.Getenv("NFC_FOUNDRY_LOCK_DIR"),
	})
	if err := manager.Connect(context.Background(), os.Getenv("NFC_FOUNDRY_LOCK_CONN")); err != nil {
		t.Fatalf("helper connect: %v", err)
	}
	defer manager.Shutdown()
	os.Stdout.WriteString("locked\n")
	select {}
}

func TestReaderLockRejectsOtherProcess(t *testing.T) {
	dir := t.TempDir()
	connString := "pn532_uart:/dev/cross-process"
	command := exec.Command(os.Args[0], "-test.run=^TestReaderLockHelperProcess$")
	command.Env = append(os.Environ(),
		"NFC_FOUNDRY_LOCK_HELPER=1",
		"NFC_FOUNDRY_LOCK_DIR="+dir,
		"NFC_FOUNDRY_LOCK_CONN="+connString,
	)
	stdout, err := command.StdoutPipe()
	if err != nil {
		t.Fatalf("stdout pipe: %v", err)
	}
	command.Stderr = os.Stderr
	if err := command.Start(); err != nil {
		t.Fatalf("start helper: %v", err)
	}
	t.Cleanup(func() {
		_ = command.Process.Kill()
		_, _ = command.Process.Wait()
	})

	ready := make(chan error, 1)
	go func() {
		scanner := bufio.NewScanner(stdout)
		for scanner.Scan() {
			if scanner.Text() == "locked" {
				ready <- nil
				return
			}
		}
		ready <- errors.New("helper exited before acquiring the lock")
	}()
	select {
	case err := <-ready:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("helper did not acquire the lock in time")
	}

	manager := device.NewManager(mock.NewEnumerator(nil, nil), func() nfc.Reader { return mock.NewReader() }, nil, device.Config{
		PollInterval: time.Millisecond, PollTimeout: 20 * time.Millisecond, OpenTimeout: 20 * time.Millisecond,
		RemovalMisses: 2, ReconnectBackoffs: []time.Duration{time.Millisecond}, LockDir: dir,
	})
	err = manager.Connect(context.Background(), connString)
	if !errors.Is(err, device.ErrReaderLocked) {
		t.Fatalf("cross-process connect error = %v; want ErrReaderLocked", err)
	}
}
