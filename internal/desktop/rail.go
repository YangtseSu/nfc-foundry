package desktop

import (
	"fmt"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func (s *shell) rail(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Width(236).Shrink(0).MinHeight(0).Background(p.rail).
		BorderWidth(0, 0, 0, 1).BorderColor(p.border).Padding(16, 14, 14).
		Gap(12).Children(func() {
		sectionLabel(c, p, l.RailReader())
		s.readerPanel(c, p)
		sectionLabel(c, p, l.RailCard())
		s.cardPanel(c, p)
		sectionLabel(c, p, l.RailHealth())
		s.healthPanel(c, p)
	})
}

func (s *shell) readerPanel(c *ui.Context, p palette) {
	l := s.locales()
	device, hasDevice := s.selectedDevice()
	ui.Column(c).Padding(12).Gap(9).Background(p.surface).Radius(10).Border(1, p.border).
		Children(func() {
			ui.Row(c).Gap(9).AlignItems(ui.Center).Children(func() {
				ui.Row(c).Width(28).Height(28).Shrink(0).Radius(8).Background(p.accentSoft).
					AlignItems(ui.Center).Justify(ui.Center).Children(func() {
					ui.Icon(c, readerIcon).FontSize(16).TextColor(p.accent)
				})
				ui.Column(c).Grow(1).Gap(2).Children(func() {
					name := l.RailNoReader()
					if hasDevice {
						name = device.Name
					}
					ui.Text(c, name).FontSize(12.5).Bold().TextColor(p.text).SingleLine()
					ui.Row(c).Gap(4).AlignItems(ui.Center).Children(func() {
						color := p.faint
						if s.connected() {
							color = p.success
						} else if hasDevice && s.connection.Status == "error" {
							color = p.danger
						}
						dot(c, color, 5)
						ui.Text(c, s.connectionLabel(l)).FontSize(10).TextColor(color).SingleLine()
					})
				})
				refresh := iconButton(c, p, refreshIcon, l.CommonRefresh(), p.faint)
				if refresh.Clicked() {
					s.refreshDevices()
				}
			})
			if hasDevice {
				ui.Text(c, device.ConnString).Font("monospace").FontSize(9.5).
					TextColor(p.muted).SingleLine()
				s.readerCaps(c, p, device)
				ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
					if len(s.devices) > 1 {
						ui.PopoverMenuButton(c, l.RailPickReader(), func(m *ui.Menu) {
							for _, candidate := range s.devices {
								label := candidate.Name + " · " + candidate.ConnString
								if m.Item(label).Radio(candidate.ConnString == s.connectedConn).Chosen() {
									if candidate.ConnString != s.connectedConn {
										s.connectDevice(candidate.ConnString)
									}
								}
							}
						}).Height(28).FontSize(10)
					}
					if s.connected() {
						if ui.Button(c, l.CommonDisconnect()).Height(28).FontSize(10).
							Disabled(s.pendingOps > 0 || s.activeTask != "").Clicked() {
							s.disconnectDevice()
						}
					} else if primaryButton(c, p, l.CommonConnect()).Height(28).
						Disabled(s.pendingOps > 0 || s.activeTask != "").Clicked() {
						s.connectDevice(device.ConnString)
					}
				})
			} else {
				ui.Text(c, l.RailNoReaderHint()).FontSize(10).TextColor(p.muted)
			}
		})
}

func (s *shell) readerCaps(c *ui.Context, p palette, device app.DeviceDTO) {
	caps := []string{device.Transport}
	if device.Nested {
		caps = append(caps, s.locales().CapNested())
	}
	if device.Darkside {
		caps = append(caps, s.locales().CapDarkside())
	}
	if device.Hardnested {
		caps = append(caps, s.locales().CapHardnested())
	}
	for _, row := range splitCaps(caps, 2) {
		ui.Row(c).Gap(4).Children(func() {
			for _, cap := range row {
				pill(c, cap, p.muted, p.rail, p.border)
			}
		})
	}
}

// splitCaps keeps the capability pills within the narrow rail.
func splitCaps(caps []string, perRow int) [][]string {
	var rows [][]string
	for len(caps) > 0 {
		n := min(perRow, len(caps))
		rows = append(rows, caps[:n])
		caps = caps[n:]
	}
	return rows
}

