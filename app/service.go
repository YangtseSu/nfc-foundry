package app

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/attack"
	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/diagnostic"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/libnfc"
	runtimebundle "github.com/YangtseSu/nfc-foundry/internal/runtime"
	"github.com/YangtseSu/nfc-foundry/internal/terminal"
	"github.com/YangtseSu/nfc-foundry/internal/workbench"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

const (
	DeviceEventName     = "nfc-foundry:device-state"
	CardEventName       = "nfc-foundry:card-state"
	TaskEventName       = "nfc-foundry:task"
	WorkbenchEventName  = "nfc-foundry:workbench"
	KeyEventName        = "nfc-foundry:sector-keys"
	KeyCatalogEventName = "nfc-foundry:key-catalog"

	TaskEventStarted   = "started"
	TaskEventProgress  = "progress"
	TaskEventCompleted = "completed"
	TaskEventCancelled = "cancelled"
)

var (
	ErrTaskIDRequired = errors.New("task ID is required")
	ErrTaskNotFound   = errors.New("task not found")
)

// EventEmitter keeps the application service independent of the UI runtime.
type EventEmitter interface {
	Emit(name string, payload any)
}

// Diagnostics runs the same read-only, no-hardware checks used by release CI.
func (s *Service) Diagnostics() diagnostic.Report {
	ctx, cancel := context.WithTimeout(context.Background(), 25*time.Second)
	defer cancel()
	currentDevice := ""
	if s.deviceManager != nil {
		currentDevice = s.deviceManager.Snapshot().Device.ConnString
	}
	return diagnostic.Run(ctx, s.mfocLocator, currentDevice)
}

// OpenCommandTerminal releases the selected reader, then opens a user-driven
// shell whose PATH includes the application's private runtime. The application
// cannot observe when a user-run command ends, so the reader intentionally
// remains disconnected until the user reconnects it in the GUI.
func (s *Service) OpenCommandTerminal() error {
	s.mu.Lock()
	busy := len(s.operations) != 0
	s.mu.Unlock()
	if busy {
		return errors.New("wait for the active NFC Foundry task to finish before opening a command terminal")
	}
	runtimeDir, err := s.mfocLocator.RuntimeRoot()
	if err != nil {
		return fmt.Errorf("locate NFC Foundry command-line runtime: %w", err)
	}
	snapshot := s.deviceManager.Snapshot()
	connString := snapshot.Device.ConnString
	if connString != "" {
		if err := s.deviceManager.Disconnect(); err != nil {
			return fmt.Errorf("release reader for command terminal: %w", err)
		}
	}
	if err := terminal.Open(terminal.Environment{RuntimeDir: runtimeDir, Device: connString}); err != nil {
		if connString != "" {
			ctx, cancel := context.WithTimeout(context.Background(), s.deviceTimeout)
			defer cancel()
			if reconnectErr := s.deviceManager.Connect(ctx, connString); reconnectErr != nil {
				return fmt.Errorf("open command terminal: %w; reconnect reader: %v", err, reconnectErr)
			}
		}
		return fmt.Errorf("open command terminal: %w", err)
	}
	return nil
}

type noopEmitter struct{}

func (noopEmitter) Emit(string, any) {}

type operationTask struct {
	cancel          context.CancelFunc
	kind            string
	cancelRequested bool
}

// Service provides the UI-facing application behaviour.
type Service struct {
	mu               sync.Mutex
	emitter          EventEmitter
	nextID           atomic.Uint64
	deviceManager    *device.Manager
	devices          []nfc.DeviceInfo
	deviceTimeout    time.Duration
	keyStore         *keys.Store
	keyStorePath     string
	bench            *workbench.Model
	keyScanner       *workflow.KeyScanService
	keyRecovery      *workflow.KeyRecoveryService
	dumpService      *workflow.DumpService
	restoreService   *workflow.RestoreService
	uidService       *workflow.UIDService
	fixtureAttack    *attack.Coordinator
	mfocAttack       *workflow.MFoCService
	mfcukPipeline    *workflow.MFCUKPipelineService
	hardnestedAttack *workflow.HardnestedService
	mfocLocator      *runtimebundle.Locator
	operations       map[string]*operationTask
	preflights       map[string]writePreflightRecord
	uidPreflights    map[string]uidPreflightRecord
	dialogs          FileDialogs
}

