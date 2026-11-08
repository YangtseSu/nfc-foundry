package desktop

import (
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/diagnostic"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/i18n"
	"github.com/egoist/mygo/ui"
)

var languageTags = []string{"en", "zh-Hans"}

type logEntry struct {
	time      time.Time
	kind      string
	eventType string
	message   string
	progress  int
}

type pendingEvent struct {
	name    string
	payload any
}

type tabID string

const (
	tabBlocks     tabID = "blocks"
	tabKeys       tabID = "keys"
	tabDetails    tabID = "details"
	tabMemory     tabID = "memory"
	tabNDEF       tabID = "ndef"
	tabProtection tabID = "protection"
	tabRecovery   tabID = "recovery"
	tabAPDU       tabID = "apdu"
	tabDESFire    tabID = "desfire"
)

type tabSpec struct {
	id      tabID
	label   func(locales.Messages) string
	enabled bool
	count   func(*shell) string
}

type cardFamily int

const (
	familyNone cardFamily = iota
	familyClassic
	familyType2
	familyISO14443
	familyUnknown
)

type blockRow struct {
	header bool
	sector int
	block  int
}

type blockRowKey struct {
	header bool
	sector int
	block  int
}

type classicWriteIntent struct {
	Kind      string
	Block     int
	Hex       string
	Preflight app.WritePreflightDTO
	UID       string
	UIDPlan   app.UIDWritePreflightDTO
}

type ndefRecordDraft struct {
	Kind  string
	Text  string
	URI   string
	Title string
}

// shell is the native UI view-model. It is only mutated on the UI thread;
// background work posts updates through Window.Update.
type shell struct {
	api Backend

	windowMu sync.RWMutex
	window   *mygo.Window
	pending  []pendingEvent
	stateMu  sync.Mutex

	locale string

	devices       []app.DeviceDTO
	selectedID    string
	connectedConn string
	connection    app.ConnectionDTO
	card          *app.CardDTO
	workbench     app.WorkbenchDTO
	type2         app.Type2DumpDTO
	classicNDEF   app.ClassicNDEFDTO
	sectorKeys    []app.SectorKeyDTO
	keys          []app.KeyDTO

	logs           []logEntry
	keyInput       string
	activeTask     string
	activeKind     string
	activeMessage  string
	activeProgress int
	pendingOps     int
	errorText      string
	toastText      string

	selectedTab tabID
	tabIndex    int

	advanced             bool
	classicDialog        string
	classicDialogOpen    bool
	classicIntent        *classicWriteIntent
	blockChosen          int
	blockInput           string
	blockInputBlock      int
	blockRows            []blockRow
	blockRowChosen       int
	uidInput             string
	sectorChosen         int
	type2PageChosen      int
	type2PageInput       string
	type2PageInputPage   int
	type2Password        string
	ndefTemplate         string
	ndefFields           map[string]*string
	ndefTemplates        []app.Type2NDEFTemplateDTO
	ndefProviders        map[string][]app.Type2ProviderDTO
	ndefShowRaw          bool
	ndefReveal           bool
	ndefMode             string
	ndefRaw              string
	ndefDrafts           []ndefRecordDraft
	ndefRecordRaw        map[int]bool
	type2WriteOpen       bool
	type2WriteIntent     *type2WriteIntent
	classicWriteOpen     bool
	classicWriteIntent   *type2WriteIntent
	classicFormatOpen    bool
	classicFormatPlan    *app.ClassicNDEFFormatPreviewDTO
	classicFormatStart   string
	classicFormatEnd     string
	classicFormatConfirm string

	type2Source        app.Type2SourceDTO
	type2CopyProtected bool
	type2CopySameUID   bool
	type2CopyRepeat    string
	type2ProtAuth0     string
	type2ProtAccess    string
	type2ProtPassword  string
	type2ProtPack      string
	type2ProtStatic    string
	type2ProtDynamic   string
	type2Plan          *app.Type2LifecyclePlanDTO
	type2PlanRequest   app.Type2LifecycleRequestDTO
	type2PlanOpen      bool
	type2PlanConfirm   string
	type2ProtPlan      *app.Type2ProtectionPreflightDTO
	type2ProtRequest   app.Type2ProtectionWriteRequestDTO
	type2ProtOpen      bool
	type2ProtConfirm   string
	keyChosen          string
	keyChosenRow       int

	recoveryStatuses map[string]app.ExternalEngineStatusDTO
	recoveryLoaded   bool
	recoveryLoading  bool
	recoveryIntent   string
	recoveryOpen     bool

	apduInput    string
	apduResponse *app.APDUResponseDTO
	apduError    string
	apduLoading  bool

	desfireAID     string
	desfireFileID  string
	desfireInfo    *app.DESFireVersionDTO
	desfireApps    []string
	desfireFiles   *app.DESFireFilesDTO
	desfireRead    *app.DESFireReadDTO
	desfireError   string
	desfireLoading bool

	type4            app.Type4NDEFDTO
	type4Loading     bool
	type4WriteOpen   bool
	type4WriteIntent *type4NDEFIntent

	diagnosticsOpen    bool
	diagnosticsLoading bool
	diagnostics        *diagnostic.Report

	blockList    ui.ListState
	type2List    ui.ListState
	sectorList   ui.ListState
	keyList      ui.ListState
	activityList ui.ListState
}

