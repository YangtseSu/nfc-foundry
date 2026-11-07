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
)

// FileType4BackupStore writes previous NDEF messages with mode 0600.
type FileType4BackupStore struct {
	Root string
}

type type4BackupFile struct {
	Version int    `json:"version"`
	UID     string `json:"uid"`
	ATQA    string `json:"atqa"`
	SAK     byte   `json:"sak"`
	CC      string `json:"cc"`
	MaxNDEF int    `json:"maxNdef"`
	Read    byte   `json:"readAccess"`
	Write   byte   `json:"writeAccess"`
	Message string `json:"message"`
	FileID  string `json:"fileId"`
}

func (s FileType4BackupStore) Save(ctx context.Context, backup Type4Backup) (string, error) {
	if ctx == nil || strings.TrimSpace(s.Root) == "" {
		return "", errors.New("Type 4 backup directory is unavailable")
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	if err := os.MkdirAll(s.Root, 0o700); err != nil {
		return "", err
	}
	if err := os.Chmod(s.Root, 0o700); err != nil {
		return "", err
	}
	document := type4BackupFile{
		Version: 1,
		UID:     fmt.Sprintf("%X", backup.Card.UID),
		ATQA:    fmt.Sprintf("%X", backup.Card.ATQA),
		SAK:     backup.Card.SAK,
		CC:      fmt.Sprintf("%X", backup.CC.Raw),
		MaxNDEF: backup.CC.MaxMessage,
		Read:    backup.CC.ReadAccess,
		Write:   backup.CC.WriteAccess,
		Message: strings.ToUpper(hex.EncodeToString(backup.Message)),
		FileID:  fmt.Sprintf("%04X", backup.CC.FileID),
	}
	data, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		return "", err
	}
	data = append(data, '\n')
	temporary, err := os.CreateTemp(s.Root, ".type4-backup-*.tmp")
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
	uid := document.UID
	if uid == "" {
		uid = "unknown"
	}
	unique := strings.TrimSuffix(strings.TrimPrefix(filepath.Base(temporaryPath), ".type4-backup-"), ".tmp")
	name := fmt.Sprintf("type4-%s-%s.json", safeBackupName(uid), unique)
	path := filepath.Join(s.Root, name)
	if err := os.Rename(temporaryPath, path); err != nil {
		return "", err
	}
	keep = true
	return path, nil
}

var _ Type4BackupStore = FileType4BackupStore{}