// NewService wires the application service to the current libnfc backend.
func NewService(emitter EventEmitter) *Service {
	if emitter == nil {
		emitter = noopEmitter{}
	}
	backend := libnfc.NewBackend()
	manager := device.NewManager(backend, func() nfc.Reader { return backend.NewReader() }, deviceEventSink{emitter: emitter}, device.DefaultConfig())
	locator := runtimebundle.DefaultLocator()
	fixtureAttack := attack.NewCoordinator(manager, attack.NewFixtureEngine(locator))
	keyStore := keys.NewStore()
	mfocAttack := workflow.NewMFoCService(manager, manager, keyStore, func(invocation attack.MFoCInvocation) attack.AttackEngine {
		return attack.NewMFoCEngine(locator, invocation)
	})
	mfcukPipeline := workflow.NewMFCUKPipelineService(manager, manager, keyStore, func(invocation attack.MFCUKInvocation) attack.AttackEngine {
		return attack.NewMFCUKEngine(locator, invocation)
	}, mfocAttack)
	hardnestedAttack := workflow.NewHardnestedService(manager, manager, keyStore, func(invocation attack.MFoCInvocation) attack.AttackEngine {
		return attack.NewHardnestedEngine(locator, invocation)
	})
	keyScanner := workflow.NewKeyScanService(manager, keyStore)
	dumpService := workflow.NewDumpService(manager)
	keyRecovery := workflow.NewKeyRecoveryService(keyScanner, keyStore, mfcukPipeline, mfocAttack, hardnestedAttack, dumpService)
	configRoot, _ := os.UserConfigDir()
	keyStorePath := ""
	uidBackupRoot := ""
	if configRoot != "" {
		keyStorePath = filepath.Join(configRoot, "NFC Foundry", "keys.json")
		uidBackupRoot = filepath.Join(configRoot, "NFC Foundry", "uid-backups")
		_ = keyStore.Load(keyStorePath)
	}
	return &Service{
		emitter:          emitter,
		deviceManager:    manager,
		deviceTimeout:    5 * time.Second,
		keyStore:         keyStore,
		keyStorePath:     keyStorePath,
		bench:            workbench.New(),
		keyScanner:       keyScanner,
		keyRecovery:      keyRecovery,
		dumpService:      dumpService,
		restoreService:   workflow.NewRestoreService(manager),
		uidService:       workflow.NewUIDService(manager, workflow.FileUIDBackupStore{Root: uidBackupRoot}, workflow.NewGen1AUIDWriter(manager, locator)),
		fixtureAttack:    fixtureAttack,
		mfocAttack:       mfocAttack,
		mfcukPipeline:    mfcukPipeline,
		hardnestedAttack: hardnestedAttack,
		mfocLocator:      locator,
		operations:       make(map[string]*operationTask),
		preflights:       make(map[string]writePreflightRecord),
		uidPreflights:    make(map[string]uidPreflightRecord),
		dialogs:          noFileDialogs{},
	}
}

