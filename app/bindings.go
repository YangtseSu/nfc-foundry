package app

import "github.com/YangtseSu/nfc-foundry/internal/diagnostic"

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

func (b *Bindings) ImportKeyDictionary() (ImportReportDTO, error) {
	return b.service.ImportKeyDictionary()
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

// SaveDump opens the native dump save dialog.
func (b *Bindings) SaveDump() (SaveResultDTO, error) { return b.service.SaveDump() }

// SaveDumpFile writes the current dump to an explicit path.
func (b *Bindings) SaveDumpFile(path string) (SaveResultDTO, error) {
	return b.service.SaveDumpFile(path)
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

func (b *Bindings) StartReadCard(discardDirty bool) (TaskDTO, error) {
	return b.service.StartReadCard(discardDirty)
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

func (b *Bindings) CancelTask(taskID string) error { return b.service.CancelTask(taskID) }
