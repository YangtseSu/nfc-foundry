package workflow

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/attack"
	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

const KeyRecoveryTotalSteps = 5

const (
	defaultRecoveryDarksideTimeout   = time.Hour
	defaultRecoveryNestedTimeout     = 30 * time.Minute
	defaultRecoveryHardnestedTimeout = 6 * time.Hour
)

type KeyRecoveryStage string

const (
	KeyRecoveryCommonKeys KeyRecoveryStage = "common_keys"
	KeyRecoveryDarkside   KeyRecoveryStage = "darkside"
	KeyRecoveryNested     KeyRecoveryStage = "nested"
	KeyRecoveryHardnested KeyRecoveryStage = "hardnested"
	KeyRecoveryReadiness  KeyRecoveryStage = "readiness"
)

type KeyRecoveryStepStatus string

const (
	KeyRecoveryPending     KeyRecoveryStepStatus = "pending"
	KeyRecoveryRunning     KeyRecoveryStepStatus = "running"
	KeyRecoveryCompleted   KeyRecoveryStepStatus = "completed"
	KeyRecoverySkipped     KeyRecoveryStepStatus = "skipped"
	KeyRecoveryFailed      KeyRecoveryStepStatus = "failed"
	KeyRecoveryUnavailable KeyRecoveryStepStatus = "unavailable"
)

type KeyRecoveryOutcome string

const (
	KeyRecoveryComplete KeyRecoveryOutcome = "complete"
	KeyRecoveryPartial  KeyRecoveryOutcome = "partial"
	KeyRecoveryFailure  KeyRecoveryOutcome = "failed"
)

type KeyRecoveryStep struct {
	Number  int
	Stage   KeyRecoveryStage
	Label   string
	Status  KeyRecoveryStepStatus
	Message string
}

type KeyRecoveryEvent struct {
	Step          int
	Stage         KeyRecoveryStage
	Status        KeyRecoveryStepStatus
	Progress      int
	Indeterminate bool
	Message       string
	Sector        int
	KeyType       nfc.KeyType
	KeyTypeSet    bool
	Attempted     int
	Found         int
	Attack        *attack.AttackEvent
}

type KeyRecoveryRequest struct {
	TaskID            string
	Card              nfc.CardInfo
	Device            nfc.DeviceInfo
	Authorized        bool
	DarksideTimeout   time.Duration
	NestedTimeout     time.Duration
	HardnestedTimeout time.Duration
	TemporaryPolicy   attack.TemporaryPolicy
}

type KeyRecoveryResult struct {
	Card                nfc.CardInfo
	Steps               []KeyRecoveryStep
	Outcome             KeyRecoveryOutcome
	Scan                KeyScanResult
	MFCUK               MFCUKPipelineResult
	MFoC                MFoCResult
	Hardnested          HardnestedResult
	Read                DumpResult
	VerifiedKeySlots    int
	CoveredSectors      int
	ReadableSectors     int
	TotalSectors        int
	DumpComplete        bool
	FailedStep          int
	FailedStage         KeyRecoveryStage
	Reason              string
	HardnestedAvailable bool
}

type KeyRecoveryError struct {
	Step  int
	Stage KeyRecoveryStage
	Cause error
}

func (e *KeyRecoveryError) Error() string {
	if e == nil {
		return "<nil>"
	}
	return fmt.Sprintf("key recovery step %d/%d (%s) failed: %v", e.Step, KeyRecoveryTotalSteps, recoveryStageLabel(e.Stage), e.Cause)
}

func (e *KeyRecoveryError) Unwrap() error { return e.Cause }

type keyRecoveryScanner interface {
	Scan(context.Context, KeyScanRequest, func(KeyScanProgress)) (KeyScanResult, error)
}

type keyRecoveryMFCUK interface {
	Run(context.Context, MFCUKPipelineRequest, func(attack.AttackEvent)) (MFCUKPipelineResult, error)
}

type keyRecoveryMFoC interface {
	Run(context.Context, MFoCRequest, func(attack.AttackEvent)) (MFoCResult, error)
}

