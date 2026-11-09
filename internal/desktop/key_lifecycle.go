package desktop

import (
	"fmt"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func (s *shell) openRecoveryReview(review *app.RecoveryReviewDTO) {
	if review == nil {
		s.recoveryReview = nil
		s.recoveryReviewOpen = false
		return
	}
	s.recoveryReview = review
	s.recoveryReviewOpen = true
	s.errorText = ""
	s.recoverySetName = review.DefaultKeySetName
	s.recoveryKeyNames = make(map[string]*string, len(review.Recovered))
	s.recoverySelected = make(map[string]bool, len(review.Recovered))
	for _, ref := range review.Recovered {
		name := ""
		s.recoveryKeyNames[ref.KeyID] = &name
		s.recoverySelected[ref.KeyID] = true
	}
}

func (s *shell) recoveryKeyName(id string) *string {
	name, ok := s.recoveryKeyNames[id]
	if !ok || name == nil {
		value := ""
		name = &value
		s.recoveryKeyNames[id] = name
	}
	return name
}

func (s *shell) keyByID(id string) (app.KeyDTO, bool) {
	for _, key := range s.keys {
		if key.ID == id {
			return key, true
		}
	}
	return app.KeyDTO{}, false
}

func (s *shell) keySetByID(id string) (app.KeySetDTO, bool) {
	for _, set := range s.keySets {
		if set.ID == id {
			return set, true
		}
	}
	return app.KeySetDTO{}, false
}

func (s *shell) reloadKeySnapshot() {
	s.keys = append([]app.KeyDTO(nil), s.api.GetKeyCatalog()...)
	s.keySets = append([]app.KeySetDTO(nil), s.api.GetKeySets()...)
	s.syncKeyChoice()
}

func (s *shell) reloadKeyStore() {
	if s.pendingOps > 0 || s.activeTask != "" {
		return
	}
	s.pendingOps++
	go func() {
		result, err := s.api.ReloadKeyStore()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().KeysReloadFailed(err.Error())
				return
			}
			s.reloadKeySnapshot()
			s.errorText = ""
			s.toastText = s.locales().KeysReloaded(result.Keys, result.Verifications, result.KeySets)
		})
	}()
}

func (s *shell) openKeyRename(key app.KeyDTO) {
	s.keyRenameID = key.ID
	s.keyRenameInput = key.Name
	s.errorText = ""
	s.keyRenameOpen = true
}

func (s *shell) saveKeyRename() {
	id := s.keyRenameID
	if id == "" || s.pendingOps > 0 {
		return
	}
	name := strings.TrimSpace(s.keyRenameInput)
	s.pendingOps++
	go func() {
		keys, err := s.api.RenameKey(id, name)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().KeysRenameFailed(err.Error())
				return
			}
			s.keyRenameOpen = false
			s.keyRenameID = ""
			s.keyRenameInput = ""
			s.keys = append([]app.KeyDTO(nil), keys...)
			s.errorText = ""
			s.syncKeyChoice()
		})
	}()
}

func (s *shell) createVerifiedKeySet() {
	name := strings.TrimSpace(s.keySetNameInput)
	if name == "" || s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		sets, err := s.api.CreateVerifiedKeySet(name)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.keySetError = s.locales().KeySetActionFailed(err.Error())
				return
			}
			s.keySetCreateOpen = false
			s.keySetNameInput = ""
			s.keySetError = ""
			s.keySets = append([]app.KeySetDTO(nil), sets...)
			s.reloadKeySnapshot()
		})
	}()
}

func (s *shell) saveKeySetRename() {
	id := s.keySetRenameID
	name := strings.TrimSpace(s.keySetRenameInput)
	if id == "" || name == "" || s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		sets, err := s.api.RenameKeySet(id, name)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.keySetError = s.locales().KeySetActionFailed(err.Error())
				return
			}
			s.keySetRenameOpen = false
			s.keySetRenameID = ""
			s.keySetRenameInput = ""
			s.keySetError = ""
			s.keySets = append([]app.KeySetDTO(nil), sets...)
		})
	}()
}