// Dashboard returns the latest real device/card snapshot. Memory and key data
// remain empty until their respective specifications are implemented.
func (s *Service) Dashboard() DashboardDTO {
	s.mu.Lock()
	knownDevices := append([]nfc.DeviceInfo(nil), s.devices...)
	s.mu.Unlock()
	snapshot := s.deviceManager.Snapshot()
	devices := make([]DeviceDTO, 0, len(knownDevices)+1)
	for _, info := range knownDevices {
		devices = append(devices, deviceDTO(info))
	}
	if snapshot.Device.ConnString != "" && !containsDevice(knownDevices, snapshot.Device.ConnString) {
		devices = append(devices, deviceDTO(snapshot.Device))
	}
	var benchSnapshot workbench.Snapshot
	if s.bench != nil {
		benchSnapshot = s.bench.Snapshot()
	}
	return DashboardDTO{
		Devices:        devices,
		SelectedDevice: deviceID(snapshot.Device.ConnString),
		Connection:     connectionDTO(snapshot),
		Card:           cardPointerDTO(snapshot.Card),
		Blocks:         blockDTOs(benchSnapshot),
		Keys:           sectorKeyDTOs(s.keyStore, snapshot.Card),
		Actions: []ActionDTO{
			{ID: "read", Label: "Read card", Enabled: false, Risk: "normal"},
			{ID: "save", Label: "Save dump", Enabled: false, Risk: "normal"},
			{ID: "write", Label: "Write card", Enabled: false, Risk: "danger"},
			{ID: "scan", Label: "Scan default keys", Enabled: false, Risk: "normal"},
		},
	}
}

// RefreshDevices updates the GUI list without disturbing the open Reader.
func (s *Service) RefreshDevices() ([]DeviceDTO, error) {
	ctx, cancel := context.WithTimeout(context.Background(), s.deviceTimeout)
	defer cancel()
	devices, err := s.deviceManager.ListDevices(ctx)
	if err != nil {
		return nil, err
	}
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Device.ConnString != "" && !containsDevice(devices, snapshot.Device.ConnString) {
		devices = append(devices, snapshot.Device)
	}
	s.mu.Lock()
	s.devices = append([]nfc.DeviceInfo(nil), devices...)
	s.mu.Unlock()
	result := make([]DeviceDTO, 0, len(devices))
	for _, info := range devices {
		result = append(result, deviceDTO(info))
	}
	return result, nil
}

// ConnectDevice passes the exact user-selected connstring to DeviceManager.
func (s *Service) ConnectDevice(connString string) error {
	ctx, cancel := context.WithTimeout(context.Background(), s.deviceTimeout)
	defer cancel()
	return s.deviceManager.Connect(ctx, connString)
}

func (s *Service) DisconnectDevice() error { return s.deviceManager.Disconnect() }

// Shutdown cancels all active work when the GUI exits.
func (s *Service) Shutdown() {
	s.mu.Lock()
	for _, task := range s.operations {
		task.cancelRequested = true
		task.cancel()
	}
	s.mu.Unlock()
	s.deviceManager.Shutdown()
}

type deviceEventSink struct{ emitter EventEmitter }

func (s deviceEventSink) DeviceStateChanged(snapshot device.Snapshot) {
	s.emitter.Emit(DeviceEventName, DeviceStateEventDTO{
		Connection: connectionDTO(snapshot),
		Device:     deviceDTO(snapshot.Device),
		Card:       cardPointerDTO(snapshot.Card),
		Time:       snapshot.UpdatedAt.Format(time.RFC3339Nano),
	})
}

func (s deviceEventSink) CardChanged(event device.CardEvent) {
	s.emitter.Emit(CardEventName, CardEventDTO{
		Type: string(event.Type),
		Card: cardDTO(event.Card),
		Time: event.Time.Format(time.RFC3339Nano),
	})
}

func connectionDTO(snapshot device.Snapshot) ConnectionDTO {
	labels := map[device.Status]string{
		device.StatusDisconnected: "Disconnected",
		device.StatusConnecting:   "Connecting",
		device.StatusReady:        "Reader ready",
		device.StatusPolling:      "Polling for a card",
		device.StatusBusy:         "Reader busy",
		device.StatusRecovering:   "Recovering the reader",
		device.StatusError:        "Reader error",
	}
	detail := snapshot.Detail
	if snapshot.ErrorMessage != "" {
		detail = snapshot.ErrorMessage
	}
	return ConnectionDTO{
		Status:    string(snapshot.Status),
		Label:     labels[snapshot.Status],
		Detail:    detail,
		ErrorCode: errorCodeName(snapshot.ErrorCode),
	}
}