type keyRecoveryHardnested interface {
	Available(context.Context) error
	Run(context.Context, HardnestedRequest, func(attack.AttackEvent)) (HardnestedResult, error)
}

type keyRecoveryReader interface {
	ReadCard(context.Context, DumpRequest) (DumpResult, error)
}

type KeyRecoveryService struct {
	scanner    keyRecoveryScanner
	keys       *keys.Store
	mfcuk      keyRecoveryMFCUK
	mfoc       keyRecoveryMFoC
	hardnested keyRecoveryHardnested
	reader     keyRecoveryReader
}

func NewKeyRecoveryService(scanner *KeyScanService, store *keys.Store, mfcuk *MFCUKPipelineService, mfoc *MFoCService, hardnested *HardnestedService, reader *DumpService) *KeyRecoveryService {
	return &KeyRecoveryService{scanner: scanner, keys: store, mfcuk: mfcuk, mfoc: mfoc, hardnested: hardnested, reader: reader}
}

func (s *KeyRecoveryService) Run(ctx context.Context, request KeyRecoveryRequest, emit func(KeyRecoveryEvent)) (KeyRecoveryResult, error) {
	request.Card = request.Card.Clone()
	result := KeyRecoveryResult{Card: request.Card.Clone(), Steps: newKeyRecoverySteps()}
	if ctx == nil || s == nil || s.scanner == nil || s.keys == nil || s.reader == nil {
		return failKeyRecovery(result, 1, KeyRecoveryCommonKeys, errors.New("key recovery service is not configured"))
	}
	if !request.Authorized {
		return failKeyRecovery(result, 1, KeyRecoveryCommonKeys, errors.New("the card must be confirmed as authorized"))
	}
	layout, err := layoutForCard(request.Card)
	if err != nil {
		return failKeyRecovery(result, 1, KeyRecoveryCommonKeys, errors.New("only MIFARE Classic 1K/4K is supported"))
	}
	result.TotalSectors, _ = layout.SectorCount()
	emitter := keyRecoveryEmitter(emit)

	updateRecoveryStep(&result, emitter, 1, KeyRecoveryRunning, 0, false, "step 1/5: scanning history, the user dictionary, and common keys")
	result.Scan, err = s.scanner.Scan(ctx, KeyScanRequest{Card: request.Card}, func(progress KeyScanProgress) {
		emitter(KeyRecoveryEvent{
			Step: 1, Stage: KeyRecoveryCommonKeys, Status: KeyRecoveryRunning,
			Progress: recoveryScanProgress(layout, progress.Sector, progress.KeyType),
			Message:  fmt.Sprintf("step 1/5: sector %d key %s, %d tried, %d found", progress.Sector, recoveryKeyTypeName(progress.KeyType), progress.Attempted, progress.Found),
			Sector:   progress.Sector, KeyType: progress.KeyType, KeyTypeSet: true,
			Attempted: progress.Attempted, Found: progress.Found,
		})
	})
	if err != nil {
		return failAndEmitKeyRecovery(result, emitter, 1, KeyRecoveryCommonKeys, err)
	}
	refreshRecoveryCoverage(&result, s.keys, layout)
	updateRecoveryStep(&result, emitter, 1, KeyRecoveryCompleted, 20, false, fmt.Sprintf("step 1/5 complete: %d/%d sectors have a verifiable key", result.CoveredSectors, result.TotalSectors))

	var nestedErr error
	if result.CoveredSectors == 0 {
		if s.mfcuk == nil || nfc.InferCardType(request.Card) != nfc.CardTypeMIFAREClassic1K || !device.CapabilitiesFor(request.Device).Darkside {
			reason := errors.New("no seed key is available and the current card or reader cannot run Darkside")
			return failAndEmitKeyRecovery(result, emitter, 2, KeyRecoveryDarkside, reason)
		}
		darksideTimeout := request.DarksideTimeout
		if darksideTimeout <= 0 {
			darksideTimeout = defaultRecoveryDarksideTimeout
		}
		nestedTimeout := request.NestedTimeout
		if nestedTimeout <= 0 {
			nestedTimeout = defaultRecoveryNestedTimeout
		}
		updateRecoveryStep(&result, emitter, 2, KeyRecoveryRunning, 0, true, fmt.Sprintf("step 2/5: Darkside is recovering the first key, maximum %s", darksideTimeout))
		result.MFCUK, err = s.mfcuk.Run(ctx, MFCUKPipelineRequest{
			TaskID: request.TaskID + "-darkside", Card: request.Card, Device: request.Device, Authorized: true,
			Timeout: darksideTimeout + nestedTimeout + 5*time.Minute, MFCUKTimeout: darksideTimeout,
			MFoCTimeout: nestedTimeout, TemporaryPolicy: request.TemporaryPolicy,
		}, func(event attack.AttackEvent) {
			step, stage := 2, KeyRecoveryDarkside
			if event.Engine == "mfoc" || event.State == attack.StateMFoCStage {
				step, stage = 3, KeyRecoveryNested
				if result.Steps[2].Status == KeyRecoveryPending {
					updateRecoveryStep(&result, emitter, 2, KeyRecoveryCompleted, 40, false, "step 2/5 complete: Darkside seed verified by libnfc")
					result.Steps[2].Status = KeyRecoveryRunning
				}
			}
			copy := event
			emitter(KeyRecoveryEvent{Step: step, Stage: stage, Status: KeyRecoveryRunning, Indeterminate: true, Message: fmt.Sprintf("step %d/5: %s", step, event.Message), Attack: &copy})
		})
		refreshRecoveryCoverage(&result, s.keys, layout)
		if result.CoveredSectors == 0 {
			return failAndEmitKeyRecovery(result, emitter, 2, KeyRecoveryDarkside, errOr(err, errors.New("Darkside produced no seed key accepted by libnfc verification")))
		}
		if err != nil && isRecoveryCancellation(err) {
			step, stage := 2, KeyRecoveryDarkside
			if result.Steps[2].Status != KeyRecoveryPending {
				step, stage = 3, KeyRecoveryNested
			}
			return failAndEmitKeyRecovery(result, emitter, step, stage, err)
		}
		if result.Steps[1].Status != KeyRecoveryCompleted {
			updateRecoveryStep(&result, emitter, 2, KeyRecoveryCompleted, 40, false, "step 2/5 complete: Darkside seed verified by libnfc")
		}
		if err != nil {
			nestedErr = err
			updateRecoveryStep(&result, emitter, 3, KeyRecoveryFailed, 60, false, "step 3/5 failed: "+err.Error())
		} else {
			updateRecoveryStep(&result, emitter, 3, KeyRecoveryCompleted, 60, false, "step 3/5 complete: Nested returned and its result was re-verified")
		}
	} else {
		updateRecoveryStep(&result, emitter, 2, KeyRecoverySkipped, 40, false, "step 2/5 skipped: a verified seed key already exists")
		if result.CoveredSectors == result.TotalSectors {
			updateRecoveryStep(&result, emitter, 3, KeyRecoverySkipped, 60, false, "step 3/5 skipped: every sector already has a verifiable key")
		} else if s.mfoc == nil || !device.CapabilitiesFor(request.Device).Nested {
			nestedErr = errors.New("the current reader does not enable Nested or MFOC is unavailable")
			updateRecoveryStep(&result, emitter, 3, KeyRecoveryUnavailable, 60, false, "step 3/5 unavailable: "+nestedErr.Error())
		} else {
			nestedTimeout := request.NestedTimeout
			if nestedTimeout <= 0 {
				nestedTimeout = defaultRecoveryNestedTimeout
			}
			updateRecoveryStep(&result, emitter, 3, KeyRecoveryRunning, 0, true, fmt.Sprintf("step 3/5: Nested is recovering the remaining keys, maximum %s", nestedTimeout))
			result.MFoC, nestedErr = s.mfoc.Run(ctx, MFoCRequest{
				TaskID: request.TaskID + "-nested", Card: request.Card, Device: request.Device, Authorized: true,
				Timeout: nestedTimeout, TemporaryPolicy: request.TemporaryPolicy,
			}, func(event attack.AttackEvent) {
				copy := event
				emitter(KeyRecoveryEvent{Step: 3, Stage: KeyRecoveryNested, Status: KeyRecoveryRunning, Indeterminate: true, Message: "step 3/5: " + event.Message, Attack: &copy})
			})
			if nestedErr != nil {
				if isRecoveryCancellation(nestedErr) {
					return failAndEmitKeyRecovery(result, emitter, 3, KeyRecoveryNested, nestedErr)
				}
				updateRecoveryStep(&result, emitter, 3, KeyRecoveryFailed, 60, false, "step 3/5 failed: "+nestedErr.Error())
			} else {
				updateRecoveryStep(&result, emitter, 3, KeyRecoveryCompleted, 60, false, "step 3/5 complete: Nested returned and its result was re-verified")
			}
		}
	}
	if nestedErr != nil && (keyRecoveryNestedRecoveryError(result) != nil || (!errors.Is(nestedErr, attack.ErrTimeout) && fatalCardOperation(nestedErr))) {
		return failAndEmitKeyRecovery(result, emitter, 3, KeyRecoveryNested, nestedErr)
	}

	refreshRecoveryCoverage(&result, s.keys, layout)
	if result.CoveredSectors == result.TotalSectors {
		updateRecoveryStep(&result, emitter, 4, KeyRecoverySkipped, 80, false, "step 4/5 skipped: current key coverage is sufficient; Hardnested is unnecessary")
	} else if s.hardnested == nil || !device.CapabilitiesFor(request.Device).Hardnested {
		message := "step 4/5 unavailable: the current reader does not enable Hardnested"
		updateRecoveryStep(&result, emitter, 4, KeyRecoveryUnavailable, 80, false, message)
		if nestedErr != nil {
			result.FailedStep, result.FailedStage, result.Reason = 3, KeyRecoveryNested, nestedErr.Error()
		} else {
			result.FailedStep, result.FailedStage, result.Reason = 4, KeyRecoveryHardnested, message
		}
	} else if availabilityErr := s.hardnested.Available(ctx); availabilityErr != nil {
		message := "step 4/5 unavailable: " + availabilityErr.Error()
		updateRecoveryStep(&result, emitter, 4, KeyRecoveryUnavailable, 80, false, message)
		if nestedErr != nil {
			result.FailedStep, result.FailedStage, result.Reason = 3, KeyRecoveryNested, nestedErr.Error()
		} else {
			result.FailedStep, result.FailedStage, result.Reason = 4, KeyRecoveryHardnested, message
		}
	} else {
		result.HardnestedAvailable = true
		hardnestedTimeout := request.HardnestedTimeout
		if hardnestedTimeout <= 0 {
			hardnestedTimeout = defaultRecoveryHardnestedTimeout
		}
		updateRecoveryStep(&result, emitter, 4, KeyRecoveryRunning, 0, true, fmt.Sprintf("step 4/5: Hardnested is recovering the remaining keys, maximum %s", hardnestedTimeout))
		var hardnestedErr error
		result.Hardnested, hardnestedErr = s.hardnested.Run(ctx, HardnestedRequest{
			TaskID: request.TaskID + "-hardnested", Card: request.Card, Device: request.Device, Authorized: true,
			Timeout: hardnestedTimeout, TemporaryPolicy: request.TemporaryPolicy,
		}, func(event attack.AttackEvent) {
			copy := event
			emitter(KeyRecoveryEvent{Step: 4, Stage: KeyRecoveryHardnested, Status: KeyRecoveryRunning, Indeterminate: true, Message: "step 4/5: " + event.Message, Attack: &copy})
		})
		refreshRecoveryCoverage(&result, s.keys, layout)
		if hardnestedErr != nil {
			if isRecoveryCancellation(hardnestedErr) {
				return failAndEmitKeyRecovery(result, emitter, 4, KeyRecoveryHardnested, hardnestedErr)
			}
			if result.Hardnested.External.RecoveryError != nil || (!errors.Is(hardnestedErr, attack.ErrTimeout) && fatalCardOperation(hardnestedErr)) {
				return failAndEmitKeyRecovery(result, emitter, 4, KeyRecoveryHardnested, hardnestedErr)
			}
			updateRecoveryStep(&result, emitter, 4, KeyRecoveryFailed, 80, false, "step 4/5 failed: "+hardnestedErr.Error())
			result.FailedStep, result.FailedStage, result.Reason = 4, KeyRecoveryHardnested, hardnestedErr.Error()
		} else {
			updateRecoveryStep(&result, emitter, 4, KeyRecoveryCompleted, 80, false, fmt.Sprintf("step 4/5 complete: key coverage %d/%d sectors", result.CoveredSectors, result.TotalSectors))
			result.FailedStep, result.FailedStage, result.Reason = 0, "", ""
		}
	}

	updateRecoveryStep(&result, emitter, 5, KeyRecoveryRunning, 0, true, "step 5/5: reading every sector with verified keys to confirm actual readability")
	result.Read, err = s.reader.ReadCard(ctx, DumpRequest{Card: request.Card, Device: request.Device, Keys: recoverySectorKeys(s.keys, request.Card, layout)})
	result.DumpComplete = result.Read.Complete()
	result.ReadableSectors = readableRecoverySectors(result.Read.Dump)
	refreshRecoveryCoverage(&result, s.keys, layout)
	if err != nil {
		return failAndEmitKeyRecovery(result, emitter, 5, KeyRecoveryReadiness, err)
	}
	if result.ReadableSectors == 0 {
		return failAndEmitKeyRecovery(result, emitter, 5, KeyRecoveryReadiness, errors.New("no sector passed a real read verification"))
	}
	updateRecoveryStep(&result, emitter, 5, KeyRecoveryCompleted, 100, false, fmt.Sprintf("step 5/5 complete: %d/%d sectors are actually readable", result.ReadableSectors, result.TotalSectors))
	if result.ReadableSectors == result.TotalSectors {
		result.Outcome = KeyRecoveryComplete
		result.FailedStep, result.FailedStage, result.Reason = 0, "", ""
	} else {
		result.Outcome = KeyRecoveryPartial
		if result.Reason == "" {
			result.FailedStep, result.FailedStage = 5, KeyRecoveryReadiness
			result.Reason = fmt.Sprintf("%d sectors remain unreadable", result.TotalSectors-result.ReadableSectors)
		}
	}
	return result, nil
}

