package app

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workbench"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

type clonePreflightRecord struct {
	request     workflow.CloneRequest
	fingerprint string
	sourcePath  string
	backupPath  string
	expires     time.Time
}

type factoryPreflightRecord struct {
	card       nfc.CardInfo
	device     nfc.DeviceInfo
	keys       []workflow.SectorKeys
	keepData   bool
	backupPath string
	expires    time.Time
}

func (s *Service) PreviewValueBlock(input ValueBlockPreviewRequestDTO) (ValueBlockPreviewDTO, error) {
	result, err := s.bench.Result()
	if err != nil {
		return ValueBlockPreviewDTO{}, err
	}
	if input.Block < 0 || input.Block >= len(result.Dump.Blocks) {
		return ValueBlockPreviewDTO{}, fmt.Errorf("block %d is outside the loaded dump", input.Block)
	}
	if workbench.KindForBlock(result.Dump.Layout, input.Block) != workbench.BlockData {
		return ValueBlockPreviewDTO{}, errors.New("value blocks can only be encoded into ordinary data blocks")
	}
	value, err := parseValueBlockValue(input.Value)
	if err != nil {
		return ValueBlockPreviewDTO{}, err
	}
	address := input.Address
	if address < 0 {
		address = input.Block
	}
	if address > 0xff {
		return ValueBlockPreviewDTO{}, errors.New("value block address must be between 0 and 255")
	}
	data := mifare.EncodeValueBlock(value, byte(address))
	decoded, err := mifare.DecodeValueBlock(data)
	if err != nil {
		return ValueBlockPreviewDTO{}, err
	}
	dto := ValueBlockPreviewDTO{
		Block: input.Block, Sector: result.Dump.Blocks[input.Block].Sector,
		Hex:     strings.ToUpper(fmt.Sprintf("%X", data)),
		Decoded: ValueBlockDTO{Valid: true, Value: int64(decoded.Value), Address: int(decoded.Address), ValueHex: fmt.Sprintf("%08X", uint32(decoded.Value))},
	}
	if address != input.Block {
		dto.Warning = fmt.Sprintf("the value block address is %d, but the block number is %d; increment and decrement may fail", address, input.Block)
	}
	return dto, nil
}

func parseValueBlockValue(value string) (int32, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return 0, errors.New("a value is required")
	}
	base := 10
	digits := value
	negative := strings.HasPrefix(value, "-")
	if negative {
		digits = value[1:]
	}
	if strings.HasPrefix(digits, "0x") || strings.HasPrefix(digits, "0X") {
		base = 16
		digits = digits[2:]
		if negative {
			digits = "-" + digits
		}
	}
	parsed, err := strconv.ParseInt(digits, base, 32)
	if err != nil {
		return 0, fmt.Errorf("value must be a signed 32-bit decimal or 0x-prefixed hexadecimal number")
	}
	if negative {
		parsed = -parsed
	}
	return int32(parsed), nil
}

