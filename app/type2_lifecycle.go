package app

import (
	"context"
	"encoding/hex"
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

// Type2ChangeDTO is one old/new value in a lifecycle plan.
type Type2ChangeDTO struct {
	Target string `json:"target"`
	Old    string `json:"old"`
	New    string `json:"new"`
	OneWay bool   `json:"oneWay"`
}

// Type2LifecyclePlanDTO is the reviewed plan returned before a destructive
// Type 2 operation.
type Type2LifecyclePlanDTO struct {
	Operation   string              `json:"operation"`
	CardUID     string              `json:"cardUid"`
	SourceUID   string              `json:"sourceUid,omitempty"`
	Product     string              `json:"product"`
	ProductName string              `json:"productName"`
	Changes     []Type2ChangeDTO    `json:"changes"`
	Writes      []Type2PageWriteDTO `json:"writes"`
	Token       string              `json:"token"`
	BackupRoot  string              `json:"backupRoot,omitempty"`
}

// Type2LifecycleRequestDTO selects one copy, erase, lock, or password plan.
type Type2LifecycleRequestDTO struct {
	Operation        string          `json:"operation"`
	Source           *type2.Document `json:"source,omitempty"`
	IncludeProtected bool            `json:"includeProtected"`
	SameUIDOnly      bool            `json:"sameUidOnly"`
	StaticLock       string          `json:"staticLock,omitempty"`
	DynamicLock      string          `json:"dynamicLock,omitempty"`
	CCReadOnly       bool            `json:"ccReadOnly"`
	ConfigLock       bool            `json:"configLock"`
	NewPassword      string          `json:"newPassword,omitempty"`
	NewPack          string          `json:"newPack,omitempty"`
	Auth0            *int            `json:"auth0,omitempty"`
	Protect          bool            `json:"protect"`
	CurrentPassword  string          `json:"currentPassword,omitempty"`
	DryRun           bool            `json:"dryRun"`
	Token            string          `json:"token,omitempty"`
	Repeat           int             `json:"repeat,omitempty"`
	ContinueOnError  bool            `json:"continueOnError,omitempty"`
}

// PreviewType2Lifecycle reads the card and returns the exact plan without
// writing.
func (s *Service) PreviewType2Lifecycle(ctx context.Context, request Type2LifecycleRequestDTO) (Type2LifecyclePlanDTO, error) {
	workflowRequest, err := s.buildType2LifecycleRequest(request)
	if err != nil {
		return Type2LifecyclePlanDTO{}, err
	}
	plan, err := s.type2Service.PreviewType2Lifecycle(ctx, workflowRequest)
	if err != nil {
		return Type2LifecyclePlanDTO{}, err
	}
	dto := type2LifecyclePlanDTO(plan)
	dto.BackupRoot = s.type2BackupRoot
	return dto, nil
}

// ExecuteType2Lifecycle applies a reviewed plan.
func (s *Service) ExecuteType2Lifecycle(ctx context.Context, request Type2LifecycleRequestDTO) (Type2WriteResultDTO, error) {
	workflowRequest, err := s.buildType2LifecycleRequest(request)
	if err != nil {
		return Type2WriteResultDTO{}, err
	}
	result, err := s.type2Service.ExecuteType2Lifecycle(ctx, workflowRequest)
	if err != nil {
		return type2WriteFailure(result, request.DryRun), type2BackupError(err, result.BackupPath)
	}
	dto := s.finishType2Write(result, request.DryRun)
	dto.Operation = request.Operation
	dto.SourceUID = workflowRequest.Source.UID
	dto.Token = request.Token
	return dto, nil
}

// StartType2Lifecycle executes a previewed plan as an asynchronous task.
func (s *Service) StartType2Lifecycle(request Type2LifecycleRequestDTO) (TaskDTO, error) {
	if strings.TrimSpace(request.Token) == "" {
		return TaskDTO{}, nfc.NewError("Type 2 lifecycle", nfc.CodeInvalidArgument, "preview the operation before starting it", nil)
	}
	if request.Repeat < 0 || request.Repeat > 1000 {
		return TaskDTO{}, nfc.NewError("Type 2 lifecycle", nfc.CodeInvalidArgument, "repeat count must be between 1 and 1000", nil)
	}
	if request.Repeat > 1 && request.Operation != workflow.Type2OperationCopy {
		return TaskDTO{}, nfc.NewError("Type 2 lifecycle", nfc.CodeInvalidArgument, "only copy can target multiple tags", nil)
	}
	return s.startOperation("type2_lifecycle", "Type 2 "+request.Operation, func(ctx context.Context, taskID string) error {
		if request.Repeat > 1 {
			return s.runType2CopySequence(ctx, taskID, request)
		}
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_lifecycle", Type: TaskEventProgress, Message: "applying Type 2 " + request.Operation})
		result, err := s.ExecuteType2Lifecycle(ctx, request)
		if err != nil {
			return err
		}
		message := fmt.Sprintf("%s: wrote %d page(s), verified %d; backup %s", request.Operation, len(result.Written), len(result.Verified), result.Backup)
		event := TaskEventDetailDTO{TaskID: taskID, Kind: "type2_lifecycle", Type: TaskEventProgress, Progress: 100, Message: message}
		if result.Dump != nil {
			event.Type2 = result.Dump
		}
		s.emitOperation(event)
		return nil
	})
}

func (s *Service) runType2CopySequence(ctx context.Context, taskID string, request Type2LifecycleRequestDTO) error {
	firstToken := request.Token
	var completed int
	var lastErr error
	for index := 0; index < request.Repeat; index++ {
		if _, err := s.WaitForCard(ctx); err != nil {
			return err
		}
		request.Token = firstToken
		if index > 0 {
			request.Token = ""
		}
		plan, err := s.PreviewType2Lifecycle(ctx, request)
		if err == nil && index == 0 && firstToken != "" && plan.Token != firstToken {
			return nfc.NewError("Type 2 copy", nfc.CodeCardChanged, "card or planned writes changed after the confirmation", nil)
		}
		if err == nil {
			request.Token = plan.Token
			var result Type2WriteResultDTO
			result, err = s.ExecuteType2Lifecycle(ctx, request)
			if err == nil {
				completed++
				message := fmt.Sprintf("copy target %d/%d: wrote %d page(s), verified %d", index+1, request.Repeat, len(result.Written), len(result.Verified))
				event := TaskEventDetailDTO{TaskID: taskID, Kind: "type2_lifecycle", Type: TaskEventProgress, Message: message}
				if result.Dump != nil {
					event.Type2 = result.Dump
				}
				s.emitOperation(event)
			}
		}
		if err != nil {
			lastErr = err
			if !request.ContinueOnError {
				return err
			}
			s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_lifecycle", Type: TaskEventProgress, Message: "copy target failed: " + err.Error()})
		}
		if index+1 < request.Repeat {
			if err := s.WaitForCardRemoval(ctx); err != nil {
				return err
			}
		}
	}
	if lastErr != nil {
		return lastErr
	}
	s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_lifecycle", Type: TaskEventProgress, Progress: 100, Message: fmt.Sprintf("copied to %d tags", completed)})
	return nil
}

func (s *Service) buildType2LifecycleRequest(request Type2LifecycleRequestDTO) (workflow.Type2LifecycleRequest, error) {
	card, err := s.type2Card()
	if err != nil {
		return workflow.Type2LifecycleRequest{}, err
	}
	password, err := parseType2Password(request.CurrentPassword)
	if err != nil {
		return workflow.Type2LifecycleRequest{}, err
	}
	workflowRequest := workflow.Type2LifecycleRequest{
		Card: card, Operation: request.Operation, IncludeProtected: request.IncludeProtected,
		SameUIDOnly: request.SameUIDOnly,
		Password:    password, DryRun: request.DryRun, ExpectedToken: request.Token,
	}
	switch request.Operation {
	case workflow.Type2OperationCopy:
		if request.Source != nil {
			workflowRequest.Source = *request.Source
		} else {
			source, ok := s.type2SourceDocument()
			if !ok {
				return workflow.Type2LifecycleRequest{}, nfc.NewError("Type 2 copy", nfc.CodeInvalidArgument, "load a source dump first", nil)
			}
			workflowRequest.Source = source
		}
	case workflow.Type2OperationErase:
	case workflow.Type2OperationLock:
		lock, err := parseType2LockOptions(request)
		if err != nil {
			return workflow.Type2LifecycleRequest{}, err
		}
		workflowRequest.Lock = lock
	case workflow.Type2OperationPasswordSet:
		options, err := parseType2PasswordOptions(request)
		if err != nil {
			return workflow.Type2LifecycleRequest{}, err
		}
		workflowRequest.PasswordOptions = options
	case workflow.Type2OperationPasswordRemove:
	default:
		return workflow.Type2LifecycleRequest{}, nfc.NewError("Type 2 lifecycle", nfc.CodeInvalidArgument,
			fmt.Sprintf("unknown lifecycle operation %q", request.Operation), nil)
	}
	return workflowRequest, nil
}

func parseType2LockOptions(request Type2LifecycleRequestDTO) (type2.LockOptions, error) {
	var options type2.LockOptions
	if strings.TrimSpace(request.StaticLock) != "" {
		value, err := hex.DecodeString(normalizeHexValue(request.StaticLock))
		if err != nil || len(value) != 2 {
			return options, nfc.NewError("Type 2 lock", nfc.CodeInvalidArgument, "static lock must contain 4 hexadecimal digits", nil)
		}
		lock := uint16(value[0])<<8 | uint16(value[1])
		options.StaticLock = &lock
	}
	if strings.TrimSpace(request.DynamicLock) != "" {
		value, err := hex.DecodeString(normalizeHexValue(request.DynamicLock))
		if err != nil || len(value) != 3 {
			return options, nfc.NewError("Type 2 lock", nfc.CodeInvalidArgument, "dynamic lock must contain 6 hexadecimal digits", nil)
		}
		var lock [3]byte
		copy(lock[:], value)
		options.DynamicLock = &lock
	}
	options.CCReadOnly = request.CCReadOnly
	options.ConfigLock = request.ConfigLock
	return options, nil
}

func parseType2PasswordOptions(request Type2LifecycleRequestDTO) (type2.PasswordOptions, error) {
	var options type2.PasswordOptions
	password, err := parseType2Password(request.NewPassword)
	if err != nil || len(password) != 4 {
		return options, nfc.NewError("Type 2 password", nfc.CodeInvalidArgument, "new password must contain 8 hexadecimal digits", nil)
	}
	pack, err := hex.DecodeString(normalizeHexValue(request.NewPack))
	if err != nil || len(pack) != 2 {
		return options, nfc.NewError("Type 2 password", nfc.CodeInvalidArgument, "PACK must contain 4 hexadecimal digits", nil)
	}
	copy(options.Password[:], password)
	copy(options.Pack[:], pack)
	options.Auth0 = -1
	if request.Auth0 != nil {
		options.Auth0 = *request.Auth0
	}
	options.Protect = request.Protect
	return options, nil
}

func type2ChangeDTOs(changes []type2.Change) []Type2ChangeDTO {
	if len(changes) == 0 {
		return nil
	}
	result := make([]Type2ChangeDTO, len(changes))
	for index, change := range changes {
		result[index] = Type2ChangeDTO{Target: change.Target, Old: change.Old, New: change.New, OneWay: change.OneWay}
	}
	return result
}

func type2LifecyclePlanDTO(plan workflow.Type2LifecyclePlan) Type2LifecyclePlanDTO {
	changes := make([]Type2ChangeDTO, len(plan.Changes))
	for index, change := range plan.Changes {
		changes[index] = Type2ChangeDTO{Target: change.Target, Old: change.Old, New: change.New, OneWay: change.OneWay}
	}
	writes := make([]Type2PageWriteDTO, len(plan.Writes))
	for index, write := range plan.Writes {
		writes[index] = Type2PageWriteDTO{Page: write.Page, Hex: write.Hex}
	}
	return Type2LifecyclePlanDTO{
		Operation: plan.Operation, CardUID: formatHex(plan.Card.UID), SourceUID: plan.SourceUID,
		Product: string(plan.Document.Product), ProductName: plan.Document.ProductName,
		Changes: changes, Writes: writes, Token: plan.Token,
	}
}
