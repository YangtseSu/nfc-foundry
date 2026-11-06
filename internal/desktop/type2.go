package desktop

import (
	"encoding/hex"
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo/ui"
)

type type2WriteIntent struct {
	Kind    string
	Page    int
	Hex     string
	Mode    string
	Message string
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
		ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Text(c, l.Type2NDEFPanel()).FontSize(12.5).Bold().TextColor(p.text)
			switch {
			case s.type2.NDEFError != "":
				ui.Text(c, l.Type2NDEFError(s.type2.NDEFError)).FontSize(10.5).
					TextColor(p.danger).Wrap()
			case s.type2.NDEF == nil || !s.type2.NDEF.Present:
				ui.Text(c, l.Type2NoNDEF()).FontSize(10.5).TextColor(p.muted).Wrap()
			default:
				if s.type2.NDEF.Text != "" {
					ui.Text(c, l.Type2TextRecord()).FontSize(10).Bold().TextColor(p.muted)
					ui.Text(c, s.type2.NDEF.Text).FontSize(11).TextColor(p.text).Selectable().Wrap()
				}
				if s.type2.NDEF.URI != "" {
					ui.Text(c, l.Type2URIRecord()).FontSize(10).Bold().TextColor(p.muted)
					ui.Text(c, s.type2.NDEF.URI).FontSize(11).TextColor(p.accent).Selectable().Wrap()
				}
				ui.Text(c, l.Type2Records(len(s.type2.NDEF.Records))).FontSize(10).
					TextColor(p.faint)
				for index, record := range s.type2.NDEF.Records {
					ui.Textf(c, "%d · TNF %d · %s", index+1, record.TNF, record.Type).
						Font("monospace").FontSize(9.5).TextColor(p.muted).Wrap()
					if record.Payload != "" && record.Text == "" && record.URI == "" {
						ui.Text(c, spacedHex(record.Payload)).Font("monospace").FontSize(9).
							TextColor(p.faint).Selectable().Wrap()
					}
				}
				ui.Text(c, spacedHex(s.type2.NDEF.Raw)).Font("monospace").FontSize(9).
					TextColor(p.faint).Selectable().Wrap()
			}
		})
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
				fieldRow(c, p, l.Type2NDEFPanel(), l.Type2Records(len(s.type2.NDEF.Records)))
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
	l := s.locales()
	ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.Type2NDEFPanel()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Spacer(c)
			if s.ndefMode == "uri" {
				if primaryButton(c, p, l.Type2URIRecord()).Height(26).Clicked() {
					s.ndefMode = "uri"
				}
			} else if ui.Button(c, l.Type2URIRecord()).Height(26).FontSize(10).Clicked() {
				s.ndefMode = "uri"
				s.ndefInput = ""
			}
			if s.ndefMode == "text" {
				if primaryButton(c, p, l.Type2TextRecord()).Height(26).Clicked() {
					s.ndefMode = "text"
				}
			} else if ui.Button(c, l.Type2TextRecord()).Height(26).FontSize(10).Clicked() {
				s.ndefMode = "text"
				s.ndefInput = ""
			}
		})
		placeholder := l.Type2NDEFTextPlaceholder()
		if s.ndefMode == "uri" {
			placeholder = l.Type2NDEFURIPlaceholder()
		}
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			input := ui.TextInput(c, &s.ndefInput).Placeholder(placeholder)
			if input.Submitted() {
				s.requestType2NDEFWrite()
			}
		})
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.Type2WriteBackupHint()).FontSize(9).TextColor(p.faint).Grow(1).Wrap()
			if ui.Button(c, l.Type2WriteNDEF()).Height(28).FontSize(10).
				Disabled(s.type2WriteBusy() || strings.TrimSpace(s.ndefInput) == "").Clicked() {
				s.requestType2NDEFWrite()
			}
		})
	})
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
	message := strings.TrimSpace(s.ndefInput)
	if message == "" {
		s.errorText = s.locales().Type2NDEFEmpty()
		return
	}
	mode := s.ndefMode
	if mode == "" {
		mode = "text"
	}
	s.type2WriteIntent = &type2WriteIntent{Kind: "ndef", Mode: mode, Message: message}
	s.type2WriteOpen = true
}

func (s *shell) startType2Write(intent type2WriteIntent) {
	s.pendingOps++
	go func() {
		var err error
		if intent.Kind == "page" {
			_, err = s.api.StartType2Write(app.Type2WriteStartRequestDTO{
				Pages: []app.Type2PageWriteDTO{{Page: intent.Page, Hex: intent.Hex}},
			})
		} else {
			request := app.Type2NDEFWriteStartRequestDTO{Mode: intent.Mode}
			if intent.Mode == "uri" {
				request.URI = intent.Message
			} else {
				request.Text = intent.Message
			}
			_, err = s.api.StartType2NDEFWrite(request)
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
	if intent.Kind == "page" {
		title = l.Type2ConfirmPageTitle(intent.Page)
		message = l.Type2ConfirmPageMessage(intent.Page, spacedHex(intent.Hex))
	} else {
		mode := l.Type2TextRecord()
		if intent.Mode == "uri" {
			mode = l.Type2URIRecord()
		}
		title = l.Type2ConfirmNDEFTitle()
		message = l.Type2ConfirmNDEFMessage(mode, len([]byte(intent.Message)))
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
