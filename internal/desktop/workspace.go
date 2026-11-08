package desktop

import (
	"fmt"
	"strings"

	"github.com/egoist/mygo/ui"
)

func (s *shell) cardName() string {
	if !s.cardPresent() {
		return s.locales().CardEmptyTitle()
	}
	if s.card.Type == "" || strings.EqualFold(s.card.Type, "unknown") {
		return s.locales().CommonUnknown()
	}
	return s.card.Type
}

func (s *shell) familyLabel() string {
	switch s.cardFamily() {
	case familyClassic:
		return s.locales().FamilyClassic()
	case familyType2:
		return s.locales().FamilyType2()
	case familyISO14443:
		return s.locales().FamilyISO14443()
	default:
		return s.locales().CommonUnknown()
	}
}

func (s *shell) familyBadge() string {
	switch s.cardFamily() {
	case familyClassic:
		return s.locales().BadgeClassic()
	case familyType2:
		return s.locales().BadgeType2()
	case familyISO14443:
		return s.locales().BadgeISO14443()
	default:
		return s.locales().CommonUnknown()
	}
}

func (s *shell) workspace(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Grow(1).MinWidth(0).Padding(18, 22, 0).Children(func() {
		if s.errorText != "" {
			ui.Row(c).Padding(8, 10).Gap(10).AlignItems(ui.Center).Radius(8).
				Background(p.dangerSoft).Border(1, p.dangerBorder).Children(func() {
				ui.Icon(c, warningIcon).FontSize(13).TextColor(p.danger)
				ui.Text(c, l.ErrorDetail(s.errorText)).FontSize(10.5).TextColor(p.danger).
					Grow(1).Wrap()
				if linkButton(c, p, l.CommonClear()).Clicked() {
					s.errorText = ""
				}
			})
		}
		ui.Text(c, strings.ToUpper(l.WorkspaceEyebrow())).FontSize(9.5).Bold().
			LetterSpacing(1.4).TextColor(p.faint)
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, s.cardName()).FontSize(24).Bold().LetterSpacing(-0.2).
				TextColor(p.text).SingleLine()
			if s.cardPresent() {
				ui.Row(c).Height(22).Padding(0, 8).Radius(999).Background(p.accentSoft).
					Border(1, p.accentBorder).AlignItems(ui.Center).Children(func() {
					ui.Text(c, s.familyBadge()).FontSize(10).Bold().TextColor(p.accent)
				})
				if s.cardFamily() == familyType2 {
					pill(c, l.TabNDEF(), p.muted, p.surfaceAlt, p.border)
				}
			}
		})
		if subtitle := s.cardSubtitle(); subtitle != "" {
			ui.Text(c, subtitle).FontSize(11).TextColor(p.muted).Wrap()
		}

		s.tabs(c, p)

		ui.Column(c).Grow(1).MinHeight(0).PaddingY(14).Children(func() {
			switch s.selectedTab {
			case tabBlocks:
				s.blocksView(c, p)
			case tabKeys:
				s.keysView(c, p)
			case tabRecovery:
				s.recoveryView(c, p)
			case tabAPDU:
				s.apduView(c, p)
			case tabDESFire:
				s.desfireView(c, p)
			case tabDetails:
				s.detailsView(c, p)
			case tabMemory:
				s.type2MemoryView(c, p)
			case tabNDEF:
				switch s.cardFamily() {
				case familyClassic:
					s.classicNDEFView(c, p)
				case familyISO14443:
					s.type4View(c, p)
				default:
					s.type2NDEFView(c, p)
				}
			case tabProtection:
				s.type2ProtectionView(c, p)
			default:
				s.emptyWorkspace(c, p)
			}
		})
	})
}

func (s *shell) cardSubtitle() string {
	l := s.locales()
	switch s.cardFamily() {
	case familyClassic:
		if !s.workbench.Loaded {
			return l.CardReadHint()
		}
		return l.ClassicSubtitle(len(s.workbench.Blocks), s.sectorCount(), s.verifiedKeyCount())
	case familyType2:
		if !s.type2.Loaded {
			return l.Type2NoData()
		}
		return l.Type2Subtitle(s.type2.Info.ProductName, s.type2.Info.TotalPages, s.type2.Info.UserBytes)
	case familyISO14443:
		return l.CardISO14443Hint()
	case familyUnknown:
		return l.CardUnsupported()
	default:
		return ""
	}
}

