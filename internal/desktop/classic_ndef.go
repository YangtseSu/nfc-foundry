package desktop

import (
	"fmt"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func (s *shell) classicNDEFView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Scroll(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		if !s.classicNDEF.Loaded {
			ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
				Border(1, p.border).Children(func() {
				ui.Text(c, l.ClassicNDEFPanel()).FontSize(12.5).Bold().TextColor(p.text)
				ui.Text(c, l.ClassicNDEFNoData()).FontSize(10.5).TextColor(p.muted).Wrap()
				if s.classicNDEF.Error != "" {
					ui.Text(c, l.ClassicNDEFReadFailed(s.classicNDEF.Error)).FontSize(10).TextColor(p.danger).Wrap()
				}
				ui.Row(c).Gap(8).Children(func() {
					if ui.Button(c, l.ClassicNDEFReadAction()).Height(28).FontSize(10).
						Disabled(s.writeBusy()).Clicked() {
						s.readClassicNDEF()
					}
				})
			})
			return
		}
		if s.classicNDEF.Error != "" {
			ui.Text(c, l.ClassicNDEFReadFailed(s.classicNDEF.Error)).FontSize(10.5).
				TextColor(p.danger).Wrap()
		}
		s.classicNDEFMappingPanel(c, p)
		if !s.classicNDEF.Application {
			s.classicNDEFFormatPanel(c, p)
			return
		}
		s.classicNDEFMessagePanel(c, p)
		if !s.classicNDEF.ReadOnly {
			s.type2NDEFEditor(c, p)
		} else {
			ui.Text(c, l.ClassicNDEFReadOnly()).FontSize(10.5).TextColor(p.warning).Wrap()
		}
		s.classicNDEFFormatPanel(c, p)
	})
}

func (s *shell) classicNDEFMappingPanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.ClassicNDEFPanel()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Spacer(c)
			if s.classicNDEF.MAD.Present {
				pill(c, fmt.Sprintf("MAD%d", s.classicNDEF.MAD.Version), p.accent, p.accentSoft, p.accentBorder)
			}
		})
		if !s.classicNDEF.MAD.Present {
			ui.Text(c, l.ClassicNDEFNoMAD()).FontSize(10.5).TextColor(p.warning).Wrap()
			return
		}
		fieldRow(c, p, l.CardUID(), s.classicNDEF.Card.UID)
		fieldRow(c, p, l.CardType(), s.classicNDEF.Layout)
		fieldRow(c, p, l.ClassicNDEFMADVersion(fmt.Sprintf("%d", s.classicNDEF.MAD.Version)), fmt.Sprintf("GPB 0x%02X", s.classicNDEF.MAD.GPB))
		if len(s.classicNDEF.MAD.NDEFSectors) != 0 {
			fieldRow(c, p, l.ClassicNDEFMADSectors(formatSectorRange(s.classicNDEF.MAD.NDEFSectors)), l.Type2Capacity(s.classicNDEF.Capacity))
		} else {
			fieldRow(c, p, l.ClassicNDEFFreeSectors(formatSectorRange(s.classicNDEF.MAD.FreeSectors)), l.CommonNotAvailable())
		}
		state := l.Type2Writable()
		if s.classicNDEF.ReadOnly {
			state = l.Type2ReadOnly()
		}
		if s.classicNDEF.Application {
			fieldRow(c, p, l.ClassicNDEFPanel(), state)
		}
	})
}

func (s *shell) classicNDEFMessagePanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Padding(14).Gap(10).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.Type2NDEFPanel()).FontSize(12.5).Bold().TextColor(p.text)
			if s.classicNDEF.NDEF != nil {
				ui.Text(c, l.Type2Records(len(s.classicNDEF.NDEF.Records))).FontSize(10).TextColor(p.faint)
			}
			ui.Spacer(c)
			revealLabel := l.NdefRevealSecrets()
			if s.ndefReveal {
				revealLabel = l.NdefHideSecrets()
			}
			if linkButton(c, p, revealLabel).Clicked() {
				s.ndefReveal = !s.ndefReveal
			}
			rawLabel := l.NdefShowRaw()
			if s.ndefShowRaw {
				rawLabel = l.NdefHideRaw()
			}
			if linkButton(c, p, rawLabel).Clicked() {
				s.ndefShowRaw = !s.ndefShowRaw
			}
		})
		if s.classicNDEF.NDEF == nil || !s.classicNDEF.NDEF.Present {
			ui.Text(c, l.Type2NoNDEF()).FontSize(10.5).TextColor(p.muted).Wrap()
			return
		}
		for index, record := range s.classicNDEF.NDEF.Records {
			s.ndefRecordCard(c, p, index, record)
		}
		if s.ndefShowRaw {
			ui.Text(c, l.NdefRawMessage()).FontSize(9.5).Bold().TextColor(p.faint)
			ui.Text(c, spacedHex(s.classicNDEF.NDEF.Raw)).Font("monospace").FontSize(9).
				TextColor(p.faint).Selectable().Wrap()
		}
	})
}

