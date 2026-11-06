package desktop

import (
	"os"
	"strings"
	"sync"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// Backend is the presentation-facing application contract. It is implemented
// by app.Bindings and replaced by a fake in UI tests.
type Backend interface {
	GetDashboard() app.DashboardDTO
	RefreshDevices() ([]app.DeviceDTO, error)
	ConnectDevice(connString string) error
	DisconnectDevice() error
	GetKeyCatalog() []app.KeyDTO
	AddKey(value string) ([]app.KeyDTO, error)
	DeleteKey(id string) ([]app.KeyDTO, error)
	ImportKeyDictionary() (app.ImportReportDTO, error)
	ExportKeyDictionary() (app.ExportResultDTO, error)
	GetWorkbench() app.WorkbenchDTO
	GetType2Dump() app.Type2DumpDTO
	StartReadCard(discardDirty bool) (app.TaskDTO, error)
	StartType2Read(password string) (app.TaskDTO, error)
	StartKeyScan(discardDirty bool) (app.TaskDTO, error)
	CancelTask(taskID string) error
}

type eventEmitter struct {
	mu   sync.RWMutex
	sink func(string, any)
}

func (e *eventEmitter) Emit(name string, payload any) {
	e.mu.RLock()
	sink := e.sink
	e.mu.RUnlock()
	if sink != nil {
		sink(name, payload)
	}
}

func (e *eventEmitter) setSink(sink func(string, any)) {
	e.mu.Lock()
	e.sink = sink
	e.mu.Unlock()
}

// Application owns the MyGo lifecycle and wires the service to the native UI.
type Application struct {
	service  *app.Service
	bindings *app.Bindings
	emitter  *eventEmitter
	shell    *shell
}

// NewApplication creates the GUI service stack without opening a window.
func NewApplication() *Application {
	configureLibNFCDiscovery()
	emitter := &eventEmitter{}
	service := app.NewService(emitter)
	service.SetFileDialogs(mygoFileDialogs{})
	bindings := app.NewBindings(service)
	shell := newShell(bindings)
	emitter.setSink(shell.handleEvent)
	return &Application{service: service, bindings: bindings, emitter: emitter, shell: shell}
}

// Run starts the native event loop. The application quits when its last
// window closes, and the service shuts down with it.
func (a *Application) Run() error {
	mygo.App.SetName("NFC Foundry")
	if !mygo.App.RequestSingleInstanceLock() {
		return nil
	}
	mygo.App.WhenReady(a.open)
	mygo.App.OnQuit(a.service.Shutdown)
	return mygo.App.Run()
}

func (a *Application) open() {
	a.shell.setLocale(mygo.App.Locale(), false)
	window := mygo.NewWindow(mygo.WindowOptions{
		Title:           a.shell.locales().AppTitle(),
		Width:           1360,
		Height:          860,
		MinWidth:        1120,
		MinHeight:       720,
		StateKey:        "workbench",
		BackgroundColor: "light-dark(#F7F8FA, #0F1216)",
		Content:         ui.View(a.shell.view),
	})
	a.shell.attachWindow(window)
	mygo.App.SetMenu(a.shell.menu())
	a.shell.refreshDevices()
	mygo.App.OnLocaleChanged(func(tag string) {
		a.shell.setLocale(tag, false)
		window.SetTitle(a.shell.locales().AppTitle())
		mygo.App.SetMenu(a.shell.menu())
		window.Invalidate()
	})
}

// configureLibNFCDiscovery supplies application-local defaults that would
// otherwise come from a system libnfc.conf. NFC Foundry deliberately does not
// read or modify that global file. Explicit user environment values always win.
func configureLibNFCDiscovery() {
	for name, value := range map[string]string{
		"LIBNFC_AUTO_SCAN":      "true",
		"LIBNFC_INTRUSIVE_SCAN": "true",
	} {
		if _, exists := os.LookupEnv(name); !exists {
			_ = os.Setenv(name, value)
		}
	}
}

// patternExtensions converts the application's semicolon-separated glob
// patterns into MyGo file-filter extensions.
func patternExtensions(pattern string) []string {
	var extensions []string
	for _, item := range strings.Split(pattern, ";") {
		item = strings.TrimSpace(item)
		item = strings.TrimPrefix(item, "*.")
		item = strings.TrimPrefix(item, "*")
		if item != "" && !strings.ContainsAny(item, "*?") {
			extensions = append(extensions, item)
		}
	}
	return extensions
}
