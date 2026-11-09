package desktop

import (
	"encoding/hex"
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func (s *shell) classicActions(c *ui.Context, p palette) {
	l := s.locales()
	busy := s.writeBusy()
	ui.Row(c).Padding(0, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
		if linkButton(c, p, l.ClassicLoadDump()).Disabled(busy).Clicked() {
			s.loadDump()
		}
		if s.workbench.Loaded {
			choices := dumpFormatChoices(l)
			if !s.workbench.Complete {
				choices = choices[:1]
			}
			labels := make([]string, len(choices))
			selected := labels[0]
			for index, choice := range choices {
				labels[index] = choice.Label
				if choice.ID == s.dumpFormat {
					selected = choice.Label
				}
			}
			ui.Text(c, l.ClassicDumpFormat()).FontSize(9.5).TextColor(p.faint).Shrink(0)
			selector := ui.Select(c, &selected, labels)
			if selector.Changed() {
				for index, label := range labels {
					if label == selected {
						s.dumpFormat = choices[index].ID
						break
					}
				}
			}
		}
		save := linkButton(c, p, l.ClassicSaveDump()).Disabled(busy || !s.workbench.Loaded)
		if save.Clicked() {
			s.saveDump()
		}
	})
	if !s.advanced {
		return
	}
	ui.Row(c).Padding(0, 14, 10).Gap(8).AlignItems(ui.Center).Children(func() {
		changed := linkButton(c, p, l.ClassicWriteChanged()).
			Disabled(busy || !s.hasWritableChanges())
		if changed.Clicked() {
			s.requestChangedWrite()
		}
		full := linkButton(c, p, l.ClassicFullRestore()).
			Disabled(busy || !s.workbench.Loaded || !s.workbench.Complete)
		if full.Clicked() {
			s.requestFullRestore()
		}
		clone := linkButton(c, p, l.CloneAction()).
			Disabled(busy || !s.workbench.Loaded || !s.workbench.Complete || !s.cardPresent())
		if clone.Clicked() {
			s.requestClonePreflight()
		}
		factory := linkButton(c, p, l.FactoryAction()).
			Disabled(busy || !s.cardPresent() || s.cardFamily() != familyClassic)
		if factory.Clicked() {
			s.requestFactoryPreflight(s.factoryKeepData)
		}
	})
}

func (s *shell) classicBlockEditor(c *ui.Context, p palette) {
	l := s.locales()
	block, ok := s.selectedBlock()
	if !ok {
		ui.Row(c).Padding(0, 14, 8).Children(func() {
			ui.Text(c, l.ClassicSelectDataBlock()).FontSize(9.5).TextColor(p.faint).Wrap()
		})
		return
	}
	s.syncBlockEditor(false)
	block, _ = s.selectedBlock()
	if block.Kind == "manufacturer" {
		s.manufacturerBlockPanel(c, p, block)
		return
	}
	if block.Kind != "data" && block.Kind != "trailer" {
		ui.Row(c).Padding(0, 14, 8).Children(func() {
			ui.Text(c, l.ClassicSelectDataBlock()).FontSize(9.5).TextColor(p.faint).Wrap()
		})
		return
	}
	if block.Kind == "trailer" {
		s.trailerEditor(c, p, block)
		return
	}
	s.valueBlockEditor(c, p, block)
	ui.Row(c).Padding(0, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
		ui.Textf(c, "%s %d", l.ClassicBlock(), block.Block).Font("monospace").FontSize(9.5).
			TextColor(p.faint).Shrink(0)
		input := ui.TextInput(c, &s.blockInput).Placeholder(l.ClassicBlockHexPlaceholder()).
			Label(l.ClassicBlockEditor()).Width(380)
		if input.Submitted() {
			s.requestBlockWrite()
		}
		if ui.Button(c, l.ClassicWriteBlock()).Height(28).FontSize(10).
			Disabled(s.writeBusy() || strings.TrimSpace(s.blockInput) == "").Clicked() {
			s.requestBlockWrite()
		}
	})
}

