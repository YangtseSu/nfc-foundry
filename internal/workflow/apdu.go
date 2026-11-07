package workflow

import (
	"context"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/desfire"
	"github.com/YangtseSu/nfc-foundry/internal/isodep"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/type4"
)

// APDURequest sends one diagnostic command APDU to the selected card.
type APDURequest struct {
	Card nfc.CardInfo
	APDU []byte
}

// APDUResult is the ATS and response of one APDU exchange.
type APDUResult struct {
	ATS      isodep.ATS
	Response isodep.Response
}

// Type4ReadRequest reads the Type 4 NDEF application.
type Type4ReadRequest struct {
	Card nfc.CardInfo
}

// Type4WriteRequest replaces the Type 4 NDEF message.
type Type4WriteRequest struct {
	Card    nfc.CardInfo
	Device  nfc.DeviceInfo
	Message []byte
	DryRun  bool
}

// Type4WriteResult reports the previous message, final document, and backup.
type Type4WriteResult struct {
	Previous   []byte
	Document   type4.Document
	BackupPath string
	DryRun     bool
}

// Type4BackupStore persists the previous raw NDEF message before a write.
type Type4BackupStore interface {
	Save(context.Context, Type4Backup) (string, error)
}

// Type4Backup is the complete pre-write NDEF state of a Type 4 tag.
type Type4Backup struct {
	Card    nfc.CardInfo
	Device  nfc.DeviceInfo
	CC      type4.CC
	Message []byte
}

// APDUService owns ISO-DEP activation and read-only card-family commands.
type APDUService struct {
	devices ReaderExecutor
}

func NewAPDUService(devices ReaderExecutor) *APDUService {
	return &APDUService{devices: devices}
}

// Transmit sends one raw short APDU and returns the ATS and status word.
func (s *APDUService) Transmit(ctx context.Context, request APDURequest) (APDUResult, error) {
	request.Card = request.Card.Clone()
	var result APDUResult
	err := s.withSession(ctx, request.Card, func(operationCtx context.Context, session *isodep.Session) error {
		result.ATS = session.ATS
		response, transmitErr := session.Transmit(operationCtx, request.APDU)
		if transmitErr != nil {
			return transmitErr
		}
		result.Response = response
		return nil
	})
	return result, err
}

// DESFireIdentify reads the native GetVersion response.
func (s *APDUService) DESFireIdentify(ctx context.Context, card nfc.CardInfo) (desfire.Version, error) {
	card = card.Clone()
	var version desfire.Version
	err := s.withSession(ctx, card, func(operationCtx context.Context, session *isodep.Session) error {
		var identifyErr error
		version, identifyErr = desfire.New(session).Identify(operationCtx)
		return identifyErr
	})
	return version, err
}

// DESFireApplications reads the native application IDs.
func (s *APDUService) DESFireApplications(ctx context.Context, card nfc.CardInfo) ([]uint32, error) {
	card = card.Clone()
	var applications []uint32
	err := s.withSession(ctx, card, func(operationCtx context.Context, session *isodep.Session) error {
		var readErr error
		applications, readErr = desfire.New(session).Applications(operationCtx)
		return readErr
	})
	return applications, err
}

// DESFireFiles lists the files in one application.
func (s *APDUService) DESFireFiles(ctx context.Context, card nfc.CardInfo, aid uint32) ([]desfire.File, error) {
	card = card.Clone()
	var files []desfire.File
	err := s.withSession(ctx, card, func(operationCtx context.Context, session *isodep.Session) error {
		var readErr error
		files, readErr = desfire.New(session).Files(operationCtx, aid)
		return readErr
	})
	return files, err
}

// DESFireReadFile reads one standard or backup data file.
func (s *APDUService) DESFireReadFile(ctx context.Context, card nfc.CardInfo, aid uint32, fileID byte) ([]byte, error) {
	card = card.Clone()
	var data []byte
	err := s.withSession(ctx, card, func(operationCtx context.Context, session *isodep.Session) error {
		var readErr error
		data, readErr = desfire.New(session).ReadFile(operationCtx, aid, fileID)
		return readErr
	})
	return data, err
}

