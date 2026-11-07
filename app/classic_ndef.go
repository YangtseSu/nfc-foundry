package app

import (
	"context"
	"encoding/hex"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

// ClassicMADEntryDTO is one managed sector and its application identifier.
type ClassicMADEntryDTO struct {
	Sector int    `json:"sector"`
	AID    string `json:"aid"`
}

// ClassicMADDTO is the validated MIFARE Application Directory.
type ClassicMADDTO struct {
	Present          bool                 `json:"present"`
	Version          int                  `json:"version"`
	MultiApplication bool                 `json:"multiApplication"`
	GPB              byte                 `json:"gpb"`
	PublisherSector  int                  `json:"publisherSector"`
	NDEFSectors      []int                `json:"ndefSectors"`
	FreeSectors      []int                `json:"freeSectors"`
	Entries          []ClassicMADEntryDTO `json:"entries"`
}

// ClassicNDEFDTO is the Classic NDEF mapping and decoded message.
type ClassicNDEFDTO struct {
	Loaded      bool          `json:"loaded"`
	Card        CardDTO       `json:"card"`
	Layout      string        `json:"layout"`
	MAD         ClassicMADDTO `json:"mad"`
	Application bool          `json:"application"`
	Sectors     []int         `json:"sectors"`
	Capacity    int           `json:"capacity"`
	ReadOnly    bool          `json:"readOnly"`
	NDEF        *Type2NDEFDTO `json:"ndef,omitempty"`
	Error       string        `json:"error,omitempty"`
	ReadAt      string        `json:"readAt,omitempty"`
}

// ClassicNDEFWriteRequestDTO writes one raw NDEF message.
type ClassicNDEFWriteRequestDTO struct {
	Message []byte `json:"message"`
	DryRun  bool   `json:"dryRun"`
}

// ClassicNDEFWriteResultDTO reports exactly which blocks were changed.
type ClassicNDEFWriteResultDTO struct {
	NDEF      *ClassicNDEFDTO  `json:"ndef,omitempty"`
	Backup    string           `json:"backup,omitempty"`
	Planned   []int            `json:"planned"`
	Written   []int            `json:"written"`
	Verified  []int            `json:"verified"`
	DryRun    bool             `json:"dryRun"`
	Formatted bool             `json:"formatted"`
	Changes   []Type2ChangeDTO `json:"changes,omitempty"`
}

// ClassicNDEFFormatRequestDTO initializes a contiguous NDEF application.
type ClassicNDEFFormatRequestDTO struct {
	Sectors      []int  `json:"sectors"`
	Confirmation string `json:"confirmation,omitempty"`
	DryRun       bool   `json:"dryRun"`
}

// ClassicNDEFFormatPreviewDTO is the reviewed formatting plan.
type ClassicNDEFFormatPreviewDTO struct {
	Valid      bool             `json:"valid"`
	Error      string           `json:"error,omitempty"`
	Card       CardDTO          `json:"card"`
	Layout     string           `json:"layout"`
	Version    int              `json:"version"`
	CreateMAD  bool             `json:"createMad"`
	Sectors    []int            `json:"sectors"`
	Capacity   int              `json:"capacity"`
	DataBlocks []int            `json:"dataBlocks"`
	Trailer    []int            `json:"trailerBlocks"`
	MADBlocks  []int            `json:"madBlocks"`
	Changes    []Type2ChangeDTO `json:"changes,omitempty"`
	Warnings   []string         `json:"warnings"`
	BackupRoot string           `json:"backupRoot,omitempty"`
}

// ReadClassicNDEF reads and validates the Classic NDEF mapping.
func (s *Service) ReadClassicNDEF(ctx context.Context) (ClassicNDEFDTO, error) {
	card, layout, err := s.classicNDEFCard()
	if err != nil {
		return ClassicNDEFDTO{}, err
	}
	state, err := s.classicNDEF.Read(ctx, workflow.ClassicNDEFRequest{
		Card: card, Keys: verifiedSectorKeys(s.keyStore, card, layout),
	})
	if err != nil {
		dto := ClassicNDEFDTO{Loaded: false, Card: cardDTO(card), Layout: classicLayoutLabel(layout), Error: err.Error()}
		s.emitter.Emit(ClassicNDEFEventName, dto)
		return dto, err
	}
	s.registerClassicNDEFKeys(card, state.Keys)
	dto := classicNDEFDTO(state)
	s.emitter.Emit(ClassicNDEFEventName, dto)
	return dto, nil
}

// StartClassicNDEFRead reads NDEF as an asynchronous GUI task.
func (s *Service) StartClassicNDEFRead() (TaskDTO, error) {
	return s.startOperation("classic_ndef_read", "Read Classic NDEF", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "classic_ndef_read", Type: TaskEventProgress, Message: "reading Classic NDEF mapping"})
		dto, err := s.ReadClassicNDEF(ctx)
		if err != nil {
			return err
		}
		event := TaskEventDetailDTO{TaskID: taskID, Kind: "classic_ndef_read", Type: TaskEventProgress, Progress: 100, Message: "Classic NDEF mapping loaded", ClassicNDEF: &dto}
		s.emitOperation(event)
		return nil
	})
}

