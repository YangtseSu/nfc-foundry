package desktop

import (
	"encoding/hex"
	"fmt"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

type type2WriteIntent struct {
	Kind     string
	Page     int
	Hex      string
	Template string
	Fields   map[string]string
	Raw      string
	Records  []app.Type2NDEFRecordInputDTO
	Length   int
}

func (s *shell) type2MemoryView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Grow(1).MinHeight(0).Children(func() {
		ui.Column(c).Grow(1).MinHeight(0).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Row(c).Padding(12, 14, 10).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, l.Type2MemoryPanel()).FontSize(12.5).Bold().TextColor(p.text)
				if s.type2.Loaded {
					ui.Text(c, l.Type2Capacity(s.type2.Info.UserBytes)).FontSize(10).TextColor(p.faint)
				}
				ui.Spacer(c)
				ui.Text(c, l.Type2Password()).FontSize(9.5).TextColor(p.muted).Shrink(0)
				ui.TextInput(c, &s.type2Password).Password().Placeholder(l.Type2PasswordPlaceholder()).
					Width(150).Label(l.Type2Password())
				if linkButton(c, p, l.CommonImport()).Clicked() {
					s.importType2Dump()
				}
				if linkButton(c, p, l.CommonExport()).Disabled(!s.type2.Loaded).Clicked() {
					s.exportType2Dump()
				}
			})
			if !s.type2.Loaded {
				s.emptyPanelBody(c, p, l.Type2NoData())
				return
			}
			s.type2PageEditor(c, p)
			columns := []ui.TableColumn{
				{Title: l.Type2PageColumn(), ID: "page", Width: 64, Align: ui.End, Fixed: true},
				{Title: l.Type2RegionColumn(), ID: "region", Width: 116, Fixed: true},
				{Title: l.Type2DataColumn(), ID: "data"},
				{Title: l.Type2ASCIIColumn(), ID: "ascii", Width: 92, Fixed: true},
			}
			s.type2List.Selected = &s.type2PageChosen
			s.type2List.Key = func(index int) any { return s.type2.Pages[index].Page }
			s.type2List.Label = func(index int) string {
				return fmt.Sprintf("%s %d", l.Type2PageColumn(), s.type2.Pages[index].Page)
			}
			table := ui.Table(c, &s.type2List, columns, len(s.type2.Pages), func(rowIndex, col int) {
				if rowIndex < 0 || rowIndex >= len(s.type2.Pages) {
					return
				}
				page := s.type2.Pages[rowIndex]
				switch col {
				case 0:
					ui.Textf(c, "%03d", page.Page).Font("monospace").FontSize(9.5).TextColor(p.faint)
				case 1:
					color := p.muted
					if page.Editable {
						color = p.accent
					}
					ui.Text(c, page.Region).FontSize(9.5).TextColor(color).SingleLine()
				case 2:
					ui.Text(c, spacedHex(page.Hex)).Font("monospace").FontSize(10.5).
						TextColor(p.text).SingleLine()
				case 3:
					ui.Text(c, page.ASCII).Font("monospace").FontSize(9.5).
						TextColor(p.muted).SingleLine()
				}
			}).Grow(1).MinHeight(0)
			if table.Changed() {
				s.type2PageInputPage = -1
				s.syncType2PageSelection(false)
			}
		})
	})
}

func (s *shell) type2PageEditor(c *ui.Context, p palette) {
	l := s.locales()
	page, ok := s.selectedType2Page()
	if !ok || !page.Editable {
		ui.Row(c).Padding(0, 14, 8).Children(func() {
			ui.Text(c, l.Type2SelectUserPage()).FontSize(9.5).TextColor(p.faint).Wrap()
		})
		return
	}
	ui.Row(c).Padding(0, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
		ui.Textf(c, "%s %03d", l.Type2PageColumn(), page.Page).Font("monospace").FontSize(9.5).
			TextColor(p.faint).Shrink(0)
		ui.TextInput(c, &s.type2PageInput).Placeholder(l.Type2PageHexPlaceholder()).Width(180)
		if ui.Button(c, l.Type2WritePage()).Height(28).FontSize(10).
			Disabled(s.type2WriteBusy() || strings.TrimSpace(s.type2PageInput) == "").Clicked() {
			s.requestType2PageWrite()
		}
	})
}

