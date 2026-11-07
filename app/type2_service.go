package app

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

// Type2PageRangeDTO is an inclusive page range.
type Type2PageRangeDTO struct {
	Start int `json:"start"`
	End   int `json:"end"`
}

// Type2InfoDTO contains the detected product and geometry.
type Type2InfoDTO struct {
	Product     string            `json:"product"`
	ProductName string            `json:"productName"`
	UID         string            `json:"uid"`
	TotalPages  int               `json:"totalPages"`
	UserPages   Type2PageRangeDTO `json:"userPages"`
	UserBytes   int               `json:"userBytes"`
	Version     string            `json:"version,omitempty"`
	DetectedBy  string            `json:"detectedBy"`
}

// Type2CCDTO is the capability container shown by the GUI and CLI.
type Type2CCDTO struct {
	Valid    bool   `json:"valid"`
	Raw      string `json:"raw"`
	Magic    byte   `json:"magic"`
	Version  byte   `json:"version"`
	Size     byte   `json:"size"`
	Access   byte   `json:"access"`
	Capacity int    `json:"capacity"`
	Writable bool   `json:"writable"`
}

// Type2ProtectionDTO contains decoded protection flags without secret values.
type Type2ProtectionDTO struct {
	StaticLock   string `json:"staticLock"`
	DynamicLock  string `json:"dynamicLock,omitempty"`
	Auth0        int    `json:"auth0"`
	Access       byte   `json:"access"`
	Protect      bool   `json:"protect"`
	ConfigLocked bool   `json:"configLocked"`
	AuthLimit    int    `json:"authLimit"`
	PasswordSet  bool   `json:"passwordSet"`
	PackSet      bool   `json:"packSet"`
}

// Type2NDEFRecordFieldDTO is one decoded record attribute.
type Type2NDEFRecordFieldDTO struct {
	Key       string `json:"key"`
	Value     string `json:"value"`
	Sensitive bool   `json:"sensitive,omitempty"`
}

// Type2NDEFRecordDTO is one decoded NDEF record.
type Type2NDEFRecordDTO struct {
	TNF     byte                      `json:"tnf"`
	Type    string                    `json:"type"`
	ID      string                    `json:"id,omitempty"`
	Payload string                    `json:"payload"`
	Text    string                    `json:"text,omitempty"`
	URI     string                    `json:"uri,omitempty"`
	Kind    string                    `json:"kind"`
	Summary string                    `json:"summary,omitempty"`
	Fields  []Type2NDEFRecordFieldDTO `json:"fields,omitempty"`
}

// Type2NDEFDTO is the parsed NDEF TLV.
type Type2NDEFDTO struct {
	Present bool                 `json:"present"`
	Length  int                  `json:"length"`
	Raw     string               `json:"raw"`
	Records []Type2NDEFRecordDTO `json:"records"`
	Text    string               `json:"text,omitempty"`
	URI     string               `json:"uri,omitempty"`
	Summary string               `json:"summary,omitempty"`
}

// Type2PageDTO is one page row.
type Type2PageDTO struct {
	Page     int    `json:"page"`
	Region   string `json:"region"`
	Hex      string `json:"hex"`
	ASCII    string `json:"ascii"`
	Editable bool   `json:"editable"`
}

// Type2DumpDTO is the GUI/CLI representation of a Type 2 dump.
type Type2DumpDTO struct {
	Loaded     bool               `json:"loaded"`
	Info       Type2InfoDTO       `json:"info"`
	Pages      []Type2PageDTO     `json:"pages"`
	CC         Type2CCDTO         `json:"cc"`
	Protection Type2ProtectionDTO `json:"protection"`
	NDEF       *Type2NDEFDTO      `json:"ndef,omitempty"`
	NDEFError  string             `json:"ndefError,omitempty"`
	ReadAt     string             `json:"readAt,omitempty"`
}

// Type2PageWriteDTO is one requested page update.
type Type2PageWriteDTO struct {
	Page int    `json:"page"`
	Hex  string `json:"hex"`
}

// Type2WriteRequestDTO writes ordinary user pages.
type Type2WriteRequestDTO struct {
	Writes   []Type2PageWriteDTO `json:"writes"`
	Password string              `json:"password,omitempty"`
	DryRun   bool                `json:"dryRun"`
}

