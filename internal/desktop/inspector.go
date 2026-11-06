package desktop

import (
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo/ui"
)

func (s *shell) sidePanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Width(308).Shrink(0).MinHeight(0).Background(p.surface).
		BorderWidth(0, 0, 0, 1).BorderColor(p.border).Padding(16, 16, 14).
		Gap(12).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.InspectorTitle()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Text(c, s.inspectorSub()).FontSize(9.5).TextColor(p.faint).
				Grow(1).SingleLine()
		})
		s.inspector(c, p)
		hairline(c, p)
		s.activity(c, p)
	})
}

func (s *shell) inspectorSub() string {
	l := s.locales()
	switch s.selectedTab {
	case tabBlocks:
		if block, ok := s.selectedBlock(); ok {
			return l.BlockOf(block.Block, len(s.workbench.Blocks))
		}
		return l.NoBlockSelected()
	case tabKeys:
		if key, ok := s.selectedCatalogKey(); ok {
			return key.Value
		}
		if keys, ok := s.selectedSectorKeys(); ok {
			return l.SectorHeader(keys.Sector)
		}
		return l.NoSectorSelected()
	case tabDetails:
		return l.CardInspectorSub()
	case tabMemory:
		if s.type2.Loaded {
			return s.type2.Info.ProductName
		}
		return l.Type2NoData()
	case tabNDEF:
		if s.type2.Loaded && s.type2.NDEF != nil && s.type2.NDEF.Present {
			return l.Type2Records(len(s.type2.NDEF.Records))
		}
		return l.Type2NoNDEF()
	case tabProtection:
		return l.Type2ProtectionPanel()
	default:
		return l.NoCardSelected()
	}
}

func (s *shell) inspector(c *ui.Context, p palette) {
	switch s.selectedTab {
	case tabBlocks:
		if block, ok := s.selectedBlock(); ok {
			s.blockInspector(c, p, block)
			return
		}
		s.inspectorEmpty(c, p, s.locales().BlocksEmpty())
	case tabKeys:
		if key, ok := s.selectedCatalogKey(); ok {
			s.keyInspector(c, p, key)
			return
		}
		if keys, ok := s.selectedSectorKeys(); ok {
			s.sectorInspector(c, p, keys)
			return
		}
		s.inspectorEmpty(c, p, s.locales().KeysMatrixEmpty())
	case tabDetails:
		if s.cardPresent() {
			s.cardInspector(c, p)
			return
		}
		s.inspectorEmpty(c, p, s.locales().CardEmpty())
	case tabMemory, tabNDEF, tabProtection:
		s.type2Inspector(c, p)
	default:
		s.inspectorEmpty(c, p, s.locales().CardEmpty())
	}
}

func (s *shell) inspectorCard(c *ui.Context, p palette, build func()) {
	ui.Column(c).Padding(11).Gap(9).Background(p.surfaceAlt).Radius(10).
		Border(1, p.border).Children(build)
}

func (s *shell) inspectorEmpty(c *ui.Context, p palette, text string) {
	ui.Column(c).Padding(12).Children(func() {
		ui.Text(c, text).FontSize(10).TextColor(p.muted).Wrap()
	})
}