// WriteClassicNDEF writes one message to an existing Classic NDEF application.
func (s *Service) WriteClassicNDEF(ctx context.Context, request ClassicNDEFWriteRequestDTO) (ClassicNDEFWriteResultDTO, error) {
	card, layout, err := s.classicNDEFCard()
	if err != nil {
		return ClassicNDEFWriteResultDTO{}, err
	}
	result, err := s.classicNDEF.Write(ctx, workflow.ClassicNDEFWriteRequest{
		Card: card, Device: s.deviceManager.Snapshot().Device,
		Keys: verifiedSectorKeys(s.keyStore, card, layout), Message: request.Message, DryRun: request.DryRun,
	})
	s.registerClassicNDEFKeys(card, result.State.Keys)
	dto := classicNDEFWriteDTO(result)
	s.emitter.Emit(ClassicNDEFEventName, dto.NDEF)
	if err != nil {
		return dto, annotateBackup(err, result.BackupPath)
	}
	return dto, nil
}

// PreviewClassicNDEFFormat validates a formatting range without writing.
func (s *Service) PreviewClassicNDEFFormat(sectors []int) (ClassicNDEFFormatPreviewDTO, error) {
	card, layout, err := s.classicNDEFCard()
	if err != nil {
		return ClassicNDEFFormatPreviewDTO{}, err
	}
	previewCtx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	plan, err := s.classicNDEF.PreviewFormat(previewCtx, workflow.ClassicNDEFFormatRequest{
		Card: card, Keys: verifiedSectorKeys(s.keyStore, card, layout), Sectors: sectors,
	})
	if err != nil {
		return ClassicNDEFFormatPreviewDTO{
			Valid: false, Error: err.Error(), Card: cardDTO(card), Layout: classicLayoutLabel(layout),
			Warnings: []string{},
		}, nil
	}
	return ClassicNDEFFormatPreviewDTO{
		Valid: true, Card: cardDTO(card), Layout: classicLayoutLabel(layout),
		Version: plan.Version, CreateMAD: plan.CreateMAD, Sectors: nonNilInts(plan.Sectors),
		Capacity: plan.Capacity, DataBlocks: nonNilInts(plan.DataBlocks),
		Trailer: nonNilInts(plan.Trailer), MADBlocks: nonNilInts(plan.MADBlocks),
		Changes: type2ChangeDTOs(plan.Changes), Warnings: nonNilStrings(plan.Warnings),
		BackupRoot: s.classicBackupRoot,
	}, nil
}

// FormatClassicNDEF initializes a Classic NDEF application.
func (s *Service) FormatClassicNDEF(ctx context.Context, request ClassicNDEFFormatRequestDTO) (ClassicNDEFWriteResultDTO, error) {
	card, layout, err := s.classicNDEFCard()
	if err != nil {
		return ClassicNDEFWriteResultDTO{}, err
	}
	if request.Confirmation != "" && normalizeUID(request.Confirmation) != normalizeUID(formatHex(card.UID)) {
		return ClassicNDEFWriteResultDTO{}, ErrUIDConfirm
	}
	result, err := s.classicNDEF.Format(ctx, workflow.ClassicNDEFFormatRequest{
		Card: card, Device: s.deviceManager.Snapshot().Device,
		Keys: verifiedSectorKeys(s.keyStore, card, layout), Sectors: request.Sectors, DryRun: request.DryRun,
	})
	s.registerClassicNDEFKeys(card, result.State.Keys)
	dto := classicNDEFWriteDTO(result)
	s.emitter.Emit(ClassicNDEFEventName, dto.NDEF)
	if err != nil {
		return dto, annotateBackup(err, result.BackupPath)
	}
	return dto, nil
}

