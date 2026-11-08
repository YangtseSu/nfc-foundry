package app

import (
	"context"
	"encoding/hex"
	"fmt"
	"os"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

// Type2NDEFRawRequestDTO writes a complete NDEF message from hexadecimal.
type Type2NDEFRawRequestDTO struct {
	Raw      string `json:"raw"`
	Password string `json:"password,omitempty"`
	DryRun   bool   `json:"dryRun"`
}

// Type2NDEFRecordInputDTO is one record in a multi-record message.
type Type2NDEFRecordInputDTO struct {
	Kind   string            `json:"kind"`
	Fields map[string]string `json:"fields"`
}

// Type2NDEFRecordsRequestDTO writes a multi-record NDEF message.
type Type2NDEFRecordsRequestDTO struct {
	Records  []Type2NDEFRecordInputDTO `json:"records"`
	Password string                    `json:"password,omitempty"`
	DryRun   bool                      `json:"dryRun"`
}

// Type2NDEFImportDTO is a validated NDEF message loaded from a file.
type Type2NDEFImportDTO struct {
	Path    string               `json:"path"`
	Length  int                  `json:"length"`
	Raw     string               `json:"raw"`
	Records []Type2NDEFRecordDTO `json:"records"`
}

// Type2SourceDTO describes the loaded copy source image.
type Type2SourceDTO struct {
	Loaded      bool   `json:"loaded"`
	Path        string `json:"path,omitempty"`
	Product     string `json:"product,omitempty"`
	ProductName string `json:"productName,omitempty"`
	UID         string `json:"uid,omitempty"`
	Pages       int    `json:"pages,omitempty"`
}

func decodeType2NDEFHex(raw string) ([]byte, error) {
	value := normalizeHexValue(raw)
	if value == "" {
		return nil, nfc.NewError("decode NDEF message", nfc.CodeInvalidArgument, "NDEF message is empty", nil)
	}
	message, err := hex.DecodeString(value)
	if err != nil {
		return nil, nfc.NewError("decode NDEF message", nfc.CodeInvalidArgument, "NDEF message must be hexadecimal", nil)
	}
	if _, err := type2.ParseMessage(message); err != nil {
		return nil, nfc.NewError("decode NDEF message", nfc.CodeInvalidArgument, err.Error(), err)
	}
	return message, nil
}

// Type2ProviderDTO is one provider preset for a URL template.
type Type2ProviderDTO struct {
	ID  string `json:"id"`
	URL string `json:"url"`
}

// Type2NDEFProviders returns provider presets for search, social, and payment
// templates.
func Type2NDEFProviders() map[string][]Type2ProviderDTO {
	return map[string][]Type2ProviderDTO{
		"search":  providerDTOs(type2.SearchProviders()),
		"social":  providerDTOs(type2.SocialProviders()),
		"payment": providerDTOs(type2.PaymentProviders()),
	}
}

func providerDTOs(providers []type2.Provider) []Type2ProviderDTO {
	result := make([]Type2ProviderDTO, len(providers))
	for index, provider := range providers {
		result[index] = Type2ProviderDTO{ID: provider.ID, URL: provider.URL}
	}
	return result
}

// PreviewType2NDEFRaw validates a raw NDEF message without touching a card.
func (s *Service) PreviewType2NDEFRaw(request Type2NDEFRawRequestDTO) (Type2NDEFPreviewDTO, error) {
	message, err := decodeType2NDEFHex(request.Raw)
	if err != nil {
		return Type2NDEFPreviewDTO{}, err
	}
	return Type2NDEFPreviewDTO{Template: "raw", Length: len(message), Message: strings.ToUpper(hex.EncodeToString(message))}, nil
}

// PreviewType2NDEFRecords validates a multi-record message without touching a
// card.
func (s *Service) PreviewType2NDEFRecords(request Type2NDEFRecordsRequestDTO) (Type2NDEFPreviewDTO, error) {
	message, err := encodeType2NDEFRecords(request.Records)
	if err != nil {
		return Type2NDEFPreviewDTO{}, err
	}
	return Type2NDEFPreviewDTO{Template: "records", Length: len(message), Message: strings.ToUpper(hex.EncodeToString(message))}, nil
}

func encodeType2NDEFRecords(records []Type2NDEFRecordInputDTO) ([]byte, error) {
	inputs := make([]type2.RecordInput, len(records))
	for index, record := range records {
		inputs[index] = type2.RecordInput{Kind: record.Kind, Fields: record.Fields}
	}
	message, err := type2.EncodeInputs(inputs)
	if err != nil {
		return nil, nfc.NewError("encode NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	return message, nil
}

// StartType2NDEFRawWrite writes a raw NDEF message as an asynchronous task.
func (s *Service) StartType2NDEFRawWrite(request Type2NDEFRawRequestDTO) (TaskDTO, error) {
	message, err := decodeType2NDEFHex(request.Raw)
	if err != nil {
		return TaskDTO{}, err
	}
	return s.startType2NDEFMessage(message, request.Password)
}

// StartType2NDEFRecordsWrite writes a multi-record NDEF message as an
// asynchronous task.
func (s *Service) StartType2NDEFRecordsWrite(request Type2NDEFRecordsRequestDTO) (TaskDTO, error) {
	message, err := encodeType2NDEFRecords(request.Records)
	if err != nil {
		return TaskDTO{}, err
	}
	return s.startType2NDEFMessage(message, request.Password)
}

func (s *Service) startType2NDEFMessage(message []byte, password string) (TaskDTO, error) {
	return s.startOperation("type2_ndef", "Write NDEF message", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "type2_ndef", Type: TaskEventProgress, Message: "writing NDEF message"})
		result, writeErr := s.WriteType2NDEF(ctx, Type2NDEFWriteRequestDTO{Message: message, Password: password})
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

// ImportType2NDEF reads and validates a raw NDEF message file.
func (s *Service) ImportType2NDEF() (Type2NDEFImportDTO, error) {
	path, err := s.dialogs.OpenFile("Import NDEF message", "*.ndef;*.bin;*.txt")
	if err != nil {
		return Type2NDEFImportDTO{}, err
	}
	return s.ImportType2NDEFFile(path)
}

// ImportType2NDEFFile reads a raw NDEF message from an explicit path.
func (s *Service) ImportType2NDEFFile(path string) (Type2NDEFImportDTO, error) {
	info, err := os.Stat(path)
	if err != nil {
		return Type2NDEFImportDTO{}, err
	}
	if info.Size() > 64<<10 {
		return Type2NDEFImportDTO{}, nfc.NewError("import NDEF", nfc.CodeInvalidArgument, "NDEF file is too large", nil)
	}
	message, err := os.ReadFile(path)
	if err != nil {
		return Type2NDEFImportDTO{}, err
	}
	records, err := type2.ParseMessage(message)
	if err != nil {
		return Type2NDEFImportDTO{}, nfc.NewError("import NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	return Type2NDEFImportDTO{
		Path: path, Length: len(message), Raw: strings.ToUpper(hex.EncodeToString(message)),
		Records: type2RecordDTOs(records),
	}, nil
}

// ExportType2NDEF writes a validated raw message to a file.
func (s *Service) ExportType2NDEF(raw string) (string, error) {
	message, err := decodeType2NDEFHex(raw)
	if err != nil {
		return "", err
	}
	path, err := s.dialogs.SaveFile("Export NDEF message", "message.ndef", "*.ndef;*.bin")
	if err != nil {
		return "", err
	}
	if strings.TrimSpace(path) == "" {
		return "", ErrDialogUnavailable
	}
	if err := os.WriteFile(path, message, 0o600); err != nil {
		return "", err
	}
	return path, nil
}

// LoadType2Source opens a source image for the Advanced copy workflow.
func (s *Service) LoadType2Source() (Type2SourceDTO, error) {
	path, err := s.dialogs.OpenFile("Load Type 2 source image", "*.json;*.bin")
	if err != nil {
		return Type2SourceDTO{}, err
	}
	return s.LoadType2SourceFile(path)
}

// LoadType2SourceFile loads an explicit source image and keeps it for copy.
func (s *Service) LoadType2SourceFile(path string) (Type2SourceDTO, error) {
	document, err := s.LoadType2DumpFile(path)
	if err != nil {
		return Type2SourceDTO{}, err
	}
	s.type2Mu.Lock()
	s.type2Source = document
	s.type2SourcePath = path
	s.type2SourceLoaded = true
	s.type2Mu.Unlock()
	return type2SourceDTO(path, document), nil
}

// GetType2Source returns the loaded copy source summary.
func (s *Service) GetType2Source() Type2SourceDTO {
	s.type2Mu.Lock()
	defer s.type2Mu.Unlock()
	if !s.type2SourceLoaded {
		return Type2SourceDTO{}
	}
	return type2SourceDTO(s.type2SourcePath, s.type2Source)
}

func (s *Service) type2SourceDocument() (type2.Document, bool) {
	s.type2Mu.Lock()
	defer s.type2Mu.Unlock()
	return s.type2Source, s.type2SourceLoaded
}

func type2SourceDTO(path string, document type2.Document) Type2SourceDTO {
	return Type2SourceDTO{
		Loaded: true, Path: path, Product: string(document.Product),
		ProductName: document.ProductName, UID: document.UID, Pages: len(document.Pages),
	}
}

// ExportType2Dump writes the loaded dump through the native save dialog. The
// extension selects JSON or the binary full-image format.
func (s *Service) ExportType2Dump() (string, error) {
	s.type2Mu.Lock()
	document := s.type2Dump
	loaded := s.type2Loaded
	s.type2Mu.Unlock()
	if !loaded {
		return "", nfc.NewError("export Type 2 dump", nfc.CodeNotOpen, "no Type 2 dump is loaded", nil)
	}
	path, err := s.dialogs.SaveFile("Export Type 2 dump", "type2-"+document.UID+".json", "*.json;*.bin")
	if err != nil {
		return "", err
	}
	if err := s.SaveType2DumpFile(path, document); err != nil {
		return "", err
	}
	return path, nil
}

// ImportType2Dump loads a JSON or binary image as the active Type 2 view.
func (s *Service) ImportType2Dump() (Type2DumpDTO, error) {
	path, err := s.dialogs.OpenFile("Load Type 2 dump", "*.json;*.bin")
	if err != nil {
		return Type2DumpDTO{}, err
	}
	document, err := s.LoadType2DumpFile(path)
	if err != nil {
		return Type2DumpDTO{}, err
	}
	s.storeType2Document(document)
	dto := type2DumpDTO(document, false)
	s.emitter.Emit(Type2EventName, dto)
	return dto, nil
}

func type2RecordDTOs(records []type2.Record) []Type2NDEFRecordDTO {
	result := make([]Type2NDEFRecordDTO, len(records))
	for index, record := range records {
		fields := make([]Type2NDEFRecordFieldDTO, len(record.Fields))
		for fieldIndex, field := range record.Fields {
			fields[fieldIndex] = Type2NDEFRecordFieldDTO{Key: field.Key, Value: field.Value, Sensitive: field.Sensitive}
		}
		result[index] = Type2NDEFRecordDTO{
			TNF: record.TNF, Type: record.Type, ID: record.ID, Payload: record.Payload,
			Text: record.Text, URI: record.URI, Kind: record.Kind, Summary: record.Summary, Fields: fields,
		}
	}
	return result
}