func newShell(api Backend) *shell {
	s := &shell{
		api:                api,
		locale:             "en",
		selectedTab:        tabBlocks,
		type2PageChosen:    -1,
		type2PageInputPage: -1,
		blockInputBlock:    -1,
		ndefTemplate:       "text",
		ndefFields:         make(map[string]*string),
		ndefMode:           "template",
		ndefRecordRaw:      make(map[int]bool),
		classicFormatStart: "1",
		classicFormatEnd:   "3",
	}
	s.reloadSnapshot()
	return s
}

func (s *shell) reloadSnapshot() {
	dashboard := s.api.GetDashboard()
	s.devices = append([]app.DeviceDTO(nil), dashboard.Devices...)
	s.selectedID = dashboard.SelectedDevice
	s.connection = dashboard.Connection
	s.sectorKeys = append([]app.SectorKeyDTO(nil), dashboard.Keys...)
	if dashboard.Card.Present {
		card := dashboard.Card
		s.card = &card
	} else {
		s.card = nil
	}
	s.connectedConn = ""
	for _, device := range s.devices {
		if device.ID == s.selectedID {
			s.connectedConn = device.ConnString
			break
		}
	}
	s.workbench = s.api.GetWorkbench()
	s.type2 = s.api.GetType2Dump()
	s.type2Source = s.api.GetType2Source()
	s.ndefTemplates = s.api.GetType2NDEFTemplates()
	s.ndefProviders = s.api.GetType2NDEFProviders()
	s.syncType2PageSelection(true)
	s.keys = append([]app.KeyDTO(nil), s.api.GetKeyCatalog()...)
	s.clampSelections()
}

func (s *shell) locales() locales.Messages { return locales.In(s.locale) }

func (s *shell) setLocale(tag string, notify bool) {
	if tag == "" {
		tag = "en"
	}
	s.locale = tag
	if !notify {
		return
	}
	mygo.App.SetLocale(tag)
	s.windowMu.RLock()
	window := s.window
	s.windowMu.RUnlock()
	if window != nil {
		window.SetTitle(s.locales().AppTitle())
		window.Invalidate()
	}
}

func (s *shell) attachWindow(window *mygo.Window) {
	s.windowMu.Lock()
	s.window = window
	pending := s.pending
	s.pending = nil
	s.windowMu.Unlock()
	for _, event := range pending {
		s.applyEvent(event.name, event.payload)
	}
}

func (s *shell) handleEvent(name string, payload any) {
	s.windowMu.RLock()
	window := s.window
	s.windowMu.RUnlock()
	if window == nil {
		s.windowMu.Lock()
		s.pending = append(s.pending, pendingEvent{name: name, payload: payload})
		s.windowMu.Unlock()
		return
	}
	window.Update(func() { s.applyEvent(name, payload) })
}

// update posts an application-state change to the UI thread. Without a window
// the change applies directly, for view tests.
func (s *shell) update(fn func()) {
	s.windowMu.RLock()
	window := s.window
	s.windowMu.RUnlock()
	if window == nil {
		s.stateMu.Lock()
		defer s.stateMu.Unlock()
		fn()
		return
	}
	window.Update(fn)
}

