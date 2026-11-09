package desktop

import (
	"fmt"
	"sort"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func (s *shell) diffView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		ui.Column(c).Grow(1).MinHeight(0).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Row(c).Padding(12, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, l.DiffTitle()).FontSize(12.5).Bold().TextColor(p.text)
				if s.diffPath != "" {
					ui.Text(c, s.diffPath).FontSize(10).TextColor(p.faint).SingleLine()
				}
				ui.Spacer(c)
				if ui.Button(c, l.DiffLoad()).Height(26).FontSize(10).Disabled(s.pendingOps > 0).Clicked() {
					s.loadComparisonDump()
				}
				if s.diffLoaded {
					if ui.Button(c, l.DiffRefresh()).Height(26).FontSize(10).Disabled(s.pendingOps > 0).Clicked() {
						s.refreshComparison()
					}
					if linkButton(c, p, l.DiffClear()).Clicked() {
						s.clearComparison()
					}
				}
			})
			if !s.diffLoaded {
				s.emptyPanelBody(c, p, l.DiffNoComparison())
				return
			}
			ui.Row(c).Padding(0, 14, 8).Gap(10).AlignItems(ui.Center).Children(func() {
				pill(c, l.DiffChanged(s.diff.ChangedBlocks), p.warning, p.warningSoft, p.warningBorder)
				pill(c, l.DiffEqual(s.diff.EqualBlocks), p.muted, p.surfaceAlt, p.border)
				pill(c, l.DiffUnknown(s.diff.UnknownBlocks), p.danger, p.dangerSoft, p.dangerBorder)
				if s.diff.TrailerChanges != 0 {
					pill(c, l.DiffTrailerChanges(s.diff.TrailerChanges), p.accent, p.accentSoft, p.accentBorder)
				}
			})
			if s.diff.IdentityChanged {
				ui.Text(c, l.DiffIdentityChanged(s.diff.Before.UID, s.diff.After.UID)).
					FontSize(10).TextColor(p.warning).Wrap()
			}
			ui.Row(c).Padding(0, 14, 8).Gap(12).AlignItems(ui.Center).Children(func() {
				ui.Checkbox(c, &s.diffChangedOnly, l.DiffChangedOnly())
				ui.Checkbox(c, &s.diffUnknownOnly, l.DiffUnknownOnly())
				ui.Checkbox(c, &s.diffTrailersOnly, l.DiffTrailersOnly())
				ui.Spacer(c)
				ui.Text(c, l.DiffReadOnly()).FontSize(9.5).TextColor(p.faint).Wrap()
			})
			s.rebuildDiffRows()
			if len(s.diffRows) == 0 {
				s.emptyPanelBody(c, p, l.DiffEmpty())
				return
			}
			columns := []ui.TableColumn{
				{Title: l.ClassicBlock(), ID: "block", Width: 62, Align: ui.End, Fixed: true},
				{Title: l.ClassicSector(), ID: "sector", Width: 60, Align: ui.End, Fixed: true},
				{Title: l.ClassicKind(), ID: "kind", Width: 92, Fixed: true},
				{Title: l.DiffState(), ID: "state", Width: 76, Fixed: true},
				{Title: l.DiffBefore(), ID: "before"},
				{Title: l.DiffAfter(), ID: "after"},
			}
			s.diffList.Selected = nil
			s.diffList.Key = func(index int) any { return s.diffRows[index] }
			s.diffList.Label = func(index int) string {
				block := s.diff.Blocks[s.diffRows[index]]
				return fmt.Sprintf("%s %d", l.ClassicBlock(), block.Block)
			}
			ui.Table(c, &s.diffList, columns, len(s.diffRows), func(row, col int) {
				if row < 0 || row >= len(s.diffRows) {
					return
				}
				block := s.diff.Blocks[s.diffRows[row]]
				switch col {
				case 0:
					ui.Textf(c, "%d", block.Block).Font("monospace").FontSize(10).TextColor(p.faint)
				case 1:
					ui.Textf(c, "%d", block.Sector).Font("monospace").FontSize(10).TextColor(p.faint)
				case 2:
					s.kindPill(c, p, block.Kind)
				case 3:
					s.diffStatePill(c, p, block.State)
				case 4:
					ui.Text(c, spacedHex(block.BeforeHex)).Font("monospace").FontSize(9.5).TextColor(p.muted).SingleLine()
				case 5:
					ui.Text(c, spacedHex(block.AfterHex)).Font("monospace").FontSize(9.5).TextColor(p.text).SingleLine()
				}
			}).Grow(1).MinHeight(0)
		})
	})
}

