package app

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

var ErrRecoveryReviewPending = errors.New("a recovered-key review is pending; save or discard it first")

func keyValidationError(err error) error {
	if err == nil {
		return nil
	}
	return nfc.NewError("key store", nfc.CodeInvalidArgument, err.Error(), err)
}

func recoveryPolicy(value string) (string, error) {
	switch value {
	case "", RecoverySavePersist:
		return RecoverySavePersist, nil
	case RecoverySaveReview:
		return RecoverySaveReview, nil
	case RecoverySaveNone:
		return RecoverySaveNone, nil
	default:
		return "", fmt.Errorf("unknown recovery save policy %q", value)
	}
}

func validateRecoverySaveOptions(keyName, keySetName string) error {
	if keyName != "" {
		if err := keys.ValidateKeyName(keyName); err != nil {
			return err
		}
	}
	if keySetName != "" {
		if err := keys.ValidateKeySetName(keySetName); err != nil {
			return err
		}
	}
	return nil
}

func (s *Service) keyReviewPending() bool {
	s.reviewMu.Lock()
	defer s.reviewMu.Unlock()
	return s.recoveryReview != nil
}

func (s *Service) requireKeyMutationAllowed() error {
	if s.keyReviewPending() {
		return ErrRecoveryReviewPending
	}
	s.mu.Lock()
	running := len(s.operations) != 0
	s.mu.Unlock()
	if running {
		return ErrOperationRunning
	}
	return nil
}

func (s *Service) beginRecovery(card nfc.CardInfo) (workflow.RecoveryBaseline, error) {
	s.reviewMu.Lock()
	defer s.reviewMu.Unlock()
	if s.recoveryReview != nil {
		return workflow.RecoveryBaseline{}, ErrRecoveryReviewPending
	}
	return workflow.CaptureRecoveryBaseline(s.keyStore, card), nil
}

func (s *Service) finishRecovery(card nfc.CardInfo, baseline workflow.RecoveryBaseline, policy, keyName, keySetName string, runErr error) error {
	refs := baseline.Recovered(s.keyStore, card)
	var operationErr error
	switch policy {
	case RecoverySavePersist:
		if keyName != "" && len(refs) == 1 {
			if _, err := s.keyStore.RenameKey(refs[0].KeyID, keyName); err != nil {
				operationErr = err
			}
		}
		if operationErr == nil && keySetName != "" && len(refs) != 0 {
			_, operationErr = s.upsertRecoveryKeySet(card, keySetName, refs, true)
		}
		if operationErr == nil {
			operationErr = s.persistKeys()
		}
	case RecoverySaveReview:
		if len(refs) != 0 {
			s.setRecoveryReview(card, refs)
		}
		// The catalog is needed to show values in the review, but the sector
		// matrix must not claim these keys are saved before the reload.
		s.emitter.Emit(KeyCatalogEventName, keyDTOs(s.keyStore))
		s.emitter.Emit(KeySetEventName, keySetDTOs(s.keyStore))
		if runErr != nil {
			return runErr
		}
		return operationErr
	case RecoverySaveNone:
	}
	s.emitKeyState(&card)
	if runErr != nil {
		return runErr
	}
	return operationErr
}

func (s *Service) upsertRecoveryKeySet(card nfc.CardInfo, name string, refs []workflow.RecoveredKeyRef, add bool) (keys.KeySet, error) {
	cardID := keys.CardID(card)
	set, err := s.keyStore.CreateKeySet(name, cardID, keys.KeySetSourceRecovery)
	if err != nil {
		return keys.KeySet{}, err
	}
	members := make([]string, 0, len(refs))
	for _, ref := range refs {
		members = append(members, ref.KeyID)
	}
	if len(members) == 0 {
		return set, nil
	}
	if add {
		return s.keyStore.AddKeySetKeys(set.ID, members)
	}
	return s.keyStore.SetKeySetKeys(set.ID, members)
}

func defaultRecoveryKeySetName(card nfc.CardInfo) string {
	return fmt.Sprintf("Recovered %X %s", card.UID, time.Now().UTC().Format("2006-01-02"))
}

func (s *Service) setRecoveryReview(card nfc.CardInfo, refs []workflow.RecoveredKeyRef) {
	review := &RecoveryReviewDTO{
		Card:              cardDTO(card),
		CardID:            keys.CardID(card),
		Recovered:         recoveredKeyRefDTOs(refs),
		DefaultKeySetName: defaultRecoveryKeySetName(card),
	}
	s.reviewMu.Lock()
	s.recoveryReview = review
	s.reviewMu.Unlock()
	s.emitter.Emit(RecoveryReviewEventName, s.cloneRecoveryReview(review))
}

