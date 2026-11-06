package release_test

import (
	"path/filepath"
	"runtime"
	"testing"

	nfcxrelease "github.com/YangtseSu/nfc-foundry/internal/release"
)

func TestLockedComponentsUsesRepositoryLocks(t *testing.T) {
	_, file, _, _ := runtime.Caller(0)
	root := filepath.Clean(filepath.Join(filepath.Dir(file), "..", ".."))
	components, err := nfcxrelease.LockedComponents(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(components) != 4 || components[0].ID != "libnfc" || components[0].Version != "1.8.0" {
		t.Fatalf("components = %#v", components)
	}
}