func (s *shell) rebuildDiffRows() {
	s.diffRows = s.diffRows[:0]
	for index, block := range s.diff.Blocks {
		if s.diffTrailersOnly && block.Kind != "trailer" {
			continue
		}
		if s.diffUnknownOnly {
			if block.State != "unknown" {
				continue
			}
		} else if s.diffChangedOnly && block.State == "equal" {
			continue
		}
		s.diffRows = append(s.diffRows, index)
	}
}

func (s *shell) diffStatePill(c *ui.Context, p palette, state string) {
	l := s.locales()
	switch state {
	case "changed":
		pill(c, l.DiffStateChanged(), p.warning, p.warningSoft, p.warningBorder)
	case "unknown":
		pill(c, l.DiffStateUnknown(), p.danger, p.dangerSoft, p.dangerBorder)
	default:
		pill(c, l.DiffStateEqual(), p.muted, p.surfaceAlt, p.border)
	}
}

func (s *shell) diffInspector(c *ui.Context, p palette) {
	l := s.locales()
	if !s.diffLoaded {
		s.inspectorEmpty(c, p, l.DiffNoComparison())
		return
	}
	s.inspectorCard(c, p, func() {
		ui.Text(c, l.DiffTitle()).FontSize(11.5).Bold().TextColor(p.text)
		ui.Column(c).Gap(7).Children(func() {
			fieldRow(c, p, l.DiffBefore(), s.diff.Before.UID)
			fieldRow(c, p, l.DiffAfter(), s.diff.After.UID)
			fieldRow(c, p, l.DetailsLayout(), s.diff.Layout)
			fieldRow(c, p, l.DetailsReadable(), fmt.Sprintf("%d / %d", s.diff.After.ReadableBlocks, s.diff.After.TotalBlocks))
			fieldRow(c, p, l.DiffChanged(0), fmt.Sprintf("%d", s.diff.ChangedBlocks))
			fieldRow(c, p, l.DiffUnknown(0), fmt.Sprintf("%d", s.diff.UnknownBlocks))
		})
		ui.Text(c, l.DiffReadOnly()).FontSize(9.5).TextColor(p.muted).Wrap()
	})
}

func (s *shell) loadComparisonDump() {
	if s.pendingOps > 0 || s.activeTask != "" {
		return
	}
	s.pendingOps++
	go func() {
		diff, err := s.api.OpenComparisonDump()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().DiffLoadFailed(err.Error())
				return
			}
			if diff.Cancelled {
				return
			}
			s.diff = diff
			s.diffLoaded = true
			s.diffPath = s.api.ComparisonPath()
			s.errorText = ""
			s.selectedTab = tabDiff
		})
	}()
}

func (s *shell) refreshComparison() {
	if s.pendingOps > 0 || s.activeTask != "" {
		return
	}
	s.pendingOps++
	go func() {
		diff, err := s.api.ComparisonDiff()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().DiffLoadFailed(err.Error())
				return
			}
			s.diff = diff
			s.diffLoaded = true
			s.diffPath = s.api.ComparisonPath()
			s.errorText = ""
		})
	}()
}

func (s *shell) clearComparison() {
	s.api.ClearComparisonDump()
	s.diff = app.DumpDiffDTO{}
	s.diffLoaded = false
	s.diffPath = ""
	s.diffRows = nil
}

func (s *shell) valueBlockEditor(c *ui.Context, p palette, block app.WorkbenchBlockDTO) {
	l := s.locales()
	ui.Row(c).Padding(0, 14, 8).Gap(8).AlignItems(ui.End).Children(func() {
		ui.Text(c, l.ClassicValueTitle()).FontSize(9.5).TextColor(p.faint).Shrink(0)
		ui.TextInput(c, &s.valueInput).Placeholder(l.ClassicValuePlaceholder()).Label(l.ClassicValueLabel()).Width(160)
		ui.TextInput(c, &s.valueAddressInput).Placeholder(l.ClassicAddressPlaceholder()).Label(l.ClassicAddressLabel()).Width(90)
		if ui.Button(c, l.ClassicValuePreview()).Height(28).FontSize(10).
			Disabled(s.writeBusy() || strings.TrimSpace(s.valueInput) == "").Clicked() {
			s.requestValuePreview(block.Block)
		}
	})
	if block.Value != nil {
		ui.Text(c, l.ClassicValueDecoded(fmt.Sprintf("%d", block.Value.Value), fmt.Sprintf("%d", block.Value.Address))).FontSize(9.5).TextColor(p.muted).Wrap()
	}
	if s.valuePreview != nil && s.valuePreview.Block == block.Block {
		ui.Text(c, l.ClassicValuePreviewHex(s.valuePreview.Hex)).Font("monospace").FontSize(9).TextColor(p.faint).Wrap()
	}
}

