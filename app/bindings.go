package app

import (
	"context"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/diagnostic"
)

// Bindings is the UI-agnostic application facade used by the native shell and,
// later, by nfctl. It exposes DTOs only; no UI framework type crosses it.

// Bindings is the narrow object the presentation layer calls.
type Bindings struct {
	service *Service
}

// NewBindings wraps a service for a presentation surface.
func NewBindings(service *Service) *Bindings {
	return &Bindings{service: service}
}

// Service returns the underlying service for lifecycle handling.
func (b *Bindings) Service() *Service { return b.service }

// GetDashboard returns the current device, card, workbench, and key snapshot.
func (b *Bindings) GetDashboard() DashboardDTO { return b.service.Dashboard() }

func (b *Bindings) GetDiagnostics() diagnostic.Report { return b.service.Diagnostics() }

func (b *Bindings) OpenCommandTerminal() error { return b.service.OpenCommandTerminal() }

func (b *Bindings) RefreshDevices() ([]DeviceDTO, error) { return b.service.RefreshDevices() }

func (b *Bindings) ConnectDevice(connString string) error {
	return b.service.ConnectDevice(connString)
}

func (b *Bindings) DisconnectDevice() error { return b.service.DisconnectDevice() }

func (b *Bindings) GetKeyCatalog() []KeyDTO { return b.service.KeyCatalog() }

func (b *Bindings) AddKey(value string) ([]KeyDTO, error) { return b.service.AddKey(value) }

func (b *Bindings) DeleteKey(id string) ([]KeyDTO, error) { return b.service.DeleteKey(id) }

func (b *Bindings) RenameKey(id, name string) ([]KeyDTO, error) {
	return b.service.RenameKey(id, name)
}

func (b *Bindings) GetKeySets() []KeySetDTO { return b.service.KeySets() }

func (b *Bindings) CreateVerifiedKeySet(name string) ([]KeySetDTO, error) {
	return b.service.CreateVerifiedKeySet(name)
}

func (b *Bindings) RenameKeySet(id, name string) ([]KeySetDTO, error) {
	return b.service.RenameKeySet(id, name)
}

func (b *Bindings) DeleteKeySet(id string) ([]KeySetDTO, error) {
	return b.service.DeleteKeySet(id)
}

func (b *Bindings) ReloadKeyStore() (KeyStoreReloadDTO, error) {
	return b.service.ReloadKeyStore()
}

func (b *Bindings) ExportKeySet(name, format string) (ExportResultDTO, error) {
	extension := "*.dic;*.txt"
	fileName := "nfc-foundry-key-set.dic"
	switch format {
	case "keys":
		extension = "*.keys"
		fileName = "nfc-foundry-key-set.keys"
	case "json":
		extension = "*.json"
		fileName = "nfc-foundry-key-set.json"
	}
	path, err := b.service.dialogs.SaveFile("Export key set", fileName, extension)
	if err != nil {
		return ExportResultDTO{}, err
	}
	if path == "" {
		return ExportResultDTO{Cancelled: true}, nil
	}
	return b.service.ExportKeySetFile(name, format, path)
}

func (b *Bindings) PendingRecoveryReview() *RecoveryReviewDTO {
	return b.service.PendingRecoveryReview()
}

func (b *Bindings) SaveRecoveryReview(request RecoverySaveRequestDTO) (TaskDTO, error) {
	return b.service.SaveRecoveryReview(request)
}

func (b *Bindings) DiscardRecoveryReview() error { return b.service.DiscardRecoveryReview() }

func (b *Bindings) ImportKeyDictionary() (ImportReportDTO, error) {
	return b.service.ImportKeyDictionary()
}

// ImportKeyFileToSet imports a text dictionary or raw key array with an
// optional named key set. An empty set name keeps the import catalog-only.
func (b *Bindings) ImportKeyFileToSet(format, setName string) (ImportReportDTO, error) {
	return b.service.ImportKeyFileToSet(format, setName)
}

func (b *Bindings) ExportKeyDictionary() (ExportResultDTO, error) {
	return b.service.ExportKeyDictionary()
}