func (s *shell) view(c *ui.Context) {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	if s.toastText != "" {
		c.Toast(s.toastText)
		s.toastText = ""
	}
	if s.activeTask == "" && s.pendingOps == 0 && c.Shortcut(ui.Cmd, ui.KeyR) {
		s.refreshDevices()
	}
	p := paletteFor(c.Theme())
	applyWorkbenchTheme(c, p)
	ui.Column(c).Fill().Background(p.background).Children(func() {
		s.topbar(c, p)
		ui.Row(c).Grow(1).MinHeight(0).AlignItems(ui.Stretch).Children(func() {
			s.rail(c, p)
			s.workspace(c, p)
			s.sidePanel(c, p)
		})
		s.actionBar(c, p)
	})
	s.type2WriteDialog(c)
	s.type2PlanDialog(c)
	s.classicNDEFWriteDialog(c)
	s.classicNDEFFormatDialog(c)
	s.type2ProtectionDialog(c)
	s.type4WriteDialog(c)
	s.diagnosticsModal(c, p)
	s.recoveryDialog(c)
	s.classicDialogView(c)
}

func languages() []string {
	names := make([]string, len(languageTags))
	for index, tag := range languageTags {
		names[index] = i18n.Autonym(tag)
	}
	return names
}

func (s *shell) languageShort() string {
	switch s.locale {
	case "zh-Hans":
		return s.locales().LanguageShortZH()
	default:
		return s.locales().LanguageShortEN()
	}
}

func (s *shell) refreshDevices() {
	if s.pendingOps > 0 || s.activeTask != "" {
		return
	}
	s.pendingOps++
	go func() {
		devices, err := s.api.RefreshDevices()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().DevicesRefreshFailed(err.Error())
				return
			}
			s.devices = append([]app.DeviceDTO(nil), devices...)
			s.errorText = ""
		})
	}()
}

func (s *shell) connectDevice(connString string) {
	if s.pendingOps > 0 || s.activeTask != "" {
		s.toastText = s.locales().DeviceBlocked()
		return
	}
	s.pendingOps++
	go func() {
		err := s.api.ConnectDevice(connString)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().DevicesConnectFailed(err.Error())
				return
			}
			s.connectedConn = connString
			s.errorText = ""
			for _, device := range s.devices {
				if device.ConnString == connString {
					s.selectedID = device.ID
					break
				}
			}
		})
	}()
}

func (s *shell) disconnectDevice() {
	if s.pendingOps > 0 || s.activeTask != "" {
		s.toastText = s.locales().DeviceBlocked()
		return
	}
	s.pendingOps++
	go func() {
		err := s.api.DisconnectDevice()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().DevicesDisconnectFailed(err.Error())
				return
			}
			s.connectedConn = ""
			s.selectedID = ""
			s.card = nil
			s.errorText = ""
			s.clampSelections()
		})
	}()
}

func (s *shell) readCard() {
	if s.activeTask != "" || s.pendingOps > 0 || !s.cardPresent() {
		return
	}
	family := s.cardFamily()
	if family != familyClassic && family != familyType2 {
		return
	}
	password := s.type2Password
	s.pendingOps++
	go func() {
		var err error
		if family == familyType2 {
			_, err = s.api.StartType2Read(password)
		} else {
			_, err = s.api.StartReadCard(false)
		}
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().CardReadFailed(err.Error())
				return
			}
			s.errorText = ""
		})
	}()
}

func (s *shell) scanKeys() {
	if s.activeTask != "" || s.pendingOps > 0 || !s.cardPresent() || s.cardFamily() != familyClassic {
		return
	}
	s.pendingOps++
	go func() {
		_, err := s.api.StartKeyScan(false)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().CardScanFailed(err.Error())
				return
			}
			s.errorText = ""
		})
	}()
}

func (s *shell) cancelActiveTask() {
	taskID := s.activeTask
	if taskID == "" {
		return
	}
	go func() {
		_ = s.api.CancelTask(taskID)
	}()
}

func (s *shell) addKey() {
	value := strings.TrimSpace(s.keyInput)
	if value == "" || s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		keys, err := s.api.AddKey(value)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().KeysAddFailed(err.Error())
				return
			}
			s.keys = append([]app.KeyDTO(nil), keys...)
			s.keyInput = ""
			s.errorText = ""
			s.syncKeyChoice()
		})
	}()
}

func (s *shell) deleteKey(id string) {
	if s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		keys, err := s.api.DeleteKey(id)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().KeysDeleteFailed(err.Error())
				return
			}
			s.keys = append([]app.KeyDTO(nil), keys...)
			s.errorText = ""
			s.syncKeyChoice()
		})
	}()
}