func (s *shell) blockInspector(c *ui.Context, p palette, block app.WorkbenchBlockDTO) {
	l := s.locales()
	s.inspectorCard(c, p, func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			s.kindPill(c, p, block.Kind)
			ui.Spacer(c)
			ui.Textf(c, "%02d", block.Block).Font("monospace").FontSize(9.5).
				TextColor(p.faint)
		})
		ui.Column(c).Gap(7).Children(func() {
			fieldRow(c, p, l.ClassicSector(), fmt.Sprintf("%d", block.Sector))
			fieldRow(c, p, l.ClassicStatus(), s.statusLabel(l, block.Status))
			if block.Dirty {
				fieldRow(c, p, l.HealthChanges(), l.ClassicDirty())
			}
		})
		payloadBox(c, p, blockHexLines(block.Hex))
		ui.Text(c, block.ASCII).Font("monospace").FontSize(9).TextColor(p.muted).
			Selectable().Wrap()
		if block.Dirty {
			pill(c, l.ClassicDirty(), p.warning, p.warningSoft, p.warningBorder).
				AlignSelf(ui.Start)
		}
		switch block.Kind {
		case "manufacturer":
			ui.Text(c, l.BlockWarningManufacturer()).FontSize(9.5).TextColor(p.warning).Wrap()
		case "trailer":
			ui.Text(c, l.BlockWarningTrailer()).FontSize(9.5).TextColor(p.warning).Wrap()
		}
		if block.Access != nil {
			ui.Text(c, l.ClassicAccess()).FontSize(9.5).Bold().TextColor(p.muted)
			for _, group := range block.Access.Groups {
				readable := l.CommonNo()
				if group.KeyBReadable {
					readable = l.CommonYes()
				}
				ui.Textf(c, "G%d  %s  ·  W %s  ·  T %s  ·  B %s",
					group.Group, group.Code, group.DataWriteKeys, group.TrailerWriteKeys,
					readable).
					Font("monospace").FontSize(8.5).TextColor(p.muted).Wrap()
			}
		}
	})
}

func (s *shell) sectorInspector(c *ui.Context, p palette, keys app.SectorKeyDTO) {
	l := s.locales()
	s.inspectorCard(c, p, func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Icon(c, keyIcon).FontSize(13).TextColor(p.accent)
			ui.Text(c, l.SectorHeader(keys.Sector)).FontSize(11.5).Bold().TextColor(p.text)
		})
		ui.Column(c).Gap(7).Children(func() {
			s.keyRow(c, p, l.KeyColumnA(), keys.KeyAStatus, keys.KeyA)
			s.keyRow(c, p, l.KeyColumnB(), keys.KeyBStatus, keys.KeyB)
		})
	})
}

func (s *shell) keyRow(c *ui.Context, p palette, label, status, value string) {
	if status != "verified" || value == "" {
		fieldRow(c, p, label, s.locales().KeyUnknown())
		return
	}
	ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
		ui.Text(c, label).Width(78).Shrink(0).FontSize(9.5).TextColor(p.faint)
		dot(c, p.success, 5)
		ui.Text(c, value).Font("monospace").FontSize(9.5).TextColor(p.text).
			Selectable().SingleLine()
	})
}

func (s *shell) keyInspector(c *ui.Context, p palette, key app.KeyDTO) {
	l := s.locales()
	s.inspectorCard(c, p, func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Icon(c, keyIcon).FontSize(14).TextColor(p.accent)
			ui.Text(c, key.Value).Font("monospace").FontSize(11).Bold().TextColor(p.text).
				Grow(1).Selectable().SingleLine()
		})
		ui.Column(c).Gap(7).Children(func() {
			kind := l.KeysCustom()
			if key.Builtin {
				kind = l.KeysBuiltin()
			}
			fieldRow(c, p, l.KeyType(), kind)
			if sources := s.sourcesLabel(key.Sources); sources != "" {
				fieldRow(c, p, l.KeysSources(), sources)
			}
			if key.Removable {
				pill(c, l.KeyRemovable(), p.accent, p.accentSoft, p.accentBorder).
					AlignSelf(ui.Start)
			}
		})
	})
}

func (s *shell) cardInspector(c *ui.Context, p palette) {
	l := s.locales()
	s.inspectorCard(c, p, func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Icon(c, cardIcon).FontSize(14).TextColor(p.accent)
			ui.Text(c, s.cardName()).FontSize(11.5).Bold().TextColor(p.text).Grow(1).SingleLine()
		})
		ui.Column(c).Gap(7).Children(func() {
			monoFieldRow(c, p, l.CardUID(), s.card.UID)
			fieldRow(c, p, l.CardFamily(), s.familyLabel())
			monoFieldRow(c, p, l.CardATQA(), s.card.ATQA)
			monoFieldRow(c, p, l.CardSAK(), s.card.SAK)
		})
	})
}