func (s *shell) type2NDEFView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Scroll(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		if !s.type2.Loaded {
			ui.Column(c).Grow(1).Children(func() { s.emptyPanelBody(c, p, l.Type2NoData()) })
			return
		}
		s.type2NDEFEditor(c, p)
		ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Text(c, l.Type2CapabilityPanel()).FontSize(12.5).Bold().TextColor(p.text)
			if !s.type2.CC.Valid {
				ui.Text(c, l.Type2NotFormatted()).FontSize(10.5).TextColor(p.warning).Wrap()
				if s.type2.CC.Raw != "" {
					ui.Text(c, spacedHex(s.type2.CC.Raw)).Font("monospace").FontSize(10).
						TextColor(p.muted).Selectable().Wrap()
				}
				return
			}
			status := l.Type2Writable()
			if !s.type2.CC.Writable {
				status = l.Type2ReadOnly()
			}
			fieldRow(c, p, l.CardType(), s.type2.Info.ProductName)
			fieldRow(c, p, l.Type2Capacity(s.type2.CC.Capacity), status)
			ui.Text(c, spacedHex(s.type2.CC.Raw)).Font("monospace").FontSize(10).
				TextColor(p.muted).Selectable().Wrap()
		})
		ui.Column(c).Padding(14).Gap(10).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, l.Type2NDEFPanel()).FontSize(12.5).Bold().TextColor(p.text)
				if s.type2.NDEF != nil {
					ui.Text(c, l.Type2Records(len(s.type2.NDEF.Records))).FontSize(10).TextColor(p.faint)
				}
				ui.Spacer(c)
				if linkButton(c, p, l.CommonImport()).Clicked() {
					s.importNDEFMessage()
				}
				if linkButton(c, p, l.CommonExport()).Clicked() {
					s.exportNDEFMessage()
				}
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
			switch {
			case s.type2.NDEFError != "":
				ui.Text(c, l.Type2NDEFError(s.type2.NDEFError)).FontSize(10.5).
					TextColor(p.danger).Wrap()
			case s.type2.NDEF == nil || !s.type2.NDEF.Present:
				ui.Text(c, l.Type2NoNDEF()).FontSize(10.5).TextColor(p.muted).Wrap()
			default:
				for index, record := range s.type2.NDEF.Records {
					s.ndefRecordCard(c, p, index, record)
				}
				if s.ndefShowRaw {
					ui.Text(c, l.NdefRawMessage()).FontSize(9.5).Bold().TextColor(p.faint)
					ui.Text(c, spacedHex(s.type2.NDEF.Raw)).Font("monospace").FontSize(9).
						TextColor(p.faint).Selectable().Wrap()
				}
			}
		})
	})
}

func (s *shell) ndefRecordCard(c *ui.Context, p palette, index int, record app.Type2NDEFRecordDTO) {
	l := s.locales()
	ui.Column(c).Padding(10).Gap(6).Background(p.surfaceAlt).Radius(8).
		Border(1, p.border).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			pill(c, ndefKindLabel(l, record.Kind), p.accent, p.accentSoft, p.accentBorder)
			ui.Text(c, record.Summary).FontSize(11).Bold().TextColor(p.text).
				Grow(1).Wrap()
			rawLabel := l.NdefRecordShowRaw()
			if s.ndefRecordRaw[index] {
				rawLabel = l.NdefRecordHideRaw()
			}
			if linkButton(c, p, rawLabel).Clicked() {
				s.ndefRecordRaw[index] = !s.ndefRecordRaw[index]
			}
		})
		if record.Text != "" && len(record.Fields) == 0 {
			ui.Row(c).Gap(8).AlignItems(ui.Start).Children(func() {
				ui.Text(c, record.Text).FontSize(10.5).TextColor(p.text).Grow(1).Selectable().Wrap()
				if linkButton(c, p, l.NdefCopy()).Clicked() {
					s.copyText(record.Text)
				}
			})
		}
		if record.URI != "" && len(record.Fields) == 0 {
			ui.Row(c).Gap(8).AlignItems(ui.Start).Children(func() {
				ui.Text(c, record.URI).FontSize(10.5).TextColor(p.accent).Grow(1).Selectable().Wrap()
				if linkButton(c, p, l.NdefCopy()).Clicked() {
					s.copyText(record.URI)
				}
			})
		}
		for _, field := range record.Fields {
			value := field.Value
			if field.Sensitive && !s.ndefReveal {
				value = "••••••"
			}
			ui.Row(c).Gap(8).AlignItems(ui.Start).Children(func() {
				ui.Text(c, ndefFieldLabel(l, field.Key)).Width(104).Shrink(0).
					FontSize(9.5).TextColor(p.faint).Wrap()
				ui.Text(c, value).FontSize(10).TextColor(p.text).
					Grow(1).Selectable().Wrap()
				if field.Sensitive && !s.ndefReveal {
					if linkButton(c, p, l.NdefReveal()).Clicked() {
						s.ndefReveal = true
					}
				}
				if linkButton(c, p, l.NdefCopy()).Clicked() {
					s.copyText(field.Value)
				}
			})
		}
		if len(record.Fields) == 0 && record.Text == "" && record.URI == "" {
			ui.Text(c, l.NdefNoFields()).FontSize(9.5).TextColor(p.faint).Wrap()
		}
		if s.ndefShowRaw || s.ndefRecordRaw[index] {
			ui.Textf(c, "TNF %d · %s", record.TNF, record.Type).Font("monospace").
				FontSize(9).TextColor(p.faint).Wrap()
			ui.Row(c).Gap(8).AlignItems(ui.Start).Children(func() {
				ui.Text(c, l.NdefRawPayload()).FontSize(9).Bold().TextColor(p.faint)
				ui.Spacer(c)
				if linkButton(c, p, l.NdefCopy()).Clicked() {
					s.copyText(record.Payload)
				}
			})
			ui.Text(c, spacedHex(record.Payload)).Font("monospace").FontSize(8.5).
				TextColor(p.faint).Selectable().Wrap()
		}
	})
}

