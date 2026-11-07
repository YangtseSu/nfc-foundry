//go:build unix

package device

import (
	"crypto/sha256"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"golang.org/x/sys/unix"
)

type readerLock struct {
	file *os.File
}

func defaultLockDir() string {
	if dir := strings.TrimSpace(os.Getenv("XDG_RUNTIME_DIR")); dir != "" {
		return filepath.Join(dir, "nfc-foundry")
	}
	return filepath.Join(os.TempDir(), fmt.Sprintf("nfc-foundry-%d", os.Getuid()))
}

// acquireReaderLock takes a non-blocking exclusive flock keyed by the exact
// connstring. The lock file lives in a 0700 runtime directory so reader
// identifiers are not exposed to other users.
func acquireReaderLock(dir, connString string) (*readerLock, error) {
	if strings.TrimSpace(dir) == "" {
		dir = defaultLockDir()
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	sum := sha256.Sum256([]byte(readerLockIdentity(connString)))
	path := filepath.Join(dir, fmt.Sprintf("reader-%x.lock", sum[:16]))
	file, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}
	if err := unix.Flock(int(file.Fd()), unix.LOCK_EX|unix.LOCK_NB); err != nil {
		file.Close()
		if errors.Is(err, unix.EWOULDBLOCK) {
			holder := readLockHolder(path)
			return nil, &LockedError{ConnString: connString, PID: holder.pid, Program: holder.program}
		}
		return nil, err
	}
	if err := file.Truncate(0); err == nil {
		_, _ = fmt.Fprintf(file, "pid=%d\nprogram=%s\nconn=%s\n", os.Getpid(), filepath.Base(os.Args[0]), connString)
		_ = file.Sync()
	}
	return &readerLock{file: file}, nil
}

func (l *readerLock) release() error {
	if l == nil || l.file == nil {
		return nil
	}
	unlockErr := unix.Flock(int(l.file.Fd()), unix.LOCK_UN)
	closeErr := l.file.Close()
	l.file = nil
	if unlockErr != nil {
		return unlockErr
	}
	return closeErr
}

type lockHolder struct {
	pid     int
	program string
}

func readLockHolder(path string) lockHolder {
	data, err := os.ReadFile(path)
	if err != nil {
		return lockHolder{}
	}
	var holder lockHolder
	for _, line := range strings.Split(string(data), "\n") {
		key, value, found := strings.Cut(line, "=")
		if !found {
			continue
		}
		switch key {
		case "pid":
			holder.pid, _ = strconv.Atoi(strings.TrimSpace(value))
		case "program":
			holder.program = strings.TrimSpace(value)
		}
	}
	return holder
}