// Type2RestoreRequestDTO restores a validated dump.
type Type2RestoreRequestDTO struct {
	Document         type2.Document `json:"document"`
	IncludeProtected bool           `json:"includeProtected"`
	Password         string         `json:"password,omitempty"`
	DryRun           bool           `json:"dryRun"`
}

// Type2NDEFWriteRequestDTO writes one raw NDEF message.
type Type2NDEFWriteRequestDTO struct {
	Message  []byte `json:"message"`
	Password string `json:"password,omitempty"`
	DryRun   bool   `json:"dryRun"`
}

// Type2WriteStartRequestDTO starts a GUI page-write task.
type Type2WriteStartRequestDTO struct {
	Pages    []Type2PageWriteDTO `json:"pages"`
	Password string              `json:"password,omitempty"`
}

// Type2NDEFWriteStartRequestDTO starts a GUI NDEF-write task from a common
// template.
type Type2NDEFWriteStartRequestDTO struct {
	Template string            `json:"template"`
	Fields   map[string]string `json:"fields"`
	Password string            `json:"password,omitempty"`
}

// Type2NDEFPreviewDTO reports the encoded message size before a write.
type Type2NDEFPreviewDTO struct {
	Template string `json:"template"`
	Length   int    `json:"length"`
}

// Type2NDEFTemplateFieldDTO is one field of a common NDEF template.
type Type2NDEFTemplateFieldDTO struct {
	ID       string   `json:"id"`
	Kind     string   `json:"kind"`
	Required bool     `json:"required"`
	Options  []string `json:"options,omitempty"`
}

// Type2NDEFTemplateDTO is one common NDEF message shape.
type Type2NDEFTemplateDTO struct {
	ID     string                      `json:"id"`
	Fields []Type2NDEFTemplateFieldDTO `json:"fields"`
}

// Type2NDEFTemplates returns the supported common NDEF templates.
func Type2NDEFTemplates() []Type2NDEFTemplateDTO {
	templates := type2.Templates()
	result := make([]Type2NDEFTemplateDTO, len(templates))
	for index, template := range templates {
		fields := make([]Type2NDEFTemplateFieldDTO, len(template.Fields))
		for fieldIndex, field := range template.Fields {
			fields[fieldIndex] = Type2NDEFTemplateFieldDTO{
				ID: field.ID, Kind: field.Kind, Required: field.Required,
				Options: append([]string(nil), field.Options...),
			}
		}
		result[index] = Type2NDEFTemplateDTO{ID: template.ID, Fields: fields}
	}
	return result
}

// Type2WriteResultDTO reports planned, written, and verified pages.
type Type2WriteResultDTO struct {
	Backup    string           `json:"backup,omitempty"`
	Planned   []int            `json:"planned"`
	Written   []int            `json:"written"`
	Verified  []int            `json:"verified"`
	Skipped   []int            `json:"skipped"`
	DryRun    bool             `json:"dryRun"`
	Changes   []Type2ChangeDTO `json:"changes,omitempty"`
	Operation string           `json:"operation,omitempty"`
	SourceUID string           `json:"sourceUid,omitempty"`
	Token     string           `json:"token,omitempty"`
	Dump      *Type2DumpDTO    `json:"dump,omitempty"`
}

// Type2Info detects the product without writing.
func (s *Service) Type2Info(ctx context.Context) (Type2InfoDTO, error) {
	card, err := s.type2Card()
	if err != nil {
		return Type2InfoDTO{}, err
	}
	identity, err := s.type2Service.Identify(ctx, card)
	if err != nil {
		return Type2InfoDTO{}, err
	}
	return type2InfoDTO(card, identity), nil
}

// ReadType2Card reads and stores a complete Type 2 dump.
func (s *Service) ReadType2Card(ctx context.Context, password string) (Type2DumpDTO, error) {
	card, err := s.type2Card()
	if err != nil {
		return Type2DumpDTO{}, err
	}
	parsedPassword, err := parseType2Password(password)
	if err != nil {
		return Type2DumpDTO{}, err
	}
	document, err := s.type2Service.Read(ctx, workflow.Type2ReadRequest{Card: card, Password: parsedPassword})
	if err != nil {
		return Type2DumpDTO{}, err
	}
	dto := type2DumpDTO(document, false)
	s.storeType2Document(document)
	s.emitter.Emit(Type2EventName, dto)
	return dto, nil
}

