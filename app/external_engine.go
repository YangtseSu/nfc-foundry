package app

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/attack"
	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func (s *Service) ExternalEngineFixtureStatus() ExternalEngineStatusDTO {
	status := ExternalEngineStatusDTO{ID: "nfcx-fake-engine"}
	if s.fixtureAttack == nil {
		status.Detail = "development fixture engine is not configured"
		return status
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := s.fixtureAttack.Available(ctx); err != nil {
		status.Detail = "development fixture engine is not installed"
		return status
	}
	status.Available = true
	status.Detail = "validates device handoff, logging, cancellation, and recovery only"
	return status
}

func (s *Service) MFoCStatus() ExternalEngineStatusDTO {
	status := ExternalEngineStatusDTO{ID: "mfoc"}
	if s.mfocLocator == nil {
		status.Detail = "MFOC is not configured"
		return status
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := attack.NewMFoCEngine(s.mfocLocator, attack.MFoCInvocation{}).Available(ctx); err != nil {
		status.Detail = err.Error()
		return status
	}
	status.Available = true
	status.Detail = "MFOC 0.10.7 · Nested starts only with verified NFC Foundry keys"
	return status
}

func (s *Service) MFCUKStatus() ExternalEngineStatusDTO {
	status := ExternalEngineStatusDTO{ID: "mfcuk-pipeline"}
	if s.mfcukPipeline == nil || s.mfocLocator == nil {
		status.Detail = "MFCUK pipeline is not configured"
		return status
	}
	ctx, cancel := context.WithTimeout(context.Background(), 6*time.Second)
	defer cancel()
	if err := attack.NewMFCUKEngine(s.mfocLocator, attack.MFCUKInvocation{}).Available(ctx); err != nil {
		status.Detail = err.Error()
		return status
	}
	if err := attack.NewMFoCEngine(s.mfocLocator, attack.MFoCInvocation{}).Available(ctx); err != nil {
		status.Detail = "MFCUK is available but the follow-up MFOC is unavailable: " + err.Error()
		return status
	}
	status.Available = true
	status.Detail = "MFCUK 0.3.8 → MFOC 0.10.7 · authorized Classic 1K only"
	return status
}

func (s *Service) StartMFoC(input MFoCStartRequestDTO) (TaskDTO, error) {
	return s.startMFoCFamily(mfoCFamilyMFoC, input.Authorized)
}

func (s *Service) StartHardnested(input HardnestedStartRequestDTO) (TaskDTO, error) {
	return s.startMFoCFamily(mfoCFamilyHardnested, input.Authorized)
}

type mfoCFamily int

const (
	mfoCFamilyMFoC mfoCFamily = iota
	mfoCFamilyHardnested
)

func (s *Service) startMFoCFamily(family mfoCFamily, authorized bool) (TaskDTO, error) {
	if !authorized {
		return TaskDTO{}, workflow.ErrMFoCAuthorization
	}
	if s.mfocLocator == nil {
		return TaskDTO{}, attack.ErrUnavailable
	}
	if family == mfoCFamilyHardnested {
		if s.hardnestedAttack == nil {
			return TaskDTO{}, attack.ErrUnavailable
		}
	} else if s.mfocAttack == nil {
		return TaskDTO{}, attack.ErrUnavailable
	}
	kind, name, label := "mfoc", "MFOC nested key recovery", "MFOC"
	timeout := 30 * time.Minute
	capabilityErr := workflow.ErrMFoCNested
	capability := func(capabilities device.Capabilities) bool { return capabilities.Nested }
	available := func(ctx context.Context) error {
		return attack.NewMFoCEngine(s.mfocLocator, attack.MFoCInvocation{}).Available(ctx)
	}
	run := func(ctx context.Context, request workflow.MFoCRequest, emit func(attack.AttackEvent)) (workflow.MFoCResult, error) {
		return s.mfocAttack.Run(ctx, request, emit)
	}
	switch family {
	case mfoCFamilyHardnested:
		kind, name, label = "hardnested", "Hardnested key recovery", "Hardnested"
		timeout = 6 * time.Hour
		capabilityErr = workflow.ErrHardnestedCapability
		capability = func(capabilities device.Capabilities) bool { return capabilities.Hardnested }
		available = s.hardnestedAttack.Available
		run = func(ctx context.Context, request workflow.MFoCRequest, emit func(attack.AttackEvent)) (workflow.MFoCResult, error) {
			return s.hardnestedAttack.Run(ctx, workflow.HardnestedRequest{
				TaskID: request.TaskID, Card: request.Card, Device: request.Device, Authorized: request.Authorized,
				Timeout: request.Timeout, TemporaryPolicy: request.TemporaryPolicy,
			}, emit)
		}
	}

	availabilityCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := available(availabilityCtx); err != nil {
		return TaskDTO{}, err
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Status != device.StatusReady && snapshot.Status != device.StatusPolling {
		if snapshot.Status == device.StatusBusy || snapshot.Status == device.StatusConnecting || snapshot.Status == device.StatusRecovering {
			return TaskDTO{}, nfc.ErrBusy
		}
		return TaskDTO{}, nfc.ErrNotOpen
	}
	if snapshot.Card == nil {
		return TaskDTO{}, ErrCardRequired
	}
	if !capability(snapshot.Capabilities) {
		return TaskDTO{}, capabilityErr
	}
	card := snapshot.Card.Clone()
	if len(s.keyStore.Verified(keys.CardID(card))) == 0 {
		return TaskDTO{}, workflow.ErrMFoCSeedRequired
	}
	deviceInfo := snapshot.Device
	return s.startOperation(kind, name, func(ctx context.Context, taskID string) error {
		result, runErr := run(ctx, workflow.MFoCRequest{
			TaskID: taskID, Card: card, Device: deviceInfo, Authorized: true,
			Timeout: timeout, TemporaryPolicy: attack.CleanupAlways,
		}, func(event attack.AttackEvent) {
			s.emitOperation(attackEventDTO(taskID, kind, event))
		})
		report := MFoCResultDTO{
			OutputValidated: result.External.Validated, Claimed: len(result.Claims), Added: result.Added,
			Existing: result.Existing, Rejected: result.Rejected, Conflicts: result.Conflicts,
		}
		if len(result.Claims) != 0 {
			s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
			s.emitter.Emit(KeyCatalogEventName, keyDTOs(s.keyStore))
			if err := s.persistKeys(); err != nil && runErr == nil {
				runErr = err
			}
		}
		if runErr == nil {
			report.FilledBytes, report.DataConflicts, report.KeyConflicts, runErr = s.readAndMergeAttackResult(ctx, card, deviceInfo)
			if runErr == nil {
				workbench := s.emitWorkbench()
				s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: kind, Type: TaskEventProgress, Phase: string(attack.StateMergingResult), Message: "re-read with libnfc and merged into the workbench", Workbench: &workbench})
			}
		}
		exitCode := result.External.ExitCode
		recoveryError := ""
		if result.External.RecoveryError != nil {
			recoveryError = result.External.RecoveryError.Error()
		}
		s.emitOperation(TaskEventDetailDTO{
			TaskID: taskID, Kind: kind, Type: TaskEventProgress, Phase: string(result.External.State),
			Engine: result.External.Engine, Version: result.External.Version, ExitCode: &exitCode, RecoveryError: recoveryError, MFoC: &report,
			Message: fmt.Sprintf("%s: claimed %d, marked %d new keys verified, %d conflicts, %d failed re-verification", label, report.Claimed, report.Added, report.Conflicts, report.Rejected),
		})
		return runErr
	})
}

func (s *Service) StartMFCUKPipeline(input MFCUKStartRequestDTO) (TaskDTO, error) {
	if s.mfcukPipeline == nil || s.mfocLocator == nil {
		return TaskDTO{}, attack.ErrUnavailable
	}
	if !input.Authorized {
		return TaskDTO{}, workflow.ErrMFCUKAuthorization
	}
	status := s.MFCUKStatus()
	if !status.Available {
		return TaskDTO{}, fmt.Errorf("%w: %s", attack.ErrUnavailable, status.Detail)
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Status != device.StatusReady && snapshot.Status != device.StatusPolling {
		if snapshot.Status == device.StatusBusy || snapshot.Status == device.StatusConnecting || snapshot.Status == device.StatusRecovering {
			return TaskDTO{}, nfc.ErrBusy
		}
		return TaskDTO{}, nfc.ErrNotOpen
	}
	if snapshot.Card == nil {
		return TaskDTO{}, ErrCardRequired
	}
	card := snapshot.Card.Clone()
	if nfc.InferCardType(card) != nfc.CardTypeMIFAREClassic1K {
		return TaskDTO{}, workflow.ErrMFCUKClassic1K
	}
	if !snapshot.Capabilities.Darkside {
		return TaskDTO{}, workflow.ErrMFCUKDarkside
	}
	if !snapshot.Capabilities.Nested {
		return TaskDTO{}, workflow.ErrMFoCNested
	}
	if len(s.keyStore.Verified(keys.CardID(card))) != 0 {
		return TaskDTO{}, workflow.ErrMFCUKSeedAlreadyKnown
	}
	deviceInfo := snapshot.Device
	return s.startOperation("mfcuk_pipeline", "MFCUK → MFOC automatic key recovery", func(ctx context.Context, taskID string) error {
		result, runErr := s.mfcukPipeline.Run(ctx, workflow.MFCUKPipelineRequest{
			TaskID: taskID, Card: card, Device: deviceInfo, Authorized: true, Timeout: 60 * time.Minute,
			MFCUKTimeout: 30 * time.Minute, MFoCTimeout: 30 * time.Minute, TemporaryPolicy: attack.CleanupAlways,
		}, func(event attack.AttackEvent) {
			s.emitOperation(attackEventDTO(taskID, "mfcuk_pipeline", event))
		})
		report := MFCUKResultDTO{
			CandidateOutputValidated: result.MFCUK.Validated, Candidates: len(result.Candidates), Verified: result.Verified,
			Existing: result.Existing, Rejected: result.Rejected, Conflicts: result.Conflicts,
			MFCUKVersion: result.MFCUK.Version, MFCUKDurationMillis: result.MFCUK.Duration().Milliseconds(),
			MFoCOutputValidated: result.MFoC.External.Validated, MFoCVersion: result.MFoC.External.Version,
			MFoCDurationMillis: result.MFoC.External.Duration().Milliseconds(), MFoCAdded: result.MFoC.Added, MFoCRejected: result.MFoC.Rejected,
		}
		if len(result.Candidates) != 0 || len(result.MFoC.Claims) != 0 {
			s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
			s.emitter.Emit(KeyCatalogEventName, keyDTOs(s.keyStore))
			if err := s.persistKeys(); err != nil && runErr == nil {
				runErr = err
			}
		}
		if runErr == nil {
			s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "mfcuk_pipeline", Type: TaskEventProgress, Phase: string(attack.StateReadingDump), Message: "two-stage key verification complete; re-reading the card"})
			report.FilledBytes, report.DataConflicts, report.KeyConflicts, runErr = s.readAndMergeAttackResult(ctx, card, deviceInfo)
			if runErr == nil {
				workbench := s.emitWorkbench()
				s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "mfcuk_pipeline", Type: TaskEventProgress, Phase: string(attack.StateMergingResult), Message: "re-read with libnfc and merged into the workbench", Workbench: &workbench})
			}
		}
		s.emitOperation(TaskEventDetailDTO{
			TaskID: taskID, Kind: "mfcuk_pipeline", Type: TaskEventProgress, Phase: string(result.MFoC.External.State), MFCUK: &report,
			Message: fmt.Sprintf("MFCUK candidates %d, verified %d, rejected %d; MFOC added %d verified keys", report.Candidates, report.Verified, report.Rejected, report.MFoCAdded),
		})
		return runErr
	})
}