func newKeyRecoverySteps() []KeyRecoveryStep {
	stages := []KeyRecoveryStage{KeyRecoveryCommonKeys, KeyRecoveryDarkside, KeyRecoveryNested, KeyRecoveryHardnested, KeyRecoveryReadiness}
	result := make([]KeyRecoveryStep, len(stages))
	for index, stage := range stages {
		result[index] = KeyRecoveryStep{Number: index + 1, Stage: stage, Label: recoveryStageLabel(stage), Status: KeyRecoveryPending}
	}
	return result
}

func updateRecoveryStep(result *KeyRecoveryResult, emit func(KeyRecoveryEvent), step int, status KeyRecoveryStepStatus, progress int, indeterminate bool, message string) {
	if step > 0 && step <= len(result.Steps) {
		result.Steps[step-1].Status = status
		result.Steps[step-1].Message = message
	}
	emit(KeyRecoveryEvent{Step: step, Stage: result.Steps[step-1].Stage, Status: status, Progress: progress, Indeterminate: indeterminate, Message: message})
}

func failAndEmitKeyRecovery(result KeyRecoveryResult, emit func(KeyRecoveryEvent), step int, stage KeyRecoveryStage, cause error) (KeyRecoveryResult, error) {
	result.Outcome, result.FailedStep, result.FailedStage, result.Reason = KeyRecoveryFailure, step, stage, cause.Error()
	if step > 0 && step <= len(result.Steps) {
		result.Steps[step-1].Status = KeyRecoveryFailed
		result.Steps[step-1].Message = cause.Error()
	}
	emit(KeyRecoveryEvent{Step: step, Stage: stage, Status: KeyRecoveryFailed, Progress: (step - 1) * 20, Message: fmt.Sprintf("step %d/5 failed: %v", step, cause)})
	return result, &KeyRecoveryError{Step: step, Stage: stage, Cause: cause}
}