func (s *shell) tabLabel(id tabID) string {
	switch id {
	case tabBlocks:
		return s.locales().TabBlocks()
	case tabKeys:
		return s.locales().TabKeys()
	case tabDetails:
		return s.locales().TabDetails()
	case tabMemory:
		return s.locales().TabMemory()
	case tabNDEF:
		return s.locales().TabNDEF()
	case tabProtection:
		return s.locales().TabProtection()
	case tabRecovery:
		return s.locales().TabRecovery()
	case tabAPDU:
		return s.locales().TabAPDU()
	case tabDESFire:
		return s.locales().TabDESFire()
	default:
		return ""
	}
}

func (s *shell) tabs(c *ui.Context, p palette) {
	specs := s.tabSpecs()
	if len(specs) == 0 {
		return
	}
	s.tabIndex = 0
	for index, spec := range specs {
		if spec.id == s.selectedTab {
			s.tabIndex = index
			break
		}
	}
	ui.Column(c).Shrink(0).BorderWidth(0, 0, 1, 0).BorderColor(p.border).Children(func() {
		tabs := ui.TabsBase(c, &s.tabIndex, len(specs))
		tabs.List.Gap(22).Height(34).AlignItems(ui.Center).Children(func() {
			for index, spec := range specs {
				active := spec.id == s.selectedTab
				color := p.muted
				if active {
					color = p.accent
				}
				if !spec.enabled {
					color = p.faint
				}
				tab := tabs.Tab(index).Height(34).Padding(0, 1).Gap(6).
					AlignItems(ui.Center).BorderWidth(0, 0, 2, 0).
					BorderColor(ui.Transparent)
				if active {
					tab.BorderColor(p.accent)
				}
				if !spec.enabled {
					tab.Disabled(true)
				}
				tab.Children(func() {
					label := ui.Text(c, spec.label(s.locales())).FontSize(12).
						TextColor(color).SingleLine()
					if active {
						label.Bold()
					}
					if spec.count != nil {
						if count := spec.count(s); count != "" {
							countPill(c, p, count)
						}
					}
				})
			}
		})
		if tabs.List.Changed() && s.tabIndex >= 0 && s.tabIndex < len(specs) {
			if specs[s.tabIndex].enabled {
				s.selectedTab = specs[s.tabIndex].id
				s.ensureTab()
			}
		}
	})
}

func (s *shell) emptyWorkspace(c *ui.Context, p palette) {
	ui.Column(c).Grow(1).AlignItems(ui.Center).Justify(ui.Center).Children(func() {
		ui.Column(c).Width(360).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Icon(c, cardIcon).FontSize(28).TextColor(p.faint)
			ui.Text(c, s.locales().CardEmpty()).FontSize(10.5).TextColor(p.muted).
				TextAlign(ui.Center).Wrap()
		})
	})
}

func (s *shell) blocksView(c *ui.Context, p palette) {
	l := s.locales()
	s.rebuildBlockRows()
	s.syncBlockRowChoice()
	ui.Column(c).Grow(1).MinHeight(0).Children(func() {
		ui.Column(c).Grow(1).MinHeight(0).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Row(c).Padding(12, 14, 10).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, l.BlocksPanelTitle()).FontSize(12.5).Bold().TextColor(p.text)
				if len(s.workbench.Blocks) > 0 {
					ui.Text(c, l.BlocksSummary(len(s.workbench.Blocks))).FontSize(10).
						TextColor(p.faint)
				}
			})
			s.classicActions(c, p)
			s.classicUIDPanel(c, p)
			if len(s.workbench.Blocks) == 0 {
				s.emptyPanelBody(c, p, l.BlocksEmpty())
				return
			}
			s.classicBlockEditor(c, p)
			s.blockTable(c, p)
		})
	})
}

