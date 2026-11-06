package workflow

import (
	"bytes"
	"context"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

// Type2ReadRequest identifies the selected card and optional authentication.
type Type2ReadRequest struct {
	Card     nfc.CardInfo
	Password []byte
	Pack     []byte
}

// Type2WriteRequest carries validated page writes and authentication.
type Type2WriteRequest struct {
	TaskID           string
	Card             nfc.CardInfo
	Device           nfc.DeviceInfo
	Product          type2.Product
	Writes           []type2.PageWrite
	Password         []byte
	Pack             []byte
	IncludeProtected bool
	DryRun           bool
}

// Type2WriteResult reports exactly which pages were changed and verified.
type Type2WriteResult struct {
	Card       nfc.CardInfo
	Document   type2.Document
	BackupPath string
	Planned    []int
	Written    []int
	Verified   []int
	Skipped    []int
}

// Type2RestoreRequest restores a validated dump.
type Type2RestoreRequest struct {
	TaskID           string
	Card             nfc.CardInfo
	Device           nfc.DeviceInfo
	Document         type2.Document
	Password         []byte
	Pack             []byte
	IncludeProtected bool
	DryRun           bool
}

// Type2NDEFWriteRequest writes one raw NDEF message.
type Type2NDEFWriteRequest struct {
	TaskID   string
	Card     nfc.CardInfo
	Device   nfc.DeviceInfo
	Message  []byte
	Password []byte
	Pack     []byte
	DryRun   bool
}

// Type2Service serializes Type 2 operations through the device manager.
type Type2Service struct {
	devices ReaderExecutor
	backups Type2BackupStore
}

func NewType2Service(devices ReaderExecutor, backups Type2BackupStore) *Type2Service {
	return &Type2Service{devices: devices, backups: backups}
}

// Identify probes the selected Type 2 tag without writing.
func (s *Type2Service) Identify(ctx context.Context, card nfc.CardInfo) (type2.Identity, error) {
	card = card.Clone()
	if err := validateType2Card(card); err != nil {
		return type2.Identity{}, err
	}
	var identity type2.Identity
	err := s.withType2Reader(ctx, card, func(operationCtx context.Context, _ nfc.Reader, transport nfc.RawTransceiver) error {
		var err error
		identity, err = type2.Identify(operationCtx, transport)
		return err
	})
	return identity, err
}

// Read returns a validated full-memory dump.
func (s *Type2Service) Read(ctx context.Context, request Type2ReadRequest) (type2.Document, error) {
	request.Card = request.Card.Clone()
	request.Password = cloneBytesCopy(request.Password)
	request.Pack = cloneBytesCopy(request.Pack)
	if err := validateType2Card(request.Card); err != nil {
		return type2.Document{}, err
	}
	var document type2.Document
	err := s.withType2Reader(ctx, request.Card, func(operationCtx context.Context, _ nfc.Reader, transport nfc.RawTransceiver) error {
		var err error
		document, err = type2.Read(operationCtx, transport, request.Card, type2.ReadOptions{Password: request.Password, Pack: request.Pack})
		return err
	})
	return document, err
}

// Write applies page writes with a mandatory full-image backup and immediate
// read-back. User pages are the default; protected pages require the explicit
// IncludeProtected flag from the confirmed restore path.
func (s *Type2Service) Write(ctx context.Context, request Type2WriteRequest) (Type2WriteResult, error) {
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
		if request.Product != "" && current.Product != request.Product {
			return nfc.NewError("Type 2 write", nfc.CodeCardChanged, "selected product does not match the dump", nil)
		}
		return s.applyWrites(operationCtx, reader, session, current, request.Writes, request.IncludeProtected, request.Password, request.Pack, request.DryRun, &result)
	})
	return result, err
}

// Restore applies a validated dump to the selected card.
func (s *Type2Service) Restore(ctx context.Context, request Type2RestoreRequest) (Type2WriteResult, error) {
	request.Card = request.Card.Clone()
	if err := request.Document.ValidateForCard(request.Card); err != nil {
		return Type2WriteResult{}, err
	}
	writes, err := type2.RestorePlan(request.Document, request.IncludeProtected)
	if err != nil {
		return Type2WriteResult{}, nfc.NewError("plan Type 2 restore", nfc.CodeInvalidArgument, err.Error(), err)
	}
	return s.Write(ctx, Type2WriteRequest{
		TaskID: request.TaskID, Card: request.Card, Device: request.Device,
		Product: request.Document.Product, Writes: writes,
		Password: request.Password, Pack: request.Pack, IncludeProtected: request.IncludeProtected, DryRun: request.DryRun,
	})
}

// WriteNDEF validates a message against the current tag and writes it with a
// full-image backup and read-back verification.
func (s *Type2Service) WriteNDEF(ctx context.Context, request Type2NDEFWriteRequest) (Type2WriteResult, error) {
	request.Card = request.Card.Clone()
	request.Message = cloneBytesCopy(request.Message)
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
		writes, err := type2.PlanNDEFWrites(current, request.Message)
		if err != nil {
			return nfc.NewError("plan NDEF write", nfc.CodeInvalidArgument, err.Error(), err)
		}
		if err := s.applyWrites(operationCtx, reader, session, current, writes, false, request.Password, request.Pack, request.DryRun, &result); err != nil {
			return err
		}
		if request.DryRun {
			return nil
		}
		area, err := type2.BuildNDEFArea(current.CC.Capacity, request.Message)
		if err != nil {
			return err
		}
		finalArea, err := result.Document.NDEFArea()
		if err != nil {
			return err
		}
		if !bytes.Equal(finalArea, area) {
			return nfc.NewError("verify NDEF", nfc.CodeVerificationFailed, "NDEF area does not match the requested message", nil)
		}
		return nil
	})
	return result, err
}