func (s *Service) cloneRecoveryReview(review *RecoveryReviewDTO) *RecoveryReviewDTO {
	if review == nil {
		return nil
	}
	clone := *review
	clone.Recovered = append([]RecoveredKeyRefDTO(nil), review.Recovered...)
	return &clone
}

func (s *Service) PendingRecoveryReview() *RecoveryReviewDTO {
	s.reviewMu.Lock()
	defer s.reviewMu.Unlock()
	return s.cloneRecoveryReview(s.recoveryReview)
}

func (s *Service) clearRecoveryReview() {
	s.reviewMu.Lock()
	s.recoveryReview = nil
	s.reviewMu.Unlock()
}

// DiscardRecoveryReview removes only keys introduced by the pending recovery
// task and leaves pre-existing records and verifications untouched.
func (s *Service) DiscardRecoveryReview() error {
	s.mu.Lock()
	running := len(s.operations) != 0
	s.mu.Unlock()
	if running {
		return ErrOperationRunning
	}
	var card *nfc.CardInfo
	if snapshot := s.deviceManager.Snapshot(); snapshot.Card != nil {
		cloned := snapshot.Card.Clone()
		card = &cloned
	}
	s.abandonRecoveryReview(card)
	return nil
}

func (s *Service) abandonRecoveryReview(card *nfc.CardInfo) {
	s.reviewMu.Lock()
	review := s.recoveryReview
	s.recoveryReview = nil
	s.reviewMu.Unlock()
	if review == nil {
		return
	}
	for _, ref := range review.Recovered {
		if ref.Status != string(workflow.RecoveredKeyNew) {
			continue
		}
		_ = s.keyStore.Delete(ref.KeyID)
	}
	s.emitKeyState(card)
}

func (s *Service) handleCardChanged(event device.CardEvent) {
	s.mu.Lock()
	running := len(s.operations) != 0
	s.mu.Unlock()
	if running {
		// The running task owns the card transition and reports failure. Keep
		// the review so the user can re-present the card and retry the save.
		return
	}
	s.reviewMu.Lock()
	review := s.recoveryReview
	s.reviewMu.Unlock()
	if review == nil {
		return
	}
	if event.Type == device.CardRemoved || keys.CardID(event.Card) != review.CardID {
		var card *nfc.CardInfo
		if event.Type != device.CardRemoved {
			cloned := event.Card.Clone()
			card = &cloned
		}
		s.abandonRecoveryReview(card)
	}
}