func (s *shell) importKeys() {
	if s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		report, err := s.api.ImportKeyDictionary()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().KeysImportFailed(err.Error())
				return
			}
			if report.Cancelled {
				return
			}
			s.keys = append([]app.KeyDTO(nil), report.Keys...)
			l := s.locales()
			s.appendLog("keys", app.TaskEventCompleted, l.KeysImported(report.Added, report.Duplicates, report.Ignored), 100)
			if len(report.Issues) > 0 {
				s.appendLog("keys", app.TaskEventProgress, l.KeysImportIssues(len(report.Issues)), 0)
			}
			s.toastText = l.KeysImported(report.Added, report.Duplicates, report.Ignored)
			s.errorText = ""
			s.syncKeyChoice()
		})
	}()
}

func (s *shell) exportKeys() {
	if s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		result, err := s.api.ExportKeyDictionary()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().KeysExportFailed(err.Error())
				return
			}
			if result.Cancelled {
				return
			}
			l := s.locales()
			s.appendLog("keys", app.TaskEventCompleted, l.KeysExportDone(result.Count), 100)
			s.toastText = l.KeysExportDone(result.Count)
			s.errorText = ""
		})
	}()
}

func (s *shell) appendLog(kind, eventType, message string, progress int) {
	if message == "" {
		return
	}
	s.logs = append(s.logs, logEntry{time: time.Now(), kind: kind, eventType: eventType, message: message, progress: progress})
	if len(s.logs) > 500 {
		s.logs = append([]logEntry(nil), s.logs[len(s.logs)-500:]...)
	}
}

func (s *shell) clampSelections() {
	if !s.cardPresent() {
		s.sectorKeys = nil
	}
	if !s.cardPresent() || (s.type2.Loaded && s.type2.Info.UID != "" && s.card.UID != s.type2.Info.UID) {
		s.type2 = app.Type2DumpDTO{}
		s.syncType2PageSelection(true)
	}
	s.ensureTab()
	if len(s.workbench.Blocks) == 0 {
		s.blockChosen = 0
	} else {
		s.blockChosen = max(0, min(s.blockChosen, len(s.workbench.Blocks)-1))
	}
	s.rebuildBlockRows()
	s.syncBlockRowChoice()
	s.syncBlockEditor(false)
	if len(s.sectorKeys) == 0 {
		s.sectorChosen = 0
	} else {
		s.sectorChosen = max(0, min(s.sectorChosen, len(s.sectorKeys)-1))
	}
	s.syncKeyChoice()
}

func (s *shell) cardPresent() bool {
	return s.card != nil && s.card.Present
}

func cardIdentity(card *app.CardDTO) string {
	if card == nil || !card.Present {
		return ""
	}
	return card.UID + "|" + card.ATQA + "|" + card.SAK
}

func (s *shell) resetAdvanced() {
	s.advanced = false
	s.classicDialog = ""
	s.classicDialogOpen = false
	s.classicIntent = nil
	s.uidInput = ""
	s.blockInput = ""
	s.blockInputBlock = -1
	s.type2Password = ""
	s.type2Plan = nil
	s.type2PlanOpen = false
	s.type2PlanConfirm = ""
	s.classicWriteOpen = false
	s.classicWriteIntent = nil
	s.classicFormatOpen = false
	s.classicFormatPlan = nil
	s.classicFormatConfirm = ""
	s.type2ProtPlan = nil
	s.type2ProtOpen = false
	s.type2ProtConfirm = ""
	s.type2ProtAuth0 = ""
	s.type2ProtAccess = ""
	s.type2ProtPassword = ""
	s.type2ProtPack = ""
	s.type2ProtStatic = ""
	s.type2ProtDynamic = ""
	s.type2CopyProtected = false
	s.type2CopySameUID = false
	s.type2CopyRepeat = ""
	s.ndefReveal = false
}

func (s *shell) cardFamily() cardFamily {
	if !s.cardPresent() {
		return familyNone
	}
	code := strings.ToLower(s.card.TypeCode)
	switch {
	case strings.HasPrefix(code, "mifare_classic"):
		return familyClassic
	case strings.Contains(code, "ntag"), strings.Contains(code, "ultralight"), strings.Contains(code, "type_2"), strings.Contains(code, "type2"):
		return familyType2
	case strings.Contains(code, "iso14443"):
		return familyISO14443
	default:
		return familyUnknown
	}
}

