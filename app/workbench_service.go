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
	backupPath  string
	expires     time.Time
}

func (s *Service) KeyCatalog() []KeyDTO { return keyDTOs(s.keyStore) }

func (s *Service) AddKey(value string) ([]KeyDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return nil, err
	}
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
	if err := s.requireKeyMutationAllowed(); err != nil {
		return nil, err
	}
	if err := s.keyStore.Delete(id); err != nil {
		return nil, err
	}
	if err := s.persistKeys(); err != nil {
		return nil, err
	}
	result := keyDTOs(s.keyStore)
	s.emitKeyState(s.currentCardPtr())
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
	return s.ImportKeyDictionaryFile(path)
}

// ImportKeyDictionaryFile imports a key dictionary without a native file
// dialog. The CLI uses it for --keys and `nfctl keys import`.
func (s *Service) ImportKeyDictionaryFile(path string) (ImportReportDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return ImportReportDTO{}, err
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
	return s.ExportKeyDictionaryFile(path)
}

// ExportKeyDictionaryFile writes the complete key catalog with mode 0600.
// The CLI calls it directly so a script can choose the destination.
func (s *Service) ExportKeyDictionaryFile(path string) (ExportResultDTO, error) {
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
	s.mu.Lock()
	loadErr := s.keyStoreLoadErr
	s.mu.Unlock()
	if loadErr != nil {
		return fmt.Errorf("the key store could not be loaded; refusing to overwrite %s: %w", s.keyStorePath, loadErr)
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
	return s.LoadDumpFile(path, discardDirty)
}

// LoadDumpFile loads a raw dump or NFC Foundry project from an explicit path.
// It keeps the same validation and key registration as the GUI file dialog.
func (s *Service) LoadDumpFile(path string, discardDirty bool) (WorkbenchDTO, error) {
	if s.bench.Snapshot().Dirty && !discardDirty {
		return WorkbenchDTO{}, ErrDirtyWorkbench
	}
	var result workflow.DumpResult
	var err error
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
	// Keys stored in a dump are source metadata. They are not proof that the
	// same key authenticates the card currently in the field, so they must
	// never be registered as verified target keys.
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
	return s.SaveDumpFile(path)
}

// SaveDumpFile writes the current workbench image to an explicit path. A .json
// path always stores an NFC Foundry project, complete or partial. Other paths
// store a complete raw image plus a metadata sidecar; partial images are
// rejected so unknown bytes are never presented as card data.
func (s *Service) SaveDumpFile(path string) (SaveResultDTO, error) {
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
	project := strings.HasSuffix(strings.ToLower(path), ".json")
	if !complete && !project {
		return SaveResultDTO{}, nfc.NewError("save dump", nfc.CodeInvalidArgument, "an incomplete dump must be saved as an .json project; unknown bytes are never zero-filled", nil)
	}
	response := SaveResultDTO{Path: filepath.Base(path)}
	if project {
		if err := workflow.SaveDumpProject(path, result); err != nil {
			return SaveResultDTO{}, err
		}
	} else {
		sidecar, err := workflow.SaveRawDump(path, result)
		if err != nil {
			return SaveResultDTO{}, err
		}
		response.Sidecar = filepath.Base(sidecar)
	}
	if err := s.bench.MarkSaved(path); err != nil {
		return SaveResultDTO{}, err
	}
	s.emitWorkbench()
	return response, nil
}

func (s *Service) StartKeyScan(discardDirty bool) (TaskDTO, error) {
	if err := s.requireClassicRead("scan keys"); err != nil {
		return TaskDTO{}, err
	}
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
		s.emitKeyState(&card)
		if persistErr := s.persistKeys(); persistErr != nil && err == nil {
			err = persistErr
		}
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

func (s *Service) StartReadCard(input ReadCardStartRequestDTO) (TaskDTO, error) {
	if err := s.requireClassicRead("read card"); err != nil {
		return TaskDTO{}, err
	}
	if s.bench.Snapshot().Dirty && !input.DiscardDirty {
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
	return s.startPreparedOperation("read", "Read card", s.clearWorkbench, func(ctx context.Context, taskID string) error {
		if input.ScanKeys {
			if s.keyScanner == nil {
				return errors.New("key scanning is not configured")
			}
			s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "read", Type: TaskEventProgress, Message: "scanning known keys before reading"})
			_, scanErr := s.keyScanner.Scan(ctx, workflow.KeyScanRequest{Card: card}, func(progress workflow.KeyScanProgress) {
				message := fmt.Sprintf("scan before read: sector %d Key %s, tried %d, found %d", progress.Sector, keyTypeName(progress.KeyType), progress.Attempted, progress.Found)
				if progress.Matched {
					message = fmt.Sprintf("scan before read: sector %d Key %s verified", progress.Sector, keyTypeName(progress.KeyType))
				}
				s.emitOperation(TaskEventDetailDTO{
					TaskID: taskID, Kind: "read", Type: TaskEventProgress, Sector: progress.Sector,
					KeyType: keyTypeName(progress.KeyType), Attempted: progress.Attempted, Found: progress.Found,
					Progress: scanProgress(card, progress.Sector, progress.KeyType) / 2, Message: message,
				})
			})
			s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
			s.emitter.Emit(KeyCatalogEventName, keyDTOs(s.keyStore))
			if scanErr != nil {
				return scanErr
			}
		}
		before := s.keyStore.Verified(keys.CardID(card))
		request := workflow.DumpRequest{Card: card, Device: deviceInfo, Keys: verifiedSectorKeys(s.keyStore, card, layout)}
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "read", Type: TaskEventProgress, Progress: 55, Message: "reading the current card"})
		result, readErr := s.dumpService.ReadCard(ctx, request)
		if len(result.Dump.Blocks) != 0 {
			if s.dropStaleReadKeys(card, before, result) {
				_ = s.persistKeys()
			}
			s.registerDumpKeys(result)
			_ = s.persistKeys()
			s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
			if loadErr := s.bench.Load(result, ""); loadErr != nil && readErr == nil {
				readErr = loadErr
			}
			var classicNDEF *ClassicNDEFDTO
			if nfc.InferCardType(card) == nfc.CardTypeMIFAREClassic1K || nfc.InferCardType(card) == nfc.CardTypeMIFAREClassic4K {
				ndefDTO, _ := s.ReadClassicNDEF(ctx)
				classicNDEF = &ndefDTO
			}
			dto := s.emitWorkbench()
			s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "read", Type: TaskEventProgress, Progress: 100, Message: "read result loaded into the workbench", Workbench: &dto, ClassicNDEF: classicNDEF})
		}
		return readErr
	})
}