func (s *shell) classicUIDPanel(c *ui.Context, p palette) {
	if !s.advanced {
		return
	}
	l := s.locales()
	ui.Row(c).Padding(0, 14, 8).Gap(8).AlignItems(ui.Center).Children(func() {
		ui.Text(c, l.ClassicUIDInput()).FontSize(9.5).TextColor(p.faint).Shrink(0)
		input := ui.TextInput(c, &s.uidInput).Placeholder(l.ClassicUIDPlaceholder()).
			Label(l.ClassicUIDInput()).Width(180)
		if input.Submitted() {
			s.requestUIDWrite()
		}
		if ui.Button(c, l.ClassicUIDSet()).Height(28).FontSize(10).
			Disabled(s.writeBusy() || strings.TrimSpace(s.uidInput) == "").Clicked() {
			s.requestUIDWrite()
		}
	})
}

func (s *shell) writeBusy() bool {
	return s.activeTask != "" || s.pendingOps > 0
}

func (s *shell) toggleAdvanced() {
	if s.writeBusy() {
		s.toastText = s.locales().DeviceBlocked()
		return
	}
	if s.advanced {
		s.resetAdvanced()
		return
	}
	s.openClassicDialog("advanced", nil)
}

func (s *shell) loadDump() {
	if s.writeBusy() {
		return
	}
	if s.workbench.Dirty {
		s.openClassicDialog("discard", nil)
		return
	}
	s.startLoadDump(false)
}

func (s *shell) startLoadDump(discardDirty bool) {
	s.pendingOps++
	go func() {
		workbench, err := s.api.LoadDump(discardDirty)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicLoadFailed(err.Error())
				return
			}
			s.workbench = workbench
			s.errorText = ""
			s.clampSelections()
			for _, warning := range workbench.Warnings {
				s.appendLog("dump", app.TaskEventProgress, warning, 0)
			}
			if len(workbench.Warnings) > 0 {
				s.toastText = s.locales().ClassicLoadWarnings(len(workbench.Warnings))
			}
		})
	}()
}

func (s *shell) saveDump() {
	if s.writeBusy() || !s.workbench.Loaded {
		return
	}
	format := s.dumpFormat
	if format == "" {
		format = string(app.DumpFormatProject)
	}
	if format != string(app.DumpFormatProject) && !s.workbench.Complete {
		s.errorText = s.locales().ClassicIncompleteFormat()
		return
	}
	s.pendingOps++
	go func() {
		result, err := s.api.SaveDump(format)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicSaveFailed(err.Error())
				return
			}
			if result.Cancelled {
				return
			}
			s.errorText = ""
			s.toastText = s.locales().ClassicDumpSaved(result.Path)
		})
	}()
}

func (s *shell) requestBlockWrite() {
	block, ok := s.selectedBlock()
	if !ok || block.Kind != "data" {
		s.errorText = s.locales().ClassicSelectDataBlock()
		return
	}
	if s.writeBusy() {
		return
	}
	value := normalizeClassicHex(s.blockInput)
	decoded, err := hex.DecodeString(value)
	if err != nil || len(decoded) != 16 {
		s.errorText = s.locales().ClassicInvalidBlockHex()
		return
	}
	s.pendingOps++
	go func() {
		workbench, editErr := s.api.EditBlockHex(block.Block, value)
		var preflight app.WritePreflightDTO
		var preflightErr error
		if editErr == nil {
			preflight, preflightErr = s.api.PreflightWrite(app.WritePreflightRequestDTO{
				Mode: "block", Blocks: []int{block.Block},
			})
		}
		s.update(func() {
			s.pendingOps--
			if editErr != nil {
				s.errorText = s.locales().ClassicWriteFailed(editErr.Error())
				return
			}
			s.workbench = workbench
			s.clampSelections()
			if preflightErr != nil {
				s.errorText = s.locales().ClassicWriteFailed(preflightErr.Error())
				return
			}
			if !preflight.Valid {
				s.errorText = s.locales().ClassicPreflightFailed(preflight.Error)
				return
			}
			s.errorText = ""
			intent := &classicWriteIntent{
				Kind: "block", Block: block.Block, Hex: value, Preflight: preflight,
			}
			if s.valuePreview != nil && s.valuePreview.Block == block.Block {
				intent.Value = s.valuePreview
			}
			s.openClassicDialog("block", intent)
		})
	}()
}

func (s *shell) requestChangedWrite() {
	if s.writeBusy() || !s.hasWritableChanges() {
		return
	}
	s.pendingOps++
	go func() {
		preflight, err := s.api.PreflightWrite(app.WritePreflightRequestDTO{Mode: "changes"})
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicWriteFailed(err.Error())
				return
			}
			if !preflight.Valid {
				s.errorText = s.locales().ClassicPreflightFailed(preflight.Error)
				return
			}
			s.errorText = ""
			s.openClassicDialog("changes", &classicWriteIntent{Kind: "changes", Preflight: preflight})
		})
	}()
}