// StartClassicNDEFFormat runs a confirmed format as an asynchronous task.
func (s *Service) StartClassicNDEFFormat(request ClassicNDEFFormatRequestDTO) (TaskDTO, error) {
	return s.startOperation("classic_ndef_format", "Format Classic NDEF", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "classic_ndef_format", Type: TaskEventProgress, Message: "formatting Classic NDEF application"})
		result, err := s.FormatClassicNDEF(ctx, request)
		if err != nil {
			return err
		}
		text := fmt.Sprintf("formatted sectors %v; backup %s", result.Planned, result.Backup)
		s.emitOperation(TaskEventDetailDTO{
			TaskID: taskID, Kind: "classic_ndef_format", Type: TaskEventProgress, Progress: 100,
			Message: text, ClassicNDEF: result.NDEF, ClassicNDEFWrite: &result,
		})
		return nil
	})
}

// StartNDEFTemplateWrite dispatches a template write to the selected family.
func (s *Service) StartNDEFTemplateWrite(request Type2NDEFWriteStartRequestDTO) (TaskDTO, error) {
	if !s.classicCardSelected() {
		return s.StartType2NDEFWrite(request)
	}
	message, err := encodeType2NDEF(request)
	if err != nil {
		return TaskDTO{}, err
	}
	return s.startClassicNDEFMessage(message)
}

// PreviewNDEFTemplate validates a template for the selected family.
func (s *Service) PreviewNDEFTemplate(request Type2NDEFWriteStartRequestDTO) (Type2NDEFPreviewDTO, error) {
	if !s.classicCardSelected() {
		return s.PreviewType2NDEF(request)
	}
	message, err := encodeType2NDEF(request)
	if err != nil {
		return Type2NDEFPreviewDTO{}, err
	}
	return Type2NDEFPreviewDTO{Template: request.Template, Length: len(message)}, nil
}

// StartNDEFRawWrite dispatches a raw message write to the selected family.
func (s *Service) StartNDEFRawWrite(request Type2NDEFRawRequestDTO) (TaskDTO, error) {
	if !s.classicCardSelected() {
		return s.StartType2NDEFRawWrite(request)
	}
	message, err := decodeType2NDEFHex(request.Raw)
	if err != nil {
		return TaskDTO{}, err
	}
	return s.startClassicNDEFMessage(message)
}

// PreviewNDEFRaw validates a raw message for the selected family.
func (s *Service) PreviewNDEFRaw(request Type2NDEFRawRequestDTO) (Type2NDEFPreviewDTO, error) {
	if !s.classicCardSelected() {
		return s.PreviewType2NDEFRaw(request)
	}
	message, err := decodeType2NDEFHex(request.Raw)
	if err != nil {
		return Type2NDEFPreviewDTO{}, err
	}
	return Type2NDEFPreviewDTO{Template: "raw", Length: len(message)}, nil
}

// StartNDEFRecordsWrite dispatches a multi-record write to the selected family.
func (s *Service) StartNDEFRecordsWrite(request Type2NDEFRecordsRequestDTO) (TaskDTO, error) {
	if !s.classicCardSelected() {
		return s.StartType2NDEFRecordsWrite(request)
	}
	message, err := encodeType2NDEFRecords(request.Records)
	if err != nil {
		return TaskDTO{}, err
	}
	return s.startClassicNDEFMessage(message)
}

// PreviewNDEFRecords validates a multi-record message for the selected family.
func (s *Service) PreviewNDEFRecords(request Type2NDEFRecordsRequestDTO) (Type2NDEFPreviewDTO, error) {
	if !s.classicCardSelected() {
		return s.PreviewType2NDEFRecords(request)
	}
	message, err := encodeType2NDEFRecords(request.Records)
	if err != nil {
		return Type2NDEFPreviewDTO{}, err
	}
	return Type2NDEFPreviewDTO{Template: "records", Length: len(message)}, nil
}

func (s *Service) startClassicNDEFMessage(message []byte) (TaskDTO, error) {
	return s.startOperation("classic_ndef", "Write Classic NDEF message", func(ctx context.Context, taskID string) error {
		s.emitOperation(TaskEventDetailDTO{TaskID: taskID, Kind: "classic_ndef", Type: TaskEventProgress, Message: "writing Classic NDEF message"})
		result, err := s.WriteClassicNDEF(ctx, ClassicNDEFWriteRequestDTO{Message: message})
		if err != nil {
			return err
		}
		text := fmt.Sprintf("wrote %d NDEF bytes, verified %d block(s); backup %s", len(message), len(result.Verified), result.Backup)
		s.emitOperation(TaskEventDetailDTO{
			TaskID: taskID, Kind: "classic_ndef", Type: TaskEventProgress, Progress: 100,
			Message: text, ClassicNDEF: result.NDEF, ClassicNDEFWrite: &result,
		})
		return nil
	})
}