func (s *shell) blockTable(c *ui.Context, p palette) {
	l := s.locales()
	columns := []ui.TableColumn{
		{Title: l.ClassicBlock(), ID: "block", Width: 68, Align: ui.End, Fixed: true},
		{Title: l.ClassicKind(), ID: "kind", Width: 104, Fixed: true},
		{Title: l.ClassicData(), ID: "data"},
		{Title: l.ClassicASCII(), ID: "ascii", Width: 108, Fixed: true},
		{Title: l.ClassicStatus(), ID: "status", Width: 92, Fixed: true},
	}
	s.blockList.Selected = &s.blockRowChosen
	s.blockList.Key = func(index int) any {
		row := s.blockRows[index]
		return blockRowKey{header: row.header, sector: row.sector, block: row.block}
	}
	s.blockList.Label = func(index int) string {
		row := s.blockRows[index]
		if row.header {
			return l.SectorHeader(row.sector)
		}
		return fmt.Sprintf("%s %d", l.ClassicBlock(), s.workbench.Blocks[row.block].Block)
	}
	s.blockList.Header = func(index int) bool { return s.blockRows[index].header }
	table := ui.Table(c, &s.blockList, columns, len(s.blockRows), func(rowIndex, col int) {
		if rowIndex < 0 || rowIndex >= len(s.blockRows) {
			return
		}
		row := s.blockRows[rowIndex]
		if row.header {
			if col != 0 {
				return
			}
			s.sectorHeaderCell(c, p, row.sector)
			return
		}
		if row.block < 0 || row.block >= len(s.workbench.Blocks) {
			return
		}
		block := s.workbench.Blocks[row.block]
		switch col {
		case 0:
			ui.Textf(c, "%d", block.Block).Font("monospace").FontSize(9.5).
				TextColor(p.faint)
		case 1:
			s.kindPill(c, p, block.Kind)
		case 2:
			color := p.text
			if block.Dirty {
				color = p.accent
			}
			ui.Text(c, block.Hex).Font("monospace").FontSize(10.5).TextColor(color).
				SingleLine()
		case 3:
			ui.Text(c, block.ASCII).Font("monospace").FontSize(9.5).
				TextColor(p.muted).SingleLine()
		case 4:
			if block.Dirty {
				pill(c, l.ClassicDirty(), p.warning, p.warningSoft, p.warningBorder)
			} else {
				ui.Text(c, s.statusLabel(l, block.Status)).FontSize(10).
					TextColor(s.blockStatusColor(p, block.Status)).SingleLine()
			}
		}
	}).Grow(1).MinHeight(0)
	if table.Changed() {
		if s.blockRowChosen >= 0 && s.blockRowChosen < len(s.blockRows) {
			if row := s.blockRows[s.blockRowChosen]; !row.header {
				s.blockChosen = row.block
				s.syncBlockEditor(true)
			}
		}
	}
}

func (s *shell) sectorHeaderCell(c *ui.Context, p palette, sector int) {
	l := s.locales()
	blockCount := 0
	for _, block := range s.workbench.Blocks {
		if block.Sector == sector {
			blockCount++
		}
	}
	verified := 0
	if sector >= 0 && sector < len(s.sectorKeys) {
		if s.sectorKeys[sector].KeyAStatus == "verified" {
			verified++
		}
		if s.sectorKeys[sector].KeyBStatus == "verified" {
			verified++
		}
	}
	ui.Row(c).FillWidth().Padding(6, 10).Gap(8).AlignItems(ui.Center).
		Background(p.header).Children(func() {
		ui.Text(c, l.SectorHeader(sector)).FontSize(10).Bold().TextColor(p.muted)
		ui.Text(c, l.SectorSummary(blockCount, verified)).FontSize(9).TextColor(p.faint)
	})
}

func (s *shell) kindPill(c *ui.Context, p palette, kind string) {
	l := s.locales()
	switch kind {
	case "manufacturer":
		pill(c, l.KindManufacturer(), p.accent, p.accentSoft, p.accentBorder)
	case "trailer":
		pill(c, l.KindTrailer(), p.warning, p.warningSoft, p.warningBorder)
	default:
		pill(c, l.KindData(), p.muted, p.surfaceAlt, p.border)
	}
}

func (s *shell) blockStatusColor(p palette, status string) ui.Color {
	switch status {
	case "auth_failed", "read_failed":
		return p.danger
	case "verified":
		return p.success
	case "unread":
		return p.faint
	default:
		return p.muted
	}
}

func (s *shell) keysView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		if s.recoveryReview != nil && !s.recoveryReviewOpen {
			ui.Row(c).Padding(10, 14).Gap(10).AlignItems(ui.Center).
				Background(p.warningSoft).Radius(8).Border(1, p.warningBorder).Children(func() {
				ui.Text(c, l.RecoveryReviewPending()).FontSize(10.5).TextColor(p.warning).Grow(1).Wrap()
				if ui.Button(c, l.RecoveryReviewTitle()).Height(26).FontSize(10).Clicked() {
					s.recoveryReviewOpen = true
				}
			})
		}
		s.sectorKeysPanel(c, p)
		s.keyCatalogPanel(c, p)
		s.keySetsPanel(c, p)
	})
}