func (s *Service) SaveRecoveryReview(input RecoverySaveRequestDTO) (TaskDTO, error) {
	review := s.PendingRecoveryReview()
	if review == nil {
		return TaskDTO{}, errors.New("no recovered-key review is pending")
	}
	keySetName := strings.TrimSpace(input.KeySetName)
	if keySetName == "" {
		keySetName = review.DefaultKeySetName
	}
	if err := keys.ValidateKeySetName(keySetName); err != nil {
		return TaskDTO{}, err
	}
	for _, name := range input.KeyNames {
		if err := keys.ValidateKeyName(name); err != nil {
			return TaskDTO{}, err
		}
	}
	available := make(map[string]struct{}, len(review.Recovered))
	for _, ref := range review.Recovered {
		available[ref.KeyID] = struct{}{}
	}
	selected := make(map[string]struct{}, len(input.SelectedKeyIDs))
	for _, id := range input.SelectedKeyIDs {
		if _, ok := available[id]; !ok {
			return TaskDTO{}, fmt.Errorf("key %q is not part of this recovery review", id)
		}
		selected[id] = struct{}{}
	}
	for id := range input.KeyNames {
		if _, ok := selected[id]; !ok {
			return TaskDTO{}, fmt.Errorf("key %q was not selected", id)
		}
	}
	refs := make([]RecoveredKeyRefDTO, 0, len(review.Recovered))
	for _, ref := range review.Recovered {
		if _, ok := selected[ref.KeyID]; ok {
			refs = append(refs, ref)
		}
	}
	if len(refs) == 0 {
		return TaskDTO{}, errors.New("select at least one recovered key to save")
	}
	if s.keyStorePath == "" {
		return TaskDTO{}, errors.New("the key store path is not configured")
	}
	if s.keyVerify == nil {
		return TaskDTO{}, errors.New("key verification is not configured")
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Status != device.StatusReady && snapshot.Status != device.StatusPolling {
		return TaskDTO{}, ErrCardRequired
	}
	if snapshot.Card == nil || keys.CardID(*snapshot.Card) != review.CardID {
		return TaskDTO{}, nfc.NewError("save recovered keys", nfc.CodeCardChanged, "the reviewed card is no longer present", nil)
	}
	card := snapshot.Card.Clone()
	return s.startOperation("recovery_save", "Save and verify recovered keys", func(ctx context.Context, taskID string) error {
		keep := make(map[string]struct{}, len(refs))
		for _, ref := range refs {
			keep[ref.KeyID] = struct{}{}
		}
		for _, ref := range review.Recovered {
			if ref.Status == string(workflow.RecoveredKeyNew) {
				if _, ok := keep[ref.KeyID]; !ok {
					_ = s.keyStore.Delete(ref.KeyID)
				}
			}
		}
		for id, name := range input.KeyNames {
			if _, err := s.keyStore.RenameKey(id, name); err != nil {
				return err
			}
		}
		set, err := s.keyStore.CreateKeySet(keySetName, review.CardID, keys.KeySetSourceRecovery)
		if err != nil {
			return err
		}
		members := make([]string, 0, len(refs))
		for _, ref := range refs {
			members = append(members, ref.KeyID)
		}
		if set, err = s.keyStore.SetKeySetKeys(set.ID, members); err != nil {
			return err
		}
		if err := s.persistKeys(); err != nil {
			return err
		}
		if _, err := s.keyStore.Reload(s.keyStorePath); err != nil {
			return fmt.Errorf("keys were saved but the store could not be reloaded: %w", err)
		}
		s.clearRecoveryReview()
		result := RecoverySaveResultDTO{
			KeySetID: set.ID, KeySetName: set.Name, SavedKeys: len(set.KeyIDs),
			Reloaded: true, StorePath: s.keyStorePath,
		}
		checks := make([]workflow.KeyCheck, 0, len(refs))
		for _, ref := range refs {
			record, exists := s.keyStore.Record(ref.KeyID)
			if !exists {
				continue
			}
			keyType, keyErr := keyTypeFromName(ref.KeyType)
			if keyErr != nil {
				return keyErr
			}
			checks = append(checks, workflow.KeyCheck{
				Sector: ref.Sector, KeyType: keyType, KeyID: ref.KeyID, Key: record.Value,
			})
		}
		if len(checks) != len(refs) {
			return fmt.Errorf("keys were saved but %d selected records are missing after reload", len(refs)-len(checks))
		}
		result.TotalSlots = len(checks)
		verifyResult, verifyErr := s.keyVerify.Verify(ctx, card, checks)
		if verifyErr != nil {
			return fmt.Errorf("keys were saved and reloaded but verification failed: %w", verifyErr)
		}
		result.VerifiedSlots = verifyResult.Verified
		for _, check := range verifyResult.Checks {
			if check.Verified {
				continue
			}
			result.Rejected = append(result.Rejected, RecoveryRejectedKeyDTO{
				KeyID: check.KeyID, Sector: check.Sector, KeyType: keyTypeName(check.KeyType), Error: check.Error,
			})
			s.keyStore.Unverify(review.CardID, check.Sector, check.KeyType)
		}
		if len(result.Rejected) != 0 {
			if err := s.persistKeys(); err != nil {
				return fmt.Errorf("verification removed rejected keys but the store could not be updated: %w", err)
			}
		}
		s.emitKeyState(&card)
		s.emitOperation(TaskEventDetailDTO{
			TaskID: taskID, Kind: "recovery_save", Type: TaskEventProgress, Progress: 100,
			Message: "recovered keys saved, reloaded, and verified", RecoverySave: &result,
		})
		return nil
	})
}

func keyTypeFromName(value string) (nfc.KeyType, error) {
	switch strings.ToUpper(strings.TrimSpace(value)) {
	case "A":
		return nfc.KeyTypeA, nil
	case "B":
		return nfc.KeyTypeB, nil
	default:
		return 0, fmt.Errorf("unknown key type %q", value)
	}
}

func (s *Service) ReloadKeyStore() (KeyStoreReloadDTO, error) {
	s.reviewMu.Lock()
	pending := s.recoveryReview != nil
	s.reviewMu.Unlock()
	if pending {
		return KeyStoreReloadDTO{}, ErrRecoveryReviewPending
	}
	s.mu.Lock()
	running := len(s.operations) != 0
	s.mu.Unlock()
	if running {
		return KeyStoreReloadDTO{}, ErrOperationRunning
	}
	if s.keyStorePath == "" {
		return KeyStoreReloadDTO{}, errors.New("the key store path is not configured")
	}
	report, err := s.keyStore.Reload(s.keyStorePath)
	s.mu.Lock()
	s.keyStoreLoadErr = err
	s.mu.Unlock()
	if err != nil {
		return KeyStoreReloadDTO{}, err
	}
	var card *nfc.CardInfo
	if snapshot := s.deviceManager.Snapshot(); snapshot.Card != nil {
		cloned := snapshot.Card.Clone()
		card = &cloned
	}
	s.emitKeyState(card)
	return KeyStoreReloadDTO{
		Path: s.keyStorePath, Keys: len(s.keyStore.Entries()),
		Verifications: len(s.keyStore.Verifications()), KeySets: len(s.keyStore.KeySets()),
		PrunedKeyIDs: report.PrunedKeyIDs, DroppedKeySets: report.DroppedKeySets,
	}, nil
}

func (s *Service) RenameKey(id, name string) ([]KeyDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return nil, err
	}
	if _, err := s.keyStore.RenameKey(id, name); err != nil {
		return nil, keyValidationError(err)
	}
	if err := s.persistKeys(); err != nil {
		return nil, err
	}
	result := keyDTOs(s.keyStore)
	s.emitKeyState(s.currentCardPtr())
	return result, nil
}

