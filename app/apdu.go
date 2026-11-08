package app

import (
	"context"
	"encoding/hex"
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/isodep"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type4"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

// ATSDTO is the parsed ISO-DEP Answer To Select.
type ATSDTO struct {
	Raw        string `json:"raw"`
	TL         int    `json:"tl"`
	FSCI       int    `json:"fsci"`
	FSC        int    `json:"fsc"`
	FWI        int    `json:"fwi"`
	TA         *byte  `json:"ta,omitempty"`
	TB         *byte  `json:"tb,omitempty"`
	TC         *byte  `json:"tc,omitempty"`
	Historical string `json:"historical,omitempty"`
}

// APDUResponseDTO is one diagnostic APDU exchange.
type APDUResponseDTO struct {
	ATS  ATSDTO `json:"ats"`
	Data string `json:"data"`
	SW   string `json:"sw"`
	SW1  byte   `json:"sw1"`
	SW2  byte   `json:"sw2"`
	OK   bool   `json:"ok"`
}

// DESFireVersionDTO is the native GetVersion product information.
type DESFireVersionDTO struct {
	Vendor   byte   `json:"vendor"`
	Type     byte   `json:"type"`
	Subtype  byte   `json:"subtype"`
	Major    byte   `json:"major"`
	Minor    byte   `json:"minor"`
	Storage  byte   `json:"storage"`
	Protocol byte   `json:"protocol"`
	Version  string `json:"version"`
}

// DESFireFileDTO is one DESFire file and its settings.
type DESFireFileDTO struct {
	ID              byte   `json:"id"`
	Type            string `json:"type"`
	Communication   byte   `json:"communication"`
	AccessRights    uint16 `json:"accessRights"`
	Size            int    `json:"size,omitempty"`
	RecordSize      int    `json:"recordSize,omitempty"`
	MaxRecords      int    `json:"maxRecords,omitempty"`
	CurrentRecords  int    `json:"currentRecords,omitempty"`
	ValueLower      int32  `json:"valueLower,omitempty"`
	ValueUpper      int32  `json:"valueUpper,omitempty"`
	LimitedCredit   int32  `json:"limitedCredit,omitempty"`
	LimitedCreditOn bool   `json:"limitedCreditOn,omitempty"`
}

// DESFireFilesDTO is the file list of one application.
type DESFireFilesDTO struct {
	AID   string           `json:"aid"`
	Files []DESFireFileDTO `json:"files"`
}

// DESFireReadDTO is one read DESFire file.
type DESFireReadDTO struct {
	AID    string `json:"aid"`
	File   byte   `json:"file"`
	Length int    `json:"length"`
	Hex    string `json:"hex"`
	ASCII  string `json:"ascii"`
}

// Type4CCDTO is the validated Type 4 capability container.
type Type4CCDTO struct {
	Raw            string `json:"raw"`
	Length         int    `json:"length"`
	MappingVersion byte   `json:"mappingVersion"`
	MLe            int    `json:"mle"`
	MLc            int    `json:"mlc"`
	FileID         string `json:"fileId"`
	MaxMessage     int    `json:"maxMessage"`
	ReadAccess     byte   `json:"readAccess"`
	WriteAccess    byte   `json:"writeAccess"`
}

// Type4NDEFDTO is the Type 4 NDEF application and decoded message.
type Type4NDEFDTO struct {
	Loaded        bool          `json:"loaded"`
	Card          CardDTO       `json:"card"`
	CardType      string        `json:"cardType"`
	CC            Type4CCDTO    `json:"cc"`
	NDEF          *Type2NDEFDTO `json:"ndef,omitempty"`
	MessageLength int           `json:"messageLength"`
	ReadOnly      bool          `json:"readOnly"`
}

// Type4NDEFWriteRequestDTO replaces the Type 4 NDEF message.
type Type4NDEFWriteRequestDTO struct {
	Message      []byte `json:"message"`
	DryRun       bool   `json:"dryRun"`
	ExpectedUID  string `json:"expectedUid,omitempty"`
	ExpectedATQA string `json:"expectedAtqa,omitempty"`
	ExpectedSAK  string `json:"expectedSak,omitempty"`
}

// Type4NDEFWriteResultDTO reports the backup and final Type 4 state.
type Type4NDEFWriteResultDTO struct {
	NDEF     *Type4NDEFDTO `json:"ndef,omitempty"`
	Backup   string        `json:"backup,omitempty"`
	Previous string        `json:"previous,omitempty"`
	DryRun   bool          `json:"dryRun"`
}

// TransmitAPDU sends one diagnostic short APDU to the selected ISO-DEP card.
func (s *Service) TransmitAPDU(ctx context.Context, raw string) (APDUResponseDTO, error) {
	card, err := s.iso14443Card()
	if err != nil {
		return APDUResponseDTO{}, err
	}
	apdu, err := decodeHexValue(raw, "APDU")
	if err != nil {
		return APDUResponseDTO{}, err
	}
	if len(apdu) < 4 {
		return APDUResponseDTO{}, nfc.NewError("APDU", nfc.CodeInvalidArgument, "APDU header must contain four bytes", nil)
	}
	result, err := s.apduService.Transmit(ctx, workflow.APDURequest{Card: card, APDU: apdu})
	if err != nil {
		return APDUResponseDTO{}, err
	}
	return APDUResponseDTO{
		ATS:  atsDTO(result.ATS),
		Data: strings.ToUpper(hex.EncodeToString(result.Response.Data)),
		SW:   fmt.Sprintf("%02X%02X", result.Response.SW1, result.Response.SW2),
		SW1:  result.Response.SW1, SW2: result.Response.SW2, OK: result.Response.OK(),
	}, nil
}

// DESFireInfo reads the native GetVersion response.
func (s *Service) DESFireInfo(ctx context.Context) (DESFireVersionDTO, error) {
	card, err := s.iso14443Card()
	if err != nil {
		return DESFireVersionDTO{}, err
	}
	version, err := s.apduService.DESFireIdentify(ctx, card)
	if err != nil {
		return DESFireVersionDTO{}, err
	}
	return DESFireVersionDTO{
		Vendor: version.Vendor, Type: version.Type, Subtype: version.Subtype,
		Major: version.Major, Minor: version.Minor, Storage: version.Storage,
		Protocol: version.Protocol,
		Version:  fmt.Sprintf("0x%02X.0x%02X", version.Major, version.Minor),
	}, nil
}

// DESFireApplications lists native application IDs.
func (s *Service) DESFireApplications(ctx context.Context) ([]string, error) {
	card, err := s.iso14443Card()
	if err != nil {
		return nil, err
	}
	applications, err := s.apduService.DESFireApplications(ctx, card)
	if err != nil {
		return nil, err
	}
	result := make([]string, len(applications))
	for index, aid := range applications {
		result[index] = fmt.Sprintf("%06X", aid)
	}
	return result, nil
}

// DESFireFiles lists files in one application.
func (s *Service) DESFireFiles(ctx context.Context, aid string) (DESFireFilesDTO, error) {
	card, err := s.iso14443Card()
	if err != nil {
		return DESFireFilesDTO{}, err
	}
	application, err := parseAID(aid)
	if err != nil {
		return DESFireFilesDTO{}, err
	}
	files, err := s.apduService.DESFireFiles(ctx, card, application)
	if err != nil {
		return DESFireFilesDTO{}, err
	}
	result := DESFireFilesDTO{AID: fmt.Sprintf("%06X", application), Files: make([]DESFireFileDTO, len(files))}
	for index, file := range files {
		result.Files[index] = DESFireFileDTO{
			ID: file.ID, Type: file.Type.String(), Communication: file.Communication,
			AccessRights: file.AccessRights, Size: file.Size,
			RecordSize: file.RecordSize, MaxRecords: file.MaxRecords, CurrentRecords: file.CurrentRecords,
			ValueLower: file.ValueLower, ValueUpper: file.ValueUpper,
			LimitedCredit: file.LimitedCredit, LimitedCreditOn: file.LimitedCreditOn,
		}
	}
	return result, nil
}

// DESFireReadFile reads one standard or backup file.
func (s *Service) DESFireReadFile(ctx context.Context, aid string, fileID byte) (DESFireReadDTO, error) {
	card, err := s.iso14443Card()
	if err != nil {
		return DESFireReadDTO{}, err
	}
	application, err := parseAID(aid)
	if err != nil {
		return DESFireReadDTO{}, err
	}
	data, err := s.apduService.DESFireReadFile(ctx, card, application, fileID)
	if err != nil {
		return DESFireReadDTO{}, err
	}
	return DESFireReadDTO{
		AID: fmt.Sprintf("%06X", application), File: fileID, Length: len(data),
		Hex: strings.ToUpper(hex.EncodeToString(data)), ASCII: printableASCII(data),
	}, nil
}

// ReadType4NDEF reads and decodes the Type 4 NDEF application.
func (s *Service) ReadType4NDEF(ctx context.Context) (Type4NDEFDTO, error) {
	card, err := s.iso14443Card()
	if err != nil {
		return Type4NDEFDTO{}, err
	}
	document, err := s.apduService.ReadType4NDEF(ctx, workflow.Type4ReadRequest{Card: card})
	if err != nil {
		return Type4NDEFDTO{}, err
	}
	return type4NDEFDTO(card, document), nil
}

// StartType4NDEFWrite replaces the Type 4 NDEF message as an asynchronous
// task with the same backup and read-back rules as the synchronous path.
func (s *Service) StartType4NDEFWrite(request Type4NDEFWriteRequestDTO) (TaskDTO, error) {
	return s.startOperation("type4_ndef", "Write Type 4 NDEF message", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type4_ndef", Type: TaskEventProgress, Message: "writing the Type 4 NDEF message"})
		result, err := s.WriteType4NDEF(ctx, request)
		if err != nil {
			return err
		}
		message := fmt.Sprintf("wrote %d Type 4 NDEF bytes; backup %s", len(request.Message), result.Backup)
		event := TaskEventDetailDTO{TaskID: taskID, Kind: "type4_ndef", Type: TaskEventProgress, Progress: 100, Message: message, BackupPath: result.Backup}
		if result.NDEF != nil {
			event.Type4 = result.NDEF
		}
		s.emitOperation(event)
		return nil
	})
}