func (s *Service) readAndMergeAttackResult(ctx context.Context, card nfc.CardInfo, deviceInfo nfc.DeviceInfo) (int, int, int, error) {
	layout, err := appLayoutForCard(card)
	if err != nil {
		return 0, 0, 0, err
	}
	readResult, readErr := s.dumpService.ReadCard(ctx, workflow.DumpRequest{
		Card: card, Device: deviceInfo, Keys: verifiedSectorKeys(s.keyStore, card, layout),
	})
	filled, dataConflicts, keyConflicts := 0, 0, 0
	if dumpHasKnownBytes(readResult.Dump) {
		merge, mergeErr := s.bench.MergeReadResult(readResult)
		if mergeErr != nil {
			return 0, 0, 0, mergeErr
		}
		filled, dataConflicts, keyConflicts = merge.FilledBytes, len(merge.Conflicts), len(merge.KeyConflicts)
	}
	return filled, dataConflicts, keyConflicts, readErr
}

func dumpHasKnownBytes(image mifare.Dump) bool {
	if !image.Layout.Valid() {
		return false
	}
	for _, block := range image.Blocks {
		if block.KnownMask != 0 {
			return true
		}
	}
	return false
}

func (s *Service) StartExternalEngineFixture() (TaskDTO, error) {
	if s.fixtureAttack == nil {
		return TaskDTO{}, attack.ErrUnavailable
	}
	availabilityCtx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := s.fixtureAttack.Available(availabilityCtx); err != nil {
		return TaskDTO{}, err
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Status != device.StatusReady && snapshot.Status != device.StatusPolling {
		if snapshot.Status == device.StatusBusy || snapshot.Status == device.StatusConnecting || snapshot.Status == device.StatusRecovering {
			return TaskDTO{}, nfc.ErrBusy
		}
		return TaskDTO{}, nfc.ErrNotOpen
	}
	return s.startOperation("external_fixture", "external engine framework self-test", func(ctx context.Context, taskID string) error {
		result, err := s.fixtureAttack.Run(ctx, attack.AttackRequest{
			TaskID:          taskID,
			Timeout:         30 * time.Second,
			TemporaryPolicy: attack.CleanupAlways,
		}, func(event attack.AttackEvent) {
			s.emitOperation(externalAttackEventDTO(taskID, event))
		})
		exitCode := result.ExitCode
		s.emitOperation(TaskEventDetailDTO{
			TaskID: taskID, Kind: "external_fixture", Type: TaskEventProgress, Phase: string(result.State),
			Engine: result.Engine, Version: result.Version, ExitCode: &exitCode,
			Message: fmt.Sprintf("external engine diagnostic: version %s, exit code %d, duration %s", result.Version, result.ExitCode, result.Duration().Round(time.Millisecond)),
		})
		if result.RecoveryError != nil {
			s.emitOperation(TaskEventDetailDTO{
				TaskID: taskID, Kind: "external_fixture", Type: TaskEventProgress, Phase: string(attack.StateReopeningDevice),
				Engine: result.Engine, RecoveryError: result.RecoveryError.Error(), Message: "external result kept, but reader recovery failed",
			})
		}
		return err
	})
}

func externalAttackEventDTO(taskID string, event attack.AttackEvent) TaskEventDetailDTO {
	return attackEventDTO(taskID, "external_fixture", event)
}

func attackEventDTO(taskID, kind string, event attack.AttackEvent) TaskEventDetailDTO {
	message := event.Message
	if message == "" {
		message = attackStateMessage(event.State)
	}
	return TaskEventDetailDTO{
		TaskID: taskID, Kind: kind, Type: TaskEventProgress, Phase: string(event.State),
		Engine: event.Engine, Stream: string(event.Stream), Log: event.Data, Sequence: event.Sequence,
		Message: message, Time: event.Time.Format(time.RFC3339Nano),
	}
}

func attackStateMessage(state attack.State) string {
	switch state {
	case attack.StateQueued:
		return "external task queued"
	case attack.StatePreflight:
		return "reconfirming the target card"
	case attack.StateReleasingDevice:
		return "stopping polling and releasing the reader"
	case attack.StateStarting:
		return "starting the controlled external program"
	case attack.StateRunning:
		return "external program is running"
	case attack.StateCancelling:
		return "terminating the external program and its children"
	case attack.StateValidatingResult:
		return "validating the external program result"
	case attack.StateReopeningDevice:
		return "reopening the reader"
	case attack.StateVerifyingKeys:
		return "re-verifying candidate keys with libnfc"
	case attack.StateMergingResult:
		return "merging verified results safely"
	case attack.StateMFCUKStage:
		return "stage 1/2: MFCUK darkside"
	case attack.StateVerifyingSeed:
		return "verifying MFCUK candidates with libnfc"
	case attack.StateMFoCStage:
		return "stage 2/2: MFOC nested"
	case attack.StateReadingDump:
		return "reading the card with verified keys"
	case attack.StateSucceeded:
		return "external program result validated"
	case attack.StateFailed:
		return "external program failed"
	case attack.StateCancelled:
		return "external program cancelled"
	default:
		return strings.TrimSpace(string(state))
	}
}
