package app

import (
	"context"
	"encoding/hex"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

const type2ProtectionPhrase = "PROTECTED"

// type2ProtectionTokenTTL bounds how long a reviewed protection plan stays
// valid. It is a variable so tests can exercise expiry without waiting.
var type2ProtectionTokenTTL = 5 * time.Minute

// Type2ProtectionWriteRequestDTO selects individual Type 2 protection fields.
// Nil leaves a field unchanged. Confirmation must be the exact word PROTECTED.
type Type2ProtectionWriteRequestDTO struct {
	Auth0           *int    `json:"auth0,omitempty"`
	Access          *byte   `json:"access,omitempty"`
	Password        *string `json:"password,omitempty"`
	Pack            *string `json:"pack,omitempty"`
	DynamicLock     *string `json:"dynamicLock,omitempty"`
	StaticLock      *string `json:"staticLock,omitempty"`
	Confirmation    string  `json:"confirmation"`
	Token           string  `json:"token,omitempty"`
	CurrentPassword string  `json:"currentPassword,omitempty"`
}

// Type2ProtectionPreflightDTO is the exact reviewed protection plan.
type Type2ProtectionPreflightDTO struct {
	CardUID     string              `json:"cardUid"`
	Product     string              `json:"product"`
	ProductName string              `json:"productName"`
	Changes     []Type2ChangeDTO    `json:"changes"`
	Writes      []Type2PageWriteDTO `json:"writes"`
	Token       string              `json:"token"`
	BackupRoot  string              `json:"backupRoot,omitempty"`
}

// PreflightType2Protection reads the tag, validates the requested fields, and
// returns the page plan and a short-lived confirmation token.
func (s *Service) PreflightType2Protection(request Type2ProtectionWriteRequestDTO) (Type2ProtectionPreflightDTO, error) {
	workflowRequest, err := s.type2ProtectionWorkflowRequest(request)
	if err != nil {
		return Type2ProtectionPreflightDTO{}, err
	}
	plan, err := s.type2Service.PreviewType2Lifecycle(context.Background(), workflowRequest)
	if err != nil {
		return Type2ProtectionPreflightDTO{}, err
	}
	if len(plan.Writes) == 0 {
		return Type2ProtectionPreflightDTO{}, nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument,
			"no protection field would change", nil)
	}
	expires := time.Now().Add(type2ProtectionTokenTTL).Unix()
	return Type2ProtectionPreflightDTO{
		CardUID: formatHex(plan.Card.UID), Product: string(plan.Document.Product),
		ProductName: plan.Document.ProductName, Changes: type2ChangeDTOs(plan.Changes),
		Writes: type2PageWriteDTOs(plan.Writes), Token: plan.Token + "." + strconv.FormatInt(expires, 10),
		BackupRoot: s.type2BackupRoot,
	}, nil
}

// StartType2ProtectionWrite applies a reviewed protection plan as an
// asynchronous task.
func (s *Service) StartType2ProtectionWrite(request Type2ProtectionWriteRequestDTO) (TaskDTO, error) {
	if request.Confirmation != type2ProtectionPhrase {
		return TaskDTO{}, nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument,
			"type PROTECTED exactly to confirm the protection write", nil)
	}
	expectedToken, err := parseType2ProtectionToken(request.Token, time.Now())
	if err != nil {
		return TaskDTO{}, err
	}
	workflowRequest, err := s.type2ProtectionWorkflowRequest(request)
	if err != nil {
		return TaskDTO{}, err
	}
	workflowRequest.ExpectedToken = expectedToken
	return s.startOperation("type2_protection", "Write Type 2 protection", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_protection", Type: TaskEventProgress, Message: "writing Type 2 protection"})
		result, err := s.type2Service.ExecuteType2Lifecycle(ctx, workflowRequest)
		if err != nil {
			return type2BackupError(err, result.BackupPath)
		}
		dto := s.finishType2Write(result, false)
		dto.Operation = workflow.Type2OperationProtection
		dto.Token = expectedToken
		message := fmt.Sprintf("protect: wrote %d page(s), verified %d; backup %s", len(dto.Written), len(dto.Verified), dto.Backup)
		event := TaskEventDetailDTO{TaskID: taskID, Kind: "type2_protection", Type: TaskEventProgress, Progress: 100, Message: message}
		if dto.Dump != nil {
			event.Type2 = dto.Dump
		}
		s.emitOperation(event)
		return nil
	})
}

