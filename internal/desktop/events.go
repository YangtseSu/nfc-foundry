package desktop

import (
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
)

func (s *shell) applyEvent(name string, payload any) {
	switch name {
	case app.DeviceEventName:
		if event, ok := payload.(app.DeviceStateEventDTO); ok {
			s.applyDeviceState(event)
		}
	case app.CardEventName:
		if event, ok := payload.(app.CardEventDTO); ok {
			s.applyCardEvent(event)
		}
	case app.WorkbenchEventName:
		if event, ok := payload.(app.WorkbenchDTO); ok {
			s.workbench = event
			s.clampSelections()
		}
	case app.KeyEventName:
		if event, ok := payload.([]app.SectorKeyDTO); ok {
			s.sectorKeys = append([]app.SectorKeyDTO(nil), event...)
			s.clampSelections()
		}
	case app.Type2EventName:
		if event, ok := payload.(app.Type2DumpDTO); ok {
			s.type2 = event
			s.syncType2PageSelection(event.Loaded)
		}
	case app.KeyCatalogEventName:
		if event, ok := payload.([]app.KeyDTO); ok {
			s.keys = append([]app.KeyDTO(nil), event...)
			s.syncKeyChoice()
		}
	case app.TaskEventName:
		if event, ok := payload.(app.TaskEventDetailDTO); ok {
			if event.Type2 != nil {
				s.type2 = *event.Type2
				s.syncType2PageSelection(true)
			}
			s.applyTaskEvent(event)
		}
	}
}

func (s *shell) applyDeviceState(event app.DeviceStateEventDTO) {
	s.connection = event.Connection
	if event.Card.Present {
		card := event.Card
		s.card = &card
	} else {
		s.card = nil
	}
	if event.Device.ConnString != "" {
		found := false
		for index, device := range s.devices {
			if device.ConnString == event.Device.ConnString {
				s.devices[index] = event.Device
				found = true
				break
			}
		}
		if !found {
			s.devices = append(s.devices, event.Device)
		}
	}
	switch event.Connection.Status {
	case "disconnected", "error":
		s.connectedConn = ""
		s.selectedID = ""
	case "connecting", "ready", "polling", "busy", "recovering":
		if event.Device.ConnString != "" {
			s.connectedConn = event.Device.ConnString
			s.selectedID = event.Device.ID
		}
	}
	s.clampSelections()
}

func (s *shell) applyCardEvent(event app.CardEventDTO) {
	if event.Type == "removed" {
		s.card = nil
		s.clampSelections()
		return
	}
	card := event.Card
	s.card = &card
	s.clampSelections()
}

func (s *shell) applyTaskEvent(event app.TaskEventDetailDTO) {
	if event.Workbench != nil {
		s.workbench = *event.Workbench
		s.clampSelections()
	}
	switch event.Type {
	case app.TaskEventStarted:
		s.activeTask = event.TaskID
		s.activeKind = event.Kind
		s.activeProgress = event.Progress
		s.activeMessage = event.Message
	case app.TaskEventCompleted, app.TaskEventCancelled, "failed":
		if s.activeTask == event.TaskID {
			s.activeTask = ""
			s.activeProgress = 0
			s.activeMessage = ""
		}
	default:
		if event.TaskID != "" {
			s.activeTask = event.TaskID
			s.activeKind = event.Kind
			s.activeProgress = event.Progress
			s.activeMessage = event.Message
		}
	}
	when := time.Now()
	if event.Time != "" {
		if parsed, err := time.Parse(time.RFC3339Nano, event.Time); err == nil {
			when = parsed
		}
	}
	switch {
	case event.Type == "failed" && (event.Kind == "type2_write" || event.Kind == "type2_ndef"):
		s.errorText = s.locales().Type2WriteFailed(event.Message)
	case event.Type == app.TaskEventCompleted && (event.Kind == "type2_write" || event.Kind == "type2_ndef"):
		s.errorText = ""
		s.toastText = s.locales().Type2WriteComplete()
	}
	entry := logEntry{
		time:      when,
		kind:      event.Kind,
		eventType: event.Type,
		message:   event.Message,
		progress:  event.Progress,
	}
	if entry.message == "" {
		return
	}
	s.logs = append(s.logs, entry)
	if len(s.logs) > 500 {
		s.logs = append([]logEntry(nil), s.logs[len(s.logs)-500:]...)
	}
}