func (s *shell) requestFullRestore() {
	if s.writeBusy() || !s.workbench.Loaded || !s.workbench.Complete {
		return
	}
	s.pendingOps++
	go func() {
		preflight, err := s.api.PreflightWrite(app.WritePreflightRequestDTO{Mode: "full"})
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicWriteFailed(err.Error())
				return
			}
			if !preflight.Valid {
				s.errorText = s.locales().ClassicPreflightFailed(preflight.Error)
				return
			}
			s.errorText = ""
			s.openClassicDialog("full", &classicWriteIntent{Kind: "full", Preflight: preflight})
		})
	}()
}

func (s *shell) requestUIDWrite() {
	if s.writeBusy() {
		return
	}
	uid := normalizeClassicHex(s.uidInput)
	decoded, err := hex.DecodeString(uid)
	if err != nil || len(decoded) != 4 {
		s.errorText = s.locales().ClassicInvalidUID()
		return
	}
	s.pendingOps++
	go func() {
		preflight, preflightErr := s.api.PreflightUIDWrite(uid)
		s.update(func() {
			s.pendingOps--
			if preflightErr != nil {
				s.errorText = s.locales().ClassicWriteFailed(preflightErr.Error())
				return
			}
			if !preflight.Valid {
				s.errorText = s.locales().ClassicPreflightFailed(preflight.Error)
				return
			}
			s.errorText = ""
			s.openClassicDialog("uid", &classicWriteIntent{
				Kind: "uid", UID: uid, UIDPlan: preflight,
			})
		})
	}()
}

func (s *shell) openClassicDialog(kind string, intent *classicWriteIntent) {
	s.classicDialog = kind
	s.classicIntent = intent
	s.classicDialogOpen = true
}

func (s *shell) startClassicWrite(intent classicWriteIntent) {
	s.pendingOps++
	go func() {
		var err error
		if intent.Kind == "uid" {
			_, err = s.api.StartUIDWrite(app.UIDWriteStartRequestDTO{
				Token: intent.UIDPlan.Token, Confirmation: intent.UID,
			})
		} else {
			request := app.WriteStartRequestDTO{Token: intent.Preflight.Token}
			if intent.Kind == "full" || intent.Kind == "changes" && len(intent.Preflight.TrailerBlocks) != 0 {
				request.ConfirmTrailers = true
			}
			if intent.Kind == "trailer" {
				request.ConfirmTrailers = true
				request.Confirmation = fmt.Sprintf("%d", intent.Block)
			}
			_, err = s.api.StartWrite(request)
		}
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().ClassicWriteFailed(err.Error())
				return
			}
			s.errorText = ""
			if intent.Kind == "uid" {
				s.toastText = s.locales().ClassicUIDStarted()
			} else {
				s.toastText = s.locales().ClassicWriteStarted()
			}
		})
	}()
}