func (s *Service) type2ProtectionWorkflowRequest(request Type2ProtectionWriteRequestDTO) (workflow.Type2LifecycleRequest, error) {
	card, err := s.type2Card(true)
	if err != nil {
		return workflow.Type2LifecycleRequest{}, err
	}
	password, err := parseType2Password(request.CurrentPassword)
	if err != nil {
		return workflow.Type2LifecycleRequest{}, err
	}
	options, err := parseType2ProtectionOptions(request)
	if err != nil {
		return workflow.Type2LifecycleRequest{}, err
	}
	return workflow.Type2LifecycleRequest{
		Card: card, Operation: workflow.Type2OperationProtection,
		Protection: options, Password: password,
	}, nil
}

func parseType2ProtectionOptions(request Type2ProtectionWriteRequestDTO) (type2.ProtectionWriteOptions, error) {
	var options type2.ProtectionWriteOptions
	selected := false
	if request.Auth0 != nil {
		if *request.Auth0 < 0 || *request.Auth0 > 255 {
			return options, nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument, "AUTH0 must be between 0 and 255", nil)
		}
		value := *request.Auth0
		options.Auth0 = &value
		selected = true
	}
	if request.Access != nil {
		value := *request.Access
		options.Access = &value
		selected = true
	}
	if request.Password != nil {
		decoded, err := parseType2Password(*request.Password)
		if err != nil || len(decoded) != 4 {
			return options, nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument, "password must contain exactly 8 hexadecimal digits", nil)
		}
		var value [4]byte
		copy(value[:], decoded)
		options.Password = &value
		selected = true
	}
	if request.Pack != nil {
		value, err := parseType2FixedHex("PACK", *request.Pack, 2)
		if err != nil {
			return options, err
		}
		var pack [2]byte
		copy(pack[:], value)
		options.Pack = &pack
		selected = true
	}
	if request.DynamicLock != nil {
		value, err := parseType2FixedHex("dynamic lock", *request.DynamicLock, 3)
		if err != nil {
			return options, err
		}
		var lock [3]byte
		copy(lock[:], value)
		options.DynamicLock = &lock
		selected = true
	}
	if request.StaticLock != nil {
		value, err := parseType2FixedHex("static lock", *request.StaticLock, 2)
		if err != nil {
			return options, err
		}
		lock := uint16(value[0])<<8 | uint16(value[1])
		options.StaticLock = &lock
		selected = true
	}
	if !selected {
		return options, nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument, "select at least one protection field", nil)
	}
	return options, nil
}

func parseType2FixedHex(label, value string, size int) ([]byte, error) {
	decoded, err := hex.DecodeString(normalizeHexValue(value))
	if err != nil || len(decoded) != size {
		return nil, nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument,
			fmt.Sprintf("%s must contain exactly %d hexadecimal digits", label, size*2), nil)
	}
	return decoded, nil
}

func parseType2ProtectionToken(value string, now time.Time) (string, error) {
	hash, expiryText, ok := strings.Cut(value, ".")
	if !ok || len(hash) != 64 {
		return "", nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument, "preview the protection write again", nil)
	}
	if _, err := hex.DecodeString(hash); err != nil {
		return "", nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument, "preview the protection write again", nil)
	}
	expiry, err := strconv.ParseInt(expiryText, 10, 64)
	if err != nil {
		return "", nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument, "preview the protection write again", nil)
	}
	if !now.Before(time.Unix(expiry, 0)) {
		return "", nfc.NewError("Type 2 protection", nfc.CodeInvalidArgument,
			"the protection confirmation expired; preview the write again", nil)
	}
	return hash, nil
}

func type2PageWriteDTOs(writes []type2.PageWrite) []Type2PageWriteDTO {
	result := make([]Type2PageWriteDTO, len(writes))
	for index, write := range writes {
		result[index] = Type2PageWriteDTO{Page: write.Page, Hex: write.Hex}
	}
	return result
}