func (s *shell) classicNDEFFormatPanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Text(c, l.ClassicNDEFFormatPanel()).FontSize(12.5).Bold().TextColor(p.text)
		ui.Text(c, l.ClassicNDEFFormatHint()).FontSize(9.5).TextColor(p.muted).Wrap()
		if !s.advanced {
			ui.Text(c, l.ClassicNDEFFormatEnableAdvanced()).FontSize(10).TextColor(p.warning).Wrap()
			return
		}
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.ClassicNDEFFormatStart()).FontSize(10).TextColor(p.muted).Width(90)
			if ui.TextInput(c, &s.classicFormatStart).Width(70).Changed() {
				s.classicFormatPlan = nil
			}
			ui.Text(c, l.ClassicNDEFFormatEnd()).FontSize(10).TextColor(p.muted).Width(80)
			if ui.TextInput(c, &s.classicFormatEnd).Width(70).Changed() {
				s.classicFormatPlan = nil
			}
			if ui.Button(c, l.ClassicNDEFFormatPreview()).Height(28).FontSize(10).
				Disabled(s.writeBusy()).Clicked() {
				s.previewClassicFormat()
			}
		})
		if s.classicFormatPlan == nil {
			return
		}
		plan := *s.classicFormatPlan
		if !plan.Valid {
			ui.Text(c, l.ClassicNDEFFormatInvalid(plan.Error)).FontSize(10).TextColor(p.danger).Wrap()
			return
		}
		fieldRow(c, p, l.ClassicNDEFMADSectors(formatSectorRange(plan.Sectors)), l.Type2Capacity(plan.Capacity))
		ui.Text(c, l.ClassicNDEFBlocks(strconv.Itoa(len(plan.DataBlocks)), strconv.Itoa(len(plan.Trailer)), strconv.Itoa(len(plan.MADBlocks)))).
			FontSize(9.5).TextColor(p.muted).Wrap()
		for _, warning := range plan.Warnings {
			ui.Text(c, warning).FontSize(9.5).TextColor(p.warning).Wrap()
		}
		if plan.BackupRoot != "" {
			ui.Text(c, l.Type2PlanBackupRoot(plan.BackupRoot)).FontSize(9.5).TextColor(p.faint).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.ClassicNDEFFormatAction()).Height(28).FontSize(10).
				Disabled(s.writeBusy()).Clicked() {
				s.classicFormatOpen = true
				s.classicFormatConfirm = ""
			}
		})
	})
}

func (s *shell) readClassicNDEF() {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		_, err := s.api.StartClassicNDEFRead()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicNDEFReadFailed(err.Error())
				return
			}
			s.errorText = ""
		})
	}()
}

func (s *shell) previewClassicFormat() {
	if s.writeBusy() {
		return
	}
	sectors, err := parseClassicFormatSectors(s.classicFormatStart, s.classicFormatEnd)
	if err != nil {
		plan := app.ClassicNDEFFormatPreviewDTO{Valid: false, Error: err.Error(), Warnings: []string{}}
		s.classicFormatPlan = &plan
		return
	}
	s.pendingOps++
	go func() {
		plan, err := s.api.PreviewClassicNDEFFormat(sectors)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				value := app.ClassicNDEFFormatPreviewDTO{Valid: false, Error: err.Error(), Warnings: []string{}}
				s.classicFormatPlan = &value
				return
			}
			s.classicFormatPlan = &plan
			s.errorText = ""
		})
	}()
}