func (s *shell) selectedDevice() (app.DeviceDTO, bool) {
	for _, device := range s.devices {
		if device.ID == s.selectedID {
			return device, true
		}
	}
	if s.connectedConn != "" {
		for _, device := range s.devices {
			if device.ConnString == s.connectedConn {
				return device, true
			}
		}
	}
	if len(s.devices) > 0 {
		return s.devices[0], true
	}
	return app.DeviceDTO{}, false
}

// readerSupports reports whether the selected or connected reader advertises
// an operation. Without a reader device the check does not block offline work.
func (s *shell) readerSupports(allowed func(app.DeviceDTO) bool) bool {
	device, ok := s.readerDevice()
	if !ok || !device.Known {
		return true
	}
	return allowed(device)
}

func (s *shell) readerDevice() (app.DeviceDTO, bool) {
	if s.connectedConn != "" {
		for _, device := range s.devices {
			if device.ConnString == s.connectedConn {
				return device, true
			}
		}
	}
	return s.selectedDevice()
}

func (s *shell) connected() bool {
	if s.connectedConn == "" {
		return false
	}
	switch s.connection.Status {
	case "disconnected", "error":
		return false
	}
	return true
}

func (s *shell) ensureTab() {
	specs := s.tabSpecs()
	for _, spec := range specs {
		if spec.id == s.selectedTab && spec.enabled {
			return
		}
	}
	s.selectedTab = ""
	for _, spec := range specs {
		if spec.enabled {
			s.selectedTab = spec.id
			return
		}
	}
}

func (s *shell) tabSpecs() []tabSpec {
	switch s.cardFamily() {
	case familyClassic:
		return []tabSpec{
			{id: tabBlocks, label: locales.Messages.TabBlocks, enabled: true, count: func(s *shell) string {
				if len(s.workbench.Blocks) == 0 {
					return ""
				}
				return fmt.Sprintf("%d", len(s.workbench.Blocks))
			}},
			{id: tabKeys, label: locales.Messages.TabKeys, enabled: true, count: func(s *shell) string {
				if count := s.verifiedKeyCount(); count > 0 {
					return fmt.Sprintf("%d", count)
				}
				return ""
			}},
			{id: tabRecovery, label: locales.Messages.TabRecovery, enabled: true},
			{id: tabNDEF, label: locales.Messages.TabNDEF, enabled: true},
			{id: tabDetails, label: locales.Messages.TabDetails, enabled: true},
		}
	case familyType2:
		return []tabSpec{
			{id: tabMemory, label: locales.Messages.TabMemory, enabled: true, count: func(s *shell) string {
				if !s.type2.Loaded {
					return ""
				}
				return fmt.Sprintf("%d", len(s.type2.Pages))
			}},
			{id: tabNDEF, label: locales.Messages.TabNDEF, enabled: true},
			{id: tabProtection, label: locales.Messages.TabProtection, enabled: true},
			{id: tabDetails, label: locales.Messages.TabDetails, enabled: true},
		}
	case familyISO14443:
		return []tabSpec{
			{id: tabAPDU, label: locales.Messages.TabAPDU, enabled: true},
			{id: tabDESFire, label: locales.Messages.TabDESFire, enabled: true},
			{id: tabNDEF, label: locales.Messages.TabNDEF, enabled: true},
			{id: tabDetails, label: locales.Messages.TabDetails, enabled: true},
		}
	case familyUnknown:
		return []tabSpec{
			{id: tabDetails, label: locales.Messages.TabDetails, enabled: true},
		}
	default:
		return nil
	}
}

func (s *shell) verifiedKeyCount() int {
	count := 0
	for _, key := range s.sectorKeys {
		if key.KeyAStatus == "verified" {
			count++
		}
		if key.KeyBStatus == "verified" {
			count++
		}
	}
	return count
}

func (s *shell) sectorCount() int {
	if len(s.sectorKeys) > 0 {
		return len(s.sectorKeys)
	}
	sectors := 0
	for _, block := range s.workbench.Blocks {
		if block.Sector+1 > sectors {
			sectors = block.Sector + 1
		}
	}
	return sectors
}

func (s *shell) rebuildBlockRows() {
	s.blockRows = s.blockRows[:0]
	lastSector := -1
	for index, block := range s.workbench.Blocks {
		if block.Sector != lastSector {
			s.blockRows = append(s.blockRows, blockRow{header: true, sector: block.Sector})
			lastSector = block.Sector
		}
		s.blockRows = append(s.blockRows, blockRow{sector: block.Sector, block: index})
	}
}

