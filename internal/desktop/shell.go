package desktop

import (
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
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

// shell is the native UI view-model. It is only mutated on the UI thread;
// background work posts updates through Window.Update.
type shell struct {
	api Backend

	windowMu sync.RWMutex
	window   *mygo.Window
	pending  []pendingEvent

	locale string
	router *ui.Router

	devices       []app.DeviceDTO
	selectedID    string
	connectedConn string
	connection    app.ConnectionDTO
	card          *app.CardDTO
	workbench     app.WorkbenchDTO
	sectorKeys    []app.SectorKeyDTO
	keys          []app.KeyDTO

	logs         []logEntry
	keyInput     string
	activeTask   string
	pendingOps   int
	errorText    string
	toastText    string
	blockChosen  int
	deviceList   ui.ListState
	blockList    ui.ListState
	keyList      ui.ListState
	activityList ui.ListState
}

func newShell(api Backend) *shell {
	s := &shell{
		api:    api,
		locale: "en",
		router: ui.NewRouter("/devices"),
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
	s.keys = append([]app.KeyDTO(nil), s.api.GetKeyCatalog()...)
	s.clampBlockChoice()
}

func (s *shell) locales() locales.Messages { return locales.In(s.locale) }

func (s *shell) setLocale(tag string, notify bool) {
	if tag == "" {
		tag = "en"
	}
	s.locale = tag
	if notify {
		mygo.App.SetLocale(tag)
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
		fn()
		return
	}
	window.Update(fn)
}

func (s *shell) view(c *ui.Context) {
	if s.toastText != "" {
		c.Toast(s.toastText)
		s.toastText = ""
	}
	if s.activeTask == "" && s.pendingOps == 0 && c.Shortcut(ui.Cmd, ui.KeyR) {
		s.refreshDevices()
	}
	t := c.Theme()
	ui.Row(c).Fill().AlignItems(ui.Stretch).Children(func() {
		s.sidebar(c)
		ui.Column(c).Grow(1).MinWidth(0).Background(t.Background).Children(func() {
			s.toolbar(c)
			if s.errorText != "" {
				ui.Row(c).Padding(12, 20).Gap(12).AlignItems(ui.Center).Background(t.Surface).Children(func() {
					ui.Text(c, s.locales().ErrorDetail(s.errorText)).TextColor(t.Danger).Grow(1)
					if ui.Button(c, s.locales().CommonClear()).Clicked() {
						s.errorText = ""
					}
				})
			}
			s.router.View(c, func(r *ui.Route) {
				l := s.locales()
				switch {
				case r.Match("/devices"):
					r.Title(l.NavDevices())
					s.devicesPage(c, l)
				case r.Match("/card"):
					r.Title(l.NavCard())
					s.cardPage(c, l)
				case r.Match("/blocks"):
					r.Title(l.NavBlocks())
					s.blocksPage(c, l)
				case r.Match("/keys"):
					r.Title(l.NavKeys())
					s.keysPage(c, l)
				case r.Match("/activity"):
					r.Title(l.NavActivity())
					s.activityPage(c, l)
				default:
					ui.Column(c).Fill().Padding(24).Children(func() {
						ui.Text(c, l.CommonUnknown())
					})
				}
			})
		})
	})
}

func (s *shell) sidebar(c *ui.Context) {
	t := c.Theme()
	l := s.locales()
	ui.Column(c).Width(232).Shrink(0).Background(t.Surface).BorderWidth(0, 0, 0, 1).BorderColor(t.Border).Children(func() {
		ui.Column(c).Padding(18, 18, 10).Gap(2).Children(func() {
			ui.Text(c, l.AppTitle()).FontSize(17).Bold()
			ui.Text(c, l.AppSubtitle()).FontSize(12).TextColor(t.TextMuted)
		})
		page := s.router.Path()
		if ui.Sidebar(c, &page, func() {
			ui.SidebarItem(c, "/devices", nil, l.NavDevices())
			ui.SidebarItem(c, "/card", nil, l.NavCard())
			ui.SidebarItem(c, "/blocks", nil, l.NavBlocks())
			ui.SidebarItem(c, "/keys", nil, l.NavKeys())
			ui.SidebarItem(c, "/activity", nil, l.NavActivity())
		}).Grow(1).Changed() {
			s.router.Push(page)
		}
		ui.Column(c).Padding(12).Children(func() {
			index := 0
			for i, tag := range languageTags {
				if tag == s.locale {
					index = i
				}
			}
			if ui.Segmented(c, &index, languages()...).Label(l.LanguageLabel()).Changed() {
				s.setLocale(languageTags[index], true)
			}
		})
	})
}

func languages() []string {
	names := make([]string, len(languageTags))
	for index, tag := range languageTags {
		names[index] = i18n.Autonym(tag)
	}
	return names
}

func (s *shell) toolbar(c *ui.Context) {
	t := c.Theme()
	l := s.locales()
	ui.Row(c).Height(56).Padding(20, 12).Gap(8).AlignItems(ui.Center).BorderWidth(0, 0, 1, 0).BorderColor(t.Border).Children(func() {
		ui.Text(c, s.pageTitle(l)).FontSize(19).Bold().Grow(1)
		if s.pendingOps > 0 || s.activeTask != "" {
			ui.Text(c, l.CommonWorking()).FontSize(12).TextColor(t.TextMuted)
		}
		if s.activeTask != "" {
			if ui.Button(c, l.CommonCancelTask()).Clicked() {
				s.cancelActiveTask()
			}
		}
		if ui.Button(c, l.CommonRefresh()).Disabled(s.pendingOps > 0 || s.activeTask != "").Clicked() {
			s.refreshDevices()
		}
	})
}

func (s *shell) pageTitle(l locales.Messages) string {
	switch s.router.Path() {
	case "/card":
		return l.NavCard()
	case "/blocks":
		return l.NavBlocks()
	case "/keys":
		return l.NavKeys()
	case "/activity":
		return l.NavActivity()
	default:
		return l.NavDevices()
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
			if s.connectedConn == "" {
				s.connectedConn = ""
			}
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
		})
	}()
}