func (s *shell) classicNDEFFormatDialog(c *ui.Context) {
	if !s.classicFormatOpen || s.classicFormatPlan == nil || !s.classicFormatPlan.Valid {
		return
	}
	l := s.locales()
	plan := *s.classicFormatPlan
	p := paletteFor(c.Theme())
	ui.Modal(c, &s.classicFormatOpen, func() {
		ui.Text(c, l.ClassicNDEFFormatConfirmTitle()).FontSize(15).Bold().TextColor(p.danger)
		ui.Text(c, l.Type2PlanCard(plan.Card.UID, plan.Layout)).FontSize(10.5).TextColor(p.muted).Wrap()
		ui.Text(c, l.ClassicNDEFFormatConfirmMessage(formatSectorRange(plan.Sectors))).FontSize(10).Wrap()
		for _, change := range plan.Changes {
			ui.Text(c, fmt.Sprintf("%s: %s → %s", change.Target, change.Old, change.New)).
				FontSize(9.5).Wrap()
		}
		ui.Text(c, l.ClassicNDEFFormatBackup(strconv.Itoa(len(plan.DataBlocks)+len(plan.Trailer)+len(plan.MADBlocks)))).
			FontSize(9.5).TextColor(p.warning).Wrap()
		if plan.BackupRoot != "" {
			ui.Text(c, l.Type2PlanBackupRoot(plan.BackupRoot)).FontSize(9.5).Wrap()
		}
		ui.Text(c, l.Type2PlanTypeUID(plan.Card.UID)).FontSize(10).Bold()
		ui.TextInput(c, &s.classicFormatConfirm).Placeholder(plan.Card.UID).AutoFocus()
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.classicFormatOpen = false
			}
			if ui.Button(c, l.Type2PlanConfirm()).Disabled(s.classicFormatConfirm != plan.Card.UID).Clicked() {
				request := app.ClassicNDEFFormatRequestDTO{Sectors: plan.Sectors, Confirmation: s.classicFormatConfirm}
				s.classicFormatOpen = false
				s.classicFormatPlan = nil
				s.startClassicNDEFFormat(request)
			}
		})
	})
}

func (s *shell) startClassicNDEFFormat(request app.ClassicNDEFFormatRequestDTO) {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		_, err := s.api.StartClassicNDEFFormat(request)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicNDEFFormatFailed(err.Error())
				return
			}
			s.errorText = ""
		})
	}()
}

func (s *shell) requestClassicNDEFWrite() {
	if s.writeBusy() {
		return
	}
	switch s.ndefMode {
	case "raw":
		preview, err := s.api.PreviewNDEFRaw(app.Type2NDEFRawRequestDTO{Raw: s.ndefRaw})
		if err != nil {
			s.errorText = s.locales().ClassicNDEFWriteFailed(err.Error())
			return
		}
		s.classicWriteIntent = &type2WriteIntent{Kind: "raw", Raw: s.ndefRaw, Length: preview.Length}
	case "records":
		records := s.ndefRecordInputs()
		preview, err := s.api.PreviewNDEFRecords(app.Type2NDEFRecordsRequestDTO{Records: records})
		if err != nil {
			s.errorText = s.locales().ClassicNDEFWriteFailed(err.Error())
			return
		}
		s.classicWriteIntent = &type2WriteIntent{Kind: "records", Records: records, Length: preview.Length}
	default:
		template, ok := s.ndefCurrentTemplate()
		if !ok {
			return
		}
		fields := make(map[string]string, len(template.Fields))
		for _, field := range template.Fields {
			fields[field.ID] = *s.ndefField(field.ID)
		}
		request := app.Type2NDEFWriteStartRequestDTO{Template: template.ID, Fields: fields}
		preview, err := s.api.PreviewNDEFTemplate(request)
		if err != nil {
			s.errorText = s.locales().ClassicNDEFWriteFailed(err.Error())
			return
		}
		s.classicWriteIntent = &type2WriteIntent{Kind: "ndef", Template: template.ID, Fields: fields, Length: preview.Length}
	}
	s.classicWriteOpen = true
}

func (s *shell) classicNDEFWriteDialog(c *ui.Context) {
	if !s.classicWriteOpen || s.classicWriteIntent == nil {
		return
	}
	l := s.locales()
	intent := *s.classicWriteIntent
	title := l.Type2ConfirmWrite()
	message := l.ClassicNDEFWriteHint()
	switch intent.Kind {
	case "raw":
		title = l.NdefConfirmRawTitle(intent.Length)
		message = l.NdefConfirmRawMessage(intent.Length) + "\n" + l.ClassicNDEFWriteHint()
	case "records":
		title = l.NdefConfirmRecordsTitle(len(intent.Records), intent.Length)
		message = l.NdefConfirmRecordsMessage(len(intent.Records), intent.Length) + "\n" + l.ClassicNDEFWriteHint()
	default:
		template := ndefTemplateLabel(l, intent.Template)
		title = l.NdefConfirmTemplateTitle(template)
		message = l.NdefConfirmTemplateMessage(template, intent.Length) + "\n" + l.ClassicNDEFWriteHint()
	}
	result := ui.AlertDialog(c, &s.classicWriteOpen, title, message, l.CommonCancel(), l.Type2ConfirmWrite())
	if result == -1 {
		return
	}
	s.classicWriteOpen = false
	s.classicWriteIntent = nil
	if result != 1 {
		return
	}
	s.startClassicNDEFWrite(intent)
}

