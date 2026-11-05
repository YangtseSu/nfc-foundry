package app

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/attack"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workbench"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

var (
	ErrCardRequired      = errors.New("a supported card must be present")
	ErrOperationRunning  = errors.New("another NFC operation is already running")
	ErrDirtyWorkbench    = errors.New("workbench has unsaved changes")
	ErrPreflightRequired = errors.New("a current successful preflight is required")
	ErrTrailerConfirm    = errors.New("sector trailer writes require explicit confirmation")
	ErrUIDConfirm        = errors.New("UID writes require the exact target UID confirmation")
)

type writePreflightRecord struct {
	request     workflow.RestoreRequest
	fingerprint string
	mode        string
	trailers    []int
	uidRequest  *workflow.UIDWriteRequest
	uidPlan     *workflow.UIDWritePlan
	expires     time.Time
}

func (s *Service) KeyCatalog() []KeyDTO { return keyDTOs(s.keyStore) }

func (s *Service) AddKey(value string) ([]KeyDTO, error) {
	if _, _, err := s.keyStore.AddHex(value, keys.SourceUserInput); err != nil {
		return nil, err
	}
	if err := s.persistKeys(); err != nil {
		return nil, err
	}
	result := keyDTOs(s.keyStore)
	s.emitter.Emit(KeyCatalogEventName, result)
	return result, nil
}

func (s *Service) DeleteKey(id string) ([]KeyDTO, error) {
	if err := s.keyStore.Delete(id); err != nil {
		return nil, err
	}
	if err := s.persistKeys(); err != nil {
		return nil, err
	}
	result := keyDTOs(s.keyStore)
	s.emitter.Emit(KeyCatalogEventName, result)
	return result, nil
}

func (s *Service) ImportKeyDictionary() (ImportReportDTO, error) {
	path, err := s.dialogs.OpenFile("Import MIFARE Classic key dictionary", "*.dic;*.txt;*.keys")
	if err != nil {
		return ImportReportDTO{}, err
	}
	if path == "" {
		return ImportReportDTO{Cancelled: true, Keys: keyDTOs(s.keyStore), Issues: []ImportIssueDTO{}}, nil
	}
	file, err := os.Open(path)
	if err != nil {
		return ImportReportDTO{}, err
	}
	report, added, importErr := s.keyStore.Import(file, keys.SourceFileImport)
	closeErr := file.Close()
	if importErr != nil {
		return ImportReportDTO{}, importErr
	}
	if closeErr != nil {
		return ImportReportDTO{}, closeErr
	}
	if err := s.persistKeys(); err != nil {
		return ImportReportDTO{}, err
	}
	dto := ImportReportDTO{
		Path: filepath.Base(path), Valid: report.Valid, Added: added, Duplicates: report.Duplicates,
		Ignored: report.Ignored, Issues: make([]ImportIssueDTO, len(report.Issues)), Keys: keyDTOs(s.keyStore),
	}
	for index, issue := range report.Issues {
		dto.Issues[index] = ImportIssueDTO{Line: issue.Line, Message: issue.Message}
	}
	s.emitter.Emit(KeyCatalogEventName, dto.Keys)
	return dto, nil
}

func (s *Service) ExportKeyDictionary() (ExportResultDTO, error) {
	path, err := s.dialogs.SaveFile("Export all known keys", "nfc-foundry-known-keys.dic", "*.dic;*.txt")
	if err != nil {
		return ExportResultDTO{}, err
	}
	if path == "" {
		return ExportResultDTO{Cancelled: true}, nil
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o600)
	if err != nil {
		return ExportResultDTO{}, err
	}
	if err := file.Chmod(0o600); err != nil {
		file.Close()
		return ExportResultDTO{}, err
	}
	count, err := s.keyStore.Export(file)
	if err != nil {
		file.Close()
		return ExportResultDTO{}, err
	}
	if err := file.Close(); err != nil {
		return ExportResultDTO{}, err
	}
	return ExportResultDTO{Path: filepath.Base(path), Count: count}, nil
}