func (request Type4NDEFWriteRequestDTO) verifyCard(card nfc.CardInfo) error {
	if request.ExpectedUID == "" && request.ExpectedATQA == "" && request.ExpectedSAK == "" {
		return nil
	}
	uid := formatHex(card.UID)
	atqa := formatHex(card.ATQA[:])
	sak := fmt.Sprintf("%02X", card.SAK)
	if !strings.EqualFold(strings.TrimSpace(request.ExpectedUID), uid) ||
		!strings.EqualFold(strings.TrimSpace(request.ExpectedATQA), atqa) ||
		!strings.EqualFold(strings.TrimSpace(request.ExpectedSAK), sak) {
		return nfc.NewError("Type 4 NDEF write", nfc.CodeCardChanged, "the card changed before the NDEF write", nil)
	}
	return nil
}

func (s *Service) type4CardSelected() bool {
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return false
	}
	return nfc.InferCardType(*snapshot.Card) == nfc.CardTypeISO14443
}

// WriteType4NDEF replaces the Type 4 NDEF message with a backup.
func (s *Service) WriteType4NDEF(ctx context.Context, request Type4NDEFWriteRequestDTO) (Type4NDEFWriteResultDTO, error) {
	card, err := s.iso14443Card()
	if err != nil {
		return Type4NDEFWriteResultDTO{}, err
	}
	if err := request.verifyCard(card); err != nil {
		return Type4NDEFWriteResultDTO{}, err
	}
	result, err := s.apduService.WriteType4NDEF(ctx, workflow.Type4WriteRequest{
		Card: card, Device: s.deviceManager.Snapshot().Device, Message: request.Message, DryRun: request.DryRun,
	}, workflow.FileType4BackupStore{Root: s.type4BackupRoot})
	dto := Type4NDEFWriteResultDTO{Backup: result.BackupPath, DryRun: request.DryRun,
		Previous: strings.ToUpper(hex.EncodeToString(result.Previous))}
	if result.Document.CC.Length != 0 {
		ndef := type4NDEFDTO(card, result.Document)
		dto.NDEF = &ndef
	}
	if err != nil {
		return dto, err
	}
	return dto, nil
}