func (s *Service) clearWorkbench() {
	s.bench.Clear()
	s.emitWorkbench()
}

// dropStaleReadKeys removes card-scoped verifications that the read proved
// wrong. It only touches sectors that the read actually attempted, so an
// interrupted operation never clears keys it never tested.
func (s *Service) dropStaleReadKeys(card nfc.CardInfo, before []keys.Verification, result workflow.DumpResult) bool {
	if !result.Dump.Layout.Valid() {
		return false
	}
	changed := false
	for _, match := range before {
		record, exists := s.keyStore.Record(match.KeyID)
		if !exists || match.Sector < 0 || match.Sector >= len(result.Keys) {
			continue
		}
		if !readAttemptedSector(result.Dump, match.Sector) {
			continue
		}
		verified := result.Keys[match.Sector]
		actual := verified.KeyA
		if match.KeyType == nfc.KeyTypeB {
			actual = verified.KeyB
		}
		if actual != nil && *actual == record.Value {
			continue
		}
		s.keyStore.Unverify(keys.CardID(card), match.Sector, match.KeyType)
		changed = true
	}
	return changed
}

func readAttemptedSector(image mifare.Dump, sector int) bool {
	layout := image.Layout
	first, err := layout.FirstBlock(sector)
	if err != nil {
		return false
	}
	count, err := layout.BlocksInSector(sector)
	if err != nil {
		return false
	}
	for block := first; block < first+count && block < len(image.Blocks); block++ {
		if image.Blocks[block].Status != mifare.BlockUnread {
			return true
		}
	}
	return false
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
	if err := s.requireClassicWrite("write card"); err != nil {
		return WritePreflightDTO{}, err
	}
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
	if len(snapshot.Result.Card.UID) == 0 {
		return WritePreflightDTO{Mode: mode, Error: "the loaded dump has no card identity; load a dump with metadata"}, nil
	}
	if !nfc.SameCard(snapshot.Result.Card, card) {
		return WritePreflightDTO{Mode: mode, Error: "the loaded dump belongs to a different card; no block was written"}, nil
	}
	var blocks []int
	switch mode {
	case "block":
		if input.WriteUID {
			return WritePreflightDTO{}, nfc.NewError("restore preflight", nfc.CodeInvalidArgument, "UID can be written only together with a full dump restore", nil)
		}
		if len(input.Blocks) != 1 {
			return WritePreflightDTO{Mode: mode, SkippedBlock0: true, Error: "one ordinary data block must be selected"}, nil
		}
		block := input.Blocks[0]
		if block < 0 || block >= len(snapshot.Result.Dump.Blocks) {
			return WritePreflightDTO{Mode: mode, SkippedBlock0: true, Error: fmt.Sprintf("block %d is outside the loaded dump", block)}, nil
		}
		kind := workbench.KindForBlock(snapshot.Result.Dump.Layout, block)
		if kind != workbench.BlockData {
			return WritePreflightDTO{Mode: mode, SkippedBlock0: true, Error: "only ordinary data blocks can be written from the block editor"}, nil
		}
		blocks = []int{block}
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
	backupPath := ""
	if mode != "block" {
		if strings.TrimSpace(s.classicBackupRoot) == "" {
			return WritePreflightDTO{Mode: mode, Error: "a pre-write backup directory is unavailable; no write is allowed"}, nil
		}
		backupPath = classicBackupPathForCard(s.classicBackupRoot, card, token)
	}
	record := writePreflightRecord{
		request: request, fingerprint: fingerprint, mode: mode, trailers: trailers,
		uidRequest: uidRequest, uidPlan: uidPlan, backupPath: backupPath, expires: time.Now().Add(5 * time.Minute),
	}
	s.mu.Lock()
	s.preflights[token] = record
	s.mu.Unlock()
	warnings := []string{"writing stops at the first unrecoverable failure"}
	if mode == "block" {
		warnings = append(warnings, "one ordinary data block is written; the reader verifies it by immediate read-back")
	} else {
		warnings = append(warnings, "a normal dump restore skips block 0")
		if backupPath != "" {
			warnings = append(warnings, "a complete pre-write backup is saved to "+backupPath)
		}
	}
	if len(trailers) != 0 {
		warnings = append(warnings, "the plan contains sector trailers; key and access-condition changes must be confirmed")
	}
	var uidDTO *UIDWritePreviewDTO
	if uidPlan != nil {
		preview := uidWritePreviewDTO(*uidPlan)
		uidDTO = &preview
		warnings = append(warnings, "UID is written last, after the full dump succeeds; supports CUID/Gen2 and Gen1A special cards on readers with raw-frame support")
	}
	return WritePreflightDTO{
		Token: token, Mode: mode, Valid: true, Blocks: planned, TrailerBlocks: trailers,
		SkippedBlock0: uidPlan == nil, Warnings: warnings, Access: access, UID: uidDTO,
		BackupPath: backupPath,
	}, nil
}

func classicBackupPathForCard(root string, card nfc.CardInfo, token string) string {
	uid := strings.ReplaceAll(formatHex(card.UID), " ", "")
	if uid == "" {
		uid = "unknown"
	}
	return filepath.Join(root, fmt.Sprintf("classic-%s-%s.nfcx.json", uid, token))
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
	if err := s.requireClassicWrite("write card"); err != nil {
		return TaskDTO{}, err
	}
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
		if record.backupPath != "" {
			if err := s.savePreWriteBackup(ctx, record.request.Card, record.request.TargetKeys, record.backupPath, taskID, "write"); err != nil {
				return err
			}
		}
		result, restoreErr := s.restoreService.Restore(ctx, record.request, func(event workflow.RestoreEvent) {
			s.emitOperation(TaskEventDetailDTO{
				TaskID: taskID, Kind: "write", Type: TaskEventProgress, Phase: string(event.Phase),
				Block: event.Block, Message: restoreMessage(event),
			})
		})
		s.registerRestoredKeys(record.request, result)
		dto := restoreResultDTO(result)
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "write", Type: TaskEventProgress, Phase: string(result.Phase), Message: "write result generated", Restore: &dto})
		if len(dto.VerifiedBlocks) != 0 {
			if err := s.bench.MarkWritten(dto.VerifiedBlocks); err != nil {
				return fmt.Errorf("write succeeded but workbench state could not be updated: %w", err)
			}
			s.emitWorkbench()
		}
		if restoreErr != nil || record.uidRequest == nil {
			return annotateBackup(restoreErr, record.backupPath)
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
			if err := s.bench.MarkWritten([]int{0}); err != nil {
				return fmt.Errorf("UID write succeeded but workbench state could not be updated: %w", err)
			}
			s.emitWorkbench()
		}
		return annotateBackup(uidErr, record.backupPath)
	})
}