func (s *shell) type2ProtectionView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Scroll(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		if !s.type2.Loaded {
			ui.Column(c).Grow(1).Children(func() { s.emptyPanelBody(c, p, l.Type2NoData()) })
			return
		}
		protection := s.type2.Protection
		ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Text(c, l.Type2ProtectionPanel()).FontSize(12.5).Bold().TextColor(p.text)
			fieldRow(c, p, l.Type2StaticLock(), protection.StaticLock)
			if protection.DynamicLock != "" {
				fieldRow(c, p, l.Type2DynamicLock(), protection.DynamicLock)
			}
			auth0 := l.Type2NotSet()
			if protection.Auth0 >= 0 && protection.Auth0 != 0xFF {
				auth0 = fmt.Sprintf("%d", protection.Auth0)
			}
			fieldRow(c, p, l.Type2Auth0(), auth0)
			fieldRow(c, p, l.Type2Access(), fmt.Sprintf("0x%02X", protection.Access))
			fieldRow(c, p, l.Type2Protect(), yesNo(l, protection.Protect))
			fieldRow(c, p, l.Type2ConfigLocked(), yesNo(l, protection.ConfigLocked))
			fieldRow(c, p, l.Type2AuthLimit(), fmt.Sprintf("%d", protection.AuthLimit))
			fieldRow(c, p, l.Type2Password(), setLabel(l, protection.PasswordSet))
			fieldRow(c, p, l.Type2Pack(), setLabel(l, protection.PackSet))
		})
		if s.type2.CC.Valid {
			ui.Column(c).Padding(14).Gap(8).Background(p.surface).Radius(10).
				Border(1, p.border).Children(func() {
				ui.Text(c, l.Type2CapabilityPanel()).FontSize(12.5).Bold().TextColor(p.text)
				fieldRow(c, p, l.Type2Capacity(s.type2.CC.Capacity), s.type2.Info.ProductName)
				if s.type2.CC.Writable {
					ui.Text(c, l.Type2Writable()).FontSize(10.5).TextColor(p.success).Wrap()
				} else {
					ui.Text(c, l.Type2ReadOnly()).FontSize(10.5).TextColor(p.warning).Wrap()
				}
			})
		}
		if s.advanced {
			s.type2AdvancedPanel(c, p)
		}
	})
}

func (s *shell) type2AdvancedPanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Padding(14).Gap(9).Background(p.dangerSoft).Radius(10).
		Border(1, p.dangerBorder).Children(func() {
		ui.Text(c, l.Type2LifecyclePanel()).FontSize(12.5).Bold().TextColor(p.danger)
		ui.Text(c, l.Type2LifecycleWarning()).FontSize(9.5).TextColor(p.warning).Wrap()
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.Type2Source()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			summary := l.Type2NoSource()
			if s.type2Source.Loaded {
				summary = l.Type2SourceSummary(s.type2Source.ProductName, s.type2Source.UID)
			}
			ui.Text(c, summary).FontSize(9.5).TextColor(p.text).Grow(1).Wrap()
			if linkButton(c, p, l.Type2LoadSource()).Disabled(s.writeBusy()).Clicked() {
				s.loadType2Source()
			}
			ui.Text(c, l.Type2Repeat()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.type2CopyRepeat).Placeholder("1").Width(60)
			if ui.Button(c, l.Type2CopyToTag()).Height(28).FontSize(10).
				Disabled(s.writeBusy() || !s.type2Source.Loaded).Clicked() {
				s.requestType2Lifecycle(app.Type2LifecycleRequestDTO{
					Operation:        "copy",
					IncludeProtected: s.type2CopyProtected,
					SameUIDOnly:      s.type2CopySameUID,
					CurrentPassword:  s.type2Password,
					Repeat:           parseRepeatInput(s.type2CopyRepeat),
				})
			}
		})
		ui.Row(c).Gap(12).AlignItems(ui.Center).Children(func() {
			ui.Checkbox(c, &s.type2CopyProtected, l.Type2CopyProtected())
			ui.Checkbox(c, &s.type2CopySameUID, l.Type2CopySameUID())
		})
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			if ui.Button(c, l.Type2Erase()).Height(28).FontSize(10).Disabled(s.writeBusy()).Clicked() {
				s.requestType2Lifecycle(app.Type2LifecycleRequestDTO{
					Operation: "erase", CurrentPassword: s.type2Password,
				})
			}
		})
	})
	ui.Column(c).Padding(14).Gap(8).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Text(c, l.Type2LockPanel()).FontSize(12.5).Bold().TextColor(p.text)
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.Type2StaticLock()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.type2LockStatic).Placeholder("0000").Font("monospace").Width(100)
			ui.Text(c, l.Type2DynamicLock()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.type2LockDynamic).Placeholder("000000").Font("monospace").Width(120)
		})
		ui.Row(c).Gap(12).AlignItems(ui.Center).Children(func() {
			ui.Checkbox(c, &s.type2LockCC, l.Type2CCReadOnly())
			ui.Checkbox(c, &s.type2LockConfig, l.Type2ConfigLock())
			ui.Spacer(c)
			if ui.Button(c, l.Type2ApplyLocks()).Height(28).FontSize(10).
				Disabled(s.writeBusy() || strings.TrimSpace(s.type2LockStatic) == "" &&
					strings.TrimSpace(s.type2LockDynamic) == "" && !s.type2LockCC && !s.type2LockConfig).Clicked() {
				s.requestType2Lifecycle(app.Type2LifecycleRequestDTO{
					Operation: "lock", StaticLock: s.type2LockStatic, DynamicLock: s.type2LockDynamic,
					CCReadOnly: s.type2LockCC, ConfigLock: s.type2LockConfig, CurrentPassword: s.type2Password,
				})
			}
		})
	})
	ui.Column(c).Padding(14).Gap(8).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Text(c, l.Type2PasswordPanel()).FontSize(12.5).Bold().TextColor(p.text)
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.Type2NewPassword()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.type2PwdNew).Password().Placeholder("01020304").Width(130)
			ui.Text(c, l.Type2Pack()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.type2PwdPack).Placeholder("AABB").Width(90)
			ui.Text(c, l.Type2Auth0()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.type2PwdAuth0).Placeholder("4").Width(70)
			ui.Checkbox(c, &s.type2PwdProtect, l.Type2Protect())
		})
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Spacer(c)
			if ui.Button(c, l.Type2SetPassword()).Height(28).FontSize(10).
				Disabled(s.writeBusy() || strings.TrimSpace(s.type2PwdNew) == "" || strings.TrimSpace(s.type2PwdPack) == "").Clicked() {
				auth0, ok := parseAuth0Input(s.type2PwdAuth0)
				if !ok {
					s.errorText = l.Type2InvalidAuth0()
					return
				}
				s.requestType2Lifecycle(app.Type2LifecycleRequestDTO{
					Operation: "password-set", NewPassword: s.type2PwdNew, NewPack: s.type2PwdPack,
					Auth0: auth0, Protect: s.type2PwdProtect, CurrentPassword: s.type2Password,
				})
			}
			if ui.Button(c, l.Type2RemovePassword()).Height(28).FontSize(10).Disabled(s.writeBusy()).Clicked() {
				s.requestType2Lifecycle(app.Type2LifecycleRequestDTO{
					Operation: "password-remove", CurrentPassword: s.type2Password,
				})
			}
		})
	})
}