func (s *Service) PreviewTrailer(input TrailerPreviewRequestDTO) (TrailerPreviewDTO, error) {
	result, err := s.bench.Result()
	if err != nil {
		return TrailerPreviewDTO{}, err
	}
	if input.Block < 0 || input.Block >= len(result.Dump.Blocks) {
		return TrailerPreviewDTO{}, fmt.Errorf("block %d is outside the loaded dump", input.Block)
	}
	if kind := workbench.KindForBlock(result.Dump.Layout, input.Block); kind != workbench.BlockTrailer {
		return TrailerPreviewDTO{}, errors.New("only a sector trailer can be edited here")
	}
	keyA, err := parseTrailerKey(input.KeyA, "Key A")
	if err != nil {
		return TrailerPreviewDTO{}, err
	}
	keyB, err := parseTrailerKey(input.KeyB, "Key B")
	if err != nil {
		return TrailerPreviewDTO{}, err
	}
	if len(input.Groups) != 4 {
		return TrailerPreviewDTO{}, errors.New("four access-condition groups are required")
	}
	var bits mifare.AccessBits
	seen := make(map[int]bool, 4)
	for _, group := range input.Groups {
		if group.Group < 0 || group.Group >= 4 || seen[group.Group] {
			return TrailerPreviewDTO{}, errors.New("access-condition groups must be 0, 1, 2, and 3 exactly once")
		}
		seen[group.Group] = true
		bits.Groups[group.Group] = mifare.AccessCondition{C1: group.C1, C2: group.C2, C3: group.C3}
	}
	accessBytes := mifare.EncodeAccessBits(bits)
	decoded, err := mifare.DecodeAccessBits(accessBytes)
	if err != nil {
		return TrailerPreviewDTO{}, err
	}
	if decoded != bits {
		return TrailerPreviewDTO{}, errors.New("encoded access bits do not round-trip")
	}
	gpb := result.Dump.Blocks[input.Block].Data[9]
	if strings.TrimSpace(input.GPB) != "" {
		value, err := strconv.ParseUint(strings.TrimPrefix(strings.TrimPrefix(strings.TrimSpace(input.GPB), "0x"), "0X"), 16, 8)
		if err != nil {
			return TrailerPreviewDTO{}, errors.New("GPB must be one hexadecimal byte")
		}
		gpb = byte(value)
	}
	var data [nfc.BlockSize]byte
	copy(data[0:6], keyA[:])
	data[6], data[7], data[8], data[9] = accessBytes[0], accessBytes[1], accessBytes[2], gpb
	copy(data[10:16], keyB[:])
	summary := accessDTO(workbench.ExplainAccess(mifare.Block{Number: input.Block, Data: data, KnownMask: mifare.AllBytesKnown}))
	return TrailerPreviewDTO{
		Block: input.Block, Sector: result.Dump.Blocks[input.Block].Sector,
		OldHex: strings.ToUpper(result.Dump.Blocks[input.Block].Hex()),
		Hex:    strings.ToUpper(fmt.Sprintf("%X", data)),
		KeyA:   strings.ToUpper(fmt.Sprintf("%X", keyA)),
		KeyB:   strings.ToUpper(fmt.Sprintf("%X", keyB)),
		GPB:    fmt.Sprintf("%02X", gpb), Access: summary,
	}, nil
}

func parseTrailerKey(value, label string) (nfc.Key, error) {
	compact := strings.NewReplacer(" ", "", ":", "", "-", "").Replace(strings.TrimSpace(value))
	key, err := keys.ParseKey(compact)
	if err != nil {
		return nfc.Key{}, fmt.Errorf("%s must contain exactly 12 hexadecimal digits", label)
	}
	return key, nil
}

func (s *Service) OpenComparisonDump() (DumpDiffDTO, error) {
	path, err := s.dialogs.OpenFile("Open a dump to compare", "*.bin;*.mfd;*.json;*.mct;*.eml")
	if err != nil {
		return DumpDiffDTO{}, err
	}
	if path == "" {
		return DumpDiffDTO{Cancelled: true}, nil
	}
	return s.LoadComparisonDumpFile(path)
}

func (s *Service) LoadComparisonDumpFile(path string) (DumpDiffDTO, error) {
	load, err := workflow.LoadDumpFile(path)
	if err != nil {
		return DumpDiffDTO{}, err
	}
	s.comparisonMu.Lock()
	s.comparison = &load.Result
	s.comparisonPath = path
	s.comparisonMu.Unlock()
	return s.ComparisonDiff()
}

func (s *Service) ComparisonDiff() (DumpDiffDTO, error) {
	result, err := s.bench.Result()
	if err != nil {
		return DumpDiffDTO{}, err
	}
	s.comparisonMu.Lock()
	comparison := s.comparison
	s.comparisonMu.Unlock()
	if comparison == nil {
		return DumpDiffDTO{}, errors.New("no comparison dump is loaded")
	}
	diff, err := workflow.DiffDumps(result, *comparison)
	if err != nil {
		return DumpDiffDTO{}, err
	}
	return dumpDiffDTO(diff), nil
}

func (s *Service) ComparisonPath() string {
	s.comparisonMu.Lock()
	defer s.comparisonMu.Unlock()
	if s.comparisonPath == "" {
		return ""
	}
	return filepath.Base(s.comparisonPath)
}

func (s *Service) ClearComparisonDump() {
	s.comparisonMu.Lock()
	s.comparison = nil
	s.comparisonPath = ""
	s.comparisonMu.Unlock()
}