// StartType2Read runs a Type 2 read as an asynchronous GUI task.
func (s *Service) StartType2Read(password string) (TaskDTO, error) {
	return s.startPreparedOperation("type2_read", "Read Type 2 card", s.clearType2Dump, func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_read", Type: TaskEventProgress, Message: "reading Type 2 memory"})
		dto, err := s.ReadType2Card(ctx, password)
		if err != nil {
			return err
		}
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_read", Type: TaskEventProgress, Progress: 100, Message: "Type 2 memory loaded", Type2: &dto})
		return nil
	})
}

// StartType2Write runs a confirmed Type 2 page write as an asynchronous task.
func (s *Service) StartType2Write(request Type2WriteStartRequestDTO) (TaskDTO, error) {
	if len(request.Pages) == 0 {
		return TaskDTO{}, nfc.NewError("Type 2 write", nfc.CodeInvalidArgument, "at least one page is required", nil)
	}
	return s.startOperation("type2_write", "Write Type 2 page", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_write", Type: TaskEventProgress, Message: "writing Type 2 page"})
		result, err := s.WriteType2Pages(ctx, Type2WriteRequestDTO{Writes: request.Pages, Password: request.Password})
		if err != nil {
			return err
		}
		message := fmt.Sprintf("wrote %d page(s), verified %d; backup %s", len(result.Written), len(result.Verified), result.Backup)
		event := TaskEventDetailDTO{TaskID: taskID, Kind: "type2_write", Type: TaskEventProgress, Progress: 100, Message: message}
		if result.Dump != nil {
			event.Type2 = result.Dump
		}
		s.emitOperation(event)
		return nil
	})
}

// StartType2NDEFWrite builds a Text or URI NDEF message and writes it as an
// asynchronous task.
func (s *Service) StartType2NDEFWrite(request Type2NDEFWriteStartRequestDTO) (TaskDTO, error) {
	message, err := encodeType2NDEF(request)
	if err != nil {
		return TaskDTO{}, err
	}
	return s.startOperation("type2_ndef", "Write NDEF message", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_ndef", Type: TaskEventProgress, Message: "writing NDEF message"})
		result, writeErr := s.WriteType2NDEF(ctx, Type2NDEFWriteRequestDTO{Message: message, Password: request.Password})
		if writeErr != nil {
			return writeErr
		}
		text := fmt.Sprintf("wrote %d NDEF bytes, verified %d page(s); backup %s", len(message), len(result.Verified), result.Backup)
		event := TaskEventDetailDTO{TaskID: taskID, Kind: "type2_ndef", Type: TaskEventProgress, Progress: 100, Message: text}
		if result.Dump != nil {
			event.Type2 = result.Dump
		}
		s.emitOperation(event)
		return nil
	})
}