func (s *shell) trailerEditor(c *ui.Context, p palette, block app.WorkbenchBlockDTO) {
	l := s.locales()
	ui.Row(c).Padding(0, 14, 8).Gap(8).AlignItems(ui.End).Children(func() {
		ui.TextInput(c, &s.trailerKeyA).Placeholder(l.ClassicTrailerKeyAPlaceholder()).Label(l.ClassicTrailerKeyA()).Width(170)
		ui.TextInput(c, &s.trailerKeyB).Placeholder(l.ClassicTrailerKeyBPlaceholder()).Label(l.ClassicTrailerKeyB()).Width(170)
		ui.TextInput(c, &s.trailerGPB).Placeholder(l.ClassicTrailerGPBPlaceholder()).Label(l.ClassicTrailerGPB()).Width(80)
	})
	ui.Column(c).Padding(0, 14, 8).Gap(4).Children(func() {
		ui.Text(c, l.ClassicAccess()).FontSize(9.5).Bold().TextColor(p.muted)
		for group := range 4 {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				ui.Textf(c, "G%d", group).Font("monospace").FontSize(9.5).TextColor(p.faint).Width(24)
				ui.Checkbox(c, &s.trailerGroups[group][0], "C1")
				ui.Checkbox(c, &s.trailerGroups[group][1], "C2")
				ui.Checkbox(c, &s.trailerGroups[group][2], "C3")
				if block.Access != nil && group < len(block.Access.Groups) {
					summary := block.Access.Groups[group]
					if group == 3 {
						readable := l.CommonNo()
						if summary.KeyBReadable {
							readable = l.CommonYes()
						}
						ui.Text(c, l.ClassicTrailerPermissions(summary.TrailerKeyA, summary.TrailerAccess, summary.TrailerKeyB, readable)).
							FontSize(9).TextColor(p.faint).Wrap()
					} else {
						ui.Text(c, l.ClassicAccessSummary(summary.DataReadKeys, summary.DataWriteKeys, summary.IncrementKeys, summary.DecrementKeys)).
							FontSize(9).TextColor(p.faint).Wrap()
					}
				}
			})
		}
	})
	ui.Row(c).Padding(0, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
		if ui.Button(c, l.ClassicTrailerPreview()).Height(28).FontSize(10).
			Disabled(s.writeBusy() || strings.TrimSpace(s.trailerKeyA) == "" || strings.TrimSpace(s.trailerKeyB) == "").Clicked() {
			s.requestTrailerPreview(block.Block)
		}
		if s.trailerPreview != nil && s.trailerPreview.Block == block.Block {
			ui.Text(c, l.ClassicTrailerPreviewHex(s.trailerPreview.Hex)).Font("monospace").FontSize(9).TextColor(p.faint).Wrap()
		}
		ui.Spacer(c)
	})
}

func (s *shell) syncValueEditor(reset bool, block app.WorkbenchBlockDTO) {
	if block.Kind != "data" {
		s.valueInput = ""
		s.valueAddressInput = ""
		s.valuePreview = nil
		s.valuePreviewBlock = -1
		return
	}
	if reset || s.valuePreviewBlock != block.Block {
		value, address := int64(0), block.Block
		if block.Value != nil {
			value, address = block.Value.Value, block.Value.Address
		}
		s.valueInput = fmt.Sprintf("%d", value)
		s.valueAddressInput = fmt.Sprintf("%d", address)
		s.valuePreview = nil
		s.valuePreviewBlock = block.Block
	}
}

func (s *shell) syncTrailerEditor(reset bool, block app.WorkbenchBlockDTO) {
	if block.Kind != "trailer" {
		s.trailerPreview = nil
		s.trailerPreviewBlock = -1
		return
	}
	if !reset && s.trailerPreviewBlock == block.Block {
		return
	}
	hex := normalizeClassicHex(block.Hex)
	if len(hex) == 32 {
		s.trailerKeyA = hex[0:12]
		s.trailerKeyB = hex[20:32]
		s.trailerGPB = hex[18:20]
	}
	s.trailerPreview = nil
	s.trailerPreviewBlock = block.Block
	for group := range 4 {
		s.trailerGroups[group] = [3]bool{}
	}
	if block.Access != nil {
		for group, summary := range block.Access.Groups {
			code := summary.Code
			if len(code) != 3 {
				continue
			}
			s.trailerGroups[group] = [3]bool{code[0] == '1', code[1] == '1', code[2] == '1'}
		}
	}
}

