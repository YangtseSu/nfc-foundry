package workflow

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
)

// FileClassicNDEFBackupStore writes project dumps with restrictive
// permissions. It stores the whole card attempt, including masked Key B bytes,
// so a failed format can be inspected and restored where possible.
type FileClassicNDEFBackupStore struct {
	Root string
}

func (s FileClassicNDEFBackupStore) Save(ctx context.Context, result DumpResult) (string, error) {
	if ctx == nil || strings.TrimSpace(s.Root) == "" {
		return "", errors.New("Classic NDEF backup directory is unavailable")
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	if result.Dump.Layout != mifare.Classic1K && result.Dump.Layout != mifare.Classic4K {
		return "", errors.New("Classic NDEF backup requires a MIFARE Classic dump")
	}
	if len(result.Dump.Blocks) == 0 {
		return "", errors.New("Classic NDEF backup dump is empty")
	}
	if err := os.MkdirAll(s.Root, 0o700); err != nil {
		return "", err
	}
	if err := os.Chmod(s.Root, 0o700); err != nil {
		return "", err
	}
	data, err := json.MarshalIndent(MetadataFromDump(result), "", "  ")
	if err != nil {
		return "", err
	}
	data = append(data, '\n')
	temporary, err := os.CreateTemp(s.Root, ".classic-ndef-backup-*.tmp")
	if err != nil {
		return "", err
	}
	temporaryPath := temporary.Name()
	keep := false
	defer func() {
		_ = temporary.Close()
		if !keep {
			_ = os.Remove(temporaryPath)
		}
	}()
	if err := temporary.Chmod(0o600); err != nil {
		return "", err
	}
	if _, err := temporary.Write(data); err != nil {
		return "", err
	}
	if err := temporary.Sync(); err != nil {
		return "", err
	}
	if err := temporary.Close(); err != nil {
		return "", err
	}
	uid := strings.ToUpper(hex.EncodeToString(result.Card.UID))
	if uid == "" {
		uid = "unknown"
	}
	unique := strings.TrimSuffix(strings.TrimPrefix(filepath.Base(temporaryPath), ".classic-ndef-backup-"), ".tmp")
	name := fmt.Sprintf("classic-ndef-%s-%s.nfcx.json", safeBackupName(uid), unique)
	path := filepath.Join(s.Root, name)
	if err := os.Rename(temporaryPath, path); err != nil {
		return "", err
	}
	keep = true
	return path, nil
}

var _ ClassicNDEFBackupStore = FileClassicNDEFBackupStore{}