// CompareDumpFiles loads both images and compares them without a reader.
func (s *Service) CompareDumpFiles(beforePath, afterPath string) (DumpDiffDTO, error) {
	before, err := workflow.LoadDumpFile(beforePath)
	if err != nil {
		return DumpDiffDTO{}, fmt.Errorf("%s: %w", filepath.Base(beforePath), err)
	}
	after, err := workflow.LoadDumpFile(afterPath)
	if err != nil {
		return DumpDiffDTO{}, fmt.Errorf("%s: %w", filepath.Base(afterPath), err)
	}
	diff, err := workflow.DiffDumps(before.Result, after.Result)
	if err != nil {
		return DumpDiffDTO{}, err
	}
	return dumpDiffDTO(diff), nil
}

func (s *Service) PreviewKeyFile(content string) KeyFilePreviewDTO {
	analysis, err := keys.AnalyzeDictionary(strings.NewReader(content))
	if err != nil {
		return KeyFilePreviewDTO{Issues: []ImportIssueDTO{{Line: 0, Message: err.Error()}}, Lines: []KeyFileLineDTO{}}
	}
	return keyFilePreviewDTO(analysis)
}

func (s *Service) OpenKeyFileEditor() (KeyFileContentDTO, error) {
	path, err := s.dialogs.OpenFile("Open a MIFARE Classic key file", "*.dic;*.keys;*.txt")
	if err != nil {
		return KeyFileContentDTO{}, err
	}
	if path == "" {
		return KeyFileContentDTO{}, nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return KeyFileContentDTO{}, err
	}
	return KeyFileContentDTO{Path: filepath.Base(path), Content: string(data)}, nil
}

func (s *Service) SaveKeyFileEditor(content string) (ExportResultDTO, error) {
	path, err := s.dialogs.SaveFile("Save MIFARE Classic key file", "nfc-foundry-keys.keys", "*.keys;*.dic;*.txt")
	if err != nil {
		return ExportResultDTO{}, err
	}
	if path == "" {
		return ExportResultDTO{Cancelled: true}, nil
	}
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		return ExportResultDTO{}, err
	}
	preview := s.PreviewKeyFile(content)
	return ExportResultDTO{Path: filepath.Base(path), Count: preview.Valid + preview.Duplicates}, nil
}

func (s *Service) ImportKeyTextWithSet(content, setName string) (ImportReportDTO, error) {
	if err := s.requireKeyMutationAllowed(); err != nil {
		return ImportReportDTO{}, err
	}
	report, err := keys.ParseDictionary(strings.NewReader(content))
	if err != nil {
		return ImportReportDTO{}, err
	}
	return s.finishKeyImport("key-file-editor.keys", setName, report)
}

