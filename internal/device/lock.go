package device

import (
	"errors"
	"fmt"
)

// ErrReaderLocked reports that another process currently owns the reader.
var ErrReaderLocked = errors.New("reader is already open in another process")

// LockedError carries the holder information read from the lock file.
type LockedError struct {
	ConnString string
	PID        int
	Program    string
}

func (e *LockedError) Error() string {
	holder := "another process"
	if e.PID > 0 && e.Program != "" {
		holder = fmt.Sprintf("%s (pid %d)", e.Program, e.PID)
	} else if e.PID > 0 {
		holder = fmt.Sprintf("pid %d", e.PID)
	}
	return fmt.Sprintf("reader %q is in use by %s", e.ConnString, holder)
}

func (e *LockedError) Unwrap() error { return ErrReaderLocked }