func (s *shell) requestValuePreview(blockNumber int) {
	if s.writeBusy() {
		return
	}
	address := -1
	if strings.TrimSpace(s.valueAddressInput) != "" {
		if _, err := fmt.Sscanf(strings.TrimSpace(s.valueAddressInput), "%d", &address); err != nil {
			s.errorText = s.locales().ClassicValueInvalid("address must be a number")
			return
		}
	}
	value := strings.TrimSpace(s.valueInput)
	s.pendingOps++
	go func() {
		preview, err := s.api.PreviewValueBlock(app.ValueBlockPreviewRequestDTO{Block: blockNumber, Value: value, Address: address})
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicValueInvalid(err.Error())
				return
			}
			s.valuePreview = &preview
			s.valuePreviewBlock = blockNumber
			s.blockInput = preview.Hex
			s.blockInputBlock = blockNumber
			s.errorText = ""
			s.requestBlockWrite()
		})
	}()
}

func (s *shell) requestTrailerPreview(blockNumber int) {
	if s.writeBusy() {
		return
	}
	groups := make([]app.AccessConditionInputDTO, 4)
	for group := range 4 {
		groups[group] = app.AccessConditionInputDTO{
			Group: group, C1: s.trailerGroups[group][0], C2: s.trailerGroups[group][1], C3: s.trailerGroups[group][2],
		}
	}
	request := app.TrailerPreviewRequestDTO{
		Block: blockNumber, KeyA: s.trailerKeyA, KeyB: s.trailerKeyB, GPB: s.trailerGPB, Groups: groups,
	}
	s.pendingOps++
	go func() {
		preview, err := s.api.PreviewTrailer(request)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicTrailerInvalid(err.Error())
				return
			}
			s.trailerPreview = &preview
			s.trailerPreviewBlock = blockNumber
			s.errorText = ""
			s.preflightTrailerWrite(blockNumber, preview.Hex)
		})
	}()
}

func (s *shell) preflightTrailerWrite(blockNumber int, hex string) {
	s.pendingOps++
	go func() {
		_, editErr := s.api.EditBlockHex(blockNumber, hex)
		var preflight app.WritePreflightDTO
		var preflightErr error
		if editErr == nil {
			preflight, preflightErr = s.api.PreflightWrite(app.WritePreflightRequestDTO{Mode: "trailer", Blocks: []int{blockNumber}})
		}
		s.update(func() {
			s.pendingOps--
			if editErr != nil {
				s.errorText = s.locales().ClassicWriteFailed(editErr.Error())
				return
			}
			if preflightErr != nil {
				s.errorText = s.locales().ClassicWriteFailed(preflightErr.Error())
				return
			}
			if !preflight.Valid {
				s.errorText = s.locales().ClassicPreflightFailed(preflight.Error)
				return
			}
			s.trailerConfirm = ""
			s.trailerIntent = &classicWriteIntent{Kind: "trailer", Block: blockNumber, Hex: hex, Preflight: preflight}
			s.trailerDialogOpen = true
			s.errorText = ""
		})
	}()
}