func (s *Service) classicCardSelected() bool {
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return false
	}
	switch nfc.InferCardType(*snapshot.Card) {
	case nfc.CardTypeMIFAREClassic1K, nfc.CardTypeMIFAREClassic4K, nfc.CardTypeMIFAREMini:
		return true
	default:
		return false
	}
}

func (s *Service) classicNDEFCard() (nfc.CardInfo, mifare.Layout, error) {
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return nfc.CardInfo{}, 0, ErrCardRequired
	}
	card := snapshot.Card.Clone()
	layout, err := appLayoutForCard(card)
	if err != nil {
		return nfc.CardInfo{}, 0, err
	}
	return card, layout, nil
}

func (s *Service) registerClassicNDEFKeys(card nfc.CardInfo, verified []workflow.VerifiedSectorKeys) {
	if len(verified) == 0 {
		return
	}
	cardID := keys.CardID(card)
	for _, sector := range verified {
		for _, candidate := range []struct {
			kind nfc.KeyType
			key  *nfc.Key
		}{{nfc.KeyTypeA, sector.KeyA}, {nfc.KeyTypeB, sector.KeyB}} {
			if candidate.key == nil {
				continue
			}
			record, _, err := s.keyStore.Add(*candidate.key, keys.SourceReadContext)
			if err != nil {
				continue
			}
			_ = s.keyStore.Verify(cardID, sector.Sector, candidate.kind, record.ID)
		}
	}
	_ = s.persistKeys()
	s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
	s.emitter.Emit(KeyCatalogEventName, keyDTOs(s.keyStore))
}

func classicNDEFDTO(state workflow.ClassicNDEFState) ClassicNDEFDTO {
	dto := ClassicNDEFDTO{
		Loaded: true, Card: cardDTO(state.Card), Layout: classicLayoutLabel(state.Layout),
		Application: state.Application, Sectors: nonNilInts(state.Sectors),
		Capacity: state.Capacity, ReadOnly: state.ReadOnly,
		NDEF: ndefDTO(state.NDEF), ReadAt: time.Now().UTC().Format(time.RFC3339),
	}
	dto.MAD = classicMADDTO(state.MAD)
	return dto
}

func classicMADDTO(mad mifare.MAD) ClassicMADDTO {
	result := ClassicMADDTO{
		Present: mad.Present, Version: mad.Version, MultiApplication: mad.MultiApplication,
		GPB: mad.GPB(), PublisherSector: mad.PublisherSector(),
		NDEFSectors: nonNilInts(madSectorList(mad, mifare.NDEFAID)),
		FreeSectors: nonNilInts(mad.FreeSectors()), Entries: []ClassicMADEntryDTO{},
	}
	for _, sector := range mad.ManagedSectors() {
		result.Entries = append(result.Entries, ClassicMADEntryDTO{Sector: sector, AID: fmt.Sprintf("%04X", mad.AIDs[sector])})
	}
	return result
}

func madSectorList(mad mifare.MAD, aid uint16) []int {
	var sectors []int
	for sector, value := range mad.AIDs {
		if value == aid {
			sectors = append(sectors, sector)
		}
	}
	sort.Ints(sectors)
	return sectors
}

func classicNDEFWriteDTO(result workflow.ClassicNDEFWriteResult) ClassicNDEFWriteResultDTO {
	dto := ClassicNDEFWriteResultDTO{
		Backup: result.BackupPath, Planned: nonNilInts(result.Planned),
		Written: nonNilInts(result.Written), Verified: nonNilInts(result.Verified),
		DryRun: result.DryRun, Formatted: result.Formatted,
		Changes: type2ChangeDTOs(result.Changes),
	}
	if result.State.Card.UID != nil {
		state := classicNDEFDTO(result.State)
		dto.NDEF = &state
	}
	return dto
}

func classicLayoutLabel(layout mifare.Layout) string {
	switch layout {
	case mifare.Classic1K:
		return "MIFARE Classic 1K"
	case mifare.Classic4K:
		return "MIFARE Classic 4K"
	default:
		return "MIFARE Classic"
	}
}

func normalizeUID(value string) string {
	value = strings.ToUpper(strings.TrimSpace(value))
	value = strings.NewReplacer(" ", "", ":", "", "-", "").Replace(value)
	decoded, err := hex.DecodeString(value)
	if err != nil {
		return value
	}
	return strings.ToUpper(hex.EncodeToString(decoded))
}

func nonNilStrings(values []string) []string {
	if values == nil {
		return []string{}
	}
	return append([]string{}, values...)
}
