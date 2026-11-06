package desktop

import (
	"strings"

	"github.com/egoist/mygo/i18n"
	"github.com/egoist/mygo/ui"
)

func (s *shell) topbar(c *ui.Context, p palette) {
	l := s.locales()
	ui.Row(c).Height(58).Shrink(0).Background(p.topbar).Padding(0, 18).Gap(16).
		AlignItems(ui.Center).Children(func() {
		ui.Row(c).Width(200).Shrink(0).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Row(c).Width(30).Height(30).Radius(9).Background(ui.Hex("#102C42")).
				Border(1, ui.RGBA(90, 215, 235, 0.25)).AlignItems(ui.Center).
				Justify(ui.Center).Children(func() {
				ui.Icon(c, brandIcon).FontSize(19).TextColor(ui.Hex("#37CFE8"))
			})
			ui.Column(c).Gap(3).Children(func() {
				ui.Text(c, l.AppTitle()).FontSize(15).Bold().TextColor(p.topbarText).SingleLine()
				ui.Text(c, strings.ToUpper(l.AppSubtitle())).FontSize(9.5).
					LetterSpacing(1.4).TextColor(p.topbarMuted).SingleLine()
			})
		})
		ui.Row(c).Width(1).Height(24).Shrink(0).Background(p.topbarLine)

		ui.Row(c).Gap(8).MinWidth(0).Children(func() {
			s.topbarReaderChip(c, p)
			s.topbarCardChip(c, p)
			s.topbarCapacityChip(c, p)
		})
		ui.Spacer(c)

		ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
			s.topbarTaskPill(c, p)
			ui.PopoverMenuButton(c, s.languageShort(), func(m *ui.Menu) {
				for _, tag := range languageTags {
					if m.Item(i18n.Autonym(tag)).Radio(tag == s.locale).Chosen() {
						s.setLocale(tag, true)
					}
				}
			}).Height(30).Padding(0, 8).Background(ui.Transparent).BorderWidth(0).
				TextColor(p.topbarText).Label(l.LanguageLabel())
			iconButton(c, p, refreshIcon, l.CommonRefresh(), p.topbarMuted)
		})
	})
}

func (s *shell) topbarReaderChip(c *ui.Context, p palette) {
	device, ok := s.selectedDevice()
	chip := ui.Row(c).Height(28).Shrink(0).Padding(0, 10).Gap(7).Radius(999).
		Background(ui.RGBA(255, 255, 255, 0.055)).Border(1, ui.RGBA(255, 255, 255, 0.08)).
		AlignItems(ui.Center)
	chip.Children(func() {
		color := p.topbarMuted
		if s.connected() {
			color = ui.Hex("#3ED6A4")
		}
		dot(c, color, 6)
		if !ok {
			ui.Text(c, s.locales().RailNoReader()).FontSize(11.5).TextColor(p.topbarText).SingleLine()
			return
		}
		ui.Text(c, device.Name).FontSize(11.5).TextColor(p.topbarText).SingleLine()
		ui.Text(c, device.ConnString).Font("monospace").FontSize(10.5).
			TextColor(p.topbarMuted).SingleLine()
	})
}

func (s *shell) topbarCardChip(c *ui.Context, p palette) {
	if !s.cardPresent() {
		return
	}
	ui.Row(c).Height(28).Shrink(0).Padding(0, 10).Gap(7).Radius(999).
		Background(ui.RGBA(34, 195, 221, 0.10)).Border(1, ui.RGBA(34, 195, 221, 0.22)).
		AlignItems(ui.Center).Children(func() {
		ui.Text(c, s.cardName()).FontSize(11.5).TextColor(ui.Hex("#DDF7FC")).SingleLine()
		ui.Text(c, "·").FontSize(10.5).TextColor(p.topbarMuted)
		ui.Text(c, s.familyBadge()).FontSize(11.5).TextColor(ui.Hex("#DDF7FC")).SingleLine()
	})
}

func (s *shell) topbarCapacityChip(c *ui.Context, p palette) {
	l := s.locales()
	if s.cardFamily() == familyNone || s.cardFamily() == familyUnknown {
		return
	}
	ui.Row(c).Height(28).Shrink(0).Padding(0, 10).Gap(7).Radius(999).
		Background(ui.RGBA(255, 255, 255, 0.055)).Border(1, ui.RGBA(255, 255, 255, 0.08)).
		AlignItems(ui.Center).Children(func() {
		switch s.cardFamily() {
		case familyClassic:
			ui.Text(c, l.TopDump()).FontSize(11.5).TextColor(p.topbarMuted).SingleLine()
			value := l.ActionNoDump()
			if len(s.workbench.Blocks) > 0 {
				value = l.ActionBlocks(len(s.workbench.Blocks))
			}
			ui.Text(c, value).Font("monospace").FontSize(10.5).TextColor(p.topbarText).SingleLine()
		case familyType2:
			ui.Text(c, l.TabNDEF()).FontSize(11.5).TextColor(p.topbarMuted).SingleLine()
			value := "—"
			if s.type2.Loaded && s.type2.NDEF != nil && s.type2.NDEF.Present {
				value = l.Type2Capacity(s.type2.NDEF.Length)
			}
			ui.Text(c, value).Font("monospace").FontSize(10.5).TextColor(p.topbarText).SingleLine()
		}
	})
}

func (s *shell) topbarTaskPill(c *ui.Context, p palette) {
	l := s.locales()
	text := l.CommonReady()
	fg, bg, border := p.success, ui.RGBA(62, 214, 164, 0.10), ui.RGBA(62, 214, 164, 0.18)
	if s.activeTask != "" || s.pendingOps > 0 {
		text = l.CommonWorking()
		fg, bg, border = ui.Hex("#67E8F9"), ui.RGBA(34, 195, 221, 0.12), ui.RGBA(34, 195, 221, 0.22)
	} else if s.errorText != "" {
		text = l.ConnectionError()
		fg, bg, border = ui.Hex("#FDA29B"), ui.RGBA(240, 68, 56, 0.12), ui.RGBA(240, 68, 56, 0.22)
	}
	ui.Row(c).Height(27).Padding(0, 10).Gap(6).Radius(999).Background(bg).Border(1, border).
		AlignItems(ui.Center).Children(func() {
		if s.activeTask != "" || s.pendingOps > 0 {
			loadingDots(c, p)
		} else {
			ui.Icon(c, checkIcon).FontSize(11).TextColor(fg)
		}
		ui.Text(c, text).FontSize(11).TextColor(fg).SingleLine()
	})
}