func (s *Service) persistKeys() error {
	if s.keyStorePath == "" {
		return nil
	}
	return s.keyStore.Save(s.keyStorePath)
}

func (s *Service) Workbench() WorkbenchDTO { return workbenchDTO(s.bench.Snapshot()) }

func (s *Service) EditBlockHex(block int, value string) (WorkbenchDTO, error) {
	if err := s.bench.EditHex(block, value); err != nil {
		return WorkbenchDTO{}, err
	}
	return s.emitWorkbench(), nil
}

func (s *Service) EditBlockASCII(block int, value string) (WorkbenchDTO, error) {
	if err := s.bench.EditASCII(block, value); err != nil {
		return WorkbenchDTO{}, err
	}
	return s.emitWorkbench(), nil
}

func (s *Service) EditableBlock(blockNumber int) (EditableBlockDTO, error) {
	result, err := s.bench.Result()
	if err != nil {
		return EditableBlockDTO{}, err
	}
	if blockNumber < 0 || blockNumber >= len(result.Dump.Blocks) {
		return EditableBlockDTO{}, fmt.Errorf("block %d is outside the loaded dump", blockNumber)
	}
	block := result.Dump.Blocks[blockNumber]
	kind := workbench.KindForBlock(result.Dump.Layout, blockNumber)
	dto := EditableBlockDTO{
		Block: blockNumber, Sector: block.Sector, Kind: string(kind),
		Hex: strings.ToUpper(block.Hex()), ASCII: workbench.ASCII(block.Data),
	}
	if kind == workbench.BlockManufacturer {
		dto.Warning = "Manufacturer block: editable here, always skipped when writing the card"
	}
	if kind == workbench.BlockTrailer {
		dto.Warning = "Sector trailer: this view contains full keys; writing needs extra confirmation"
		access := accessDTO(workbench.ExplainAccess(block))
		dto.Access = &access
	}
	return dto, nil
}

func (s *Service) UndoWorkbench() (WorkbenchDTO, error) {
	if err := s.bench.Undo(); err != nil {
		return WorkbenchDTO{}, err
	}
	return s.emitWorkbench(), nil
}

func (s *Service) RevertWorkbench() (WorkbenchDTO, error) {
	if err := s.bench.Revert(); err != nil {
		return WorkbenchDTO{}, err
	}
	return s.emitWorkbench(), nil
}

func (s *Service) ClearWorkbench() WorkbenchDTO {
	s.bench.Clear()
	return s.emitWorkbench()
}

func (s *Service) emitWorkbench() WorkbenchDTO {
	dto := workbenchDTO(s.bench.Snapshot())
	s.emitter.Emit(WorkbenchEventName, dto)
	return dto
}

func (s *Service) LoadDump(discardDirty bool) (WorkbenchDTO, error) {
	if s.bench.Snapshot().Dirty && !discardDirty {
		return WorkbenchDTO{}, ErrDirtyWorkbench
	}
	path, err := s.dialogs.OpenFile("Load MIFARE Classic dump", "*.bin;*.mfd;*.json")
	if err != nil {
		return WorkbenchDTO{}, err
	}
	if path == "" {
		return s.Workbench(), nil
	}
	var result workflow.DumpResult
	if strings.HasSuffix(strings.ToLower(path), ".json") {
		result, err = workflow.LoadDumpProject(path)
	} else {
		result, err = workflow.LoadRawDump(path)
	}
	if err != nil {
		return WorkbenchDTO{}, err
	}
	if err := s.bench.Load(result, path); err != nil {
		return WorkbenchDTO{}, err
	}
	s.registerDumpKeys(result)
	if s.deviceManager != nil {
		if currentCard := s.deviceManager.Snapshot().Card; currentCard != nil {
			s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, currentCard))
		}
	}
	return s.emitWorkbench(), nil
}