func (s *shell) sectorKeysPanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Height(200).Shrink(0).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Row(c).Padding(12, 14, 10).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.KeysPanelTitle()).FontSize(12.5).Bold().TextColor(p.text)
			if len(s.sectorKeys) > 0 {
				ui.Text(c, l.KeysPanelSub(s.verifiedKeyCount(), len(s.sectorKeys)*2)).
					FontSize(10).TextColor(p.faint)
			}
			ui.Spacer(c)
			ui.Checkbox(c, &s.scanBeforeRead, l.KeysScanBeforeRead())
		})
		if len(s.sectorKeys) == 0 {
			s.emptyPanelBody(c, p, l.KeysMatrixEmpty())
			return
		}
		columns := []ui.TableColumn{
			{Title: l.ClassicSector(), ID: "sector", Width: 72, Fixed: true},
			{Title: l.KeyColumnA(), ID: "key-a"},
			{Title: l.KeyColumnB(), ID: "key-b"},
			{Title: l.KeyState(), ID: "state", Width: 74, Fixed: true},
		}
		s.sectorList.Selected = &s.sectorChosen
		s.sectorList.Key = func(index int) any { return s.sectorKeys[index].Sector }
		s.sectorList.Label = func(index int) string {
			return l.SectorHeader(s.sectorKeys[index].Sector)
		}
		table := ui.Table(c, &s.sectorList, columns, len(s.sectorKeys), func(row, col int) {
			if row < 0 || row >= len(s.sectorKeys) {
				return
			}
			key := s.sectorKeys[row]
			switch col {
			case 0:
				ui.Textf(c, "%d", key.Sector).Font("monospace").FontSize(10).
					TextColor(p.faint)
			case 1:
				s.matrixKey(c, p, key.KeyAStatus, key.KeyA)
			case 2:
				s.matrixKey(c, p, key.KeyBStatus, key.KeyB)
			case 3:
				verified := 0
				if key.KeyAStatus == "verified" {
					verified++
				}
				if key.KeyBStatus == "verified" {
					verified++
				}
				label := fmt.Sprintf("%d / 2", verified)
				switch verified {
				case 2:
					pill(c, label, p.success, p.successSoft, p.successBorder)
				case 1:
					pill(c, label, p.warning, p.warningSoft, p.warningBorder)
				default:
					pill(c, label, p.faint, p.surfaceAlt, p.border)
				}
			}
		}).Grow(1).MinHeight(0)
		if table.Changed() {
			s.keyChosen = ""
			s.keyChosenRow = -1
		}
	})
}

func (s *shell) matrixKey(c *ui.Context, p palette, status, value string) {
	if status != "verified" || value == "" {
		ui.Text(c, s.locales().KeyUnknown()).FontSize(10).TextColor(p.faint)
		return
	}
	ui.Row(c).Gap(5).AlignItems(ui.Center).Children(func() {
		dot(c, p.success, 5)
		ui.Text(c, value).Font("monospace").FontSize(10).TextColor(p.text).SingleLine()
	})
}