func (s *Service) KeySets() []KeySetDTO { return keySetDTOs(s.keyStore) }

func (s *Service) CreateKeySet(name, cardID string, keyIDs []string) ([]KeySetDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return nil, err
	}
	cardID = strings.TrimSpace(cardID)
	if len(keyIDs) == 0 {
		return nil, errors.New("at least one key is required")
	}
	for _, id := range keyIDs {
		if _, ok := s.keyStore.Record(id); !ok {
			return nil, keyValidationError(fmt.Errorf("key %q was not found", id))
		}
	}
	set, err := s.keyStore.CreateKeySet(name, cardID, keys.KeySetSourceManual)
	if err != nil {
		return nil, keyValidationError(err)
	}
	if _, err := s.keyStore.AddKeySetKeys(set.ID, keyIDs); err != nil {
		return nil, keyValidationError(err)
	}
	if err := s.persistKeys(); err != nil {
		return nil, err
	}
	s.emitKeyState(s.currentCardPtr())
	return keySetDTOs(s.keyStore), nil
}

// CreateVerifiedKeySet saves every verified key of the current card as a
// named set. The GUI uses it because it has no raw key-ID entry surface.
func (s *Service) CreateVerifiedKeySet(name string) ([]KeySetDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return nil, err
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return nil, ErrCardRequired
	}
	card := snapshot.Card.Clone()
	matches := s.keyStore.Verified(keys.CardID(card))
	if len(matches) == 0 {
		return nil, keyValidationError(errors.New("no verified key is available for this card"))
	}
	ids := make([]string, 0, len(matches))
	for _, match := range matches {
		ids = append(ids, match.KeyID)
	}
	return s.CreateKeySet(name, keys.CardID(card), ids)
}

func (s *Service) RenameKeySet(id, name string) ([]KeySetDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return nil, err
	}
	if _, err := s.keyStore.RenameKeySet(id, name); err != nil {
		return nil, keyValidationError(err)
	}
	if err := s.persistKeys(); err != nil {
		return nil, err
	}
	s.emitKeyState(s.currentCardPtr())
	return keySetDTOs(s.keyStore), nil
}

func (s *Service) DeleteKeySet(id string) ([]KeySetDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return nil, err
	}
	if err := s.keyStore.DeleteKeySet(id); err != nil {
		return nil, keyValidationError(err)
	}
	if err := s.persistKeys(); err != nil {
		return nil, err
	}
	s.emitKeyState(s.currentCardPtr())
	return keySetDTOs(s.keyStore), nil
}

func (s *Service) ResolveKeySet(nameOrID string) (keys.KeySet, error) {
	nameOrID = strings.TrimSpace(nameOrID)
	if nameOrID == "" {
		return keys.KeySet{}, errors.New("key set name or ID is required")
	}
	if set, ok := s.keyStore.KeySet(nameOrID); ok {
		return set, nil
	}
	matches := make([]keys.KeySet, 0)
	for _, set := range s.keyStore.KeySets() {
		if set.Name == nameOrID {
			matches = append(matches, set)
		}
	}
	switch len(matches) {
	case 0:
		return keys.KeySet{}, fmt.Errorf("key set %q was not found", nameOrID)
	case 1:
		return matches[0], nil
	default:
		return keys.KeySet{}, fmt.Errorf("multiple key sets are named %q; use the set ID", nameOrID)
	}
}