func (s *Service) SaveDump() (SaveResultDTO, error) {
	result, err := s.bench.Result()
	if err != nil {
		return SaveResultDTO{}, err
	}
	complete := result.Complete()
	if complete {
		validation := s.bench.ValidateForSave()
		if !validation.Valid {
			return SaveResultDTO{}, errors.New(strings.Join(validation.Errors, "; "))
		}
	}
	defaultName, pattern := "card-dump.nfcx.json", "*.json"
	if complete {
		defaultName, pattern = "card-dump.bin", "*.bin;*.mfd"
	}
	path, err := s.dialogs.SaveFile("Save MIFARE Classic dump", defaultName, pattern)
	if err != nil {
		return SaveResultDTO{}, err
	}
	if path == "" {
		return SaveResultDTO{Cancelled: true}, nil
	}
	response := SaveResultDTO{Path: filepath.Base(path)}
	if complete {
		sidecar, err := workflow.SaveRawDump(path, result)
		if err != nil {
			return SaveResultDTO{}, err
		}
		response.Sidecar = filepath.Base(sidecar)
	} else if err := workflow.SaveDumpProject(path, result); err != nil {
		return SaveResultDTO{}, err
	}
	if err := s.bench.MarkSaved(path); err != nil {
		return SaveResultDTO{}, err
	}
	s.emitWorkbench()
	return response, nil
}

func (s *Service) StartKeyScan(discardDirty bool) (TaskDTO, error) {
	if s.bench.Snapshot().Dirty && !discardDirty {
		return TaskDTO{}, ErrDirtyWorkbench
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return TaskDTO{}, ErrCardRequired
	}
	card := snapshot.Card.Clone()
	return s.startPreparedOperation("keyscan", "Scan known keys", s.clearWorkbench, func(ctx context.Context, taskID string) error {
		result, err := s.keyScanner.Scan(ctx, workflow.KeyScanRequest{Card: card}, func(progress workflow.KeyScanProgress) {
			message := fmt.Sprintf("Sector %d · Key %s · tried %d · found %d", progress.Sector, keyTypeName(progress.KeyType), progress.Attempted, progress.Found)
			if progress.Matched {
				message = fmt.Sprintf("Sector %d Key %s verified", progress.Sector, keyTypeName(progress.KeyType))
			}
			s.emitOperation(TaskEventDetailDTO{
				TaskID: taskID, Kind: "keyscan", Type: TaskEventProgress, Sector: progress.Sector,
				KeyType: keyTypeName(progress.KeyType), Attempted: progress.Attempted, Found: progress.Found,
				Progress: scanProgress(card, progress.Sector, progress.KeyType), Message: message,
			})
		})
		s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
		if result.Cancelled {
			return context.Canceled
		}
		return err
	})
}

func scanProgress(card nfc.CardInfo, sector int, keyType nfc.KeyType) int {
	sectors := mifare.Classic1KSectors
	if nfc.InferCardType(card) == nfc.CardTypeMIFAREClassic4K {
		sectors = mifare.Classic4KSectors
	}
	slot := sector * 2
	if keyType == nfc.KeyTypeB {
		slot++
	}
	return slot * 100 / (sectors * 2)
}

func (s *Service) StartReadCard(discardDirty bool) (TaskDTO, error) {
	if s.bench.Snapshot().Dirty && !discardDirty {
		return TaskDTO{}, ErrDirtyWorkbench
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return TaskDTO{}, ErrCardRequired
	}
	card, deviceInfo := snapshot.Card.Clone(), snapshot.Device
	layout, err := appLayoutForCard(card)
	if err != nil {
		return TaskDTO{}, err
	}
	request := workflow.DumpRequest{Card: card, Device: deviceInfo, Keys: verifiedSectorKeys(s.keyStore, card, layout)}
	return s.startPreparedOperation("read", "Read card", s.clearWorkbench, func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "read", Type: TaskEventProgress, Message: "reading the current card"})
		result, readErr := s.dumpService.ReadCard(ctx, request)
		if len(result.Dump.Blocks) != 0 {
			s.registerDumpKeys(result)
			s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
			if loadErr := s.bench.Load(result, ""); loadErr != nil && readErr == nil {
				readErr = loadErr
			}
			dto := s.emitWorkbench()
			s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "read", Type: TaskEventProgress, Progress: 100, Message: "read result loaded into the workbench", Workbench: &dto})
		}
		return readErr
	})
}