func (s *shell) keyCatalogPanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Grow(1).MinHeight(0).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Row(c).Padding(12, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.KeysTitle()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Text(c, l.KeyCount(len(s.keys))).FontSize(10).TextColor(p.faint)
			ui.Spacer(c)
			if ui.Button(c, l.KeysReload()).Height(26).FontSize(10).
				Disabled(s.pendingOps > 0 || s.recoveryReview != nil).Clicked() {
				s.reloadKeyStore()
			}
			if ui.Button(c, l.CommonImport()).Height(26).FontSize(10).
				Disabled(s.pendingOps > 0).Clicked() {
				s.importKeys()
			}
			if ui.Button(c, l.CommonExport()).Height(26).FontSize(10).
				Disabled(s.pendingOps > 0).Clicked() {
				s.exportKeys()
			}
		})
		ui.Row(c).Padding(0, 14, 10).Gap(6).AlignItems(ui.Center).Children(func() {
			input := ui.TextInput(c, &s.keyInput).Placeholder(l.KeysAddPlaceholder()).
				Label(l.CommonAdd())
			if input.Submitted() {
				s.addKey()
			}
			if ui.Button(c, l.CommonAdd()).Height(28).FontSize(10).
				Disabled(s.pendingOps > 0 || strings.TrimSpace(s.keyInput) == "").Clicked() {
				s.addKey()
			}
		})
		if len(s.keys) == 0 {
			s.emptyPanelBody(c, p, l.KeysEmpty())
			return
		}
		s.keyList.Selected = &s.keyChosenRow
		s.keyList.Key = func(index int) any { return s.keys[index].ID }
		s.keyList.Label = func(index int) string { return s.keys[index].Value }
		list := ui.List(c, &s.keyList, len(s.keys), func(index int) {
			if index < 0 || index >= len(s.keys) {
				return
			}
			key := s.keys[index]
			ui.Row(c).Key(key.ID).Padding(8, 14).Gap(10).AlignItems(ui.Center).
				BorderWidth(0, 0, 1, 0).BorderColor(p.divider).Children(func() {
				ui.Text(c, key.Value).Font("monospace").FontSize(10.5).
					TextColor(p.text).SingleLine()
				if key.Name != "" {
					ui.Text(c, key.Name).FontSize(9.5).TextColor(p.muted).SingleLine()
				}
				if key.Builtin {
					pill(c, l.KeysBuiltin(), p.accent, p.accentSoft, p.accentBorder)
				} else {
					pill(c, l.KeysCustom(), p.muted, p.surfaceAlt, p.border)
				}
				if sources := s.sourcesLabel(key.Sources); sources != "" {
					ui.Text(c, sources).FontSize(9.5).
						TextColor(p.faint).Grow(1).SingleLine()
				} else {
					ui.Spacer(c)
				}
				if key.Removable {
					button := iconButton(c, p, trashIcon, l.CommonDelete(), p.faint)
					if button.Clicked() {
						s.deleteKey(key.ID)
					}
				}
			})
		}).Grow(1).MinHeight(0)
		if list.Changed() {
			if s.keyChosenRow >= 0 && s.keyChosenRow < len(s.keys) {
				s.keyChosen = s.keys[s.keyChosenRow].ID
			}
		}
	})
}

func (s *shell) keySetsPanel(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Height(180).Shrink(0).Background(p.surface).Radius(10).
		Border(1, p.border).Children(func() {
		ui.Row(c).Padding(12, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.KeySetsTitle()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Text(c, l.KeySetListCount(len(s.keySets))).FontSize(10).TextColor(p.faint)
			ui.Spacer(c)
			if ui.Button(c, l.KeySetSaveVerified()).Height(26).FontSize(10).
				Disabled(s.pendingOps > 0 || s.recoveryReview != nil || s.verifiedKeyCount() == 0).Clicked() {
				s.keySetNameInput = ""
				s.keySetError = ""
				s.keySetCreateOpen = true
			}
		})
		if s.keySetError != "" {
			ui.Text(c, s.keySetError).FontSize(9.5).TextColor(p.danger).Wrap()
		}
		if len(s.keySets) == 0 {
			s.emptyPanelBody(c, p, l.KeySetsEmpty())
			return
		}
		ui.Scroll(c).Grow(1).MinHeight(0).Children(func() {
			for _, set := range s.keySets {
				set := set
				ui.Row(c).Padding(8, 14).Gap(10).AlignItems(ui.Center).
					BorderWidth(0, 0, 1, 0).BorderColor(p.divider).Children(func() {
					ui.Text(c, set.Name).FontSize(10.5).Bold().TextColor(p.text).SingleLine()
					pill(c, l.KeySetCount(set.KeyCount), p.muted, p.surfaceAlt, p.border)
					scope := set.CardID
					if scope == "" {
						scope = l.KeySetAllCards()
					}
					ui.Text(c, scope).FontSize(9.5).TextColor(p.faint).Grow(1).SingleLine()
					if linkButton(c, p, l.KeysRename()).Clicked() {
						s.keySetRenameID = set.ID
						s.keySetRenameInput = set.Name
						s.keySetError = ""
						s.keySetRenameOpen = true
					}
					if linkButton(c, p, l.KeySetExport()).Clicked() {
						s.keySetExportID = set.ID
						s.keySetError = ""
						s.keySetExportOpen = true
					}
					button := iconButton(c, p, trashIcon, l.CommonDelete(), p.faint)
					if button.Clicked() {
						s.keySetDeleteID = set.ID
						s.keySetError = ""
						s.keySetDeleteOpen = true
					}
				})
			}
		})
	})
}

