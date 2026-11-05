package desktop

import (
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
	"github.com/egoist/mygo/ui"
)

type appDevice = app.DeviceDTO
type appBlock = app.WorkbenchBlockDTO

func (s *shell) devicesPage(c *ui.Context, l locales.Messages) {
	t := c.Theme()
	ui.Scroll(c).Grow(1).Padding(24).Gap(14).Children(func() {
		ui.Text(c, l.DevicesTitle()).FontSize(20).Bold()
		if len(s.devices) == 0 {
			ui.Column(c).Padding(18).Gap(6).Background(t.Surface).Radius(10).Border(1, t.Border).Children(func() {
				ui.Text(c, l.DevicesEmpty()).TextColor(t.TextMuted)
			})
		}
		for _, device := range s.devices {
			s.deviceRow(c, l, device)
		}
	})
}

func (s *shell) deviceRow(c *ui.Context, l locales.Messages, device appDevice) {
	t := c.Theme()
	connected := device.ConnString == s.connectedConn && s.connectedConn != ""
	ui.Row(c.Key(device.ID)).Padding(14, 16).Gap(16).AlignItems(ui.Center).Background(t.Surface).Radius(10).Border(1, t.Border).Children(func() {
		ui.Column(c).Grow(1).Gap(3).Children(func() {
			ui.Text(c, device.Name).Bold().SingleLine()
			ui.Text(c, device.ConnString).Font("monospace").FontSize(12).TextColor(t.TextMuted).Selectable()
			ui.Text(c, device.Transport+" · "+s.deviceCapabilities(l, device)).FontSize(12).TextColor(t.TextMuted)
		})
		if connected {
			ui.Column(c).Gap(4).AlignItems(ui.End).Children(func() {
				ui.Text(c, s.connectionLabel(l)).FontSize(12).TextColor(t.Success)
				if ui.Button(c, l.CommonDisconnect()).Clicked() {
					s.disconnectDevice()
				}
			})
		} else {
			if ui.Button(c, l.CommonConnect()).Disabled(s.pendingOps > 0 || s.activeTask != "").Clicked() {
				s.connectDevice(device.ConnString)
			}
		}
	})
}

func (s *shell) cardPage(c *ui.Context, l locales.Messages) {
	t := c.Theme()
	cardPresent := s.card != nil && s.card.Present
	isClassic := cardPresent && strings.HasPrefix(s.card.TypeCode, "mifare_classic")
	ui.Scroll(c).Grow(1).Padding(24).Gap(16).Children(func() {
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.CardTitle()).FontSize(20).Bold().Grow(1)
			if cardPresent {
				if ui.PrimaryButton(c, l.CommonReadCard()).Disabled(!isClassic || s.activeTask != "" || s.pendingOps > 0).Clicked() {
					s.readCard()
				}
				if ui.Button(c, l.CommonScanKeys()).Disabled(!isClassic || s.activeTask != "" || s.pendingOps > 0).Clicked() {
					s.scanKeys()
				}
			}
		})
		if !cardPresent {
			ui.Column(c).Padding(18).Gap(6).Background(t.Surface).Radius(10).Border(1, t.Border).Children(func() {
				ui.Text(c, l.CardEmpty()).TextColor(t.TextMuted)
			})
			return
		}
		card := *s.card
		cardType := card.Type
		if card.TypeCode == "unknown" {
			cardType = l.CommonUnknown()
		}
		ui.Column(c).Padding(18).Gap(10).Background(t.Surface).Radius(12).Border(1, t.Border).Children(func() {
			s.fact(c, l.CardUID(), card.UID)
			s.fact(c, l.CardATQA(), card.ATQA)
			s.fact(c, l.CardSAK(), card.SAK)
			s.fact(c, l.CardType(), cardType)
			s.fact(c, l.CardUIDLength(), fmt.Sprintf("%d", card.UIDLength))
			if card.Inferred {
				ui.Row(c).Gap(8).Children(func() {
					ui.Badge(c, l.CardInferred())
				})
			}
		})
		ui.Text(c, l.CardReadHint()).FontSize(12).TextColor(t.TextMuted)
		if !isClassic {
			ui.Text(c, l.CardUnsupported()).FontSize(12).TextColor(t.Warning)
		}
	})
}