func (s *Service) clearWorkbench() {
	s.bench.Clear()
	s.emitWorkbench()
}

func (s *Service) registerDumpKeys(result workflow.DumpResult) {
	cardID := keys.CardID(result.Card)
	for _, sector := range result.Keys {
		for _, candidate := range []struct {
			kind nfc.KeyType
			key  *nfc.Key
		}{{nfc.KeyTypeA, sector.KeyA}, {nfc.KeyTypeB, sector.KeyB}} {
			if candidate.key == nil {
				continue
			}
			record, _, _ := s.keyStore.Add(*candidate.key, keys.SourceReadContext)
			_ = s.keyStore.Verify(cardID, sector.Sector, candidate.kind, record.ID)
		}
	}
}

func appLayoutForCard(card nfc.CardInfo) (mifare.Layout, error) {
	switch nfc.InferCardType(card) {
	case nfc.CardTypeMIFAREClassic1K:
		return mifare.Classic1K, nil
	case nfc.CardTypeMIFAREClassic4K:
		return mifare.Classic4K, nil
	default:
		return 0, nfc.NewError("workbench", nfc.CodeUnsupported, "card is not MIFARE Classic 1K/4K", nil)
	}
}

func (s *Service) PreflightWrite(input WritePreflightRequestDTO) (WritePreflightDTO, error) {
	mode := input.Mode
	s.mu.Lock()
	operationRunning := len(s.operations) != 0
	s.mu.Unlock()
	if operationRunning {
		return WritePreflightDTO{}, ErrOperationRunning
	}
	snapshot := s.bench.Snapshot()
	if !snapshot.Loaded {
		return WritePreflightDTO{}, workbench.ErrNoDump
	}
	if !snapshot.Result.Complete() {
		return WritePreflightDTO{}, mifare.ErrIncompleteDump
	}
	deviceSnapshot := s.deviceManager.Snapshot()
	if deviceSnapshot.Card == nil {
		return WritePreflightDTO{}, ErrCardRequired
	}
	card := deviceSnapshot.Card.Clone()
	layout, err := appLayoutForCard(card)
	if err != nil {
		return WritePreflightDTO{}, err
	}
	var blocks []int
	switch mode {
	case "changes":
		if input.WriteUID {
			return WritePreflightDTO{}, nfc.NewError("restore preflight", nfc.CodeInvalidArgument, "UID can be written only together with a full dump restore", nil)
		}
		blocks = append([]int(nil), snapshot.DirtyBlocks...)
		writable := blocks[:0]
		for _, block := range blocks {
			if block != 0 {
				writable = append(writable, block)
			}
		}
		blocks = writable
		if len(blocks) == 0 {
			return WritePreflightDTO{Mode: mode, SkippedBlock0: true, Error: "there are no changed blocks to write"}, nil
		}
	case "full":
		blocks = nil
	default:
		return WritePreflightDTO{}, fmt.Errorf("unknown write mode %q", mode)
	}
	request := workflow.RestoreRequest{
		Card: card, Dump: snapshot.Result.Dump.Clone(), SourceUIDLength: len(snapshot.Result.Card.UID),
		TargetKeys: verifiedSectorKeys(s.keyStore, card, layout), Blocks: blocks,
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	plan, err := s.restoreService.Preflight(ctx, request)
	if err != nil {
		return WritePreflightDTO{Mode: mode, Error: err.Error(), Warnings: []string{"preflight failed; no block was written"}}, nil
	}
	planned := make([]int, 0, len(plan.Operations))
	trailers := make([]int, 0)
	access := make([]TrailerAccessDTO, 0)
	for _, operation := range plan.Operations {
		planned = append(planned, operation.Block)
		if operation.Trailer {
			trailers = append(trailers, operation.Block)
			block := snapshot.Result.Dump.Blocks[operation.Block]
			access = append(access, TrailerAccessDTO{Block: operation.Block, Sector: operation.Sector, Access: accessDTO(workbench.ExplainAccess(block))})
		}
	}
	var uidRequest *workflow.UIDWriteRequest
	var uidPlan *workflow.UIDWritePlan
	if input.WriteUID {
		newUID, parseErr := parseFourByteUID(input.UID)
		if parseErr != nil {
			return WritePreflightDTO{Mode: mode, Error: parseErr.Error(), Warnings: []string{"preflight failed; no block was written"}}, nil
		}
		if accessErr := validatePostRestoreUIDAccess(request.Dump); accessErr != nil {
			return WritePreflightDTO{Mode: mode, Error: accessErr.Error(), Warnings: []string{"UID preflight failed; no block was written"}}, nil
		}
		sectorKeys := verifiedSectorKeys(s.keyStore, card, layout)
		candidate := workflow.UIDWriteRequest{Card: card, Device: deviceSnapshot.Device, NewUID: newUID, Sector0Keys: sectorKeys[0]}
		preview, previewErr := s.uidService.Preflight(ctx, candidate)
		if previewErr != nil {
			return WritePreflightDTO{Mode: mode, Error: previewErr.Error(), Warnings: []string{"UID preflight failed; no block was written"}}, nil
		}
		expected := preview.OldBlock0
		candidate.ExpectedBlock0 = &expected
		uidRequest, uidPlan = &candidate, &preview
	}
	fingerprint, err := restoreFingerprint(request)
	if err != nil {
		return WritePreflightDTO{}, err
	}
	token := fmt.Sprintf("write-%d-%x", s.nextID.Add(1), sha256.Sum256([]byte(fingerprint+time.Now().UTC().String())))
	if uidPlan != nil {
		fingerprint += fmt.Sprintf("|uid:%X|old:%X", uidRequest.NewUID, uidPlan.OldBlock0)
	}
	record := writePreflightRecord{request: request, fingerprint: fingerprint, mode: mode, trailers: trailers, uidRequest: uidRequest, uidPlan: uidPlan, expires: time.Now().Add(5 * time.Minute)}
	s.mu.Lock()
	s.preflights[token] = record
	s.mu.Unlock()
	warnings := []string{"a normal dump restore skips block 0", "writing stops at the first unrecoverable failure"}
	if len(trailers) != 0 {
		warnings = append(warnings, "the plan contains sector trailers; key and access-condition changes must be confirmed")
	}
	var uidDTO *UIDWritePreviewDTO
	if uidPlan != nil {
		preview := uidWritePreviewDTO(*uidPlan)
		uidDTO = &preview
		warnings = append(warnings, "UID is written last, after the full dump succeeds; supports CUID/Gen2 and Gen1A special cards on PN532 UART")
	}
	return WritePreflightDTO{
		Token: token, Mode: mode, Valid: true, Blocks: planned, TrailerBlocks: trailers,
		SkippedBlock0: uidPlan == nil, Warnings: warnings, Access: access, UID: uidDTO,
	}, nil
}

// validatePostRestoreUIDAccess checks the access conditions that will be in
// force after the full restore replaces sector 0's trailer. Restore verifies
// replacement Key A unconditionally and Key B whenever it remains an
// authentication key, so one of those verified post-restore keys must be able
// to write data group 0 before the combined operation is offered.
func validatePostRestoreUIDAccess(dump mifare.Dump) error {
	if dump.Layout != mifare.Classic1K || len(dump.Blocks) <= 3 {
		return nfc.NewError("restore UID preflight", nfc.CodeUnsupported, "only 4-byte UID MIFARE Classic 1K special cards are supported", nil)
	}
	trailer := dump.Blocks[3].Data
	access, err := mifare.DecodeAccessBits([3]byte{trailer[6], trailer[7], trailer[8]})
	if err != nil {
		return nfc.NewError("restore UID preflight", nfc.CodeInvalidArgument, "the dump has invalid sector 0 access bits", err)
	}
	mask := access.Groups[0].DataWriteKeys()
	if mask.AllowsA() || mask.AllowsB() && !access.Groups[3].KeyBReadable() {
		return nil
	}
	return nfc.NewError("restore UID preflight", nfc.CodePermission, "the restored sector 0 access conditions do not allow a verified key to write block 0", nil)
}

func (s *Service) StartWrite(input WriteStartRequestDTO) (TaskDTO, error) {
	s.mu.Lock()
	record, exists := s.preflights[input.Token]
	if exists {
		delete(s.preflights, input.Token)
	}
	s.mu.Unlock()
	if !exists || time.Now().After(record.expires) {
		return TaskDTO{}, ErrPreflightRequired
	}
	if len(record.trailers) != 0 && !input.ConfirmTrailers {
		return TaskDTO{}, ErrTrailerConfirm
	}
	if record.uidRequest != nil {
		confirmation, err := parseFourByteUID(input.ConfirmUID)
		if err != nil || !bytes.Equal(confirmation, record.uidRequest.NewUID) {
			return TaskDTO{}, ErrUIDConfirm
		}
	}
	current, err := s.currentRestoreFingerprint(record.request)
	expectedFingerprint := record.fingerprint
	if record.uidPlan != nil {
		current += fmt.Sprintf("|uid:%X|old:%X", record.uidRequest.NewUID, record.uidPlan.OldBlock0)
	}
	if err != nil || current != expectedFingerprint {
		return TaskDTO{}, ErrPreflightRequired
	}
	return s.startOperation("write", "Write and verify", func(ctx context.Context, taskID string) error {
		result, restoreErr := s.restoreService.Restore(ctx, record.request, func(event workflow.RestoreEvent) {
			s.emitOperation(TaskEventDetailDTO{
				TaskID: taskID, Kind: "write", Type: TaskEventProgress, Phase: string(event.Phase),
				Block: event.Block, Message: restoreMessage(event),
			})
		})
		s.registerRestoredKeys(record.request, result)
		dto := restoreResultDTO(result)
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "write", Type: TaskEventProgress, Phase: string(result.Phase), Message: "write result generated", Restore: &dto})
		if restoreErr != nil || record.uidRequest == nil {
			return restoreErr
		}
		uidRequest := *record.uidRequest
		uidRequest.TaskID = taskID
		layout, _ := appLayoutForCard(uidRequest.Card)
		sectorKeys := verifiedSectorKeys(s.keyStore, uidRequest.Card, layout)
		uidRequest.Sector0Keys = sectorKeys[0]
		uidResult, uidErr := s.uidService.Write(ctx, uidRequest, func(event workflow.UIDWriteEvent) {
			s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "write", Type: TaskEventProgress, Phase: string(event.Phase), Message: event.Message})
		})
		uidDTO := uidWriteResultDTO(uidResult)
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "write", Type: TaskEventProgress, Phase: string(workflow.UIDWriteVerify), Message: uidWriteResultMessage(uidResult, uidErr), UID: &uidDTO})
		if uidErr == nil {
			s.registerUIDKey(uidResult)
		}
		return uidErr
	})
}