func (s *Service) PreflightClone() (ClonePreflightDTO, error) {
	if err := s.requireClassicWrite("clone card"); err != nil {
		return ClonePreflightDTO{}, err
	}
	s.mu.Lock()
	operationRunning := len(s.operations) != 0
	s.mu.Unlock()
	if operationRunning {
		return ClonePreflightDTO{}, ErrOperationRunning
	}
	snapshot := s.bench.Snapshot()
	if !snapshot.Loaded {
		return ClonePreflightDTO{}, workbench.ErrNoDump
	}
	if !snapshot.Result.Complete() {
		return ClonePreflightDTO{}, mifare.ErrIncompleteDump
	}
	if len(snapshot.Result.Card.UID) == 0 {
		return ClonePreflightDTO{}, nfc.NewError("clone preflight", nfc.CodeInvalidArgument, "the source dump has no card identity; save it as an NFC Foundry project with a card read first", nil)
	}
	deviceSnapshot := s.deviceManager.Snapshot()
	if deviceSnapshot.Card == nil {
		return ClonePreflightDTO{}, ErrCardRequired
	}
	card := deviceSnapshot.Card.Clone()
	layout, err := appLayoutForCard(card)
	if err != nil {
		return ClonePreflightDTO{}, err
	}
	request := workflow.CloneRequest{
		Source: snapshot.Result, Card: card, Device: deviceSnapshot.Device,
		TargetKeys: verifiedSectorKeys(s.keyStore, card, layout),
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	plan, err := s.cloneService.Preflight(ctx, request)
	if err != nil {
		return ClonePreflightDTO{Valid: false, Error: err.Error(), Source: dumpIdentityDTO(snapshot.Result), Target: cardDTO(card), Warnings: []string{"clone preflight failed; no block was written"}}, nil
	}
	blocks, trailers := restorePlanBlocks(plan.Restore)
	token := s.newPreflightToken("clone")
	backupPath := classicBackupPathForCard(s.classicBackupRoot, card, token)
	if strings.TrimSpace(s.classicBackupRoot) == "" {
		return ClonePreflightDTO{Valid: false, Error: "a pre-write backup directory is unavailable; no clone is allowed", Source: dumpIdentityDTO(snapshot.Result), Target: cardDTO(card)}, nil
	}
	fingerprint, err := dumpFingerprint(snapshot.Result)
	if err != nil {
		return ClonePreflightDTO{}, err
	}
	s.mu.Lock()
	s.clonePreflights[token] = clonePreflightRecord{
		request: request, fingerprint: fingerprint, sourcePath: snapshot.Path,
		backupPath: backupPath, expires: time.Now().Add(5 * time.Minute),
	}
	s.mu.Unlock()
	warnings := []string{"writing stops at the first unrecoverable failure", "a complete pre-write backup is saved to " + backupPath}
	if plan.Block0Write {
		warnings = append(warnings, "block 0 will be written through the supported DirectWrite/Gen1A path; a genuine read-only block 0 fails and is never reported as a successful clone")
	} else {
		warnings = append(warnings, "block 0 already matches the source image and is not written")
	}
	return ClonePreflightDTO{
		Token: token, Valid: true, Source: dumpIdentityDTO(snapshot.Result), Target: cardDTO(card),
		Block0Write: plan.Block0Write, OldBlock0: strings.ToUpper(fmt.Sprintf("%X", plan.OldBlock0)),
		NewBlock0: strings.ToUpper(fmt.Sprintf("%X", plan.NewBlock0)), Blocks: blocks, TrailerBlocks: trailers,
		BackupPath: backupPath, Warnings: warnings,
	}, nil
}

func (s *Service) StartClone(input CloneStartRequestDTO) (TaskDTO, error) {
	if err := s.requireClassicWrite("clone card"); err != nil {
		return TaskDTO{}, err
	}
	s.mu.Lock()
	record, exists := s.clonePreflights[input.Token]
	if exists {
		delete(s.clonePreflights, input.Token)
	}
	s.mu.Unlock()
	if !exists || time.Now().After(record.expires) {
		return TaskDTO{}, ErrPreflightRequired
	}
	sourceUID := compactHex(record.request.Source.Card.UID)
	confirmation := strings.NewReplacer(" ", "", ":", "", "-", "").Replace(strings.TrimSpace(input.Confirmation))
	if sourceUID == "" || !strings.EqualFold(confirmation, sourceUID) {
		return TaskDTO{}, errors.New("clone confirmation must repeat the source UID")
	}
	current, err := s.bench.Result()
	if err != nil {
		return TaskDTO{}, ErrPreflightRequired
	}
	fingerprint, err := dumpFingerprint(current)
	if err != nil || fingerprint != record.fingerprint {
		return TaskDTO{}, ErrPreflightRequired
	}
	deviceSnapshot := s.deviceManager.Snapshot()
	if deviceSnapshot.Card == nil || !nfc.SameCard(*deviceSnapshot.Card, record.request.Card) {
		return TaskDTO{}, ErrPreflightRequired
	}
	return s.startOperation("clone", "Clone card", func(ctx context.Context, taskID string) error {
		if err := s.savePreWriteBackup(ctx, record.request.Card, record.request.TargetKeys, record.backupPath, taskID, "clone"); err != nil {
			return err
		}
		result, cloneErr := s.cloneService.Clone(ctx, record.request, func(event workflow.CloneEvent) {
			s.emitOperation(TaskEventDetailDTO{
				TaskID: taskID, Kind: "clone", Type: TaskEventProgress, Phase: string(event.Phase),
				Block: event.Block, Message: event.Message,
			})
		})
		dto := cloneResultDTO(result)
		dto.SourcePath = record.sourcePath
		dto.BackupPath = record.backupPath
		if cloneErr == nil && result.Restore.Phase == workflow.RestoreComplete {
			s.registerRestoredKeys(workflow.RestoreRequest{Card: result.TargetCard, Dump: record.request.Source.Dump}, result.Restore)
		}
		if cloneErr == nil && result.Verified {
			blocks := make([]int, len(record.request.Source.Dump.Blocks))
			for index := range blocks {
				blocks[index] = index
			}
			if err := s.bench.MarkWritten(blocks); err != nil {
				return fmt.Errorf("the clone succeeded but the workbench state could not be updated: %w", err)
			}
			s.emitWorkbench()
		}
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "clone", Type: TaskEventProgress, Message: "clone result generated", Phase: dto.Phase, Clone: &dto})
		return annotateBackup(cloneErr, record.backupPath)
	})
}

