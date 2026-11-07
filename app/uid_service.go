package app

import (
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"strings"
	"time"
	"unicode"

	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

type uidPreflightRecord struct {
	request     workflow.UIDWriteRequest
	plan        workflow.UIDWritePlan
	keys        []workflow.SectorKeys
	fingerprint string
	backupPath  string
	expires     time.Time
}

func (s *Service) PreflightUIDWrite(value string) (UIDWritePreflightDTO, error) {
	if err := s.requireClassicWrite("UID write"); err != nil {
		return UIDWritePreflightDTO{}, err
	}
	s.mu.Lock()
	operationRunning := len(s.operations) != 0
	s.mu.Unlock()
	if operationRunning {
		return UIDWritePreflightDTO{}, ErrOperationRunning
	}
	newUID, err := parseFourByteUID(value)
	if err != nil {
		return UIDWritePreflightDTO{Error: err.Error()}, nil
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return UIDWritePreflightDTO{}, ErrCardRequired
	}
	card := snapshot.Card.Clone()
	if nfc.InferCardType(card) != nfc.CardTypeMIFAREClassic1K {
		return UIDWritePreflightDTO{Error: "only 4-byte UID MIFARE Classic 1K special cards are supported"}, nil
	}
	sectorKeys := verifiedSectorKeys(s.keyStore, card, mifare.Classic1K)
	request := workflow.UIDWriteRequest{Card: card, Device: snapshot.Device, NewUID: newUID, Sector0Keys: sectorKeys[0]}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	plan, err := s.uidService.Preflight(ctx, request)
	if err != nil {
		return UIDWritePreflightDTO{Error: err.Error()}, nil
	}
	expected := plan.OldBlock0
	request.ExpectedBlock0 = &expected
	fingerprint := uidWriteFingerprint(request, plan)
	token := fmt.Sprintf("uid-%d-%x", s.nextID.Add(1), fingerprint[:8])
	if strings.TrimSpace(s.classicBackupRoot) == "" {
		return UIDWritePreflightDTO{Error: "a pre-write backup directory is unavailable; no write is allowed"}, nil
	}
	backupPath := classicBackupPathForCard(s.classicBackupRoot, card, token)
	record := uidPreflightRecord{
		request: request, plan: plan, keys: sectorKeys, fingerprint: fingerprint,
		backupPath: backupPath, expires: time.Now().Add(5 * time.Minute),
	}
	s.mu.Lock()
	if s.uidPreflights == nil {
		s.uidPreflights = make(map[string]uidPreflightRecord)
	}
	s.uidPreflights[token] = record
	s.mu.Unlock()
	preview := uidWritePreviewDTO(plan)
	return UIDWritePreflightDTO{
		Token: token, Valid: true, Preview: &preview, BackupPath: backupPath,
		Warning: "supports DirectWrite magic cards (CUID/Gen2, UFUID, FUID) and Gen1A cards on readers with raw-frame support; locked cards and ordinary cards with immutable UIDs fail safely.",
	}, nil
}

func (s *Service) StartUIDWrite(input UIDWriteStartRequestDTO) (TaskDTO, error) {
	if err := s.requireClassicWrite("UID write"); err != nil {
		return TaskDTO{}, err
	}
	s.mu.Lock()
	record, exists := s.uidPreflights[input.Token]
	if exists {
		delete(s.uidPreflights, input.Token)
	}
	s.mu.Unlock()
	if !exists || time.Now().After(record.expires) {
		return TaskDTO{}, ErrPreflightRequired
	}
	confirmation, err := parseFourByteUID(input.Confirmation)
	if err != nil || !bytes.Equal(confirmation, record.request.NewUID) {
		return TaskDTO{}, ErrUIDConfirm
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil || !nfc.SameCard(record.request.Card, *snapshot.Card) {
		return TaskDTO{}, ErrPreflightRequired
	}
	if uidWriteFingerprint(record.request, record.plan) != record.fingerprint {
		return TaskDTO{}, ErrPreflightRequired
	}
	return s.startOperation("uid_write", "Write UID", func(ctx context.Context, taskID string) error {
		if record.backupPath != "" {
			if err := s.savePreWriteBackup(ctx, record.request.Card, record.keys, record.backupPath, taskID, "uid_write"); err != nil {
				return err
			}
		}
		request := record.request
		request.TaskID = taskID
		result, writeErr := s.uidService.Write(ctx, request, func(event workflow.UIDWriteEvent) {
			s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "uid_write", Type: TaskEventProgress, Phase: string(event.Phase), Message: event.Message})
		})
		dto := uidWriteResultDTO(result)
		s.emitOperation(TaskEventDetailDTO{
			TaskID: taskID, Kind: "uid_write", Type: TaskEventProgress, Phase: string(workflow.UIDWriteVerify),
			Message: uidWriteResultMessage(result, writeErr), UID: &dto, BackupPath: record.backupPath,
		})
		if writeErr == nil {
			s.registerUIDKey(result)
		}
		return annotateBackup(writeErr, record.backupPath)
	})
}