func (s *shell) sourcesLabel(sources []string) string {
	labels := make([]string, 0, len(sources))
	for _, source := range sources {
		if source == "builtin" {
			continue
		}
		labels = append(labels, s.keySourceLabel(s.locales(), source))
	}
	return strings.Join(labels, " · ")
}

func (s *shell) detailsView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Scroll(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		if !s.cardPresent() {
			s.emptyPanelBody(c, p, l.CardEmpty())
			return
		}
		ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Text(c, l.DetailsCard()).FontSize(12.5).Bold().TextColor(p.text)
			s.factRow(c, p, l.CardUID(), s.card.UID, true)
			s.factRow(c, p, l.CardATQA(), s.card.ATQA, true)
			s.factRow(c, p, l.CardSAK(), s.card.SAK, true)
			s.factRow(c, p, l.CardType(), s.cardName(), false)
			s.factRow(c, p, l.CardFamily(), s.familyBadge(), false)
			s.factRow(c, p, l.CardUIDLength(), fmt.Sprintf("%d", s.card.UIDLength), false)
			if s.card.Inferred {
				s.factRow(c, p, l.CardInferred(), l.CommonYes(), false)
			}
		})
		ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Text(c, l.DetailsDump()).FontSize(12.5).Bold().TextColor(p.text)
			if !s.workbench.Loaded {
				ui.Text(c, l.NoDump()).FontSize(10.5).TextColor(p.muted)
				return
			}
			name := s.workbench.Name
			if name == "" {
				name = l.DetailsUnsaved()
			}
			s.factRow(c, p, l.DetailsSource(), name, false)
			s.factRow(c, p, l.DetailsLayout(), s.workbench.Layout, true)
			s.factRow(c, p, l.DetailsGeometry(),
				l.DumpBlocks(len(s.workbench.Blocks))+" · "+l.DumpSectors(s.sectorCount()), false)
			if s.workbench.SourceUID != "" {
				s.factRow(c, p, l.DetailsSourceUID(), s.workbench.SourceUID, true)
			}
			s.factRow(c, p, l.DetailsComplete(), s.completenessLabelValue(), false)
			s.factRow(c, p, l.DetailsValidation(), s.validationLabel(), false)
			var messages []string
			if len(s.workbench.Validation.Errors) > 0 {
				messages = append(messages, l.ValidationErrors(len(s.workbench.Validation.Errors)))
			}
			if len(s.workbench.Validation.Warnings) > 0 {
				messages = append(messages, l.ValidationWarnings(len(s.workbench.Validation.Warnings)))
			}
			if len(messages) == 0 {
				messages = append(messages, l.ValidationClean())
			}
			s.factRow(c, p, l.DetailsMessages(), strings.Join(messages, " · "), false)
			if s.cardFamily() != familyClassic {
				ui.Text(c, l.CardUnsupported()).FontSize(10).TextColor(p.warning)
			}
		})
	})
}

func (s *shell) completenessLabelValue() string {
	if s.workbench.Complete {
		return s.locales().HealthComplete()
	}
	return s.locales().HealthPartial()
}

func (s *shell) validationLabel() string {
	if s.workbench.Validation.Valid {
		return s.locales().HealthValid()
	}
	return s.locales().HealthInvalid()
}

func (s *shell) factRow(c *ui.Context, p palette, label, value string, mono bool) {
	ui.Row(c).Gap(10).AlignItems(ui.Start).Children(func() {
		ui.Text(c, label).Width(112).Shrink(0).FontSize(10).TextColor(p.faint)
		text := ui.Text(c, value).FontSize(10).TextColor(p.text).Grow(1).Wrap()
		if mono {
			text.Font("monospace").FontSize(9.5).Selectable()
		}
	})
}

func (s *shell) emptyPanelBody(c *ui.Context, p palette, text string) {
	ui.Column(c).Grow(1).Padding(18).Justify(ui.Center).Children(func() {
		ui.Text(c, text).FontSize(10.5).TextColor(p.muted).Wrap()
	})
}