func (s *shell) readCard() {
	if s.activeTask != "" || s.pendingOps > 0 || s.card == nil || !s.card.Present {
		return
	}
	s.pendingOps++
	go func() {
		task, err := s.api.StartReadCard(false)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().CardReadFailed(err.Error())
				return
			}
			s.activeTask = task.ID
			s.errorText = ""
			s.appendLog("read", app.TaskEventStarted, s.locales().ReadStarted(), 0)
		})
	}()
}

func (s *shell) scanKeys() {
	if s.activeTask != "" || s.pendingOps > 0 || s.card == nil || !s.card.Present {
		return
	}
	s.pendingOps++
	go func() {
		task, err := s.api.StartKeyScan(false)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().CardScanFailed(err.Error())
				return
			}
			s.activeTask = task.ID
			s.errorText = ""
			s.appendLog("keyscan", app.TaskEventStarted, s.locales().ScanStarted(), 0)
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

func (s *shell) clampBlockChoice() {
	if len(s.workbench.Blocks) == 0 {
		s.blockChosen = 0
		return
	}
	if s.blockChosen < 0 {
		s.blockChosen = 0
	}
	if s.blockChosen >= len(s.workbench.Blocks) {
		s.blockChosen = len(s.workbench.Blocks) - 1
	}
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

func (s *shell) kindLabel(l locales.Messages, kind string) string {
	switch kind {
	case "manufacturer":
		return l.KindManufacturer()
	case "trailer":
		return l.KindTrailer()
	default:
		return l.KindData()
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

func (s *shell) deviceCapabilities(l locales.Messages, device app.DeviceDTO) string {
	var capabilities []string
	if device.Nested {
		capabilities = append(capabilities, l.CapNested())
	}
	if device.Darkside {
		capabilities = append(capabilities, l.CapDarkside())
	}
	if device.Hardnested {
		capabilities = append(capabilities, l.CapHardnested())
	}
	if len(capabilities) == 0 {
		return l.CommonNotAvailable()
	}
	return strings.Join(capabilities, " · ")
}

// percent formats a progress value for the activity list.
func percent(progress int) string {
	if progress <= 0 {
		return ""
	}
	return fmt.Sprintf("%d%%", progress)
}