func (b *Bindings) GetWorkbench() WorkbenchDTO { return b.service.Workbench() }

// EditBlockHex edits one block in the loaded dump.
func (b *Bindings) EditBlockHex(block int, hex string) (WorkbenchDTO, error) {
	return b.service.EditBlockHex(block, hex)
}

// LoadDump opens the native dump picker.
func (b *Bindings) LoadDump(discardDirty bool) (WorkbenchDTO, error) {
	return b.service.LoadDump(discardDirty)
}

// LoadDumpFile loads a dump from an explicit path.
func (b *Bindings) LoadDumpFile(path string, discardDirty bool) (WorkbenchDTO, error) {
	return b.service.LoadDumpFile(path, discardDirty)
}

// SaveDump opens the native dump save dialog for an explicit format.
func (b *Bindings) SaveDump(format string) (SaveResultDTO, error) {
	parsed, err := ParseDumpFormat(format)
	if err != nil {
		return SaveResultDTO{}, err
	}
	return b.service.SaveDump(parsed)
}

// SaveDumpFile writes the current dump to an explicit path.
func (b *Bindings) SaveDumpFile(path string) (SaveResultDTO, error) {
	return b.service.SaveDumpFile(path)
}

// SaveDumpFileAs writes the current dump in an explicit format.
func (b *Bindings) SaveDumpFileAs(path, format string) (SaveResultDTO, error) {
	parsed, err := ParseDumpFormat(format)
	if err != nil {
		return SaveResultDTO{}, err
	}
	return b.service.SaveDumpFileAs(path, parsed)
}

// PreflightWrite validates a planned Classic write.
func (b *Bindings) PreflightWrite(request WritePreflightRequestDTO) (WritePreflightDTO, error) {
	return b.service.PreflightWrite(request)
}

// StartWrite starts a Classic write task for a successful preflight token.
func (b *Bindings) StartWrite(request WriteStartRequestDTO) (TaskDTO, error) {
	return b.service.StartWrite(request)
}

// PreflightUIDWrite validates a standalone UID write.
func (b *Bindings) PreflightUIDWrite(uid string) (UIDWritePreflightDTO, error) {
	return b.service.PreflightUIDWrite(uid)
}

// StartUIDWrite starts a standalone UID write task.
func (b *Bindings) StartUIDWrite(request UIDWriteStartRequestDTO) (TaskDTO, error) {
	return b.service.StartUIDWrite(request)
}

func (b *Bindings) StartKeyScan(discardDirty bool) (TaskDTO, error) {
	return b.service.StartKeyScan(discardDirty)
}

// Recovery engine availability for the GUI recovery panel.
func (b *Bindings) MFoCStatus() ExternalEngineStatusDTO { return b.service.MFoCStatus() }

func (b *Bindings) MFCUKStatus() ExternalEngineStatusDTO { return b.service.MFCUKStatus() }

func (b *Bindings) HardnestedStatus() ExternalEngineStatusDTO { return b.service.HardnestedStatus() }

// StartKeyRecovery runs the automatic common-keys/darkside/nested/hardnested
// pipeline and finishes with a verified read.
func (b *Bindings) StartKeyRecovery(request KeyRecoveryStartRequestDTO) (TaskDTO, error) {
	return b.service.StartKeyRecovery(request)
}

func (b *Bindings) StartMFoC(request MFoCStartRequestDTO) (TaskDTO, error) {
	return b.service.StartMFoC(request)
}

func (b *Bindings) StartMFCUKPipeline(request MFCUKStartRequestDTO) (TaskDTO, error) {
	return b.service.StartMFCUKPipeline(request)
}

func (b *Bindings) StartHardnested(request HardnestedStartRequestDTO) (TaskDTO, error) {
	return b.service.StartHardnested(request)
}

func (b *Bindings) StartReadCard(request ReadCardStartRequestDTO) (TaskDTO, error) {
	return b.service.StartReadCard(request)
}

// TransmitAPDU sends one diagnostic short APDU to the selected ISO-DEP card.
func (b *Bindings) TransmitAPDU(raw string) (APDUResponseDTO, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	return b.service.TransmitAPDU(ctx, raw)
}