func parseAuth0Input(value string) (*int, bool) {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil, true
	}
	page, err := strconv.Atoi(value)
	if err != nil || page < 0 || page > 255 {
		return nil, false
	}
	return &page, true
}

func parseRepeatInput(value string) int {
	value = strings.TrimSpace(value)
	if value == "" {
		return 1
	}
	repeat, err := strconv.Atoi(value)
	if err != nil || repeat < 1 || repeat > 1000 {
		return 1
	}
	return repeat
}

func (s *shell) loadType2Source() {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		source, err := s.api.LoadType2Source()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().Type2SourceLoadFailed(err.Error())
				return
			}
			s.type2Source = source
			s.errorText = ""
		})
	}()
}

func (s *shell) requestType2Lifecycle(request app.Type2LifecycleRequestDTO) {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		plan, err := s.api.PreviewType2Lifecycle(request)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().Type2LifecycleFailed(err.Error())
				return
			}
			s.type2Plan = &plan
			s.type2PlanRequest = request
			s.type2PlanRequest.Token = plan.Token
			s.type2PlanConfirm = ""
			s.type2PlanOpen = true
			s.errorText = ""
		})
	}()
}

func (s *shell) startType2Lifecycle(request app.Type2LifecycleRequestDTO) {
	s.pendingOps++
	go func() {
		_, err := s.api.StartType2Lifecycle(request)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().Type2LifecycleFailed(err.Error())
				return
			}
			s.errorText = ""
			s.toastText = s.locales().Type2LifecycleStarted()
		})
	}()
}

func (s *shell) type2PlanDialog(c *ui.Context) {
	if !s.type2PlanOpen || s.type2Plan == nil {
		return
	}
	l := s.locales()
	plan := *s.type2Plan
	p := paletteFor(c.Theme())
	ui.Modal(c, &s.type2PlanOpen, func() {
		ui.Text(c, l.Type2PlanTitle(plan.Operation)).FontSize(15).Bold().TextColor(p.danger)
		ui.Text(c, l.Type2PlanCard(plan.CardUID, plan.ProductName)).FontSize(10.5).TextColor(p.muted).Wrap()
		if plan.SourceUID != "" {
			ui.Text(c, l.Type2Source()+": "+plan.SourceUID).FontSize(10).TextColor(p.muted).Wrap()
		}
		for _, change := range plan.Changes {
			oneWay := ""
			if change.OneWay {
				oneWay = l.Type2OneWay()
			}
			ui.Text(c, fmt.Sprintf("%s: %s → %s%s", change.Target, change.Old, change.New, oneWay)).
				FontSize(9.5).Wrap()
		}
		ui.Text(c, l.Type2PlanBackup(len(plan.Writes))).FontSize(9.5).Wrap()
		if plan.BackupRoot != "" {
			ui.Text(c, l.Type2PlanBackupRoot(plan.BackupRoot)).FontSize(9.5).Wrap()
		}
		ui.Text(c, l.Type2PlanTypeUID(plan.CardUID)).FontSize(10).Bold()
		ui.TextInput(c, &s.type2PlanConfirm).Placeholder(plan.CardUID).AutoFocus()
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.type2PlanOpen = false
			}
			if ui.Button(c, l.Type2PlanConfirm()).Disabled(s.type2PlanConfirm != plan.CardUID).Clicked() {
				request := s.type2PlanRequest
				s.type2PlanOpen = false
				s.type2Plan = nil
				s.startType2Lifecycle(request)
			}
		})
	})
}