func (s *shell) syncBlockRowChoice() {
	for index, row := range s.blockRows {
		if !row.header && row.block == s.blockChosen {
			s.blockRowChosen = index
			return
		}
	}
	s.blockRowChosen = 0
	for index, row := range s.blockRows {
		if !row.header {
			s.blockRowChosen = index
			s.blockChosen = row.block
			return
		}
	}
}

func normalizeClassicHex(value string) string {
	replacer := strings.NewReplacer(" ", "", ":", "", "-", "")
	return strings.ToUpper(replacer.Replace(strings.TrimSpace(value)))
}

func (s *shell) syncBlockEditor(reset bool) {
	block, ok := s.selectedBlock()
	if !ok || block.Kind != "data" {
		s.blockInput = ""
		s.blockInputBlock = -1
		return
	}
	if reset || s.blockInputBlock != block.Block {
		s.blockInput = normalizeClassicHex(block.Hex)
	}
	s.blockInputBlock = block.Block
}

func (s *shell) hasWritableChanges() bool {
	for _, block := range s.workbench.DirtyBlocks {
		if block != 0 {
			return true
		}
	}
	return false
}

func (s *shell) syncKeyChoice() {
	s.keyChosenRow = -1
	if s.keyChosen != "" {
		for index, key := range s.keys {
			if key.ID == s.keyChosen {
				s.keyChosenRow = index
				return
			}
		}
	}
	s.keyChosen = ""
}

func (s *shell) selectedBlock() (app.WorkbenchBlockDTO, bool) {
	if s.blockChosen < 0 || s.blockChosen >= len(s.workbench.Blocks) {
		return app.WorkbenchBlockDTO{}, false
	}
	return s.workbench.Blocks[s.blockChosen], true
}

func (s *shell) selectedSectorKeys() (app.SectorKeyDTO, bool) {
	if s.sectorChosen < 0 || s.sectorChosen >= len(s.sectorKeys) {
		return app.SectorKeyDTO{}, false
	}
	return s.sectorKeys[s.sectorChosen], true
}

func (s *shell) selectedCatalogKey() (app.KeyDTO, bool) {
	for _, key := range s.keys {
		if key.ID == s.keyChosen {
			return key, true
		}
	}
	return app.KeyDTO{}, false
}

func (s *shell) connectionLabel(l locales.Messages) string {
	switch s.connection.Status {
	case "disconnected":
		return l.ConnectionDisconnected()
	case "connecting":
		return l.ConnectionConnecting()
	case "ready":
		return l.ConnectionReady()
	case "polling":
		return l.ConnectionPolling()
	case "busy":
		return l.ConnectionBusy()
	case "recovering":
		return l.ConnectionRecovering()
	case "error":
		return l.ConnectionError()
	default:
		return l.ConnectionUnknown()
	}
}

func (s *shell) statusLabel(l locales.Messages, status string) string {
	switch status {
	case "unread":
		return l.StatusUnread()
	case "read":
		return l.StatusRead()
	case "auth_failed":
		return l.StatusAuthFailed()
	case "read_failed":
		return l.StatusReadFailed()
	case "synthetic":
		return l.StatusSynthetic()
	case "verified":
		return l.StatusVerified()
	default:
		return l.StatusUnknown()
	}
}

func (s *shell) activityKindLabel(l locales.Messages, kind string) string {
	switch kind {
	case "read":
		return l.ActivityKindRead()
	case "keyscan":
		return l.ActivityKindKeyScan()
	case "keys":
		return l.ActivityKindKeys()
	case "key_recovery", "mfoc", "hardnested", "mfcuk_pipeline":
		return l.ActivityKindKeyRecovery()
	case "write":
		return l.ActivityKindClassicWrite()
	case "uid_write":
		return l.ActivityKindUIDWrite()
	default:
		return kind
	}
}

func (s *shell) keySourceLabel(l locales.Messages, source string) string {
	switch source {
	case "builtin":
		return l.KeysSourceBuiltin()
	case "user_input":
		return l.KeysSourceUserInput()
	case "file_import":
		return l.KeysSourceFileImport()
	case "read_context":
		return l.KeysSourceReadContext()
	case "attack_engine":
		return l.KeysSourceAttackEngine()
	default:
		return source
	}
}

// percent formats a progress value for the activity list.
func percent(progress int) string {
	if progress <= 0 {
		return ""
	}
	return fmt.Sprintf("%d%%", progress)
}