func (b *Bindings) DESFireInfo() (DESFireVersionDTO, error) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	return b.service.DESFireInfo(ctx)
}

func (b *Bindings) DESFireApplications() ([]string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	return b.service.DESFireApplications(ctx)
}

func (b *Bindings) DESFireFiles(aid string) (DESFireFilesDTO, error) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	return b.service.DESFireFiles(ctx, aid)
}

func (b *Bindings) DESFireReadFile(aid string, file byte) (DESFireReadDTO, error) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	return b.service.DESFireReadFile(ctx, aid, file)
}

func (b *Bindings) ReadType4NDEF() (Type4NDEFDTO, error) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	return b.service.ReadType4NDEF(ctx)
}

func (b *Bindings) StartType4NDEFWrite(request Type4NDEFWriteRequestDTO) (TaskDTO, error) {
	return b.service.StartType4NDEFWrite(request)
}

// GetType2Dump returns the latest Type 2 memory snapshot.
func (b *Bindings) GetType2Dump() Type2DumpDTO { return b.service.Type2Snapshot() }

// StartType2Read loads the current Type 2 tag into the workbench.
func (b *Bindings) StartType2Read(password string) (TaskDTO, error) {
	return b.service.StartType2Read(password)
}

// StartType2Write starts a confirmed Type 2 page write.
func (b *Bindings) StartType2Write(request Type2WriteStartRequestDTO) (TaskDTO, error) {
	return b.service.StartType2Write(request)
}

// StartType2NDEFWrite starts a confirmed template NDEF write.
func (b *Bindings) StartType2NDEFWrite(request Type2NDEFWriteStartRequestDTO) (TaskDTO, error) {
	return b.service.StartType2NDEFWrite(request)
}

// PreviewType2NDEF validates a template and returns its encoded size.
func (b *Bindings) PreviewType2NDEF(request Type2NDEFWriteStartRequestDTO) (Type2NDEFPreviewDTO, error) {
	return b.service.PreviewType2NDEF(request)
}

// GetType2NDEFTemplates returns the common NDEF templates.
func (b *Bindings) GetType2NDEFTemplates() []Type2NDEFTemplateDTO { return Type2NDEFTemplates() }

// GetType2NDEFProviders returns the search, social, and payment presets.
func (b *Bindings) GetType2NDEFProviders() map[string][]Type2ProviderDTO {
	return Type2NDEFProviders()
}

// PreviewType2NDEFRaw validates a raw NDEF message.
func (b *Bindings) PreviewType2NDEFRaw(request Type2NDEFRawRequestDTO) (Type2NDEFPreviewDTO, error) {
	return b.service.PreviewType2NDEFRaw(request)
}

// StartType2NDEFRawWrite starts a raw NDEF write.
func (b *Bindings) StartType2NDEFRawWrite(request Type2NDEFRawRequestDTO) (TaskDTO, error) {
	return b.service.StartType2NDEFRawWrite(request)
}

// PreviewType2NDEFRecords validates a multi-record message.
func (b *Bindings) PreviewType2NDEFRecords(request Type2NDEFRecordsRequestDTO) (Type2NDEFPreviewDTO, error) {
	return b.service.PreviewType2NDEFRecords(request)
}

// StartType2NDEFRecordsWrite starts a multi-record NDEF write.
func (b *Bindings) StartType2NDEFRecordsWrite(request Type2NDEFRecordsRequestDTO) (TaskDTO, error) {
	return b.service.StartType2NDEFRecordsWrite(request)
}

// ImportType2NDEF loads a raw NDEF message through the file dialog.
func (b *Bindings) ImportType2NDEF() (Type2NDEFImportDTO, error) {
	return b.service.ImportType2NDEF()
}

// ExportType2NDEF saves a raw NDEF message through the file dialog.
func (b *Bindings) ExportType2NDEF(raw string) (string, error) {
	return b.service.ExportType2NDEF(raw)
}

// LoadType2Source loads a source image for Advanced copy.
func (b *Bindings) LoadType2Source() (Type2SourceDTO, error) {
	return b.service.LoadType2Source()
}