func cardPointerDTO(card *nfc.CardInfo) CardDTO {
	if card == nil {
		return CardDTO{Present: false, UID: "—", ATQA: "—", SAK: "—", Type: "Unknown"}
	}
	return cardDTO(*card)
}

func cardDTO(card nfc.CardInfo) CardDTO {
	cardType, label := string(nfc.InferCardType(card)), "Unknown"
	switch nfc.InferCardType(card) {
	case nfc.CardTypeMIFAREMini:
		label = "MIFARE Mini"
	case nfc.CardTypeMIFAREClassic1K:
		label = "MIFARE Classic 1K"
	case nfc.CardTypeMIFAREClassic4K:
		label = "MIFARE Classic 4K"
	}
	return CardDTO{
		Present:   true,
		UID:       formatHex(card.UID),
		UIDLength: len(card.UID),
		ATQA:      formatHex(card.ATQA[:]),
		SAK:       fmt.Sprintf("%02X", card.SAK),
		Type:      label,
		TypeCode:  cardType,
		Inferred:  cardType != string(nfc.CardTypeUnknown),
	}
}

func deviceDTO(info nfc.DeviceInfo) DeviceDTO {
	name := info.Name
	if name == "" {
		name = info.ConnString
	}
	return DeviceDTO{
		ID:         deviceID(info.ConnString),
		Name:       name,
		ConnString: info.ConnString,
		Transport:  transportLabel(info.ConnString),
		Available:  info.ConnString != "",
		Nested:     device.CapabilitiesFor(info).Nested,
		Darkside:   device.CapabilitiesFor(info).Darkside,
		Hardnested: device.CapabilitiesFor(info).Hardnested,
	}
}

func deviceID(connString string) string {
	if connString == "" {
		return ""
	}
	sum := sha256.Sum256([]byte(connString))
	return fmt.Sprintf("reader-%x", sum[:8])
}

func transportLabel(connString string) string {
	driver := connString
	if index := strings.IndexByte(driver, ':'); index >= 0 {
		driver = driver[:index]
	}
	switch driver {
	case "pn532_uart":
		return "UART / PN532"
	case "pn53x_usb":
		return "USB / PN53x"
	case "acr122_usb":
		return "USB / ACR122"
	case "acr122_pcsc", "pcsc":
		return "PC/SC"
	case "":
		return "Unknown connection"
	default:
		return driver
	}
}

func formatHex(data []byte) string {
	parts := make([]string, len(data))
	for index, value := range data {
		parts[index] = fmt.Sprintf("%02X", value)
	}
	return strings.Join(parts, " ")
}

func containsDevice(devices []nfc.DeviceInfo, connString string) bool {
	for _, info := range devices {
		if info.ConnString == connString {
			return true
		}
	}
	return false
}

func errorCodeName(code nfc.ErrorCode) string {
	switch code {
	case nfc.CodeNoCard:
		return "no_card"
	case nfc.CodeTimeout:
		return "timeout"
	case nfc.CodeAuthenticationFailed:
		return "authentication_failed"
	case nfc.CodeDeviceDisconnected:
		return "device_disconnected"
	case nfc.CodeIO:
		return "io"
	case nfc.CodeCanceled:
		return "canceled"
	case nfc.CodeInvalidArgument:
		return "invalid_argument"
	case nfc.CodeUnsupported:
		return "unsupported"
	case nfc.CodeBusy:
		return "busy"
	case nfc.CodePermission:
		return "permission"
	case nfc.CodeCardChanged:
		return "card_changed"
	case nfc.CodeNotAuthenticated:
		return "not_authenticated"
	case nfc.CodeVerificationFailed:
		return "verification_failed"
	case nfc.CodeInternal:
		return "internal"
	default:
		return ""
	}
}