func (s *shell) type2Inspector(c *ui.Context, p palette) {
	l := s.locales()
	if !s.type2.Loaded {
		s.inspectorEmpty(c, p, l.Type2NoData())
		return
	}
	s.inspectorCard(c, p, func() {
		ui.Text(c, s.type2.Info.ProductName).FontSize(11.5).Bold().TextColor(p.text)
		ui.Column(c).Gap(7).Children(func() {
			monoFieldRow(c, p, l.CardUID(), s.type2.Info.UID)
			fieldRow(c, p, l.CardType(), s.type2.Info.ProductName)
			fieldRow(c, p, l.Type2MemoryPanel(), l.Type2Capacity(s.type2.Info.UserBytes))
			if s.type2.CC.Valid {
				fieldRow(c, p, l.Type2Access(), fmt.Sprintf("0x%02X", s.type2.CC.Access))
			} else {
				fieldRow(c, p, l.Type2NotFormatted(), "—")
			}
			if s.type2.NDEFError != "" {
				ui.Text(c, l.Type2NDEFError(s.type2.NDEFError)).FontSize(9.5).
					TextColor(p.warning).Wrap()
			} else if s.type2.NDEF != nil && s.type2.NDEF.Present {
				summary := s.type2.NDEF.Summary
				if summary == "" {
					summary = l.Type2Records(len(s.type2.NDEF.Records))
				}
				fieldRow(c, p, l.Type2NDEFPanel(), summary)
			} else {
				fieldRow(c, p, l.Type2NDEFPanel(), l.Type2NoNDEF())
			}
			if s.type2.Protection.PasswordSet {
				fieldRow(c, p, l.Type2Password(), l.Type2Set())
			}
		})
	})
}

func (s *shell) type2NDEFEditor(c *ui.Context, p palette) {
	ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		s.ndefModeSelector(c, p)
		switch s.ndefMode {
		case "raw":
			s.ndefRawEditor(c, p)
		case "records":
			s.ndefRecordsEditor(c, p)
		default:
			s.ndefTemplateEditor(c, p)
		}
		s.ndefWriteRow(c, p)
	})
}

func (s *shell) ndefModeSelector(c *ui.Context, p palette) {
	l := s.locales()
	ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
		ui.Text(c, l.NdefMode()).FontSize(10).TextColor(p.muted).Shrink(0)
		labels := []string{l.NdefModeTemplate(), l.NdefModeRaw(), l.NdefModeRecords()}
		ids := []string{"template", "raw", "records"}
		selected := labels[0]
		for index, id := range ids {
			if id == s.ndefMode {
				selected = labels[index]
			}
		}
		selector := ui.Select(c, &selected, labels)
		if selector.Changed() {
			for index, label := range labels {
				if label == selected {
					s.ndefMode = ids[index]
					break
				}
			}
		}
	})
}

func (s *shell) ndefTemplateEditor(c *ui.Context, p palette) {
	l := s.locales()
	current, ok := ndefTemplateByID(s.ndefTemplates, s.ndefTemplate)
	if !ok {
		if len(s.ndefTemplates) == 0 {
			return
		}
		s.selectNdefTemplate(s.ndefTemplates[0].ID)
		current, _ = ndefTemplateByID(s.ndefTemplates, s.ndefTemplate)
	}
	ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
		ui.Text(c, l.NdefTemplate()).FontSize(10).TextColor(p.muted).Shrink(0)
		labels := make([]string, len(s.ndefTemplates))
		ids := make([]string, len(s.ndefTemplates))
		for index, template := range s.ndefTemplates {
			labels[index] = ndefTemplateLabel(l, template.ID)
			ids[index] = template.ID
		}
		selected := ndefTemplateLabel(l, current.ID)
		selector := ui.Select(c, &selected, labels)
		if selector.Changed() {
			for index, label := range labels {
				if label == selected {
					s.selectNdefTemplate(ids[index])
					break
				}
			}
		}
	})
	for _, field := range current.Fields {
		value := s.ndefField(field.ID)
		label := ndefFieldLabel(l, field.ID)
		ui.Column(c).Gap(4).Children(func() {
			ui.Text(c, label).FontSize(9.5).TextColor(p.muted)
			if field.Kind == "select" {
				options := make([]string, len(field.Options))
				for index, option := range field.Options {
					options[index] = ndefOptionLabel(l, field.ID, option)
				}
				selected := ndefOptionLabel(l, field.ID, *value)
				if selected == "" && len(options) > 0 {
					selected = options[0]
					*value = field.Options[0]
				}
				selector := ui.Select(c, &selected, options)
				if selector.Changed() {
					for index, option := range options {
						if option == selected {
							*value = field.Options[index]
							s.applyNdefPreset(current.ID, field.ID, *value)
							break
						}
					}
				}
			} else {
				placeholder := label
				switch field.ID {
				case "text":
					placeholder = l.Type2NDEFTextPlaceholder()
				case "uri":
					placeholder = l.Type2NDEFURIPlaceholder()
				}
				input := ui.TextInput(c, value).Placeholder(placeholder)
				if input.Changed() {
					s.applyNdefHandle(current.ID, field.ID)
				}
				if input.Submitted() {
					s.requestType2NDEFWrite()
				}
			}
		})
	}
}

func (s *shell) ndefRawEditor(c *ui.Context, p palette) {
	l := s.locales()
	ui.Text(c, l.NdefRawEditorHint()).FontSize(9.5).TextColor(p.faint).Wrap()
	ui.TextArea(c, &s.ndefRaw).Placeholder(l.NdefRawPlaceholder()).Height(110).Font("monospace")
	preview := ""
	if strings.TrimSpace(s.ndefRaw) != "" {
		if result, err := s.api.PreviewType2NDEFRaw(app.Type2NDEFRawRequestDTO{Raw: s.ndefRaw}); err != nil {
			preview = l.NdefPreviewError(err.Error())
		} else {
			preview = l.NdefPreviewLength(result.Length)
		}
	}
	if preview != "" {
		ui.Text(c, preview).FontSize(9.5).TextColor(p.muted).Wrap()
	}
}