func (s *shell) fact(c *ui.Context, label, value string) {
	t := c.Theme()
	ui.Row(c).Gap(12).Children(func() {
		ui.Text(c, label).Width(120).FontSize(12).TextColor(t.TextMuted)
		ui.Text(c, value).Font("monospace").Selectable().Grow(1)
	})
}

func (s *shell) blocksPage(c *ui.Context, l locales.Messages) {
	t := c.Theme()
	ui.Column(c).Fill().Padding(24).Gap(12).Children(func() {
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.ClassicTitle()).FontSize(20).Bold().Grow(1)
			ui.Text(c, l.BlocksSummary(len(s.workbench.Blocks))).FontSize(12).TextColor(t.TextMuted)
		})
		if len(s.workbench.Blocks) == 0 {
			ui.Column(c).Padding(18).Gap(6).Background(t.Surface).Radius(10).Border(1, t.Border).Children(func() {
				ui.Text(c, l.ClassicEmpty()).TextColor(t.TextMuted)
			})
			return
		}
		if s.workbench.Layout != "" {
			ui.Textf(c, "%s: %s · %s", l.ClassicLayout(), s.workbench.Layout, s.completenessLabel(l)).FontSize(12).TextColor(t.TextMuted)
		}
		s.blockList.Selected = &s.blockChosen
		s.blockList.Key = func(index int) any { return s.workbench.Blocks[index].Block }
		s.blockList.Label = func(index int) string {
			return fmt.Sprintf("%d %s", s.workbench.Blocks[index].Block, s.workbench.Blocks[index].Hex)
		}
		columns := []ui.TableColumn{
			{Title: l.ClassicBlock(), ID: "block", Width: 70, Align: ui.End, Fixed: true},
			{Title: l.ClassicSector(), ID: "sector", Width: 70, Align: ui.End},
			{Title: l.ClassicKind(), ID: "kind", Width: 110},
			{Title: l.ClassicData(), ID: "data"},
			{Title: l.ClassicStatus(), ID: "status", Width: 120},
		}
		blocks := s.workbench.Blocks
		ui.Table(c, &s.blockList, columns, len(blocks), func(row, col int) {
			block := blocks[row]
			color := t.Text
			if block.Dirty {
				color = t.Accent
			}
			switch col {
			case 0:
				ui.Textf(c, "%d", block.Block).FontSize(12).TextColor(color)
			case 1:
				ui.Textf(c, "%d", block.Sector).FontSize(12).TextColor(color)
			case 2:
				ui.Text(c, s.kindLabel(l, block.Kind)).FontSize(12).TextColor(color)
			case 3:
				ui.Text(c, block.Hex).Font("monospace").FontSize(12).SingleLine().TextColor(color)
			case 4:
				ui.Text(c, s.statusLabel(l, block.Status)).FontSize(12).TextColor(color)
			}
		}).Grow(1)
		if s.blockChosen >= 0 && s.blockChosen < len(blocks) {
			s.blockDetail(c, l, blocks[s.blockChosen])
		}
	})
}

func (s *shell) blockDetail(c *ui.Context, l locales.Messages, block appBlock) {
	t := c.Theme()
	ui.Column(c).Padding(14, 16).Gap(6).Background(t.Surface).Radius(10).Border(1, t.Border).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.ClassicDetailTitle()).Bold()
			ui.Textf(c, "%d", block.Block).Font("monospace").FontSize(12).TextColor(t.TextMuted)
			if block.Dirty {
				ui.Badge(c, l.ClassicDirty())
			}
		})
		ui.Text(c, block.Hex).Font("monospace").FontSize(12).Selectable()
		ui.Text(c, block.ASCII).Font("monospace").FontSize(12).TextColor(t.TextMuted).Selectable()
		ui.Textf(c, "%s: %s", l.ClassicStatus(), s.statusLabel(l, block.Status)).FontSize(12).TextColor(t.TextMuted)
		if block.Warning != "" {
			ui.Text(c, block.Warning).FontSize(12).TextColor(t.Warning)
		}
		if block.Access != nil {
			ui.Text(c, l.ClassicAccess()).FontSize(12).TextColor(t.TextMuted)
			for _, group := range block.Access.Groups {
				ui.Textf(c, "G%d %s · data %s · trailer %s", group.Group, group.Code, group.DataWriteKeys, group.TrailerWriteKeys).Font("monospace").FontSize(12)
			}
		}
	})
}