func (s *shell) trailerDialog(c *ui.Context) {
	if !s.trailerDialogOpen || s.trailerIntent == nil {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	intent := *s.trailerIntent
	ui.Modal(c, &s.trailerDialogOpen, func() {
		ui.Text(c, l.ClassicTrailerConfirmTitle(intent.Block)).FontSize(15).Bold().TextColor(p.danger)
		ui.Text(c, l.ClassicTrailerConfirmNew(intent.Hex)).Font("monospace").FontSize(9.5).Wrap()
		if intent.Preflight.BackupPath != "" {
			ui.Text(c, l.ClassicBackupPath(intent.Preflight.BackupPath)).FontSize(9.5).Wrap()
		}
		for _, access := range intent.Preflight.Access {
			for _, group := range access.Access.Groups {
				ui.Textf(c, "G%d %s · read %s · write %s · increment %s · decrement %s",
					group.Group, group.Code, group.DataReadKeys, group.DataWriteKeys, group.IncrementKeys, group.DecrementKeys).
					Font("monospace").FontSize(8.5).Wrap()
			}
		}
		ui.Text(c, l.ClassicTrailerWarning()).FontSize(9.5).TextColor(p.warning).Wrap()
		ui.Text(c, l.ClassicTrailerConfirmHint(intent.Block)).FontSize(10).Bold()
		ui.TextInput(c, &s.trailerConfirm).Placeholder(fmt.Sprintf("%d", intent.Block)).AutoFocus()
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.trailerDialogOpen = false
				s.trailerIntent = nil
			}
			if ui.Button(c, l.ClassicTrailerWrite()).Disabled(s.writeBusy() || s.trailerConfirm != fmt.Sprintf("%d", intent.Block)).Clicked() {
				s.trailerDialogOpen = false
				s.trailerIntent = nil
				s.startClassicWrite(intent)
			}
		})
	})
}

func (s *shell) requestClonePreflight() {
	if s.writeBusy() || !s.workbench.Loaded || !s.workbench.Complete {
		return
	}
	s.pendingOps++
	go func() {
		plan, err := s.api.PreflightClone()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClonePreflightFailed(err.Error())
				return
			}
			if !plan.Valid {
				s.errorText = s.locales().ClonePreflightFailed(plan.Error)
				return
			}
			s.clonePlan = &plan
			s.cloneConfirm = ""
			s.cloneError = ""
			s.cloneOpen = true
			s.errorText = ""
		})
	}()
}

func (s *shell) cloneDialog(c *ui.Context) {
	if !s.cloneOpen || s.clonePlan == nil {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	plan := *s.clonePlan
	ui.Modal(c, &s.cloneOpen, func() {
		ui.Text(c, l.CloneTitle()).FontSize(15).Bold().TextColor(p.text)
		ui.Text(c, l.CloneSource(plan.Source.UID)).FontSize(10.5).Wrap()
		ui.Text(c, l.CloneTarget(plan.Target.UID)).FontSize(10.5).Wrap()
		if plan.Block0Write {
			ui.Text(c, l.CloneBlock0Write()).FontSize(10).TextColor(p.warning).Wrap()
		} else {
			ui.Text(c, l.CloneBlock0Match()).FontSize(10).TextColor(p.muted).Wrap()
		}
		ui.Text(c, l.ClonePlan(len(plan.Blocks), len(plan.TrailerBlocks))).FontSize(10).Wrap()
		if plan.BackupPath != "" {
			ui.Text(c, l.ClassicBackupPath(plan.BackupPath)).FontSize(9.5).Wrap()
		}
		ui.Text(c, l.CloneWarning()).FontSize(9.5).TextColor(p.warning).Wrap()
		ui.Text(c, l.CloneConfirmHint(plan.Source.UID)).FontSize(10).Bold()
		ui.TextInput(c, &s.cloneConfirm).Placeholder(plan.Source.UID).AutoFocus()
		if s.cloneError != "" {
			ui.Text(c, s.cloneError).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.cloneOpen = false
				s.clonePlan = nil
			}
			if ui.Button(c, l.CloneConfirmAction()).Disabled(s.writeBusy() || normalizeClassicHex(s.cloneConfirm) != normalizeClassicHex(plan.Source.UID)).Clicked() {
				s.startClone(plan)
			}
		})
	})
}

func (s *shell) startClone(plan app.ClonePreflightDTO) {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		_, err := s.api.StartClone(app.CloneStartRequestDTO{Token: plan.Token, Confirmation: normalizeClassicHex(s.cloneConfirm)})
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.cloneError = s.locales().CloneStartFailed(err.Error())
				return
			}
			s.cloneOpen = false
			s.clonePlan = nil
			s.cloneError = ""
			s.toastText = s.locales().CloneStarted()
		})
	}()
}

func (s *shell) requestFactoryPreflight(keepData bool) {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		plan, err := s.api.PreflightFactoryFormat(keepData)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().FactoryPreflightFailed(err.Error())
				return
			}
			if !plan.Valid {
				s.factoryError = s.locales().FactoryPreflightFailed(plan.Error)
				s.factoryPlan = nil
				s.factoryOpen = true
				return
			}
			s.factoryPlan = &plan
			s.factoryKeepData = keepData
			s.factoryConfirm = ""
			s.factoryError = ""
			s.factoryOpen = true
			s.errorText = ""
		})
	}()
}

