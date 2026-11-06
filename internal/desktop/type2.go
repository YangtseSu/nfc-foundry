package desktop

import (
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo/ui"
)

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
			columns := []ui.TableColumn{
				{Title: l.Type2PageColumn(), ID: "page", Width: 64, Align: ui.End, Fixed: true},
				{Title: l.Type2RegionColumn(), ID: "region", Width: 116, Fixed: true},
				{Title: l.Type2DataColumn(), ID: "data"},
				{Title: l.Type2ASCIIColumn(), ID: "ascii", Width: 92, Fixed: true},
			}
			ui.Table(c, &s.type2List, columns, len(s.type2.Pages), func(rowIndex, col int) {
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
		})
	})
}

func (s *shell) type2NDEFView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Scroll(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		if !s.type2.Loaded {
			ui.Column(c).Grow(1).Children(func() { s.emptyPanelBody(c, p, l.Type2NoData()) })
			return
		}
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