func (s *shell) deleteKeySet(id string) {
	if id == "" || s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		sets, err := s.api.DeleteKeySet(id)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.keySetError = s.locales().KeySetActionFailed(err.Error())
				return
			}
			s.keySetError = ""
			s.keySets = append([]app.KeySetDTO(nil), sets...)
		})
	}()
}

func (s *shell) exportKeySet(id, format string) {
	if id == "" || s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		result, err := s.api.ExportKeySet(id, format)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.keySetError = s.locales().KeySetExportFailed(err.Error())
				return
			}
			if result.Cancelled {
				return
			}
			s.keySetError = ""
			s.toastText = s.locales().KeySetExported(result.Count, result.Path)
		})
	}()
}

func (s *shell) saveRecoveryReview() {
	if s.recoveryReview == nil || s.pendingOps > 0 {
		return
	}
	setName := strings.TrimSpace(s.recoverySetName)
	if setName == "" {
		return
	}
	selected := make([]string, 0, len(s.recoverySelected))
	names := make(map[string]string, len(s.recoveryKeyNames))
	for _, ref := range s.recoveryReview.Recovered {
		if !s.recoverySelected[ref.KeyID] {
			continue
		}
		selected = append(selected, ref.KeyID)
		if name := strings.TrimSpace(*s.recoveryKeyName(ref.KeyID)); name != "" {
			names[ref.KeyID] = name
		}
	}
	if len(selected) == 0 {
		return
	}
	request := app.RecoverySaveRequestDTO{KeySetName: setName, KeyNames: names, SelectedKeyIDs: selected}
	s.pendingOps++
	go func() {
		_, err := s.api.SaveRecoveryReview(request)
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().RecoverySaveFailed(err.Error())
				return
			}
			s.errorText = ""
			s.recoveryReviewOpen = false
			s.toastText = s.locales().RecoveryReviewSaveStarted()
		})
	}()
}

func (s *shell) discardRecoveryReview() {
	if s.recoveryReview == nil || s.pendingOps > 0 {
		return
	}
	s.pendingOps++
	go func() {
		err := s.api.DiscardRecoveryReview()
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = s.locales().RecoverySaveFailed(err.Error())
				return
			}
			s.recoveryReview = nil
			s.recoveryReviewOpen = false
			s.recoverySelected = make(map[string]bool)
			s.recoveryKeyNames = make(map[string]*string)
			s.reloadKeySnapshot()
			s.errorText = ""
			s.toastText = s.locales().RecoveryReviewDiscarded()
		})
	}()
}

func (s *shell) keyLifecycleDialogs(c *ui.Context) {
	s.keyRenameDialog(c)
	s.keyImportDialog(c)
	s.keySetCreateDialog(c)
	s.keySetRenameDialog(c)
	s.keySetDeleteDialog(c)
	s.keySetExportDialog(c)
	s.recoveryReviewDialog(c)
}

func (s *shell) keyImportDialog(c *ui.Context) {
	if !s.keyImportOpen {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	ui.Modal(c, &s.keyImportOpen, func() {
		ui.Text(c, l.KeysImportTitle()).FontSize(15).Bold().TextColor(p.text)
		ui.Text(c, l.KeysImportSetHint()).FontSize(10.5).TextColor(p.muted).Wrap()
		ui.Text(c, l.KeysImportFormat()).FontSize(10).TextColor(p.muted)
		formatLabels := []string{l.KeysImportFormatDictionary(), l.KeysImportFormatKeyArray()}
		formatIDs := []string{"dic", "keyarray"}
		selectedFormat := formatLabels[0]
		for index, id := range formatIDs {
			if id == s.keyImportFormat {
				selectedFormat = formatLabels[index]
			}
		}
		formatSelector := ui.Select(c, &selectedFormat, formatLabels)
		if formatSelector.Changed() {
			for index, label := range formatLabels {
				if label == selectedFormat {
					s.keyImportFormat = formatIDs[index]
					break
				}
			}
		}
		ui.Text(c, l.KeysImportSetLabel()).FontSize(10).TextColor(p.muted)
		input := ui.TextInput(c, &s.keyImportSetName).Placeholder(l.KeySetNamePlaceholder()).AutoFocus()
		if input.Submitted() {
			s.startImportKeys(s.keyImportSetName)
		}
		if s.keyImportError != "" {
			ui.Text(c, s.keyImportError).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.keyImportOpen = false
				s.keyImportError = ""
			}
			if ui.Button(c, l.KeysImportAction()).Disabled(s.pendingOps > 0).Clicked() {
				s.startImportKeys(s.keyImportSetName)
			}
		})
	})
}

