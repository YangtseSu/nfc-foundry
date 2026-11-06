//go:build !unix

package device

type readerLock struct{}

// Cross-process reader locking is implemented on Linux and the other unix
// targets. Other platforms are not supported build targets.
func acquireReaderLock(string, string) (*readerLock, error) {
	return &readerLock{}, nil
}

func (l *readerLock) release() error { return nil }