func parseFourByteUID(value string) ([]byte, error) {
	compact := strings.Map(func(char rune) rune {
		if unicode.IsSpace(char) || char == ':' || char == '-' {
			return -1
		}
		return char
	}, value)
	if len(compact) != 8 {
		return nil, errorsNewUID()
	}
	decoded, err := hex.DecodeString(compact)
	if err != nil || len(decoded) != 4 {
		return nil, errorsNewUID()
	}
	return decoded, nil
}

func errorsNewUID() error {
	return nfc.NewError("parse UID", nfc.CodeInvalidArgument, "UID must be 8 hexadecimal digits (4 bytes)", nil)
}

func uidWriteFingerprint(request workflow.UIDWriteRequest, plan workflow.UIDWritePlan) string {
	return fmt.Sprintf("%s|%X|%X|%X", keys.CardID(request.Card), request.NewUID, plan.OldBlock0, plan.NewBlock0)
}

func uidWritePreviewDTO(plan workflow.UIDWritePlan) UIDWritePreviewDTO {
	return UIDWritePreviewDTO{
		CurrentUID: formatHex(plan.Card.UID), NewUID: formatHex(plan.NewBlock0[0:4]), BCC: fmt.Sprintf("%02X", plan.BCC),
		OldBlock0: formatHex(plan.OldBlock0[:]), NewBlock0: formatHex(plan.NewBlock0[:]),
	}
}

func uidWriteResultDTO(result workflow.UIDWriteResult) UIDWriteResultDTO {
	dto := UIDWriteResultDTO{
		OldUID: formatHex(result.OldCard.UID), OldBlock0: formatHex(result.OldBlock0[:]),
		NewBlock0: formatHex(result.NewBlock0[:]), BackupPath: result.BackupPath, Verified: result.Verified,
	}
	if len(result.NewCard.UID) != 0 {
		dto.NewUID = formatHex(result.NewCard.UID)
	}
	if result.ActualRead {
		dto.ActualBlock0 = formatHex(result.ActualBlock0[:])
	}
	return dto
}

func uidWriteResultMessage(result workflow.UIDWriteResult, err error) string {
	if err == nil {
		return fmt.Sprintf("UID changed to %s; original block 0 backed up to %s", formatHex(result.NewCard.UID), result.BackupPath)
	}
	if result.BackupPath != "" {
		return fmt.Sprintf("UID write or verification failed; original block 0 backed up to %s", result.BackupPath)
	}
	return "UID preflight failed; the card was not modified"
}

func (s *Service) registerUIDKey(result workflow.UIDWriteResult) {
	if !result.Verified || s.keyStore == nil {
		return
	}
	oldCardID, newCardID := keys.CardID(result.OldCard), keys.CardID(result.NewCard)
	for _, verification := range s.keyStore.Verified(oldCardID) {
		record, exists := s.keyStore.Record(verification.KeyID)
		if !exists {
			continue
		}
		_, _ = s.keyStore.MergeVerified(newCardID, verification.Sector, verification.KeyType, record.Value, keys.SourceReadContext)
	}
	_, _ = s.keyStore.MergeVerified(newCardID, 0, result.KeyType, result.Key, keys.SourceReadContext)
	_ = s.persistKeys()
	s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &result.NewCard))
}