func (s *Service) registerRestoredKeys(request workflow.RestoreRequest, result workflow.RestoreResult) {
	cardID := keys.CardID(request.Card)
	changed := false
	for _, entry := range result.Blocks {
		if !entry.Trailer || !entry.Verified || entry.Block < 0 || entry.Block >= len(request.Dump.Blocks) {
			continue
		}
		block := request.Dump.Blocks[entry.Block]
		keyA := nfc.Key(block.Data[0:6])
		record, _, _ := s.keyStore.Add(keyA, keys.SourceReadContext)
		_ = s.keyStore.Verify(cardID, entry.Sector, nfc.KeyTypeA, record.ID)
		changed = true
		access, err := mifare.DecodeAccessBits([3]byte{block.Data[6], block.Data[7], block.Data[8]})
		if err == nil && !access.Groups[3].KeyBReadable() {
			keyB := nfc.Key(block.Data[10:16])
			record, _, _ = s.keyStore.Add(keyB, keys.SourceReadContext)
			_ = s.keyStore.Verify(cardID, entry.Sector, nfc.KeyTypeB, record.ID)
		}
	}
	if changed {
		s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &request.Card))
	}
}

func restoreMessage(event workflow.RestoreEvent) string {
	switch event.Phase {
	case workflow.RestorePreflight:
		return "reconfirming preflight conditions"
	case workflow.RestoreExecute:
		return fmt.Sprintf("writing block %d", event.Block)
	case workflow.RestoreVerify:
		return fmt.Sprintf("verifying block %d", event.Block)
	case workflow.RestoreComplete:
		return "write and verification complete"
	default:
		return event.Message
	}
}