func (s *shell) activity(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Grow(1).MinHeight(0).Gap(6).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.ActivityTitle()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Spacer(c)
			if linkButton(c, p, l.ActivityClear()).Clicked() {
				s.logs = nil
			}
		})
		if s.activeTask != "" || s.pendingOps > 0 {
			s.taskCard(c, p)
		}
		if len(s.logs) == 0 {
			ui.Text(c, l.ActivityEmpty()).FontSize(10).TextColor(p.faint).Wrap()
			return
		}
		s.activityList.FollowEnd = true
		ui.List(c, &s.activityList, len(s.logs), func(index int) {
			if index < 0 || index >= len(s.logs) {
				return
			}
			entry := s.logs[index]
			ui.Row(c).Padding(7, 0).Gap(7).AlignItems(ui.Start).
				BorderWidth(0, 0, 1, 0).BorderColor(p.divider).Children(func() {
				ui.Text(c, entry.time.Format("15:04:05")).Font("monospace").FontSize(8.5).
					TextColor(p.faint).Width(42).Shrink(0)
				ui.Icon(c, s.activityIcon(entry)).FontSize(12).
					TextColor(s.activityColor(p, entry)).Shrink(0)
				ui.Column(c).Grow(1).Gap(2).Children(func() {
					ui.Text(c, entry.message).FontSize(9.5).TextColor(p.muted).
						TextAlign(ui.Start).Wrap()
					meta := s.activityKindLabel(l, entry.kind) + " · " +
						s.activityTypeLabel(l, entry.eventType)
					if progress := percent(entry.progress); progress != "" {
						meta += " · " + progress
					}
					ui.Text(c, meta).FontSize(8.5).TextColor(p.faint).
						TextAlign(ui.Start).SingleLine()
				})
			})
		}).Grow(1).MinHeight(0)
	})
}

func (s *shell) taskCard(c *ui.Context, p palette) {
	l := s.locales()
	title := l.CommonWorking()
	switch s.activeKind {
	case "read":
		title = l.ReadStarted()
	case "keyscan":
		title = l.ScanStarted()
	}
	ui.Column(c).Padding(9, 10).Gap(4).Background(p.successSoft).Border(1, p.successBorder).
		Radius(9).Children(func() {
		ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
			ui.Icon(c, checkIcon).FontSize(12).TextColor(p.success)
			ui.Text(c, title).FontSize(10.5).Bold().TextColor(p.success).Grow(1).SingleLine()
			if linkButton(c, p, l.CommonCancelTask()).Clicked() {
				s.cancelActiveTask()
			}
		})
		if s.activeMessage != "" {
			ui.Text(c, s.activeMessage).FontSize(9).TextColor(p.muted).Wrap()
		}
		if s.activeProgress > 0 {
			ui.Meter(c, float64(s.activeProgress), 0, 100, nil).Height(4)
			ui.Text(c, percent(s.activeProgress)).FontSize(8.5).TextColor(p.muted)
		} else {
			loadingDots(c, p)
		}
	})
}

func (s *shell) activityTypeLabel(l locales.Messages, eventType string) string {
	switch eventType {
	case "started":
		return l.ActivityStarted()
	case "completed":
		return l.ActivityCompleted()
	case "cancelled":
		return l.ActivityCancelled()
	case "failed":
		return l.ActivityFailed()
	default:
		return l.ActivityProgress()
	}
}

func (s *shell) activityIcon(entry logEntry) *ui.SVG {
	switch entry.eventType {
	case "completed":
		return checkIcon
	case "failed":
		return warningIcon
	case "started":
		return infoIcon
	default:
		return activityIcon
	}
}

func (s *shell) activityColor(p palette, entry logEntry) ui.Color {
	switch entry.eventType {
	case "completed":
		return p.success
	case "failed":
		return p.danger
	case "started":
		return p.accent
	default:
		return p.muted
	}
}

func blockHexLines(value string) string {
	fields := strings.Fields(value)
	if len(fields) <= 8 {
		return strings.Join(fields, " ")
	}
	return strings.Join(fields[:8], " ") + "\n" + strings.Join(fields[8:], " ")
}