func (s *shell) keyRenameDialog(c *ui.Context) {
	if !s.keyRenameOpen {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	ui.Modal(c, &s.keyRenameOpen, func() {
		ui.Text(c, l.KeysRenameTitle()).FontSize(15).Bold().TextColor(p.text)
		ui.TextInput(c, &s.keyRenameInput).Placeholder(l.KeysRenamePlaceholder()).AutoFocus()
		if s.errorText != "" {
			ui.Text(c, s.errorText).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.keyRenameOpen = false
			}
			if ui.Button(c, l.CommonSave()).Disabled(s.pendingOps > 0).Clicked() {
				s.saveKeyRename()
			}
		})
	})
}

func (s *shell) keySetCreateDialog(c *ui.Context) {
	if !s.keySetCreateOpen {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	ui.Modal(c, &s.keySetCreateOpen, func() {
		ui.Text(c, l.KeySetCreateTitle()).FontSize(15).Bold().TextColor(p.text)
		ui.Text(c, l.KeySetCreateHint()).FontSize(10.5).TextColor(p.muted).Wrap()
		ui.TextInput(c, &s.keySetNameInput).Placeholder(l.KeySetNamePlaceholder()).AutoFocus()
		if s.keySetError != "" {
			ui.Text(c, s.keySetError).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.keySetCreateOpen = false
				s.keySetError = ""
			}
			if ui.Button(c, l.CommonSave()).
				Disabled(s.pendingOps > 0 || strings.TrimSpace(s.keySetNameInput) == "").Clicked() {
				s.createVerifiedKeySet()
			}
		})
	})
}

func (s *shell) keySetRenameDialog(c *ui.Context) {
	if !s.keySetRenameOpen {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	ui.Modal(c, &s.keySetRenameOpen, func() {
		ui.Text(c, l.KeySetRenameTitle()).FontSize(15).Bold().TextColor(p.text)
		ui.TextInput(c, &s.keySetRenameInput).Placeholder(l.KeySetNamePlaceholder()).AutoFocus()
		if s.keySetError != "" {
			ui.Text(c, s.keySetError).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.keySetRenameOpen = false
				s.keySetError = ""
			}
			if ui.Button(c, l.CommonSave()).
				Disabled(s.pendingOps > 0 || strings.TrimSpace(s.keySetRenameInput) == "").Clicked() {
				s.saveKeySetRename()
			}
		})
	})
}

func (s *shell) keySetDeleteDialog(c *ui.Context) {
	if !s.keySetDeleteOpen {
		return
	}
	l := s.locales()
	set, _ := s.keySetByID(s.keySetDeleteID)
	message := l.KeySetDeleteMessage(set.Name)
	result := ui.AlertDialog(c, &s.keySetDeleteOpen, l.KeySetDeleteTitle(), message, l.CommonCancel(), l.KeySetDelete())
	if result == 1 {
		s.deleteKeySet(s.keySetDeleteID)
	}
}