func (s *shell) factoryDialog(c *ui.Context) {
	if !s.factoryOpen {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	plan := s.factoryPlan
	ui.Modal(c, &s.factoryOpen, func() {
		ui.Text(c, l.FactoryTitle()).FontSize(15).Bold().TextColor(p.danger)
		ui.Text(c, l.FactoryHint()).FontSize(10.5).Wrap()
		ui.Checkbox(c, &s.factoryKeepData, l.FactoryKeepData())
		if plan != nil {
			ui.Text(c, l.FactoryPlan(len(plan.Blocks), len(plan.TrailerBlocks))).FontSize(10).Wrap()
			ui.Text(c, l.FactoryBlocks(blockRangeSummary(plan.Blocks), blockRangeSummary(plan.TrailerBlocks))).FontSize(9.5).Wrap()
			ui.Text(c, l.FactoryTrailerBytes(plan.TrailerBytes)).Font("monospace").FontSize(9).Wrap()
			if plan.BackupPath != "" {
				ui.Text(c, l.ClassicBackupPath(plan.BackupPath)).FontSize(9.5).Wrap()
			}
			ui.Text(c, l.FactoryConfirmHint()).FontSize(10).Bold()
			ui.TextInput(c, &s.factoryConfirm).Placeholder("FORMAT").AutoFocus()
		}
		if s.factoryError != "" {
			ui.Text(c, s.factoryError).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.factoryOpen = false
				s.factoryPlan = nil
			}
			if ui.Button(c, l.FactoryUpdatePlan()).Disabled(s.pendingOps > 0).Clicked() {
				s.factoryOpen = false
				s.requestFactoryPreflight(s.factoryKeepData)
			}
			if plan != nil && ui.Button(c, l.FactoryConfirmAction()).Disabled(s.writeBusy() || s.factoryKeepData != plan.KeepData || !strings.EqualFold(strings.TrimSpace(s.factoryConfirm), "FORMAT")).Clicked() {
				s.startFactory(*plan)
			}
		})
	})
}

func (s *shell) startFactory(plan app.FactoryPreflightDTO) {
	if s.writeBusy() {
		return
	}
	s.pendingOps++
	go func() {
		_, err := s.api.StartFactoryFormat(app.FactoryStartRequestDTO{Token: plan.Token, Confirmation: "FORMAT"})
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.factoryError = s.locales().FactoryStartFailed(err.Error())
				return
			}
			s.factoryOpen = false
			s.factoryPlan = nil
			s.factoryError = ""
			s.toastText = s.locales().FactoryStarted()
		})
	}()
}

func (s *shell) openKeyFileEditor() {
	if s.writeBusy() {
		return
	}
	s.keyFilePath = ""
	s.keyFileLines = nil
	s.keyFilePreview = app.KeyFilePreviewDTO{}
	s.keyFileError = ""
	s.keyFileSetName = ""
	s.keyFileDirty = false
	s.keyFileOpen = true
}

func (s *shell) loadKeyFileEditor() {
	if s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		content, err := s.api.OpenKeyFileEditor()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.keyFileError = s.locales().KeysEditorFailed(err.Error())
				return
			}
			if content.Path == "" && content.Content == "" {
				return
			}
			s.keyFilePath = content.Path
			s.keyFileLines = strings.Split(strings.TrimSuffix(content.Content, "\n"), "\n")
			s.keyFileDirty = true
			s.validateKeyFile()
			s.keyFileError = ""
		})
	}()
}

func (s *shell) validateKeyFile() {
	content := strings.Join(s.keyFileLines, "\n")
	preview := s.api.PreviewKeyFile(content)
	s.keyFilePreview = preview
	s.keyFileDirty = false
}

func (s *shell) saveKeyFileEditor() {
	if s.pendingOps > 0 {
		return
	}
	s.validateKeyFile()
	content := strings.Join(s.keyFileLines, "\n")
	preview := s.api.PreviewKeyFile(content)
	if len(preview.Issues) != 0 {
		s.keyFileError = s.locales().KeysEditorInvalid(len(preview.Issues))
		return
	}
	s.pendingOps++
	go func() {
		result, err := s.api.SaveKeyFileEditor(content)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.keyFileError = s.locales().KeysEditorFailed(err.Error())
				return
			}
			s.keyFilePath = result.Path
			s.keyFileError = ""
			s.toastText = s.locales().KeysEditorSaved(result.Path, result.Count)
		})
	}()
}

