package desktop

import (
	"strings"
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
	case app.ClassicNDEFEventName:
		if event, ok := payload.(app.ClassicNDEFDTO); ok {
			s.classicNDEF = event
		}
	case app.KeyCatalogEventName:
		if event, ok := payload.([]app.KeyDTO); ok {
			s.keys = append([]app.KeyDTO(nil), event...)
			s.syncKeyChoice()
		}
	case app.KeySetEventName:
		if event, ok := payload.([]app.KeySetDTO); ok {
			s.keySets = append([]app.KeySetDTO(nil), event...)
		}
	case app.RecoveryReviewEventName:
		if review, ok := payload.(*app.RecoveryReviewDTO); ok {
			s.openRecoveryReview(review)
		}
	case app.TaskEventName:
		if event, ok := payload.(app.TaskEventDetailDTO); ok {
			if event.Type2 != nil {
				s.type2 = *event.Type2
				s.syncType2PageSelection(true)
			}
			if event.ClassicNDEF != nil {
				s.classicNDEF = *event.ClassicNDEF
			}
			if event.Type4 != nil {
				s.type4 = *event.Type4
			}
			s.applyTaskEvent(event)
		}
	}
}

func (s *shell) resetCardDialogs() {
	s.recoveryOpen = false
	s.recoveryIntent = ""
	s.type4WriteOpen = false
	s.type4WriteIntent = nil
	s.recoveryReview = nil
	s.recoveryReviewOpen = false
	s.recoverySelected = make(map[string]bool)
	s.recoveryKeyNames = make(map[string]*string)
}

func (s *shell) applyDeviceState(event app.DeviceStateEventDTO) {
	previousCard := cardIdentity(s.card)
	previousConn := s.connectedConn
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
		s.classicNDEF = app.ClassicNDEFDTO{}
	case "connecting", "ready", "polling", "busy", "recovering":
		if event.Device.ConnString != "" {
			s.connectedConn = event.Device.ConnString
			s.selectedID = event.Device.ID
		}
	}
	if previousCard != cardIdentity(s.card) || previousConn != s.connectedConn {
		if previousCard != cardIdentity(s.card) {
			s.classicNDEF = app.ClassicNDEFDTO{}
		}
		s.resetAdvanced()
		s.resetCardDialogs()
	}
	s.clampSelections()
}

func (s *shell) applyCardEvent(event app.CardEventDTO) {
	previous := cardIdentity(s.card)
	if event.Type == "removed" {
		s.card = nil
		s.classicNDEF = app.ClassicNDEFDTO{}
		s.resetAdvanced()
		s.resetCardDialogs()
		s.clampSelections()
		return
	}
	card := event.Card
	s.card = &card
	if previous != cardIdentity(s.card) {
		s.classicNDEF = app.ClassicNDEFDTO{}
		s.resetAdvanced()
		s.resetCardDialogs()
	}
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
	if event.RecoverySave != nil {
		result := *event.RecoverySave
		s.recoveryReview = nil
		s.recoveryReviewOpen = false
		s.reloadKeySnapshot()
		if len(result.Rejected) != 0 {
			first := result.Rejected[0]
			s.errorText = s.locales().RecoverySavedVerifyFailed(len(result.Rejected), result.TotalSlots, first.Sector, first.KeyType)
		} else {
			s.errorText = ""
			s.toastText = s.locales().RecoverySavedVerified(result.VerifiedSlots, result.TotalSlots)
		}
	}
	switch {
	case event.Type == "failed" && (event.Kind == "type2_write" || event.Kind == "type2_ndef" || event.Kind == "type2_lifecycle"):
		s.errorText = s.locales().Type2WriteFailed(event.Message)
	case event.Type == "failed" && event.Kind == "classic_ndef":
		s.errorText = s.locales().ClassicNDEFWriteFailed(event.Message)
	case event.Type == "failed" && event.Kind == "classic_ndef_format":
		s.errorText = s.locales().ClassicNDEFFormatFailed(event.Message)
	case event.Type == "failed" && event.Kind == "classic_ndef_read":
		s.errorText = s.locales().ClassicNDEFReadFailed(event.Message)
	case event.Type == "failed" && event.Kind == "type4_ndef":
		s.errorText = s.locales().Type4Failed(event.Message)
	case event.Type == "failed" && (event.Kind == "write" || event.Kind == "uid_write"):
		s.errorText = s.locales().ClassicWriteFailed(event.Message)
	case event.Type == "failed" && (event.Kind == "key_recovery" || event.Kind == "mfoc" || event.Kind == "hardnested" || event.Kind == "mfcuk_pipeline"):
		s.errorText = s.locales().RecoveryFailed(event.Message)
	case event.Type == "failed" && event.Kind == "recovery_save":
		s.errorText = s.locales().RecoverySaveFailed(event.Message)
		s.openRecoveryReview(s.api.PendingRecoveryReview())
	case event.Type == app.TaskEventCompleted && (event.Kind == "type2_write" || event.Kind == "type2_ndef" || event.Kind == "type2_lifecycle"):
		s.errorText = ""
		s.toastText = s.locales().Type2WriteComplete()
	case event.Type == app.TaskEventCompleted && event.Kind == "write":
		s.errorText = ""
		s.toastText = s.locales().ClassicWriteComplete()
	case event.Type == app.TaskEventCompleted && event.Kind == "classic_ndef":
		s.errorText = ""
		s.toastText = s.locales().ClassicNDEFWriteComplete()
	case event.Type == app.TaskEventCompleted && event.Kind == "classic_ndef_format":
		s.errorText = ""
		s.toastText = s.locales().ClassicNDEFFormatComplete()
	case event.Type == app.TaskEventCompleted && event.Kind == "uid_write":
		s.errorText = ""
		s.toastText = s.locales().ClassicUIDComplete()
	case event.Type == app.TaskEventCompleted && event.Kind == "type4_ndef":
		s.errorText = ""
		s.toastText = s.locales().Type4WriteComplete()
	case event.Type == app.TaskEventCompleted && (event.Kind == "key_recovery" || event.Kind == "mfoc" || event.Kind == "hardnested" || event.Kind == "mfcuk_pipeline"):
		s.errorText = ""
		s.toastText = s.locales().RecoveryComplete()
	}
	if event.UID != nil && event.UID.Verified && event.UID.NewUID != "" && s.card != nil {
		s.card.UID = event.UID.NewUID
		s.card.UIDLength = len(strings.Fields(event.UID.NewUID))
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