func (s *shell) ndefRecordsEditor(c *ui.Context, p palette) {
	l := s.locales()
	if len(s.ndefDrafts) == 0 {
		ui.Text(c, l.NdefRecordsEmpty()).FontSize(9.5).TextColor(p.faint).Wrap()
	}
	deleteIndex := -1
	for index := range s.ndefDrafts {
		draft := &s.ndefDrafts[index]
		ui.Column(c).Padding(9).Gap(6).Background(p.surfaceAlt).Radius(8).
			Border(1, p.border).Children(func() {
			ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Textf(c, "%d · %s", index+1, ndefKindLabel(l, draft.Kind)).FontSize(9.5).
					TextColor(p.muted).Grow(1)
				if linkButton(c, p, l.NdefMoveUp()).Disabled(index == 0).Clicked() {
					s.ndefDrafts[index-1], s.ndefDrafts[index] = s.ndefDrafts[index], s.ndefDrafts[index-1]
				}
				if linkButton(c, p, l.NdefMoveDown()).Disabled(index == len(s.ndefDrafts)-1).Clicked() {
					s.ndefDrafts[index+1], s.ndefDrafts[index] = s.ndefDrafts[index], s.ndefDrafts[index+1]
				}
				if linkButton(c, p, l.CommonDelete()).Clicked() {
					deleteIndex = index
				}
			})
			switch draft.Kind {
			case "text":
				ui.TextInput(c, &draft.Text).Placeholder(l.NdefFieldText())
			case "uri":
				ui.TextInput(c, &draft.URI).Placeholder(l.NdefFieldURI())
			case "smartposter":
				ui.TextInput(c, &draft.URI).Placeholder(l.NdefFieldURI())
				ui.TextInput(c, &draft.Title).Placeholder(l.NdefFieldTitle())
			}
		})
	}
	if deleteIndex >= 0 && deleteIndex < len(s.ndefDrafts) {
		s.ndefDrafts = append(s.ndefDrafts[:deleteIndex], s.ndefDrafts[deleteIndex+1:]...)
	}
	ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
		if linkButton(c, p, l.NdefAddText()).Clicked() {
			s.ndefDrafts = append(s.ndefDrafts, ndefRecordDraft{Kind: "text"})
		}
		if linkButton(c, p, l.NdefAddURI()).Clicked() {
			s.ndefDrafts = append(s.ndefDrafts, ndefRecordDraft{Kind: "uri"})
		}
		if linkButton(c, p, l.NdefAddSmartPoster()).Clicked() {
			s.ndefDrafts = append(s.ndefDrafts, ndefRecordDraft{Kind: "smartposter"})
		}
	})
}

func (s *shell) ndefWriteRow(c *ui.Context, p palette) {
	l := s.locales()
	ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
		ui.Text(c, l.Type2WriteBackupHint()).FontSize(9).TextColor(p.faint).Grow(1).Wrap()
		if ui.Button(c, l.NdefWriteTemplate()).Height(28).FontSize(10).
			Disabled(s.type2WriteBusy() || !s.ndefReady() || !s.type2.CC.Valid).Clicked() {
			s.requestType2NDEFWrite()
		}
	})
}

func (s *shell) ndefReady() bool {
	switch s.ndefMode {
	case "raw":
		return strings.TrimSpace(s.ndefRaw) != ""
	case "records":
		if len(s.ndefDrafts) == 0 {
			return false
		}
		for _, draft := range s.ndefDrafts {
			switch draft.Kind {
			case "text":
				if strings.TrimSpace(draft.Text) == "" {
					return false
				}
			case "uri":
				if strings.TrimSpace(draft.URI) == "" {
					return false
				}
			case "smartposter":
				if strings.TrimSpace(draft.URI) == "" {
					return false
				}
			}
		}
		return true
	default:
		return s.ndefTemplateReady()
	}
}

func (s *shell) applyNdefPreset(template, field, value string) {
	provider := type2ProviderFor(s.ndefProviders, template, value)
	if provider == "" {
		return
	}
	switch field {
	case "search_provider":
		s.setNdefField("search_prefix", provider)
	case "social_provider", "payment_provider":
		handle := cleanHandle(*s.ndefField("handle"))
		s.setNdefField("uri", provider+handle)
	}
}

func (s *shell) applyNdefHandle(template, field string) {
	if field != "handle" || template != "social" && template != "payment" {
		return
	}
	providerID := "social_provider"
	if template == "payment" {
		providerID = "payment_provider"
	}
	provider := type2ProviderFor(s.ndefProviders, template, *s.ndefField(providerID))
	if provider == "" {
		return
	}
	s.setNdefField("uri", provider+cleanHandle(*s.ndefField("handle")))
}

func cleanHandle(value string) string {
	value = strings.TrimSpace(value)
	value = strings.TrimPrefix(value, "@")
	return strings.TrimPrefix(value, "$")
}

func type2ProviderFor(providers map[string][]app.Type2ProviderDTO, template, id string) string {
	for _, provider := range providers[template] {
		if provider.ID == id {
			return provider.URL
		}
	}
	return ""
}

func (s *shell) selectNdefTemplate(id string) {
	s.ndefTemplate = id
	s.ndefFields = make(map[string]*string)
	switch id {
	case "search":
		s.setNdefField("search_provider", "duckduckgo")
		s.setNdefField("search_prefix", "https://duckduckgo.com/?q=")
	case "social":
		s.setNdefField("social_provider", "github")
	case "payment":
		s.setNdefField("payment_provider", "paypal")
	case "wifi":
		s.setNdefField("security", "wpa2")
	case "bluetooth-le":
		s.setNdefField("address_type", "public")
	case "text", "smartposter":
		s.setNdefField("language", "en")
	}
}