func (s *Service) PreflightFactoryFormat(keepData bool) (FactoryPreflightDTO, error) {
	if err := s.requireClassicWrite("factory format"); err != nil {
		return FactoryPreflightDTO{}, err
	}
	s.mu.Lock()
	operationRunning := len(s.operations) != 0
	s.mu.Unlock()
	if operationRunning {
		return FactoryPreflightDTO{}, ErrOperationRunning
	}
	deviceSnapshot := s.deviceManager.Snapshot()
	if deviceSnapshot.Card == nil {
		return FactoryPreflightDTO{}, ErrCardRequired
	}
	card := deviceSnapshot.Card.Clone()
	layout, err := appLayoutForCard(card)
	if err != nil {
		return FactoryPreflightDTO{Valid: false, Error: "target card is not MIFARE Classic 1K/4K"}, nil
	}
	targetKeys := verifiedSectorKeys(s.keyStore, card, layout)
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	backup, err := s.dumpService.ReadCard(ctx, workflow.DumpRequest{Card: card, Device: deviceSnapshot.Device, Keys: targetKeys})
	if err != nil {
		return FactoryPreflightDTO{Valid: false, Error: err.Error(), Card: cardDTO(card), Warnings: []string{"reading the target failed; no block was written"}}, nil
	}
	if !backup.Complete() {
		return FactoryPreflightDTO{Valid: false, Error: "the target card is not fully readable; factory format refuses to run", Card: cardDTO(card)}, nil
	}
	image, err := workflow.BuildFactoryImage(backup, keepData)
	if err != nil {
		return FactoryPreflightDTO{Valid: false, Error: err.Error(), Card: cardDTO(card)}, nil
	}
	request := workflow.RestoreRequest{
		Card: card, Dump: image.Dump, SourceUIDLength: len(card.UID), TargetKeys: targetKeys,
	}
	plan, err := s.restoreService.Preflight(ctx, request)
	if err != nil {
		return FactoryPreflightDTO{Valid: false, Error: err.Error(), Card: cardDTO(card), Warnings: []string{"factory preflight failed; no block was written"}}, nil
	}
	blocks, trailers := restorePlanBlocks(plan)
	token := s.newPreflightToken("factory")
	backupPath := classicBackupPathForCard(s.classicBackupRoot, card, token)
	if strings.TrimSpace(s.classicBackupRoot) == "" {
		return FactoryPreflightDTO{Valid: false, Error: "a pre-write backup directory is unavailable; no format is allowed", Card: cardDTO(card)}, nil
	}
	s.mu.Lock()
	s.factoryPreflights[token] = factoryPreflightRecord{
		card: card, device: deviceSnapshot.Device, keys: targetKeys, keepData: keepData,
		backupPath: backupPath, expires: time.Now().Add(5 * time.Minute),
	}
	s.mu.Unlock()
	return FactoryPreflightDTO{
		Token: token, Valid: true, Card: cardDTO(card), Blocks: blocks, TrailerBlocks: trailers, KeepData: keepData,
		TrailerBytes: "FFFFFFFFFFFF FF0780 69 FFFFFFFFFFFF",
		BackupPath:   backupPath,
		Warnings: []string{
			"all sector trailers will be set to Key A/B FFFFFFFFFFFF and access bytes FF 07 80",
			"a complete pre-write backup is saved to " + backupPath,
			"block 0 is not written",
		},
	}, nil
}