func failKeyRecovery(result KeyRecoveryResult, step int, stage KeyRecoveryStage, cause error) (KeyRecoveryResult, error) {
	return failAndEmitKeyRecovery(result, func(KeyRecoveryEvent) {}, step, stage, cause)
}

func keyRecoveryEmitter(emit func(KeyRecoveryEvent)) func(KeyRecoveryEvent) {
	if emit == nil {
		return func(KeyRecoveryEvent) {}
	}
	return emit
}

func refreshRecoveryCoverage(result *KeyRecoveryResult, store *keys.Store, layout mifare.Layout) {
	sectors, _ := layout.SectorCount()
	covered := make(map[int]struct{}, sectors)
	matches := store.Verified(keys.CardID(result.Card))
	for _, match := range matches {
		if match.Sector >= 0 && match.Sector < sectors {
			covered[match.Sector] = struct{}{}
		}
	}
	result.VerifiedKeySlots = len(matches)
	result.CoveredSectors = len(covered)
}

func recoverySectorKeys(store *keys.Store, card nfc.CardInfo, layout mifare.Layout) []SectorKeys {
	sectors, _ := layout.SectorCount()
	result := make([]SectorKeys, sectors)
	for sector := range result {
		result[sector].Sector = sector
	}
	for _, match := range store.Verified(keys.CardID(card)) {
		if match.Sector < 0 || match.Sector >= sectors {
			continue
		}
		record, exists := store.Record(match.KeyID)
		if !exists {
			continue
		}
		key := record.Value
		if match.KeyType == nfc.KeyTypeA {
			result[match.Sector].KeyA = &key
		} else {
			result[match.Sector].KeyB = &key
		}
	}
	return result
}