func (s *shell) startClassicNDEFWrite(intent type2WriteIntent) {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		var err error
		switch intent.Kind {
		case "raw":
			_, err = s.api.StartNDEFRawWrite(app.Type2NDEFRawRequestDTO{Raw: intent.Raw})
		case "records":
			_, err = s.api.StartNDEFRecordsWrite(app.Type2NDEFRecordsRequestDTO{Records: intent.Records})
		default:
			_, err = s.api.StartNDEFTemplateWrite(app.Type2NDEFWriteStartRequestDTO{Template: intent.Template, Fields: intent.Fields})
		}
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicNDEFWriteFailed(err.Error())
				return
			}
			s.errorText = ""
			s.toastText = s.locales().Type2WriteStarted()
		})
	}()
}

func (s *shell) classicNDEFInspector(c *ui.Context, p palette) {
	l := s.locales()
	if !s.classicNDEF.Loaded {
		s.inspectorEmpty(c, p, l.ClassicNDEFNoData())
		return
	}
	s.inspectorCard(c, p, func() {
		ui.Text(c, l.ClassicNDEFPanel()).FontSize(11.5).Bold().TextColor(p.text)
		ui.Column(c).Gap(7).Children(func() {
			monoFieldRow(c, p, l.CardUID(), s.classicNDEF.Card.UID)
			fieldRow(c, p, l.CardType(), s.classicNDEF.Layout)
			if s.classicNDEF.MAD.Present {
				fieldRow(c, p, l.ClassicNDEFMADVersion(fmt.Sprintf("%d", s.classicNDEF.MAD.Version)), fmt.Sprintf("GPB 0x%02X", s.classicNDEF.MAD.GPB))
			}
			if s.classicNDEF.Application {
				fieldRow(c, p, l.ClassicNDEFMADSectors(formatSectorRange(s.classicNDEF.Sectors)), l.Type2Capacity(s.classicNDEF.Capacity))
				if s.classicNDEF.NDEF != nil && s.classicNDEF.NDEF.Present {
					summary := s.classicNDEF.NDEF.Summary
					if summary == "" {
						summary = l.Type2Records(len(s.classicNDEF.NDEF.Records))
					}
					fieldRow(c, p, l.Type2NDEFPanel(), summary)
				} else {
					fieldRow(c, p, l.Type2NDEFPanel(), l.Type2NoNDEF())
				}
			} else if s.classicNDEF.MAD.Present {
				fieldRow(c, p, l.ClassicNDEFFreeSectors(formatSectorRange(s.classicNDEF.MAD.FreeSectors)), l.ClassicNDEFNoApplication())
			}
		})
	})
}

func parseClassicFormatSectors(start, end string) ([]int, error) {
	first, err := strconv.Atoi(strings.TrimSpace(start))
	if err != nil {
		return nil, fmt.Errorf("first sector must be a number")
	}
	last, err := strconv.Atoi(strings.TrimSpace(end))
	if err != nil {
		return nil, fmt.Errorf("last sector must be a number")
	}
	if first < 1 || last < first || last > 39 {
		return nil, fmt.Errorf("sector range must be ascending and between 1 and 39")
	}
	if first <= 16 && last >= 16 {
		return nil, fmt.Errorf("sector 16 is the MAD2 sector and cannot store NDEF")
	}
	sectors := make([]int, 0, last-first+1)
	for sector := first; sector <= last; sector++ {
		sectors = append(sectors, sector)
	}
	return sectors, nil
}

func formatSectorRange(sectors []int) string {
	if len(sectors) == 0 {
		return "none"
	}
	if len(sectors) == 1 {
		return strconv.Itoa(sectors[0])
	}
	return fmt.Sprintf("%d-%d", sectors[0], sectors[len(sectors)-1])
}