func (s *shell) ndefField(id string) *string {
	if value, ok := s.ndefFields[id]; ok {
		return value
	}
	value := new(string)
	s.ndefFields[id] = value
	return value
}

func (s *shell) setNdefField(id, value string) {
	pointer := s.ndefField(id)
	*pointer = value
}

func (s *shell) ndefCurrentTemplate() (app.Type2NDEFTemplateDTO, bool) {
	return ndefTemplateByID(s.ndefTemplates, s.ndefTemplate)
}

func (s *shell) ndefTemplateReady() bool {
	template, ok := s.ndefCurrentTemplate()
	if !ok {
		return false
	}
	for _, field := range template.Fields {
		if !field.Required {
			continue
		}
		if strings.TrimSpace(*s.ndefField(field.ID)) == "" {
			return false
		}
	}
	if s.ndefTemplate == "wifi" {
		security := strings.TrimSpace(*s.ndefField("security"))
		if security != "open" && strings.TrimSpace(*s.ndefField("password")) == "" {
			return false
		}
	}
	switch s.ndefTemplate {
	case "search":
		if strings.TrimSpace(*s.ndefField("search_prefix")) == "" &&
			strings.TrimSpace(*s.ndefField("search_provider")) == "" {
			return false
		}
	case "social", "payment":
		if strings.TrimSpace(*s.ndefField("uri")) == "" &&
			strings.TrimSpace(*s.ndefField("handle")) == "" {
			return false
		}
	}
	return true
}

func (s *shell) syncType2PageSelection(reset bool) {
	if !s.type2.Loaded || len(s.type2.Pages) == 0 {
		s.type2PageChosen = -1
		s.type2PageInput = ""
		s.type2PageInputPage = -1
		return
	}
	if reset || s.type2PageChosen < 0 || s.type2PageChosen >= len(s.type2.Pages) {
		s.type2PageChosen = s.firstEditableType2Page()
		s.type2PageInputPage = -1
	}
	page := s.type2.Pages[s.type2PageChosen]
	if !page.Editable {
		s.type2PageInput = ""
	} else if reset || s.type2PageInputPage != page.Page {
		s.type2PageInput = page.Hex
	}
	s.type2PageInputPage = page.Page
}

func (s *shell) firstEditableType2Page() int {
	for index, page := range s.type2.Pages {
		if page.Editable {
			return index
		}
	}
	return 0
}

func (s *shell) selectedType2Page() (app.Type2PageDTO, bool) {
	if s.type2PageChosen < 0 || s.type2PageChosen >= len(s.type2.Pages) {
		return app.Type2PageDTO{}, false
	}
	return s.type2.Pages[s.type2PageChosen], true
}

func (s *shell) type2WriteBusy() bool {
	return s.activeTask != "" || s.pendingOps > 0
}

func (s *shell) requestType2PageWrite() {
	page, ok := s.selectedType2Page()
	if !ok || !page.Editable {
		s.errorText = s.locales().Type2SelectUserPage()
		return
	}
	value := normalizeType2Hex(s.type2PageInput)
	decoded, err := hex.DecodeString(value)
	if err != nil || len(decoded) != 4 {
		s.errorText = s.locales().Type2InvalidPageHex()
		return
	}
	s.type2WriteIntent = &type2WriteIntent{Kind: "page", Page: page.Page, Hex: value}
	s.type2WriteOpen = true
}

func (s *shell) requestType2NDEFWrite() {
	switch s.ndefMode {
	case "raw":
		preview, err := s.api.PreviewType2NDEFRaw(app.Type2NDEFRawRequestDTO{Raw: s.ndefRaw})
		if err != nil {
			s.errorText = s.locales().Type2WriteFailed(err.Error())
			return
		}
		s.type2WriteIntent = &type2WriteIntent{Kind: "raw", Raw: s.ndefRaw, Length: preview.Length}
		s.type2WriteOpen = true
		return
	case "records":
		records := s.ndefRecordInputs()
		preview, err := s.api.PreviewType2NDEFRecords(app.Type2NDEFRecordsRequestDTO{Records: records})
		if err != nil {
			s.errorText = s.locales().Type2WriteFailed(err.Error())
			return
		}
		s.type2WriteIntent = &type2WriteIntent{Kind: "records", Records: records, Length: preview.Length}
		s.type2WriteOpen = true
		return
	}
	template, ok := s.ndefCurrentTemplate()
	if !ok {
		return
	}
	fields := make(map[string]string, len(template.Fields))
	for _, field := range template.Fields {
		fields[field.ID] = *s.ndefField(field.ID)
	}
	request := app.Type2NDEFWriteStartRequestDTO{Template: template.ID, Fields: fields}
	preview, err := s.api.PreviewType2NDEF(request)
	if err != nil {
		s.errorText = s.locales().Type2WriteFailed(err.Error())
		return
	}
	s.type2WriteIntent = &type2WriteIntent{Kind: "ndef", Template: template.ID, Fields: fields, Length: preview.Length}
	s.type2WriteOpen = true
}

