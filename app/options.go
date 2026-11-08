package app

import (
	"os"
	"path/filepath"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/attack"
	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/libnfc"
	runtimebundle "github.com/YangtseSu/nfc-foundry/internal/runtime"
	"github.com/YangtseSu/nfc-foundry/internal/workbench"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

// Options configures a Service. Zero values select the production libnfc
// backend and the standard per-user configuration paths, so NewService and
// NewServiceWithOptions share one construction path.
type Options struct {
	Emitter           EventEmitter
	Enumerator        nfc.DeviceEnumerator
	NewReader         device.ReaderFactory
	DeviceConfig      device.Config
	Locator           *runtimebundle.Locator
	KeyStorePath      string
	UIDBackupRoot     string
	Type2BackupRoot   string
	ClassicBackupRoot string
	Type4BackupRoot   string
}

// NewService wires the application service to the current libnfc backend.
func NewService(emitter EventEmitter) *Service {
	return NewServiceWithOptions(Options{Emitter: emitter})
}

// NewServiceWithOptions builds a service from explicit dependencies. Tests and
// the CLI use it to inject a mock transport, temporary key store, and temporary
// reader lock directory.
func NewServiceWithOptions(options Options) *Service {
	emitter := options.Emitter
	if emitter == nil {
		emitter = noopEmitter{}
	}
	backend := libnfc.NewBackend()
	enumerator := options.Enumerator
	if enumerator == nil {
		enumerator = backend
	}
	newReader := options.NewReader
	if newReader == nil {
		newReader = func() nfc.Reader { return backend.NewReader() }
	}
	locator := options.Locator
	if locator == nil {
		locator = runtimebundle.DefaultLocator()
	}

	configRoot, _ := os.UserConfigDir()
	keyStorePath := options.KeyStorePath
	uidBackupRoot := options.UIDBackupRoot
	type2BackupRoot := options.Type2BackupRoot
	classicBackupRoot := options.ClassicBackupRoot
	type4BackupRoot := options.Type4BackupRoot
	if keyStorePath == "" && configRoot != "" {
		keyStorePath = filepath.Join(configRoot, "NFC Foundry", "keys.json")
	}
	if uidBackupRoot == "" && configRoot != "" {
		uidBackupRoot = filepath.Join(configRoot, "NFC Foundry", "uid-backups")
	}
	if type2BackupRoot == "" && configRoot != "" {
		type2BackupRoot = filepath.Join(configRoot, "NFC Foundry", "type2-backups")
	}
	if classicBackupRoot == "" && configRoot != "" {
		classicBackupRoot = filepath.Join(configRoot, "NFC Foundry", "classic-backups")
	}
	if type4BackupRoot == "" && configRoot != "" {
		type4BackupRoot = filepath.Join(configRoot, "NFC Foundry", "type4-backups")
	}

	sink := &deviceEventSink{emitter: emitter}
	manager := device.NewManager(enumerator, newReader, sink, options.DeviceConfig)
	keyStore := keys.NewStore()
	var keyStoreLoadErr error
	if keyStorePath != "" {
		if _, err := keyStore.LoadWithReport(keyStorePath); err != nil {
			keyStoreLoadErr = err
			keyStore = keys.NewStore()
		}
	}
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
	keyVerify := workflow.NewKeyVerifyService(manager)
	dumpService := workflow.NewDumpService(manager)
	keyRecovery := workflow.NewKeyRecoveryService(keyScanner, keyStore, mfcukPipeline, mfocAttack, hardnestedAttack, dumpService)

	service := &Service{
		emitter:           emitter,
		deviceManager:     manager,
		deviceTimeout:     5 * time.Second,
		keyStore:          keyStore,
		keyStorePath:      keyStorePath,
		keyStoreLoadErr:   keyStoreLoadErr,
		bench:             workbench.New(),
		keyScanner:        keyScanner,
		keyVerify:         keyVerify,
		keyRecovery:       keyRecovery,
		dumpService:       dumpService,
		restoreService:    workflow.NewRestoreService(manager),
		classicIO:         workflow.NewClassicIO(manager),
		uidService:        workflow.NewUIDService(manager, workflow.FileUIDBackupStore{Root: uidBackupRoot}),
		type2Service:      workflow.NewType2Service(manager, workflow.FileType2BackupStore{Root: type2BackupRoot}),
		classicNDEF:       workflow.NewClassicNDEFService(manager, dumpService, workflow.FileClassicNDEFBackupStore{Root: classicBackupRoot}),
		apduService:       workflow.NewAPDUService(manager),
		type2BackupRoot:   type2BackupRoot,
		classicBackupRoot: classicBackupRoot,
		type4BackupRoot:   type4BackupRoot,
		fixtureAttack:     attack.NewCoordinator(manager, attack.NewFixtureEngine(locator)),
		mfocAttack:        mfocAttack,
		mfcukPipeline:     mfcukPipeline,
		hardnestedAttack:  hardnestedAttack,
		mfocLocator:       locator,
		operations:        make(map[string]*operationTask),
		preflights:        make(map[string]writePreflightRecord),
		uidPreflights:     make(map[string]uidPreflightRecord),
		dialogs:           noFileDialogs{},
	}
	sink.onCardChanged = service.handleCardChanged
	return service
}