func (s *shell) applyKeyFileEditor() {
	if s.pendingOps > 0 {
		return
	}
	s.validateKeyFile()
	content := strings.Join(s.keyFileLines, "\n")
	s.pendingOps++
	go func() {
		report, err := s.api.ImportKeyTextWithSet(content, s.keyFileSetName)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.keyFileError = s.locales().KeysEditorFailed(err.Error())
				return
			}
			s.reloadKeySnapshot()
			if report.KeySet != nil {
				s.keyFileError = ""
				s.toastText = s.locales().KeysImportedSet(report.Added, report.KeySet.Name, report.KeySet.KeyCount)
			} else {
				s.keyFileError = ""
				s.toastText = s.locales().KeysImported(report.Added, report.Duplicates, report.Ignored)
			}
		})
	}()
}

func (s *shell) keyFileEditorDialog(c *ui.Context) {
	if !s.keyFileOpen {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	if s.keyFileDirty {
		s.validateKeyFile()
	}
	ui.Modal(c, &s.keyFileOpen, func() {
		ui.Text(c, l.KeysEditorTitle()).FontSize(15).Bold().TextColor(p.text)
		name := s.keyFilePath
		if name == "" {
			name = l.KeysEditorUnsaved()
		}
		ui.Text(c, name).FontSize(10).TextColor(p.faint).SingleLine()
		ui.Text(c, l.KeysEditorSummary(s.keyFilePreview.Valid, s.keyFilePreview.Duplicates, s.keyFilePreview.Ignored)).
			FontSize(10).TextColor(p.muted)
		ui.Row(c).Gap(8).Children(func() {
			if ui.Button(c, l.KeysEditorOpen()).Height(26).FontSize(10).Disabled(s.pendingOps > 0).Clicked() {
				s.loadKeyFileEditor()
			}
			if ui.Button(c, l.KeysEditorSave()).Height(26).FontSize(10).Disabled(s.pendingOps > 0 || len(s.keyFileLines) == 0).Clicked() {
				s.saveKeyFileEditor()
			}
			if ui.Button(c, l.KeysEditorAddLine()).Height(26).FontSize(10).Clicked() {
				s.keyFileLines = append(s.keyFileLines, "")
				s.keyFileDirty = true
			}
		})
		if len(s.keyFileLines) == 0 {
			s.emptyPanelBody(c, p, l.KeysEditorEmpty())
			return
		}
		columns := []ui.TableColumn{
			{Title: l.KeysEditorLine(), ID: "line", Width: 48, Align: ui.End, Fixed: true},
			{Title: l.KeysEditorKey(), ID: "key"},
			{Title: l.KeysEditorState(), ID: "state", Width: 150, Fixed: true},
		}
		s.keyFileList.Selected = nil
		s.keyFileList.Key = func(index int) any { return index }
		s.keyFileList.Label = func(index int) string { return fmt.Sprintf("%d", index+1) }
		ui.Table(c, &s.keyFileList, columns, len(s.keyFileLines), func(row, col int) {
			if row < 0 || row >= len(s.keyFileLines) {
				return
			}
			line := s.keyFileLinePreview(row)
			switch col {
			case 0:
				ui.Textf(c, "%d", row+1).Font("monospace").FontSize(9.5).TextColor(p.faint)
			case 1:
				input := ui.TextInput(c, &s.keyFileLines[row]).Font("monospace").FontSize(9.5)
				if input.Changed() {
					s.keyFileDirty = true
				}
			case 2:
				switch {
				case line.Problem != "":
					ui.Text(c, line.Problem).FontSize(9).TextColor(p.danger).SingleLine()
				case line.Duplicate:
					ui.Text(c, l.KeysEditorDuplicate()).FontSize(9).TextColor(p.warning).SingleLine()
				case line.Comment:
					ui.Text(c, l.KeysEditorComment()).FontSize(9).TextColor(p.faint).SingleLine()
				case line.Key != "":
					ui.Text(c, l.KeysEditorValid()).FontSize(9).TextColor(p.success).SingleLine()
				default:
					ui.Text(c, l.KeysEditorIgnored()).FontSize(9).TextColor(p.faint).SingleLine()
				}
			}
		}).Grow(1).MinHeight(0)
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.TextInput(c, &s.keyFileSetName).Placeholder(l.KeySetNamePlaceholder()).Label(l.KeysEditorSetName()).Width(180)
			if ui.Button(c, l.KeysEditorApply()).Height(28).FontSize(10).Disabled(s.pendingOps > 0).Clicked() {
				s.applyKeyFileEditor()
			}
		})
		if s.keyFileError != "" {
			ui.Text(c, s.keyFileError).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonClose()).Clicked() {
				s.keyFileOpen = false
			}
		})
	})
}

