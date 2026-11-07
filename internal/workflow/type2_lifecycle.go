package workflow

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

const (
	Type2OperationCopy           = "copy"
	Type2OperationErase          = "erase"
	Type2OperationLock           = "lock"
	Type2OperationPasswordSet    = "password-set"
	Type2OperationPasswordRemove = "password-remove"
	Type2OperationProtection     = "protect"
)

// Type2LifecycleRequest describes one copy, erase, lock, or password change.
type Type2LifecycleRequest struct {
	TaskID           string
	Card             nfc.CardInfo
	Device           nfc.DeviceInfo
	Operation        string
	Source           type2.Document
	IncludeProtected bool
	SameUIDOnly      bool
	Lock             type2.LockOptions
	PasswordOptions  type2.PasswordOptions
	Protection       type2.ProtectionWriteOptions
	Password         []byte
	Pack             []byte
	DryRun           bool
	ExpectedToken    string
}

// Type2LifecyclePlan is the reviewed page plan and its confirmation changes.
type Type2LifecyclePlan struct {
	Operation string
	Card      nfc.CardInfo
	Document  type2.Document
	SourceUID string
	Writes    []type2.PageWrite
	Changes   []type2.Change
	Token     string
}

// Type2PlanToken binds a confirmation to the selected card, current image,
// and exact writes. Any image change invalidates the preview.
func Type2PlanToken(operation string, card nfc.CardInfo, document type2.Document, writes []type2.PageWrite) string {
	hash := sha256.New()
	fmt.Fprintf(hash, "%s\n%s\n% X\n%02X\n%s\n", operation, strings.ToUpper(hex.EncodeToString(card.UID)), card.ATQA, card.SAK, document.Product)
	for _, page := range document.Pages {
		fmt.Fprintf(hash, "%s\n", strings.ToUpper(page))
	}
	for _, write := range writes {
		fmt.Fprintf(hash, "%d:%s\n", write.Page, strings.ToUpper(write.Hex))
	}
	return hex.EncodeToString(hash.Sum(nil))
}

// PreviewType2Lifecycle reads the current image, validates the operation, and
// returns the exact old/new plan without writing.
func (s *Type2Service) PreviewType2Lifecycle(ctx context.Context, request Type2LifecycleRequest) (Type2LifecyclePlan, error) {
	request.Card = request.Card.Clone()
	request.Password = cloneBytesCopy(request.Password)
	request.Pack = cloneBytesCopy(request.Pack)
	if err := validateType2Card(request.Card); err != nil {
		return Type2LifecyclePlan{}, err
	}
	var plan Type2LifecyclePlan
	err := s.withType2Reader(ctx, request.Card, func(operationCtx context.Context, _ nfc.Reader, transport nfc.RawTransceiver) error {
		session, err := type2.OpenSession(operationCtx, transport)
		if err != nil {
			return err
		}
		defer session.Close(context.WithoutCancel(operationCtx))
		current, err := type2.ReadSession(operationCtx, session, request.Card, type2.ReadOptions{Password: request.Password, Pack: request.Pack})
		if err != nil {
			return err
		}
		plan, err = buildType2LifecyclePlan(request, current)
		return err
	})
	return plan, err
}

// ExecuteType2Lifecycle revalidates the preview plan and applies it with a
// full-image backup and read-back verification.
func (s *Type2Service) ExecuteType2Lifecycle(ctx context.Context, request Type2LifecycleRequest) (Type2WriteResult, error) {
	request.Card = request.Card.Clone()
	request.Password = cloneBytesCopy(request.Password)
	request.Pack = cloneBytesCopy(request.Pack)
	if err := validateType2Card(request.Card); err != nil {
		return Type2WriteResult{}, err
	}
	result := Type2WriteResult{Card: request.Card}
	err := s.withType2Reader(ctx, request.Card, func(operationCtx context.Context, reader nfc.Reader, transport nfc.RawTransceiver) error {
		session, err := type2.OpenSession(operationCtx, transport)
		if err != nil {
			return err
		}
		defer session.Close(context.WithoutCancel(operationCtx))
		current, err := type2.ReadSession(operationCtx, session, request.Card, type2.ReadOptions{Password: request.Password, Pack: request.Pack})
		if err != nil {
			return err
		}
		plan, err := buildType2LifecyclePlan(request, current)
		if err != nil {
			return err
		}
		if request.ExpectedToken != "" && request.ExpectedToken != plan.Token {
			return nfc.NewError("Type 2 lifecycle", nfc.CodeCardChanged, "card or planned writes changed after the confirmation", nil)
		}
		result.Changes = plan.Changes
		return s.applyWrites(operationCtx, reader, session, current, plan.Writes, true, true, request.Password, request.Pack, request.DryRun, &result)
	})
	return result, err
}

func buildType2LifecyclePlan(request Type2LifecycleRequest, current type2.Document) (Type2LifecyclePlan, error) {
	if requiresType2Password(current) && len(request.Password) == 0 {
		return Type2LifecyclePlan{}, nfc.NewError("Type 2 lifecycle", nfc.CodeAuthenticationFailed,
			"target is password protected; supply the current password", nil)
	}
	plan := Type2LifecyclePlan{
		Operation: request.Operation, Card: request.Card.Clone(), Document: current,
	}
	var err error
	switch request.Operation {
	case Type2OperationCopy:
		plan.SourceUID = request.Source.UID
		plan.Writes, plan.Changes, err = type2.PlanCopy(request.Source, current, type2.CopyOptions{
			IncludeProtected: request.IncludeProtected, SameUIDOnly: request.SameUIDOnly,
		})
	case Type2OperationErase:
		plan.Writes, plan.Changes, err = type2.PlanErase(current)
	case Type2OperationLock:
		plan.Writes, plan.Changes, err = type2.PlanLock(current, request.Lock)
	case Type2OperationPasswordSet:
		plan.Writes, plan.Changes, err = type2.PlanPasswordSet(current, request.PasswordOptions)
	case Type2OperationPasswordRemove:
		plan.Writes, plan.Changes, err = type2.PlanPasswordRemove(current)
	case Type2OperationProtection:
		plan.Writes, plan.Changes, err = type2.PlanProtectionWrites(current, request.Protection)
	default:
		return Type2LifecyclePlan{}, nfc.NewError("Type 2 lifecycle", nfc.CodeInvalidArgument,
			fmt.Sprintf("unknown lifecycle operation %q", request.Operation), nil)
	}
	if err != nil {
		return Type2LifecyclePlan{}, nfc.NewError("plan Type 2 "+request.Operation, nfc.CodeInvalidArgument, err.Error(), err)
	}
	plan.Token = Type2PlanToken(request.Operation, request.Card, current, plan.Writes)
	return plan, nil
}

func requiresType2Password(document type2.Document) bool {
	return document.Protection.Protect && document.Protection.Auth0 >= 0 && document.Protection.Auth0 != 0xFF
}