func (s *Service) StartFactoryFormat(input FactoryStartRequestDTO) (TaskDTO, error) {
	if err := s.requireClassicWrite("factory format"); err != nil {
		return TaskDTO{}, err
	}
	s.mu.Lock()
	record, exists := s.factoryPreflights[input.Token]
	if exists {
		delete(s.factoryPreflights, input.Token)
	}
	s.mu.Unlock()
	if !exists || time.Now().After(record.expires) {
		return TaskDTO{}, ErrPreflightRequired
	}
	if !strings.EqualFold(strings.TrimSpace(input.Confirmation), "FORMAT") {
		return TaskDTO{}, errors.New("factory format confirmation must be FORMAT")
	}
	deviceSnapshot := s.deviceManager.Snapshot()
	if deviceSnapshot.Card == nil || !nfc.SameCard(*deviceSnapshot.Card, record.card) {
		return TaskDTO{}, ErrPreflightRequired
	}
	return s.startOperation("factory", "Factory format", func(ctx context.Context, taskID string) error {
		backup, err := s.readAndSavePreWriteBackup(ctx, record.card, record.keys, record.backupPath, taskID, "factory")
		if err != nil {
			return err
		}
		if !nfc.SameCard(record.card, backup.Card) {
			return nfc.NewError("factory format", nfc.CodeCardChanged, "the card identity changed before the factory format; the card was not modified", nil)
		}
		image, err := workflow.BuildFactoryImage(backup, record.keepData)
		if err != nil {
			return err
		}
		request := workflow.RestoreRequest{
			Card: record.card, Dump: image.Dump, SourceUIDLength: len(record.card.UID), TargetKeys: record.keys,
		}
		restoreResult, restoreErr := s.restoreService.Restore(ctx, request, func(event workflow.RestoreEvent) {
			s.emitOperation(TaskEventDetailDTO{
				TaskID: taskID, Kind: "factory", Type: TaskEventProgress, Phase: string(event.Phase),
				Block: event.Block, Message: event.Message,
			})
		})
		s.registerRestoredKeys(request, restoreResult)
		dto := FactoryResultDTO{Phase: string(restoreResult.Phase), BackupPath: record.backupPath}
		restoreDTO := restoreResultDTO(restoreResult)
		dto.Restore = &restoreDTO
		if restoreErr == nil {
			readback, readErr := s.dumpService.ReadCard(ctx, workflow.DumpRequest{
				Card: record.card, Device: record.device, Keys: workflow.SectorKeysFromDump(image.Dump),
			})
			if readErr != nil {
				return annotateBackup(readErr, record.backupPath)
			}
			diff, diffErr := workflow.DiffDumps(image, readback)
			if diffErr != nil {
				return annotateBackup(diffErr, record.backupPath)
			}
			diffDTO := dumpDiffDTO(diff)
			dto.Diff = &diffDTO
			dto.Verified = diff.ChangedBlocks == 0 && diff.UnknownBlocks == 0 && !diff.IdentityChanged
			if !dto.Verified {
				return annotateBackup(nfc.NewError("factory verification", nfc.CodeVerificationFailed,
					fmt.Sprintf("the formatted card differs in %d blocks; the factory format is not verified", diff.ChangedBlocks), nil), record.backupPath)
			}
			dto.Phase = "complete"
		}
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "factory", Type: TaskEventProgress, Phase: dto.Phase, Message: "factory format result generated", Factory: &dto})
		return annotateBackup(restoreErr, record.backupPath)
	})
}

func (s *Service) newPreflightToken(prefix string) string {
	value := fmt.Sprintf("%s-%d-%x", prefix, s.nextID.Add(1), sha256.Sum256([]byte(time.Now().UTC().String())))
	return value
}

func dumpFingerprint(result workflow.DumpResult) (string, error) {
	raw, err := result.Dump.Raw()
	if err != nil {
		return "", err
	}
	digest := sha256.New()
	_, _ = digest.Write(raw)
	_, _ = fmt.Fprintf(digest, "|%s|%s", keys.CardID(result.Card), result.Card.UID)
	return fmt.Sprintf("%x", digest.Sum(nil)), nil
}

func compactHex(data []byte) string {
	return strings.ToUpper(hex.EncodeToString(data))
}

func restorePlanBlocks(plan workflow.RestorePlan) ([]int, []int) {
	blocks := make([]int, 0, len(plan.Operations))
	trailers := make([]int, 0)
	for _, operation := range plan.Operations {
		blocks = append(blocks, operation.Block)
		if operation.Trailer {
			trailers = append(trailers, operation.Block)
		}
	}
	return blocks, trailers
}

func cloneResultDTO(result workflow.CloneResult) CloneResultDTO {
	dto := CloneResultDTO{Phase: string(result.Phase), Verified: result.Verified}
	restore := restoreResultDTO(result.Restore)
	dto.Restore = &restore
	if result.UID != nil {
		uid := uidWriteResultDTO(*result.UID)
		dto.UID = &uid
	}
	if len(result.Diff.Blocks) != 0 || result.Diff.ChangedBlocks != 0 || result.Diff.EqualBlocks != 0 {
		diff := dumpDiffDTO(result.Diff)
		dto.Diff = &diff
	}
	return dto
}