func (s *shell) keyFileLinePreview(row int) app.KeyFileLineDTO {
	for _, line := range s.keyFilePreview.Lines {
		if line.Number == row+1 {
			return line
		}
	}
	return app.KeyFileLineDTO{Number: row + 1}
}

func (s *shell) partialReadBanner(c *ui.Context, p palette) {
	if !s.workbench.Loaded || s.workbench.Complete || s.cardFamily() != familyClassic {
		return
	}
	l := s.locales()
	ui.Row(c).Padding(8, 10).Gap(10).AlignItems(ui.Center).Radius(8).
		Background(p.warningSoft).Border(1, p.warningBorder).Children(func() {
		ui.Icon(c, warningIcon).FontSize(13).TextColor(p.warning)
		text := l.ClassicReadableSummary(s.workbench.ReadableBlocks, s.workbench.TotalBlocks, len(s.workbench.SkippedSectors))
		if len(s.workbench.SkippedSectors) != 0 {
			text += " · " + l.ClassicSkippedSectors(formatSkippedSectors(s.workbench.SkippedSectors))
		}
		ui.Text(c, text).FontSize(10.5).TextColor(p.warning).Grow(1).Wrap()
		if ui.Button(c, l.ClassicReadAgain()).Height(26).FontSize(10).
			Disabled(s.writeBusy() || !s.cardPresent()).Clicked() {
			s.requestReadAgain()
		}
	})
}

func (s *shell) requestReadAgain() {
	if s.writeBusy() {
		return
	}
	if s.workbench.Dirty {
		s.openClassicDialog("reread", nil)
		return
	}
	s.startReadAgain()
}

func (s *shell) startReadAgain() {
	if s.writeBusy() || !s.cardPresent() {
		return
	}
	s.pendingOps++
	go func() {
		_, err := s.api.StartReadCard(app.ReadCardStartRequestDTO{DiscardDirty: true, ScanKeys: true})
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().CardReadFailed(err.Error())
				return
			}
			s.errorText = ""
		})
	}()
}

func (s *shell) manufacturerBlockPanel(c *ui.Context, p palette, block app.WorkbenchBlockDTO) {
	l := s.locales()
	ui.Row(c).Padding(0, 14, 8).Gap(10).AlignItems(ui.Center).Children(func() {
		ui.Text(c, l.ClassicBCC()).FontSize(9.5).TextColor(p.faint).Shrink(0)
		if block.BCC != nil && block.BCC.Applicable {
			status := l.ClassicBCCInvalid()
			color := p.danger
			if block.BCC.Valid {
				status = l.ClassicBCCValid()
				color = p.success
			}
			ui.Text(c, l.ClassicBCCResult(block.BCC.Expected, block.BCC.Actual, status)).
				Font("monospace").FontSize(9.5).TextColor(color)
		} else {
			ui.Text(c, l.CommonNotAvailable()).FontSize(9.5).TextColor(p.faint)
		}
		ui.Spacer(c)
		ui.Text(c, l.BlockWarningManufacturer()).FontSize(9).TextColor(p.warning).Wrap()
	})
}

func (s *shell) coverageColor(p palette, readable, total int) ui.Color {
	if total == 0 || readable == total {
		return p.faint
	}
	if readable == 0 {
		return p.danger
	}
	return p.warning
}

// formatSkippedSectors writes explicit sector numbers and only compresses
// consecutive runs, so [3, 7] is "3, 7", never the misleading "3-7".
func formatSkippedSectors(sectors []int) string {
	if len(sectors) == 0 {
		return ""
	}
	sorted := append([]int(nil), sectors...)
	sort.Ints(sorted)
	parts := make([]string, 0, len(sorted))
	start, previous := sorted[0], sorted[0]
	flush := func() {
		if start == previous {
			parts = append(parts, fmt.Sprintf("%d", start))
			return
		}
		parts = append(parts, fmt.Sprintf("%d-%d", start, previous))
	}
	for _, sector := range sorted[1:] {
		if sector == previous {
			continue
		}
		if sector == previous+1 {
			previous = sector
			continue
		}
		flush()
		start, previous = sector, sector
	}
	flush()
	return strings.Join(parts, ", ")
}