// GetType2Source returns the loaded copy source.
func (b *Bindings) GetType2Source() Type2SourceDTO { return b.service.GetType2Source() }

// ExportType2Dump saves the active image as JSON or binary.
func (b *Bindings) ExportType2Dump() (string, error) { return b.service.ExportType2Dump() }

// ImportType2Dump loads a JSON or binary image into the Type 2 view.
func (b *Bindings) ImportType2Dump() (Type2DumpDTO, error) { return b.service.ImportType2Dump() }

// PreviewType2Lifecycle returns the exact old/new plan for a destructive Type
// 2 operation.
func (b *Bindings) PreviewType2Lifecycle(request Type2LifecycleRequestDTO) (Type2LifecyclePlanDTO, error) {
	return b.service.PreviewType2Lifecycle(context.Background(), request)
}

// StartType2Lifecycle executes a previewed lifecycle plan.
func (b *Bindings) StartType2Lifecycle(request Type2LifecycleRequestDTO) (TaskDTO, error) {
	return b.service.StartType2Lifecycle(request)
}

// PreflightType2Protection validates a per-field protection write.
func (b *Bindings) PreflightType2Protection(request Type2ProtectionWriteRequestDTO) (Type2ProtectionPreflightDTO, error) {
	return b.service.PreflightType2Protection(request)
}

// StartType2ProtectionWrite applies a reviewed protection plan.
func (b *Bindings) StartType2ProtectionWrite(request Type2ProtectionWriteRequestDTO) (TaskDTO, error) {
	return b.service.StartType2ProtectionWrite(request)
}

func (b *Bindings) CancelTask(taskID string) error { return b.service.CancelTask(taskID) }

// StartClassicNDEFRead loads the Classic NDEF mapping into the GUI.
func (b *Bindings) StartClassicNDEFRead() (TaskDTO, error) {
	return b.service.StartClassicNDEFRead()
}

// PreviewNDEFTemplate validates a template for the selected card family.
func (b *Bindings) PreviewNDEFTemplate(request Type2NDEFWriteStartRequestDTO) (Type2NDEFPreviewDTO, error) {
	return b.service.PreviewNDEFTemplate(request)
}

// StartNDEFTemplateWrite writes a template through the selected family.
func (b *Bindings) StartNDEFTemplateWrite(request Type2NDEFWriteStartRequestDTO) (TaskDTO, error) {
	return b.service.StartNDEFTemplateWrite(request)
}

// PreviewNDEFRaw validates a raw message for the selected card family.
func (b *Bindings) PreviewNDEFRaw(request Type2NDEFRawRequestDTO) (Type2NDEFPreviewDTO, error) {
	return b.service.PreviewNDEFRaw(request)
}

// StartNDEFRawWrite writes a raw message through the selected family.
func (b *Bindings) StartNDEFRawWrite(request Type2NDEFRawRequestDTO) (TaskDTO, error) {
	return b.service.StartNDEFRawWrite(request)
}

// PreviewNDEFRecords validates a multi-record message for the selected family.
func (b *Bindings) PreviewNDEFRecords(request Type2NDEFRecordsRequestDTO) (Type2NDEFPreviewDTO, error) {
	return b.service.PreviewNDEFRecords(request)
}

// StartNDEFRecordsWrite writes a multi-record message through the selected
// family.
func (b *Bindings) StartNDEFRecordsWrite(request Type2NDEFRecordsRequestDTO) (TaskDTO, error) {
	return b.service.StartNDEFRecordsWrite(request)
}

// PreviewClassicNDEFFormat returns the formatting plan for review.
func (b *Bindings) PreviewClassicNDEFFormat(sectors []int) (ClassicNDEFFormatPreviewDTO, error) {
	return b.service.PreviewClassicNDEFFormat(sectors)
}

// StartClassicNDEFFormat runs a confirmed Classic NDEF format.
func (b *Bindings) StartClassicNDEFFormat(request ClassicNDEFFormatRequestDTO) (TaskDTO, error) {
	return b.service.StartClassicNDEFFormat(request)
}