func (s *Service) iso14443Card() (nfc.CardInfo, error) {
	if err := s.requireAPDU("APDU operation"); err != nil {
		return nfc.CardInfo{}, err
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return nfc.CardInfo{}, ErrCardRequired
	}
	card := snapshot.Card.Clone()
	if nfc.InferCardType(card) != nfc.CardTypeISO14443 {
		return nfc.CardInfo{}, nfc.NewError("ISO-DEP", nfc.CodeUnsupported, "card does not advertise ISO/IEC 14443-4", nil)
	}
	return card, nil
}

func atsDTO(ats isodep.ATS) ATSDTO {
	return ATSDTO{
		Raw: strings.ToUpper(hex.EncodeToString(ats.Raw)), TL: ats.TL, FSCI: ats.FSCI, FSC: ats.FSC, FWI: ats.FWI,
		TA: ats.TA, TB: ats.TB, TC: ats.TC,
		Historical: strings.ToUpper(hex.EncodeToString(ats.Historical)),
	}
}

func type4NDEFDTO(card nfc.CardInfo, document type4.Document) Type4NDEFDTO {
	return Type4NDEFDTO{
		Loaded: true, Card: cardDTO(card), CardType: "NFC Forum Type 4",
		CC: Type4CCDTO{
			Raw: strings.ToUpper(hex.EncodeToString(document.CC.Raw)), Length: document.CC.Length,
			MappingVersion: document.CC.MappingVersion, MLe: document.CC.MLe, MLc: document.CC.MLc,
			FileID: fmt.Sprintf("%04X", document.CC.FileID), MaxMessage: document.CC.MaxMessage,
			ReadAccess: document.CC.ReadAccess, WriteAccess: document.CC.WriteAccess,
		},
		NDEF: ndefDTO(document.NDEF), MessageLength: len(document.Message),
		ReadOnly: document.CC.WriteAccess != 0x00,
	}
}

func parseAID(value string) (uint32, error) {
	decoded, err := hex.DecodeString(normalizeHexValue(value))
	if err != nil || len(decoded) != 3 {
		return 0, nfc.NewError("DESFire application", nfc.CodeInvalidArgument, "application ID must contain exactly 6 hexadecimal digits", nil)
	}
	return uint32(decoded[0])<<16 | uint32(decoded[1])<<8 | uint32(decoded[2]), nil
}

func decodeHexValue(value, name string) ([]byte, error) {
	decoded, err := hex.DecodeString(normalizeHexValue(value))
	if err != nil || len(decoded) == 0 {
		return nil, nfc.NewError("decode "+name, nfc.CodeInvalidArgument, name+" must contain an even number of hexadecimal digits", nil)
	}
	return decoded, nil
}

func printableASCII(data []byte) string {
	result := make([]byte, len(data))
	for index, value := range data {
		if value >= 0x20 && value <= 0x7E {
			result[index] = value
		} else {
			result[index] = '.'
		}
	}
	return string(result)
}