func (s *shell) cardPanel(c *ui.Context, p palette) {
	l := s.locales()
	if !s.cardPresent() {
		ui.Column(c).Padding(12).Gap(6).Background(p.surface).Radius(10).Border(1, p.border).
			Children(func() {
				ui.Text(c, l.CardEmptyTitle()).FontSize(13).Bold().TextColor(p.text)
				ui.Text(c, l.CardEmpty()).FontSize(10).TextColor(p.muted).Wrap()
			})
		return
	}
	ui.Column(c).Padding(12).Gap(9).Background(p.surface).Radius(10).Border(1, p.border).
		Children(func() {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				ui.Row(c).Width(42).Height(48).Shrink(0).Radius(8).
					LinearGradient(ui.LinearGradient{
						From: ui.Hex("#13354E"), To: ui.Hex("#0B1B2B"), Angle: 145,
					}).Border(1, ui.Hex("#26465F")).AlignItems(ui.Center).Justify(ui.Center).
					Children(func() {
						ui.Icon(c, cardIcon).FontSize(20).TextColor(ui.Hex("#37CFE8"))
					})
				ui.Column(c).Grow(1).Gap(3).Children(func() {
					ui.Text(c, s.cardName()).FontSize(13.5).Bold().TextColor(p.text).SingleLine()
					ui.Text(c, s.familyLabel()).FontSize(10).TextColor(p.muted).SingleLine()
				})
			})
			ui.Text(c, s.card.UID).Font("monospace").FontSize(10).TextColor(p.text).Selectable().
				SingleLine()
			ui.Row(c).Height(1).Background(p.divider)
			if s.cardFamily() == familyClassic {
				ui.Row(c).Gap(8).Children(func() {
					s.metric(c, p, l.MetricBlocks(), fmt.Sprintf("%d", len(s.workbench.Blocks)))
					s.metric(c, p, l.MetricSectors(), fmt.Sprintf("%d", s.sectorCount()))
				})
			} else {
				ui.Row(c).Gap(8).Children(func() {
					s.metric(c, p, l.CardUIDLength(), fmt.Sprintf("%d", s.card.UIDLength))
					s.metric(c, p, l.CardFamily(), s.familyBadge())
				})
			}
			if s.card.Inferred {
				pill(c, l.CardInferred(), p.warning, p.warningSoft, p.warningBorder).
					AlignSelf(ui.Start)
			}
		})
}

func (s *shell) metric(c *ui.Context, p palette, label, value string) {
	ui.Column(c).Grow(1).Basis(0).Gap(2).Children(func() {
		ui.Text(c, label).FontSize(9).Bold().LetterSpacing(0.8).TextColor(p.faint).SingleLine()
		ui.Text(c, value).FontSize(11.5).Bold().TextColor(p.text).SingleLine()
	})
}

func (s *shell) healthPanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).PaddingY(6).Background(p.surface).Radius(10).Border(1, p.border).
		Children(func() {
			dumpLabel, dumpColor := l.HealthNotLoaded(), p.faint
			if s.workbench.Loaded {
				if s.workbench.Complete {
					dumpLabel, dumpColor = l.HealthComplete(), p.success
				} else {
					dumpLabel, dumpColor = l.HealthPartial(), p.warning
				}
			}
			s.healthRow(c, p, checkIcon, dumpColor, l.HealthDump(), dumpLabel, s.workbench.Loaded)

			validationLabel, validationColor := l.CommonUnknown(), p.faint
			if s.workbench.Loaded {
				if s.workbench.Validation.Valid {
					validationLabel, validationColor = l.HealthValid(), p.success
				} else {
					validationLabel, validationColor = l.HealthInvalid(), p.danger
				}
			}
			s.healthRow(c, p, checkIcon, validationColor, l.HealthValidation(), validationLabel, s.workbench.Loaded)

			verified := s.verifiedKeyCount()
			keyLabel := l.CommonNotAvailable()
			keyColor := p.faint
			if len(s.sectorKeys) > 0 {
				keyLabel = fmt.Sprintf("%d / %d", verified, len(s.sectorKeys)*2)
				if verified > 0 {
					keyColor = p.success
				}
			}
			s.healthRow(c, p, keyIcon, keyColor, l.HealthKeys(), keyLabel, s.cardPresent() && len(s.sectorKeys) > 0)

			changesLabel, changesColor := l.HealthNoChanges(), p.success
			if len(s.workbench.DirtyBlocks) > 0 {
				changesLabel, changesColor = l.HealthUnsaved(len(s.workbench.DirtyBlocks)), p.warning
			}
			s.healthRow(c, p, warningIcon, changesColor, l.HealthChanges(), changesLabel, true)
		})
}

func (s *shell) healthRow(c *ui.Context, p palette, icon *ui.SVG, color ui.Color, label, value string, enabled bool) {
	ui.Row(c).Padding(7, 12).Gap(8).AlignItems(ui.Center).Children(func() {
		iconColor := p.faint
		if enabled {
			iconColor = color
		}
		ui.Icon(c, icon).FontSize(12).TextColor(iconColor)
		ui.Text(c, label).FontSize(10.5).TextColor(p.muted).Grow(1).SingleLine()
		ui.Text(c, value).FontSize(10).TextColor(color).SingleLine()
	})
}