func readableRecoverySectors(image mifare.Dump) int {
	if !image.Layout.Valid() {
		return 0
	}
	sectors, _ := image.Layout.SectorCount()
	readable := 0
	for sector := 0; sector < sectors; sector++ {
		first, _ := image.Layout.FirstBlock(sector)
		count, _ := image.Layout.BlocksInSector(sector)
		complete := true
		for block := first; block < first+count; block++ {
			trailer, _ := image.Layout.IsTrailer(block)
			if trailer {
				complete = complete && image.Blocks[block].KnownMask&0x03c0 == 0x03c0
			} else {
				complete = complete && image.Blocks[block].KnownMask == mifare.AllBytesKnown
			}
		}
		if complete {
			readable++
		}
	}
	return readable
}

func recoveryScanProgress(layout mifare.Layout, sector int, keyType nfc.KeyType) int {
	sectors, _ := layout.SectorCount()
	slot := sector * 2
	if keyType == nfc.KeyTypeB {
		slot++
	}
	return (slot + 1) * 20 / (sectors * 2)
}

func recoveryStageLabel(stage KeyRecoveryStage) string {
	switch stage {
	case KeyRecoveryCommonKeys:
		return "scan common keys"
	case KeyRecoveryDarkside:
		return "Darkside seed recovery"
	case KeyRecoveryNested:
		return "Nested remaining-key recovery"
	case KeyRecoveryHardnested:
		return "Hardnested recovery"
	case KeyRecoveryReadiness:
		return "real read verification"
	default:
		return string(stage)
	}
}

func recoveryKeyTypeName(keyType nfc.KeyType) string {
	if keyType == nfc.KeyTypeB {
		return "B"
	}
	return "A"
}

func errOr(value, fallback error) error {
	if value != nil {
		return value
	}
	return fallback
}

func isRecoveryCancellation(err error) bool {
	return errors.Is(err, context.Canceled) || errors.Is(err, nfc.ErrCanceled) || errors.Is(err, attack.ErrCancelled)
}

func keyRecoveryNestedRecoveryError(result KeyRecoveryResult) error {
	if result.MFoC.External.RecoveryError != nil {
		return result.MFoC.External.RecoveryError
	}
	return result.MFCUK.MFoC.External.RecoveryError
}