// ReadType4NDEF reads and decodes the Type 4 NDEF application.
func (s *APDUService) ReadType4NDEF(ctx context.Context, request Type4ReadRequest) (type4.Document, error) {
	request.Card = request.Card.Clone()
	var document type4.Document
	err := s.withSession(ctx, request.Card, func(operationCtx context.Context, session *isodep.Session) error {
		var readErr error
		document, readErr = type4.New(session).Read(operationCtx)
		return readErr
	})
	return document, err
}

// WriteType4NDEF validates, backs up, writes, and verifies one NDEF message.
func (s *APDUService) WriteType4NDEF(ctx context.Context, request Type4WriteRequest, backups Type4BackupStore) (Type4WriteResult, error) {
	request.Card = request.Card.Clone()
	request.Message = append([]byte(nil), request.Message...)
	if _, err := type2.ParseMessage(request.Message); err != nil {
		return Type4WriteResult{}, nfc.NewError("write Type 4 NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	if !request.DryRun && backups == nil {
		return Type4WriteResult{}, nfc.NewError("write Type 4 NDEF", nfc.CodeInternal, "backup storage is unavailable; the tag was not modified", nil)
	}
	var result Type4WriteResult
	result.DryRun = request.DryRun
	err := s.withSession(ctx, request.Card, func(operationCtx context.Context, session *isodep.Session) error {
		tag := type4.New(session)
		current, err := tag.Read(operationCtx)
		if err != nil {
			return err
		}
		result.Previous = append([]byte(nil), current.Message...)
		result.Document = current
		if current.CC.WriteAccess != 0x00 {
			return nfc.NewError("write Type 4 NDEF", nfc.CodePermission, fmt.Sprintf("NDEF file write access is 0x%02X; authentication is not implemented", current.CC.WriteAccess), nil)
		}
		if len(request.Message) > current.CC.MaxMessage {
			return nfc.NewError("write Type 4 NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("message needs %d bytes; NDEF file limit is %d", len(request.Message), current.CC.MaxMessage), nil)
		}
		if request.DryRun {
			return nil
		}
		backupPath, err := backups.Save(operationCtx, Type4Backup{
			Card: request.Card, Device: request.Device, CC: current.CC, Message: current.Message,
		})
		if err != nil {
			return nfc.NewError("write Type 4 NDEF", nfc.CodeIO, "could not save the pre-write backup; the tag was not modified", err)
		}
		result.BackupPath = backupPath
		written, err := tag.Write(operationCtx, request.Message)
		if err != nil {
			return err
		}
		result.Document = written.Final
		if len(written.Final.Message) != len(request.Message) {
			return nfc.NewError("verify Type 4 NDEF", nfc.CodeVerificationFailed, "final NDEF length does not match", nil)
		}
		return nil
	})
	return result, err
}

func (s *APDUService) withSession(ctx context.Context, card nfc.CardInfo, operation func(context.Context, *isodep.Session) error) error {
	if s == nil || s.devices == nil {
		return nfc.NewError("ISO-DEP", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	if err := validateISO14443Card(card); err != nil {
		return err
	}
	return s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		if err := selectExpectedCard(operationCtx, reader, card); err != nil {
			return err
		}
		transport, ok := reader.(nfc.RawTransceiver)
		if !ok {
			return nfc.NewError("ISO-DEP", nfc.CodeUnsupported, "reader has no raw-frame transport", nil)
		}
		session, err := isodep.OpenSession(operationCtx, transport, card)
		if err != nil {
			return err
		}
		operationErr := operation(operationCtx, session)
		closeErr := session.Close(context.WithoutCancel(operationCtx))
		if operationErr != nil {
			return operationErr
		}
		if closeErr != nil {
			return nfc.NewError("ISO-DEP", nfc.CodeIO, "reader could not leave raw mode", closeErr)
		}
		if err := selectExpectedCard(operationCtx, reader, card); err != nil {
			return err
		}
		return nil
	})
}

func validateISO14443Card(card nfc.CardInfo) error {
	if card.SAK&0x20 == 0 {
		return nfc.NewError("ISO-DEP", nfc.CodeUnsupported, "card does not advertise ISO/IEC 14443-4", nil)
	}
	return nil
}