type exportedKeySet struct {
	Format  string        `json:"format"`
	Version int           `json:"version"`
	Name    string        `json:"name"`
	CardID  string        `json:"cardId,omitempty"`
	Keys    []exportedKey `json:"keys"`
}

type exportedKey struct {
	Value string `json:"value"`
	Name  string `json:"name,omitempty"`
}

// KeySetArtifact renders a key set as a dictionary or a JSON exchange object.
func (s *Service) KeySetArtifact(nameOrID, format string) (string, keys.KeySet, int, error) {
	s.mu.Lock()
	running := len(s.operations) != 0
	s.mu.Unlock()
	if running {
		return "", keys.KeySet{}, 0, ErrOperationRunning
	}
	set, err := s.ResolveKeySet(nameOrID)
	if err != nil {
		return "", keys.KeySet{}, 0, keyValidationError(err)
	}
	records := make([]keys.Record, 0, len(set.KeyIDs))
	for _, id := range set.KeyIDs {
		record, ok := s.keyStore.Record(id)
		if !ok {
			continue
		}
		records = append(records, record)
	}
	switch strings.ToLower(strings.TrimSpace(format)) {
	case "", "dic":
		var builder strings.Builder
		for _, record := range records {
			builder.WriteString(record.Hex())
			builder.WriteByte('\n')
		}
		return builder.String(), set, len(records), nil
	case "keys":
		artifact := exportedKeySet{Format: "nfc-foundry-key-set", Version: 1, Name: set.Name, CardID: set.CardID}
		for _, record := range records {
			artifact.Keys = append(artifact.Keys, exportedKey{Value: record.Hex(), Name: record.Name})
		}
		encoded, marshalErr := json.MarshalIndent(artifact, "", "  ")
		if marshalErr != nil {
			return "", set, 0, marshalErr
		}
		return string(append(encoded, '\n')), set, len(records), nil
	default:
		return "", set, 0, keyValidationError(fmt.Errorf("unknown key set export format %q; use dic or keys", format))
	}
}

func (s *Service) ExportKeySetFile(nameOrID, format, path string) (ExportResultDTO, error) {
	content, _, count, err := s.KeySetArtifact(nameOrID, format)
	if err != nil {
		return ExportResultDTO{}, err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o600)
	if err != nil {
		return ExportResultDTO{}, err
	}
	if err := file.Chmod(0o600); err != nil {
		file.Close()
		return ExportResultDTO{}, err
	}
	if _, err := file.WriteString(content); err != nil {
		file.Close()
		return ExportResultDTO{}, err
	}
	if err := file.Close(); err != nil {
		return ExportResultDTO{}, err
	}
	return ExportResultDTO{Path: path, Count: count}, nil
}

func (s *Service) currentCardPtr() *nfc.CardInfo {
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return nil
	}
	card := snapshot.Card.Clone()
	return &card
}

func (s *Service) emitKeyState(card *nfc.CardInfo) {
	s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, card))
	s.emitter.Emit(KeyCatalogEventName, keyDTOs(s.keyStore))
	s.emitter.Emit(KeySetEventName, keySetDTOs(s.keyStore))
}

func recoveredKeyRefDTOs(refs []workflow.RecoveredKeyRef) []RecoveredKeyRefDTO {
	result := make([]RecoveredKeyRefDTO, len(refs))
	for index, ref := range refs {
		result[index] = RecoveredKeyRefDTO{
			KeyID: ref.KeyID, Sector: ref.Sector, KeyType: keyTypeName(ref.KeyType), Status: string(ref.Status),
		}
	}
	return result
}

func keySetDTOs(store *keys.Store) []KeySetDTO {
	if store == nil {
		return []KeySetDTO{}
	}
	sets := store.KeySets()
	result := make([]KeySetDTO, 0, len(sets))
	for _, set := range sets {
		result = append(result, KeySetDTO{
			ID: set.ID, Name: set.Name, CardID: set.CardID, KeyIDs: append([]string(nil), set.KeyIDs...),
			KeyCount: len(set.KeyIDs), CreatedAt: set.CreatedAt.UTC().Format(time.RFC3339), Source: string(set.Source),
		})
	}
	return result
}