func (s *shell) classicDialogView(c *ui.Context) {
	if !s.classicDialogOpen || s.classicDialog == "" {
		return
	}
	l := s.locales()
	kind := s.classicDialog
	intent := s.classicIntent
	title, message, confirm := l.CommonCancel(), "", l.CommonCancel()
	switch kind {
	case "advanced":
		title, message, confirm = l.ClassicAdvancedTitle(), l.ClassicAdvancedMessage(), l.ClassicAdvancedEnable()
	case "discard":
		title, message, confirm = l.ClassicDiscardTitle(), l.ClassicDiscardMessage(), l.ClassicDiscardConfirm()
	case "reread":
		title, message, confirm = l.ClassicReadAgainTitle(), l.ClassicReadAgainMessage(), l.ClassicReadAgainConfirm()
	case "block":
		if intent == nil {
			s.classicDialogOpen = false
			return
		}
		title = l.ClassicConfirmBlockTitle(intent.Block)
		message = strings.Join([]string{
			l.ClassicNewValue(spacedHex(intent.Hex)),
			l.ClassicBlockWriteNote(),
		}, "\n")
		if intent.Value != nil {
			message = strings.Join([]string{
				l.ClassicValueDecoded(fmt.Sprintf("%d", intent.Value.Decoded.Value), fmt.Sprintf("%d", intent.Value.Decoded.Address)),
				l.ClassicNewValue(spacedHex(intent.Hex)),
				l.ClassicBlockWriteNote(),
			}, "\n")
		}
		confirm = l.ClassicConfirmBlock()
	case "changes":
		if intent == nil {
			s.classicDialogOpen = false
			return
		}
		title = l.ClassicConfirmChangedTitle(len(intent.Preflight.Blocks))
		message = s.classicPreflightMessage(intent.Preflight, false)
		confirm = l.ClassicConfirmChanged()
		if len(intent.Preflight.TrailerBlocks) != 0 {
			message += "\n\n" + l.ClassicTrailerWarning()
			confirm = l.ClassicConfirmTrailers()
		}
	case "full":
		if intent == nil {
			s.classicDialogOpen = false
			return
		}
		title = l.ClassicConfirmFullTitle()
		message = s.classicPreflightMessage(intent.Preflight, true)
		confirm = l.ClassicConfirmRestore()
	case "uid":
		if intent == nil || intent.UIDPlan.Preview == nil {
			s.classicDialogOpen = false
			return
		}
		preview := *intent.UIDPlan.Preview
		title = l.ClassicConfirmUIDTitle(spacedHex(intent.UID))
		lines := []string{
			l.ClassicUIDCurrent(preview.CurrentUID),
			l.ClassicUIDBCC(preview.BCC),
			l.ClassicUIDOldBlock0(preview.OldBlock0),
			l.ClassicUIDNewBlock0(preview.NewBlock0),
		}
		if intent.UIDPlan.BackupPath != "" {
			lines = append(lines, l.ClassicBackupPath(intent.UIDPlan.BackupPath))
		}
		lines = append(lines, l.ClassicUIDWarning())
		message = strings.Join(lines, "\n")
		confirm = l.ClassicConfirmUID()
	default:
		s.classicDialogOpen = false
		return
	}
	result := ui.AlertDialog(c, &s.classicDialogOpen, title, message, l.CommonCancel(), confirm)
	if result == -1 {
		return
	}
	s.classicDialogOpen = false
	s.classicDialog = ""
	s.classicIntent = nil
	if result != 1 {
		return
	}
	switch kind {
	case "advanced":
		s.advanced = true
	case "discard":
		s.startLoadDump(true)
	case "reread":
		s.startReadAgain()
	case "block", "changes", "full", "uid":
		if intent != nil {
			s.startClassicWrite(*intent)
		}
	}
}

func (s *shell) classicPreflightMessage(preflight app.WritePreflightDTO, full bool) string {
	l := s.locales()
	lines := []string{
		l.ClassicPlannedBlocks(blockRangeSummary(preflight.Blocks)),
		l.ClassicTrailers(blockRangeSummaryOrNone(preflight.TrailerBlocks, l.CommonNone())),
		l.ClassicSkippedBlock0(),
	}
	if len(preflight.Access) != 0 {
		blocks := make([]int, 0, len(preflight.Access))
		for _, access := range preflight.Access {
			blocks = append(blocks, access.Block)
		}
		lines = append(lines, l.ClassicAccessChanges(blockRangeSummary(blocks)))
	}
	if preflight.BackupPath != "" {
		lines = append(lines, l.ClassicBackupPath(preflight.BackupPath))
	}
	if full {
		lines = append(lines, l.ClassicFullRestoreWarning())
	}
	return strings.Join(lines, "\n")
}

func blockRangeSummary(values []int) string {
	if len(values) == 0 {
		return ""
	}
	sorted := append([]int(nil), values...)
	sort.Ints(sorted)
	parts := make([]string, 0, len(sorted))
	start, previous := sorted[0], sorted[0]
	flush := func() {
		if start == previous {
			parts = append(parts, strconv.Itoa(start))
			return
		}
		parts = append(parts, strconv.Itoa(start)+"-"+strconv.Itoa(previous))
	}
	for _, value := range sorted[1:] {
		if value == previous {
			continue
		}
		if value == previous+1 {
			previous = value
			continue
		}
		flush()
		start, previous = value, value
	}
	flush()
	return strings.Join(parts, ", ")
}

func blockRangeSummaryOrNone(values []int, none string) string {
	if len(values) == 0 {
		return none
	}
	return blockRangeSummary(values)
}
