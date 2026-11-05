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

func (b *Bindings) StartKeyScan(discardDirty bool) (TaskDTO, error) {
	return b.service.StartKeyScan(discardDirty)
}

func (b *Bindings) StartReadCard(discardDirty bool) (TaskDTO, error) {
	return b.service.StartReadCard(discardDirty)
}

func (b *Bindings) CancelTask(taskID string) error { return b.service.CancelTask(taskID) }