func (s *Type2Service) applyWrites(
	ctx context.Context,
	reader nfc.Reader,
	session *type2.Session,
	current type2.Document,
	writes []type2.PageWrite,
	includeProtected bool,
	password []byte,
	pack []byte,
	dryRun bool,
	result *Type2WriteResult,
) error {
	geometry, err := current.Geometry()
	if err != nil {
		return err
	}
	seen := make(map[int]struct{}, len(writes))
	decoded := make([][4]byte, len(writes))
	for index, write := range writes {
		if _, exists := seen[write.Page]; exists {
			return nfc.NewError("Type 2 write", nfc.CodeInvalidArgument, fmt.Sprintf("page %d is planned more than once", write.Page), nil)
		}
		seen[write.Page] = struct{}{}
		if includeProtected {
			if err := type2.ValidateWritablePage(geometry, write.Page); err != nil {
				return nfc.NewError("Type 2 write", nfc.CodeInvalidArgument, err.Error(), err)
			}
		} else if err := type2.ValidateUserPage(geometry, write.Page); err != nil {
			return nfc.NewError("Type 2 protected write", nfc.CodeInvalidArgument, "protected Type 2 pages require the confirmed restore path", err)
		}
		data, err := write.Data()
		if err != nil {
			return err
		}
		decoded[index] = data
	}
	result.Document = current
	if len(writes) == 0 {
		return nil
	}
	if dryRun {
		for index, write := range writes {
			actual, err := session.ReadPage(ctx, write.Page)
			if err != nil {
				return err
			}
			if actual == decoded[index] {
				result.Skipped = append(result.Skipped, write.Page)
			} else {
				result.Planned = append(result.Planned, write.Page)
			}
		}
		return nil
	}
	if s.backups == nil {
		return nfc.NewError("Type 2 write", nfc.CodeUnsupported, "Type 2 backup storage is unavailable", nil)
	}
	backupPath, err := s.backups.Save(ctx, current)
	if err != nil {
		return nfc.NewError("Type 2 write", nfc.CodeIO, "could not save the Type 2 backup; the tag was not modified", err)
	}
	result.BackupPath = backupPath

	updatedPassword := cloneBytesCopy(password)
	updatedPack := cloneBytesCopy(pack)
	for index, write := range writes {
		data := decoded[index]
		actual, err := session.ReadPage(ctx, write.Page)
		if err != nil {
			return err
		}
		if actual == data {
			result.Skipped = append(result.Skipped, write.Page)
		} else {
			result.Planned = append(result.Planned, write.Page)
			if err := session.WritePage(ctx, write.Page, data); err != nil {
				return err
			}
			result.Written = append(result.Written, write.Page)
			actual, err = session.ReadPage(ctx, write.Page)
			if err != nil {
				return err
			}
			if actual != data {
				return nfc.NewError("verify Type 2 page", nfc.CodeVerificationFailed,
					fmt.Sprintf("page %d readback % X does not match written % X", write.Page, actual, data), nil)
			}
			result.Verified = append(result.Verified, write.Page)
		}
		if write.Page == geometry.PasswordPage {
			updatedPassword = append([]byte(nil), data[:]...)
		}
		if write.Page == geometry.PackPage {
			updatedPack = append([]byte(nil), data[0:2]...)
		}
	}

	if err := session.Deactivate(ctx); err != nil {
		return err
	}
	actualCard, err := reader.CardInfo(ctx)
	if err != nil {
		return err
	}
	if !nfc.SameCard(result.Card, actualCard) {
		return nfc.NewError("verify Type 2 card", nfc.CodeCardChanged, "card identity changed during the write", nil)
	}
	if err := session.Activate(ctx); err != nil {
		return err
	}
	final, err := type2.ReadSession(ctx, session, result.Card, type2.ReadOptions{Password: updatedPassword, Pack: updatedPack})
	if err != nil {
		return err
	}
	finalPages, err := final.PageBytes()
	if err != nil {
		return err
	}
	for index, write := range writes {
		if finalPages[write.Page] != decoded[index] {
			return nfc.NewError("verify Type 2 page", nfc.CodeVerificationFailed,
				fmt.Sprintf("final dump page %d does not match the written value", write.Page), nil)
		}
	}
	result.Document = final
	return nil
}

func (s *Type2Service) withType2Reader(ctx context.Context, card nfc.CardInfo, operation func(context.Context, nfc.Reader, nfc.RawTransceiver) error) error {
	if s == nil || s.devices == nil {
		return nfc.NewError("Type 2 operation", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	return s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		if err := selectExpectedCard(operationCtx, reader, card); err != nil {
			return err
		}
		transport, ok := reader.(nfc.RawTransceiver)
		if !ok {
			return nfc.NewError("Type 2 operation", nfc.CodeUnsupported, "reader does not support raw-frame Type 2 commands", nil)
		}
		return operation(operationCtx, reader, transport)
	})
}

func validateType2Card(card nfc.CardInfo) error {
	if len(card.UID) == 0 {
		return nfc.NewError("Type 2 operation", nfc.CodeInvalidArgument, "expected card identity is required", nil)
	}
	if nfc.InferCardType(card) != nfc.CardTypeType2 {
		return nfc.NewError("Type 2 operation", nfc.CodeUnsupported,
			"selected card does not present the standard NFC Forum Type 2 selection signature", nil)
	}
	return nil
}

func cloneBytesCopy(value []byte) []byte {
	return append([]byte(nil), value...)
}