func (s *shell) keySetExportDialog(c *ui.Context) {
	if !s.keySetExportOpen {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	set, _ := s.keySetByID(s.keySetExportID)
	ui.Modal(c, &s.keySetExportOpen, func() {
		ui.Text(c, l.KeySetExportTitle(set.Name)).FontSize(15).Bold().TextColor(p.text)
		ui.Text(c, l.KeySetExportFormat()).FontSize(10.5).TextColor(p.muted)
		labels := []string{l.KeySetFormatDIC(), l.KeySetFormatKeys(), l.KeySetFormatJSON()}
		ids := []string{"dic", "keys", "json"}
		selected := labels[0]
		for index, id := range ids {
			if id == s.keySetExportFormat {
				selected = labels[index]
			}
		}
		selector := ui.Select(c, &selected, labels)
		if selector.Changed() {
			for index, label := range labels {
				if label == selected {
					s.keySetExportFormat = ids[index]
					break
				}
			}
		}
		if s.keySetError != "" {
			ui.Text(c, s.keySetError).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.CommonCancel()).Clicked() {
				s.keySetExportOpen = false
				s.keySetError = ""
			}
			if ui.Button(c, l.CommonExport()).Disabled(s.pendingOps > 0).Clicked() {
				id := s.keySetExportID
				s.keySetExportOpen = false
				s.exportKeySet(id, s.keySetExportFormat)
			}
		})
	})
}

func (s *shell) recoveryReviewDialog(c *ui.Context) {
	if !s.recoveryReviewOpen || s.recoveryReview == nil {
		return
	}
	l := s.locales()
	p := paletteFor(c.Theme())
	review := s.recoveryReview
	ui.Modal(c, &s.recoveryReviewOpen, func() {
		ui.Text(c, l.RecoveryReviewTitle()).FontSize(15).Bold().TextColor(p.text)
		ui.Text(c, l.RecoveryReviewIntro()).FontSize(10).TextColor(p.muted).Wrap()
		ui.Text(c, l.RecoveryReviewKeySetName()).FontSize(10).TextColor(p.muted)
		ui.TextInput(c, &s.recoverySetName).Placeholder(l.KeySetNamePlaceholder())
		ui.Scroll(c).Height(360).Children(func() {
			for _, ref := range review.Recovered {
				selected := s.recoverySelected[ref.KeyID]
				row := ui.Row(c).Gap(8).AlignItems(ui.Center)
				row.Children(func() {
					ui.Checkbox(c, &selected, "")
					s.recoverySelected[ref.KeyID] = selected
					ui.Text(c, fmt.Sprintf("%s %d · Key %s", l.ClassicSector(), ref.Sector, ref.KeyType)).
						FontSize(10).TextColor(p.muted)
					value := ""
					if key, ok := s.keyByID(ref.KeyID); ok {
						value = key.Value
					}
					ui.Text(c, value).Font("monospace").FontSize(10).TextColor(p.text).SingleLine()
					label, fg, bg, border := l.RecoveryReviewStatusExisting(), p.muted, p.surfaceAlt, p.border
					switch ref.Status {
					case "new":
						label, fg, bg, border = l.RecoveryReviewStatusNew(), p.success, p.successSoft, p.successBorder
					case "replaced":
						label, fg, bg, border = l.RecoveryReviewStatusReplaced(), p.warning, p.warningSoft, p.warningBorder
					}
					pill(c, label, fg, bg, border)
					ui.TextInput(c, s.recoveryKeyName(ref.KeyID)).
						Label(l.RecoveryReviewKeyName()).Placeholder(l.RecoveryReviewKeyNamePlaceholder()).Width(170)
				})
			}
		})
		if s.errorText != "" {
			ui.Text(c, s.errorText).FontSize(10).TextColor(p.danger).Wrap()
		}
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.Button(c, l.RecoveryReviewDiscard()).Disabled(s.pendingOps > 0).Clicked() {
				s.discardRecoveryReview()
			}
			if ui.Button(c, l.RecoveryReviewSave()).
				Disabled(s.pendingOps > 0 || strings.TrimSpace(s.recoverySetName) == "").Clicked() {
				s.saveRecoveryReview()
			}
		})
	})
}
