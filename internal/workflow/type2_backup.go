package workflow

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

// Type2BackupStore persists a full Type 2 image before any write.
type Type2BackupStore interface {
	Save(context.Context, type2.Document) (string, error)
}

// FileType2BackupStore writes JSON backups with restrictive permissions.
type FileType2BackupStore struct {
	Root string
}

// Save writes one immutable backup and returns its path.
func (s FileType2BackupStore) Save(ctx context.Context, document type2.Document) (string, error) {
	if ctx == nil || strings.TrimSpace(s.Root) == "" {
		return "", errors.New("Type 2 backup directory is unavailable")
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	if err := document.Validate(); err != nil {
		return "", fmt.Errorf("backup document: %w", err)
	}
	if err := os.MkdirAll(s.Root, 0o700); err != nil {
		return "", err
	}
	if err := os.Chmod(s.Root, 0o700); err != nil {
		return "", err
	}
	data, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		return "", err
	}
	data = append(data, '\n')
	temporary, err := os.CreateTemp(s.Root, ".type2-backup-*.tmp")
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
	unique := strings.TrimSuffix(strings.TrimPrefix(filepath.Base(temporaryPath), ".type2-backup-"), ".tmp")
	name := fmt.Sprintf("type2-%s-%s.json", safeBackupName(document.UID), unique)
	path := filepath.Join(s.Root, name)
	if err := os.Rename(temporaryPath, path); err != nil {
		return "", err
	}
	keep = true
	return path, nil
}

var _ Type2BackupStore = FileType2BackupStore{}