func encodeType2NDEF(request Type2NDEFWriteStartRequestDTO) ([]byte, error) {
	if strings.TrimSpace(request.Template) == "" {
		return nil, nfc.NewError("encode NDEF", nfc.CodeInvalidArgument, "NDEF template is required", nil)
	}
	message, err := type2.EncodeTemplate(request.Template, request.Fields)
	if err != nil {
		return nil, nfc.NewError("encode NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	return message, nil
}

// PreviewType2NDEF validates and sizes a template message without touching the
// card or creating a task.
func (s *Service) PreviewType2NDEF(request Type2NDEFWriteStartRequestDTO) (Type2NDEFPreviewDTO, error) {
	message, err := encodeType2NDEF(request)
	if err != nil {
		return Type2NDEFPreviewDTO{}, err
	}
	return Type2NDEFPreviewDTO{Template: request.Template, Length: len(message)}, nil
}

// Type2Snapshot returns the latest Type 2 dump, if any.
func (s *Service) Type2Snapshot() Type2DumpDTO {
	s.type2Mu.Lock()
	defer s.type2Mu.Unlock()
	if !s.type2Loaded {
		return Type2DumpDTO{Loaded: false}
	}
	return type2DumpDTO(s.type2Dump, false)
}

// Type2Dump returns the latest dump. Password and PACK pages are masked unless
// includeSecrets is explicitly requested for --show-keys output.
func (s *Service) Type2Dump(includeSecrets bool) Type2DumpDTO {
	s.type2Mu.Lock()
	defer s.type2Mu.Unlock()
	if !s.type2Loaded {
		return Type2DumpDTO{Loaded: false}
	}
	return type2DumpDTO(s.type2Dump, includeSecrets)
}

// WriteType2Pages writes ordinary user pages with a backup and read-back.
func (s *Service) WriteType2Pages(ctx context.Context, request Type2WriteRequestDTO) (Type2WriteResultDTO, error) {
	card, err := s.type2Card()
	if err != nil {
		return Type2WriteResultDTO{}, err
	}
	parsedPassword, err := parseType2Password(request.Password)
	if err != nil {
		return Type2WriteResultDTO{}, err
	}
	writes := make([]type2.PageWrite, len(request.Writes))
	for index, write := range request.Writes {
		if write.Hex == "" {
			return Type2WriteResultDTO{}, nfc.NewError("Type 2 write", nfc.CodeInvalidArgument, "page data is required", nil)
		}
		writes[index] = type2.PageWrite{Page: write.Page, Hex: write.Hex}
	}
	result, err := s.type2Service.Write(ctx, workflow.Type2WriteRequest{
		Card: card, Writes: writes, Password: parsedPassword, DryRun: request.DryRun,
	})
	if err != nil {
		return type2WriteFailure(result, request.DryRun), type2BackupError(err, result.BackupPath)
	}
	return s.finishType2Write(result, request.DryRun), nil
}

// RestoreType2 restores a validated dump with optional protected pages.
func (s *Service) RestoreType2(ctx context.Context, request Type2RestoreRequestDTO) (Type2WriteResultDTO, error) {
	card, err := s.type2Card()
	if err != nil {
		return Type2WriteResultDTO{}, err
	}
	parsedPassword, err := parseType2Password(request.Password)
	if err != nil {
		return Type2WriteResultDTO{}, err
	}
	result, err := s.type2Service.Restore(ctx, workflow.Type2RestoreRequest{
		Card: card, Document: request.Document, IncludeProtected: request.IncludeProtected,
		Password: parsedPassword, DryRun: request.DryRun,
	})
	if err != nil {
		return type2WriteFailure(result, request.DryRun), type2BackupError(err, result.BackupPath)
	}
	return s.finishType2Write(result, request.DryRun), nil
}

// WriteType2NDEF writes one raw NDEF message to a formatted tag.
func (s *Service) WriteType2NDEF(ctx context.Context, request Type2NDEFWriteRequestDTO) (Type2WriteResultDTO, error) {
	card, err := s.type2Card()
	if err != nil {
		return Type2WriteResultDTO{}, err
	}
	parsedPassword, err := parseType2Password(request.Password)
	if err != nil {
		return Type2WriteResultDTO{}, err
	}
	result, err := s.type2Service.WriteNDEF(ctx, workflow.Type2NDEFWriteRequest{
		Card: card, Message: request.Message, Password: parsedPassword, DryRun: request.DryRun,
	})
	if err != nil {
		return type2WriteFailure(result, request.DryRun), type2BackupError(err, result.BackupPath)
	}
	return s.finishType2Write(result, request.DryRun), nil
}

func type2WriteFailure(result workflow.Type2WriteResult, dryRun bool) Type2WriteResultDTO {
	return Type2WriteResultDTO{
		Backup: result.BackupPath, Planned: nonNilInts(result.Planned),
		Written: nonNilInts(result.Written), Verified: nonNilInts(result.Verified),
		Skipped: nonNilInts(result.Skipped), DryRun: dryRun, Changes: type2ChangeDTOs(result.Changes),
	}
}

func type2BackupError(err error, backup string) error {
	if backup == "" {
		return err
	}
	return fmt.Errorf("%w; full-image backup: %s", err, backup)
}

func (s *Service) finishType2Write(result workflow.Type2WriteResult, dryRun bool) Type2WriteResultDTO {
	dto := Type2WriteResultDTO{
		Backup: result.BackupPath, Planned: nonNilInts(result.Planned),
		Written: nonNilInts(result.Written), Verified: nonNilInts(result.Verified),
		Skipped: nonNilInts(result.Skipped), DryRun: dryRun, Changes: type2ChangeDTOs(result.Changes),
	}
	if !dryRun {
		s.storeType2Document(result.Document)
		dumpDTO := type2DumpDTO(result.Document, false)
		dto.Dump = &dumpDTO
		s.emitter.Emit(Type2EventName, dumpDTO)
	}
	return dto
}

// DumpType2Card reads the card and writes a validated Type 2 JSON dump.
func (s *Service) DumpType2Card(ctx context.Context, path string, password string) (Type2DumpDTO, error) {
	dto, err := s.ReadType2Card(ctx, password)
	if err != nil {
		return Type2DumpDTO{}, err
	}
	s.type2Mu.Lock()
	document := s.type2Dump
	s.type2Mu.Unlock()
	if err := s.SaveType2DumpFile(path, document); err != nil {
		return Type2DumpDTO{}, err
	}
	return dto, nil
}

// LoadType2DumpFile reads and validates a Type 2 JSON dump.
func (s *Service) LoadType2DumpFile(path string) (type2.Document, error) {
	if strings.TrimSpace(path) == "" {
		return type2.Document{}, nfc.NewError("load Type 2 dump", nfc.CodeInvalidArgument, "dump path is required", nil)
	}
	info, err := os.Stat(path)
	if err != nil {
		return type2.Document{}, err
	}
	if info.Size() > 4<<20 {
		return type2.Document{}, nfc.NewError("load Type 2 dump", nfc.CodeInvalidArgument, "dump file is too large", nil)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return type2.Document{}, err
	}
	var document type2.Document
	if bytes.HasPrefix(data, type2.BinaryDumpMagic[:]) || strings.EqualFold(filepath.Ext(path), ".bin") {
		if err := document.UnmarshalBinary(data); err != nil {
			return type2.Document{}, err
		}
		return document, nil
	}
	if err := json.Unmarshal(data, &document); err != nil {
		return type2.Document{}, fmt.Errorf("decode Type 2 dump: %w", err)
	}
	if err := document.Validate(); err != nil {
		return type2.Document{}, err
	}
	if pages, err := document.PageBytes(); err == nil {
		document.Refresh(pages)
	}
	return document, nil
}

// SaveType2DumpFile writes a validated Type 2 dump atomically. JSON is the
// default; a .bin extension selects the binary full-image format.
func (s *Service) SaveType2DumpFile(path string, document type2.Document) error {
	if strings.TrimSpace(path) == "" {
		return nfc.NewError("save Type 2 dump", nfc.CodeInvalidArgument, "dump path is required", nil)
	}
	if err := document.Validate(); err != nil {
		return err
	}
	var data []byte
	var err error
	if strings.EqualFold(filepath.Ext(path), ".bin") {
		data, err = document.MarshalBinary()
		if err != nil {
			return err
		}
	} else {
		data, err = json.MarshalIndent(document, "", "  ")
		if err != nil {
			return err
		}
		data = append(data, '\n')
	}
	directory := filepath.Dir(path)
	temporary, err := os.CreateTemp(directory, ".type2-dump-*.tmp")
	if err != nil {
		return err
	}
	temporaryPath := temporary.Name()
	keep := false
	defer func() {
		_ = temporary.Close()
		if !keep {
			_ = os.Remove(temporaryPath)
		}
	}()
	if err := temporary.Chmod(0o600); err != nil {
		return err
	}
	if _, err := temporary.Write(data); err != nil {
		return err
	}
	if err := temporary.Sync(); err != nil {
		return err
	}
	if err := temporary.Close(); err != nil {
		return err
	}
	if err := os.Rename(temporaryPath, path); err != nil {
		return err
	}
	keep = true
	return nil
}

func (s *Service) type2Card() (nfc.CardInfo, error) {
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return nfc.CardInfo{}, ErrCardRequired
	}
	return snapshot.Card.Clone(), nil
}

func (s *Service) clearType2Dump() {
	s.type2Mu.Lock()
	s.type2Dump = type2.Document{}
	s.type2Loaded = false
	s.type2Mu.Unlock()
	s.emitter.Emit(Type2EventName, Type2DumpDTO{Loaded: false})
}

func (s *Service) storeType2Document(document type2.Document) {
	s.type2Mu.Lock()
	s.type2Dump = document
	s.type2Loaded = true
	s.type2Mu.Unlock()
}

func parseType2Password(value string) ([]byte, error) {
	if strings.TrimSpace(value) == "" {
		return nil, nil
	}
	decoded, err := hex.DecodeString(normalizeHexValue(value))
	if err != nil || len(decoded) != 4 {
		return nil, nfc.NewError("parse Type 2 password", nfc.CodeInvalidArgument, "password must contain exactly 8 hexadecimal digits", nil)
	}
	return decoded, nil
}

func normalizeHexValue(value string) string {
	replacer := strings.NewReplacer(" ", "", ":", "", "-", "")
	return replacer.Replace(strings.TrimSpace(value))
}

func type2InfoDTO(card nfc.CardInfo, identity type2.Identity) Type2InfoDTO {
	return Type2InfoDTO{
		Product: string(identity.Product), ProductName: identity.Name(),
		UID: formatHex(card.UID), TotalPages: identity.Geometry.TotalPages,
		UserPages: Type2PageRangeDTO{Start: identity.Geometry.UserStart, End: identity.Geometry.UserEnd},
		UserBytes: identity.Geometry.UserBytes(),
		Version:   strings.ToUpper(hex.EncodeToString(identity.Version)), DetectedBy: identity.DetectedBy,
	}
}

func type2DumpDTO(document type2.Document, includeSecrets bool) Type2DumpDTO {
	uid, _ := hex.DecodeString(document.UID)
	card := nfc.CardInfo{UID: uid}
	if decoded, err := hex.DecodeString(document.ATQA); err == nil && len(decoded) == 2 {
		card.ATQA = [2]byte{decoded[0], decoded[1]}
	}
	if decoded, err := hex.DecodeString(document.SAK); err == nil && len(decoded) == 1 {
		card.SAK = decoded[0]
	}
	geometry, _ := type2.GeometryFor(document.Product)
	identity := type2.Identity{Product: document.Product, Geometry: geometry}
	info := type2InfoDTO(card, identity)
	dto := Type2DumpDTO{
		Loaded: true, Info: info,
		CC: Type2CCDTO{
			Valid: document.CC.Valid, Raw: document.CC.Raw, Magic: document.CC.Magic,
			Version: document.CC.Version, Size: document.CC.Size, Access: document.CC.Access,
			Capacity: document.CC.Capacity, Writable: document.CC.Writable(),
		},
		Protection: Type2ProtectionDTO{
			StaticLock: document.Protection.StaticLock, DynamicLock: document.Protection.DynamicLock,
			Auth0: document.Protection.Auth0, Access: document.Protection.Access,
			Protect: document.Protection.Protect, ConfigLocked: document.Protection.ConfigLocked,
			AuthLimit: document.Protection.AuthLimit, PasswordSet: document.Protection.PasswordSet,
			PackSet: document.Protection.PackSet,
		},
		NDEFError: document.NDEFError, ReadAt: document.ReadAt,
	}
	for index, value := range document.Pages {
		decoded, _ := hex.DecodeString(value)
		region := geometry.Region(index)
		if !includeSecrets && (region == "password" || region == "pack") {
			value = "--------"
			decoded = nil
		}
		dto.Pages = append(dto.Pages, Type2PageDTO{
			Page: index, Region: region, Hex: value, ASCII: pageASCII(decoded),
			Editable: geometry.InUserMemory(index),
		})
	}
	if document.NDEF != nil {
		ndef := &Type2NDEFDTO{
			Present: document.NDEF.Present, Length: document.NDEF.Length, Raw: document.NDEF.Raw,
			Text: document.NDEF.Text, URI: document.NDEF.URI,
		}
		for _, record := range document.NDEF.Records {
			fields := make([]Type2NDEFRecordFieldDTO, len(record.Fields))
			for index, field := range record.Fields {
				fields[index] = Type2NDEFRecordFieldDTO{Key: field.Key, Value: field.Value, Sensitive: field.Sensitive}
			}
			ndef.Records = append(ndef.Records, Type2NDEFRecordDTO{
				TNF: record.TNF, Type: record.Type, ID: record.ID, Payload: record.Payload,
				Text: record.Text, URI: record.URI, Kind: record.Kind, Summary: record.Summary,
				Fields: fields,
			})
		}
		ndef.Summary = document.NDEF.Summary
		dto.NDEF = ndef
	}
	return dto
}

func nonNilInts(values []int) []int {
	if values == nil {
		return []int{}
	}
	return append([]int{}, values...)
}

func pageASCII(data []byte) string {
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