func restoreFingerprint(request workflow.RestoreRequest) (string, error) {
	raw, err := request.Dump.Raw()
	if err != nil {
		return "", err
	}
	digest := sha256.New()
	_, _ = digest.Write(raw)
	_, _ = fmt.Fprintf(digest, "|%s|%v|%d", keys.CardID(request.Card), request.Blocks, request.SourceUIDLength)
	return fmt.Sprintf("%x", digest.Sum(nil)), nil
}

func (s *Service) currentRestoreFingerprint(original workflow.RestoreRequest) (string, error) {
	snapshot := s.bench.Snapshot()
	deviceSnapshot := s.deviceManager.Snapshot()
	if !snapshot.Loaded || deviceSnapshot.Card == nil || !nfc.SameCard(original.Card, *deviceSnapshot.Card) {
		return "", ErrPreflightRequired
	}
	current := original
	current.Dump = snapshot.Result.Dump.Clone()
	return restoreFingerprint(current)
}

func (s *Service) startOperation(kind, name string, run func(context.Context, string) error) (TaskDTO, error) {
	return s.startPreparedOperation(kind, name, nil, run)
}

// startPreparedOperation reserves the global operation slot, applies the
// synchronous UI state transition, and only then starts hardware work. This
// ordering prevents stale workbench data from surviving into a newly accepted
// scan or read operation.
func (s *Service) startPreparedOperation(kind, name string, prepare func(), run func(context.Context, string) error) (TaskDTO, error) {
	ctx, cancel := context.WithCancel(context.Background())
	id := fmt.Sprintf("%s-%04d", kind, s.nextID.Add(1))
	s.mu.Lock()
	if len(s.operations) != 0 {
		s.mu.Unlock()
		cancel()
		return TaskDTO{}, ErrOperationRunning
	}
	s.operations[id] = &operationTask{cancel: cancel, kind: kind}
	s.mu.Unlock()
	if prepare != nil {
		prepare()
	}
	task := TaskDTO{ID: id, Name: name, Status: "running"}
	go func() {
		s.emitOperation(TaskEventDetailDTO{TaskID: id, Kind: kind, Type: TaskEventStarted, Message: name + " started"})
		err := run(ctx, id)
		s.mu.Lock()
		operation := s.operations[id]
		cancelled := operation != nil && operation.cancelRequested
		delete(s.operations, id)
		s.mu.Unlock()
		cancel()
		if cancelled || errors.Is(err, context.Canceled) || errors.Is(err, nfc.ErrCanceled) || errors.Is(err, attack.ErrCancelled) {
			s.emitOperation(TaskEventDetailDTO{TaskID: id, Kind: kind, Type: TaskEventCancelled, Message: name + " cancelled; confirmed results were kept"})
			return
		}
		if err != nil {
			s.emitOperation(TaskEventDetailDTO{TaskID: id, Kind: kind, Type: "failed", Message: err.Error()})
			return
		}
		s.emitOperation(TaskEventDetailDTO{TaskID: id, Kind: kind, Type: TaskEventCompleted, Progress: 100, Message: name + " completed"})
	}()
	return task, nil
}

func (s *Service) CancelTask(taskID string) error {
	if taskID == "" {
		return ErrTaskIDRequired
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	task, exists := s.operations[taskID]
	if !exists {
		return fmt.Errorf("%w: %s", ErrTaskNotFound, taskID)
	}
	task.cancelRequested = true
	task.cancel()
	return nil
}

func (s *Service) emitOperation(event TaskEventDetailDTO) {
	if event.Time == "" {
		event.Time = time.Now().UTC().Format(time.RFC3339Nano)
	}
	s.emitter.Emit(TaskEventName, event)
}
