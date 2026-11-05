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
			s.clampBlockChoice()
		}
	case app.KeyEventName:
		if event, ok := payload.([]app.SectorKeyDTO); ok {
			s.sectorKeys = append([]app.SectorKeyDTO(nil), event...)
		}
	case app.KeyCatalogEventName:
		if event, ok := payload.([]app.KeyDTO); ok {
			s.keys = append([]app.KeyDTO(nil), event...)
		}
	case app.TaskEventName:
		if event, ok := payload.(app.TaskEventDetailDTO); ok {
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
}

func (s *shell) applyCardEvent(event app.CardEventDTO) {
	if event.Type == "removed" {
		s.card = nil
		return
	}
	card := event.Card
	s.card = &card
}

func (s *shell) applyTaskEvent(event app.TaskEventDetailDTO) {
	if event.Workbench != nil {
		s.workbench = *event.Workbench
		s.clampBlockChoice()
	}
	switch event.Type {
	case app.TaskEventStarted:
		s.activeTask = event.TaskID
	case app.TaskEventCompleted, app.TaskEventCancelled, "failed":
		if s.activeTask == event.TaskID {
			s.activeTask = ""
		}
	}
	when := time.Now()
	if event.Time != "" {
		if parsed, err := time.Parse(time.RFC3339Nano, event.Time); err == nil {
			when = parsed
		}
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