func (s *shell) completenessLabel(l locales.Messages) string {
	if s.workbench.Complete {
		return l.ClassicComplete()
	}
	return l.ClassicIncomplete()
}

func (s *shell) keysPage(c *ui.Context, l locales.Messages) {
	t := c.Theme()
	ui.Column(c).Fill().Padding(24).Gap(12).Children(func() {
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.KeysTitle()).FontSize(20).Bold().Grow(1)
			ui.Text(c, l.KeyCount(len(s.keys))).FontSize(12).TextColor(t.TextMuted)
			if ui.Button(c, l.CommonImport()).Disabled(s.pendingOps > 0).Clicked() {
				s.importKeys()
			}
			if ui.Button(c, l.CommonExport()).Disabled(s.pendingOps > 0).Clicked() {
				s.exportKeys()
			}
		})
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			input := ui.TextInput(c, &s.keyInput).Placeholder(l.KeysAddPlaceholder()).Label(l.CommonAdd())
			if input.Submitted() {
				s.addKey()
			}
			if ui.Button(c, l.CommonAdd()).Disabled(s.pendingOps > 0 || strings.TrimSpace(s.keyInput) == "").Clicked() {
				s.addKey()
			}
		})
		if len(s.keys) == 0 {
			ui.Text(c, l.KeysEmpty()).TextColor(t.TextMuted)
			return
		}
		s.keyList.Key = func(index int) any { return s.keys[index].ID }
		ui.List(c, &s.keyList, len(s.keys), func(index int) {
			key := s.keys[index]
			ui.Row(c.Key(key.ID)).Padding(10, 12).Gap(12).AlignItems(ui.Center).Background(t.Surface).Radius(8).Border(1, t.Border).Children(func() {
				ui.Text(c, key.Value).Font("monospace").Grow(1).Selectable()
				if key.Builtin {
					ui.Badge(c, l.KeysBuiltin())
				} else {
					ui.Badge(c, l.KeysCustom())
				}
				if len(key.Sources) > 0 {
					ui.Text(c, strings.Join(key.Sources, " · ")).FontSize(12).TextColor(t.TextMuted)
				}
				if key.Removable {
					if ui.Button(c, l.CommonDelete()).Clicked() {
						s.deleteKey(key.ID)
					}
				}
			})
		}).Grow(1)
	})
}

func (s *shell) activityPage(c *ui.Context, l locales.Messages) {
	t := c.Theme()
	ui.Column(c).Fill().Padding(24).Gap(12).Children(func() {
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.ActivityTitle()).FontSize(20).Bold().Grow(1)
			if s.activeTask != "" {
				if ui.Button(c, l.CommonCancelTask()).Clicked() {
					s.cancelActiveTask()
				}
			}
			if ui.Button(c, l.ActivityClear()).Disabled(len(s.logs) == 0).Clicked() {
				s.logs = nil
			}
		})
		if len(s.logs) == 0 {
			ui.Text(c, l.ActivityEmpty()).TextColor(t.TextMuted)
			return
		}
		s.activityList.FollowEnd = true
		ui.List(c, &s.activityList, len(s.logs), func(index int) {
			entry := s.logs[index]
			ui.Row(c).Padding(8, 10).Gap(10).AlignItems(ui.Start).Children(func() {
				ui.Text(c, entry.time.Format("15:04:05")).Font("monospace").FontSize(12).TextColor(t.TextMuted)
				ui.Column(c).Grow(1).Gap(2).Children(func() {
					ui.Text(c, entry.message).FontSize(13)
					meta := entry.kind + " · " + s.activityTypeLabel(l, entry.eventType)
					if value := percent(entry.progress); value != "" {
						meta += " · " + value
					}
					ui.Text(c, meta).FontSize(11).TextColor(t.TextMuted)
				})
			})
		}).Grow(1)
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