func annotateBackup(err error, path string) error {
	if err == nil || path == "" {
		return err
	}
	return fmt.Errorf("%w; pre-write backup: %s", err, path)
}

func (s *Service) savePreWriteBackup(ctx context.Context, card nfc.CardInfo, keys []workflow.SectorKeys, path, taskID, kind string) error {
	if s.dumpService == nil || s.deviceManager == nil {
		return nfc.NewError("pre-write backup", nfc.CodeNotOpen, "the reader is unavailable; the card was not modified", nil)
	}
	card = card.Clone()
	s.emitOperation(TaskEventDetailDTO{
		TaskID: taskID, Kind: kind, Type: TaskEventProgress,
		Message: "reading the current card for the pre-write backup",
	})
	result, err := s.dumpService.ReadCard(ctx, workflow.DumpRequest{
		Card: card, Device: s.deviceManager.Snapshot().Device, Keys: keys,
	})
	if err != nil {
		return nfc.NewError("pre-write backup", nfc.CodeIO, fmt.Sprintf("could not read the current card for the pre-write backup at %s; the card was not modified", path), err)
	}
	if !nfc.SameCard(card, result.Card) {
		return nfc.NewError("pre-write backup", nfc.CodeCardChanged, "the selected card changed while reading the pre-write backup; the card was not modified", nil)
	}
	if !result.Complete() {
		return nfc.NewError("pre-write backup", nfc.CodeInvalidArgument, fmt.Sprintf("the pre-write backup at %s is incomplete; the card was not modified", path), mifare.ErrIncompleteDump)
	}
	directory := filepath.Dir(path)
	if err := os.MkdirAll(directory, 0o700); err != nil {
		return nfc.NewError("pre-write backup", nfc.CodeIO, fmt.Sprintf("could not create the pre-write backup directory for %s; the card was not modified", path), err)
	}
	if err := os.Chmod(directory, 0o700); err != nil {
		return nfc.NewError("pre-write backup", nfc.CodeIO, fmt.Sprintf("could not secure the pre-write backup directory for %s; the card was not modified", path), err)
	}
	if err := workflow.SaveDumpProject(path, result); err != nil {
		return nfc.NewError("pre-write backup", nfc.CodeIO, fmt.Sprintf("could not save the pre-write backup to %s; the card was not modified", path), err)
	}
	s.emitOperation(TaskEventDetailDTO{
		TaskID: taskID, Kind: kind, Type: TaskEventProgress, Progress: 1,
		BackupPath: path, Message: "pre-write backup saved to " + path,
	})
	return nil
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
		_ = s.persistKeys()
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
	if kind != "recovery_save" && s.keyReviewPending() {
		return TaskDTO{}, ErrRecoveryReviewPending
	}
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
