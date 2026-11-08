package desktop

import (
	"encoding/hex"
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

type type4NDEFIntent struct {
	Message []byte
	Hex     string
	Length  int
	UID     string
	ATQA    string
	SAK     string
}

func (s *shell) type4View(c *ui.Context, p palette) {
	l := s.locales()
	ui.Scroll(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Column(c).Grow(1).Gap(3).Children(func() {
				ui.Text(c, l.Type4Title()).FontSize(12.5).Bold().TextColor(p.text)
				ui.Text(c, l.Type4Intro()).FontSize(10).TextColor(p.muted).Wrap()
			})
			if ui.Button(c, l.Type4Read()).Height(28).FontSize(10).
				Disabled(s.type4Loading || s.writeBusy()).Clicked() {
				s.readType4NDEF()
			}
		})
		if !s.type4.Loaded {
			s.emptyPanelBody(c, p, l.Type4NoData())
			return
		}
		ui.Column(c).Padding(14).Gap(7).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, l.Type4CC()).FontSize(12.5).Bold().TextColor(p.text)
				status := l.Type2Writable()
				fg, bg, border := p.success, p.successSoft, p.successBorder
				if s.type4.ReadOnly {
					status, fg, bg, border = l.Type4ReadOnly(), p.warning, p.warningSoft, p.warningBorder
				}
				pill(c, status, fg, bg, border)
			})
			fieldRow(c, p, l.Type4MappingVersion(), fmt.Sprintf("%d", s.type4.CC.MappingVersion))
			fieldRow(c, p, l.Type4MLe(), fmt.Sprintf("%d", s.type4.CC.MLe))
			fieldRow(c, p, l.Type4MLc(), fmt.Sprintf("%d", s.type4.CC.MLc))
			fieldRow(c, p, l.Type4FileID(), s.type4.CC.FileID)
			fieldRow(c, p, l.Type4MaxMessage(), fmt.Sprintf("%d", s.type4.CC.MaxMessage))
			fieldRow(c, p, l.Type4ReadAccess(), fmt.Sprintf("0x%02X", s.type4.CC.ReadAccess))
			fieldRow(c, p, l.Type4WriteAccess(), fmt.Sprintf("0x%02X", s.type4.CC.WriteAccess))
		})
		if !s.type4.ReadOnly {
			s.type2NDEFEditor(c, p)
		}
		ui.Column(c).Padding(14).Gap(10).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, l.Type2NDEFPanel()).FontSize(12.5).Bold().TextColor(p.text)
				if s.type4.NDEF != nil {
					ui.Text(c, l.Type2Records(len(s.type4.NDEF.Records))).FontSize(10).TextColor(p.faint)
				}
				ui.Spacer(c)
				revealLabel := l.NdefRevealSecrets()
				if s.ndefReveal {
					revealLabel = l.NdefHideSecrets()
				}
				if linkButton(c, p, revealLabel).Clicked() {
					s.ndefReveal = !s.ndefReveal
				}
			})
			if s.type4.NDEF == nil || !s.type4.NDEF.Present {
				ui.Text(c, l.Type2NoNDEF()).FontSize(10.5).TextColor(p.muted).Wrap()
				return
			}
			for index, record := range s.type4.NDEF.Records {
				s.ndefRecordCard(c, p, index, record)
			}
		})
	})
}

func (s *shell) readType4NDEF() {
	if s.type4Loading {
		return
	}
	s.type4Loading = true
	go func() {
		dto, err := s.api.ReadType4NDEF()
		s.update(func() {
			s.type4Loading = false
			if err != nil {
				s.errorText = s.locales().Type4Failed(err.Error())
				return
			}
			s.errorText = ""
			s.type4 = dto
		})
	}()
}

func (s *shell) requestType4NDEFWrite() {
	preview := app.Type2NDEFPreviewDTO{}
	var err error
	switch s.ndefMode {
	case "raw":
		preview, err = s.api.PreviewNDEFRaw(app.Type2NDEFRawRequestDTO{Raw: s.ndefRaw})
	case "records":
		preview, err = s.api.PreviewNDEFRecords(app.Type2NDEFRecordsRequestDTO{Records: s.ndefRecordInputs()})
	default:
		template, ok := s.ndefCurrentTemplate()
		if !ok {
			return
		}
		fields := make(map[string]string, len(template.Fields))
		for _, field := range template.Fields {
			fields[field.ID] = *s.ndefField(field.ID)
		}
		preview, err = s.api.PreviewNDEFTemplate(app.Type2NDEFWriteStartRequestDTO{Template: template.ID, Fields: fields})
	}
	if err != nil {
		s.errorText = s.locales().Type4Failed(err.Error())
		return
	}
	message, err := hex.DecodeString(strings.ToUpper(preview.Message))
	if err != nil || len(message) == 0 {
		s.errorText = s.locales().Type4Failed("the preview did not contain a message")
		return
	}
	intent := &type4NDEFIntent{Message: message, Hex: preview.Message, Length: preview.Length}
	if s.card != nil {
		intent.UID, intent.ATQA, intent.SAK = s.card.UID, s.card.ATQA, s.card.SAK
	}
	s.type4WriteIntent = intent
	s.type4WriteOpen = true
}

func (s *shell) type4WriteDialog(c *ui.Context) {
	if !s.type4WriteOpen || s.type4WriteIntent == nil {
		return
	}
	l := s.locales()
	intent := *s.type4WriteIntent
	result := ui.AlertDialog(c, &s.type4WriteOpen, l.Type4WriteConfirmTitle(fmt.Sprintf("%d", intent.Length)),
		l.Type4WriteConfirmMessage(), l.CommonCancel(), l.Type4WriteConfirm())
	if result == -1 {
		return
	}
	s.type4WriteIntent = nil
	if result == 1 {
		s.startType4Write(intent)
	}
}

func (s *shell) startType4Write(intent type4NDEFIntent) {
	s.pendingOps++
	go func() {
		_, err := s.api.StartType4NDEFWrite(app.Type4NDEFWriteRequestDTO{
			Message: intent.Message, ExpectedUID: intent.UID, ExpectedATQA: intent.ATQA, ExpectedSAK: intent.SAK,
		})
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().Type4Failed(err.Error())
			}
		})
	}()
}

func (s *shell) type4Inspector(c *ui.Context, p palette) {
	l := s.locales()
	if !s.type4.Loaded {
		s.inspectorEmpty(c, p, l.Type4NoData())
		return
	}
	s.inspectorCard(c, p, func() {
		ui.Text(c, l.Type4CC()).FontSize(11).Bold().TextColor(p.text)
		fieldRow(c, p, l.Type4MappingVersion(), fmt.Sprintf("%d", s.type4.CC.MappingVersion))
		fieldRow(c, p, l.Type4MLe(), fmt.Sprintf("%d", s.type4.CC.MLe))
		fieldRow(c, p, l.Type4MLc(), fmt.Sprintf("%d", s.type4.CC.MLc))
		fieldRow(c, p, l.Type4FileID(), s.type4.CC.FileID)
		fieldRow(c, p, l.Type4MaxMessage(), fmt.Sprintf("%d", s.type4.CC.MaxMessage))
		if s.type4.ReadOnly {
			pill(c, l.Type4ReadOnly(), p.warning, p.warningSoft, p.warningBorder)
		}
	})
}