func (s *shell) ndefRecordInputs() []app.Type2NDEFRecordInputDTO {
	records := make([]app.Type2NDEFRecordInputDTO, 0, len(s.ndefDrafts))
	for _, draft := range s.ndefDrafts {
		fields := map[string]string{}
		switch draft.Kind {
		case "text":
			fields["text"] = draft.Text
		case "uri":
			fields["uri"] = draft.URI
		case "smartposter":
			fields["uri"] = draft.URI
			fields["title"] = draft.Title
		}
		records = append(records, app.Type2NDEFRecordInputDTO{Kind: draft.Kind, Fields: fields})
	}
	return records
}

func (s *shell) startType2Write(intent type2WriteIntent) {
	password := s.type2Password
	s.pendingOps++
	go func() {
		var err error
		switch intent.Kind {
		case "page":
			_, err = s.api.StartType2Write(app.Type2WriteStartRequestDTO{
				Pages: []app.Type2PageWriteDTO{{Page: intent.Page, Hex: intent.Hex}}, Password: password,
			})
		case "raw":
			_, err = s.api.StartType2NDEFRawWrite(app.Type2NDEFRawRequestDTO{Raw: intent.Raw, Password: password})
		case "records":
			_, err = s.api.StartType2NDEFRecordsWrite(app.Type2NDEFRecordsRequestDTO{Records: intent.Records, Password: password})
		default:
			_, err = s.api.StartType2NDEFWrite(app.Type2NDEFWriteStartRequestDTO{
				Template: intent.Template, Fields: intent.Fields, Password: password,
			})
		}
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().Type2WriteFailed(err.Error())
				return
			}
			s.errorText = ""
			s.toastText = s.locales().Type2WriteStarted()
		})
	}()
}

func (s *shell) type2WriteDialog(c *ui.Context) {
	if s.type2WriteIntent == nil {
		return
	}
	l := s.locales()
	intent := *s.type2WriteIntent
	title := l.Type2ConfirmWrite()
	message := l.Type2WriteBackupHint()
	switch intent.Kind {
	case "page":
		title = l.Type2ConfirmPageTitle(intent.Page)
		message = l.Type2ConfirmPageMessage(intent.Page, spacedHex(intent.Hex))
	case "raw":
		title = l.NdefConfirmRawTitle(intent.Length)
		message = l.NdefConfirmRawMessage(intent.Length)
	case "records":
		title = l.NdefConfirmRecordsTitle(len(intent.Records), intent.Length)
		message = l.NdefConfirmRecordsMessage(len(intent.Records), intent.Length)
	default:
		template := ndefTemplateLabel(l, intent.Template)
		title = l.NdefConfirmTemplateTitle(template)
		message = l.NdefConfirmTemplateMessage(template, intent.Length)
	}
	result := ui.AlertDialog(c, &s.type2WriteOpen, title, message, l.CommonCancel(), l.Type2ConfirmWrite())
	if result == -1 {
		return
	}
	s.type2WriteIntent = nil
	s.type2WriteOpen = false
	if result == 1 {
		s.startType2Write(intent)
	}
}

func (s *shell) importNDEFMessage() {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		result, err := s.api.ImportType2NDEF()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().NdefImportFailed(err.Error())
				return
			}
			s.ndefMode = "raw"
			s.ndefRaw = result.Raw
			s.errorText = ""
			s.toastText = s.locales().NdefImported(result.Length)
		})
	}()
}

func (s *shell) exportNDEFMessage() {
	if s.writeBusy() {
		return
	}
	raw := ""
	switch {
	case s.ndefMode == "raw":
		raw = s.ndefRaw
	case s.type2.NDEF != nil && s.type2.NDEF.Raw != "":
		raw = s.type2.NDEF.Raw
	}
	if strings.TrimSpace(raw) == "" {
		s.errorText = s.locales().NdefExportEmpty()
		return
	}
	s.pendingOps++
	go func() {
		path, err := s.api.ExportType2NDEF(raw)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().NdefExportFailed(err.Error())
				return
			}
			s.errorText = ""
			s.toastText = s.locales().NdefExported(path)
		})
	}()
}

func (s *shell) importType2Dump() {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		dump, err := s.api.ImportType2Dump()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().Type2DumpImportFailed(err.Error())
				return
			}
			s.type2 = dump
			s.syncType2PageSelection(true)
			s.errorText = ""
		})
	}()
}

func (s *shell) exportType2Dump() {
	if s.writeBusy() || !s.type2.Loaded {
		return
	}
	s.pendingOps++
	go func() {
		path, err := s.api.ExportType2Dump()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().Type2DumpExportFailed(err.Error())
				return
			}
			s.errorText = ""
			s.toastText = s.locales().Type2DumpExported(path)
		})
	}()
}

func (s *shell) copyText(value string) {
	if value == "" {
		return
	}
	s.windowMu.RLock()
	window := s.window
	s.windowMu.RUnlock()
	if window == nil {
		return
	}
	mygo.Clipboard.WriteText(value)
	s.toastText = s.locales().NdefCopied()
}

func normalizeType2Hex(value string) string {
	replacer := strings.NewReplacer(" ", "", ":", "", "-", "")
	return strings.ToUpper(replacer.Replace(strings.TrimSpace(value)))
}

func spacedHex(value string) string {
	parts := make([]string, 0, len(value)/2)
	for index := 0; index+1 < len(value); index += 2 {
		parts = append(parts, value[index:index+2])
	}
	return strings.Join(parts, " ")
}

func yesNo(l locales.Messages, value bool) string {
	if value {
		return l.CommonYes()
	}
	return l.CommonNo()
}

func setLabel(l locales.Messages, value bool) string {
	if value {
		return l.Type2Set()
	}
	return l.Type2NotSet()
}
