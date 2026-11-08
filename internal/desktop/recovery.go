package desktop

import (
	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

type recoveryMode struct {
	ID       string
	Title    string
	Hint     string
	Status   *app.ExternalEngineStatusDTO
	Enabled  bool
	Disabled string
}

func (s *shell) recoveryView(c *ui.Context, p palette) {
	l := s.locales()
	if !s.cardPresent() {
		s.emptyPanelBody(c, p, l.RecoveryNoCard())
		return
	}
	if !s.recoveryLoaded && !s.recoveryLoading {
		s.loadRecoveryStatuses()
	}
	modes := s.recoveryModes()
	ui.Column(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Column(c).Grow(1).Gap(3).Children(func() {
				ui.Text(c, l.RecoveryTitle()).FontSize(12.5).Bold().TextColor(p.text)
				ui.Text(c, l.RecoveryIntro()).FontSize(10).TextColor(p.muted).Wrap()
			})
			if ui.Button(c, l.RecoveryRefresh()).Height(26).FontSize(10).
				Disabled(s.recoveryLoading || s.writeBusy()).Clicked() {
				s.loadRecoveryStatuses()
			}
		})
		for _, mode := range modes {
			s.recoveryModeRow(c, p, mode)
		}
	})
}

func (s *shell) recoveryModes() []recoveryMode {
	l := s.locales()
	mfoc, hasMFoC := s.recoveryStatus("mfoc")
	mfcuk, hasMFCUK := s.recoveryStatus("mfcuk-pipeline")
	hardnested, hasHardnested := s.recoveryStatus("hardnested")
	verified := s.verifiedKeyCount()
	classic1K := s.card != nil && s.card.TypeCode == "mifare_classic_1k"

	nested := recoveryMode{ID: "nested", Title: l.RecoveryNested(), Hint: l.RecoveryNestedHint()}
	if hasMFoC {
		nested.Status = &mfoc
		nested.Enabled = mfoc.Available && verified > 0 && !s.writeBusy()
		if verified == 0 {
			nested.Disabled = l.RecoverySeedRequired()
		}
	}

	darkside := recoveryMode{ID: "darkside", Title: l.RecoveryDarkside(), Hint: l.RecoveryDarksideHint()}
	if hasMFCUK {
		darkside.Status = &mfcuk
		darkside.Enabled = mfcuk.Available && verified == 0 && classic1K &&
			s.readerSupports(func(device app.DeviceDTO) bool { return device.Darkside }) && !s.writeBusy()
		switch {
		case !classic1K:
			darkside.Disabled = l.RecoveryDarksideClassic1K()
		case verified > 0:
			darkside.Disabled = l.RecoveryDarksideSeedKnown()
		}
	}

	hard := recoveryMode{ID: "hardnested", Title: l.RecoveryHardnested(), Hint: l.RecoveryHardnestedHint()}
	if hasHardnested {
		hard.Status = &hardnested
		hard.Enabled = hardnested.Available && verified > 0 && !s.writeBusy()
		if verified == 0 {
			hard.Disabled = l.RecoverySeedRequired()
		}
	}

	automatic := recoveryMode{
		ID: "auto", Title: l.RecoveryAutomatic(), Hint: l.RecoveryAutomaticHint(),
		Enabled: s.cardFamily() == familyClassic && !s.writeBusy(),
	}
	return []recoveryMode{automatic, nested, darkside, hard}
}

func (s *shell) recoveryModeRow(c *ui.Context, p palette, mode recoveryMode) {
	l := s.locales()
	ui.Row(c).Padding(12).Gap(12).Background(p.surface).Radius(10).
		Border(1, p.border).AlignItems(ui.Center).Children(func() {
		ui.Column(c).Grow(1).Gap(3).Children(func() {
			ui.Text(c, mode.Title).FontSize(11).Bold().TextColor(p.text)
			ui.Text(c, mode.Hint).FontSize(9.5).TextColor(p.muted).Wrap()
			if mode.Disabled != "" {
				ui.Text(c, mode.Disabled).FontSize(9.5).TextColor(p.warning).Wrap()
			}
			if mode.Status != nil {
				ui.Text(c, mode.Status.Detail).FontSize(9).TextColor(p.faint).Wrap()
				label, fg, bg, border := l.RecoveryEngineAvailable(), p.success, p.successSoft, p.successBorder
				if !mode.Status.Available {
					label, fg, bg, border = l.RecoveryEngineUnavailable(), p.faint, p.surfaceAlt, p.border
				}
				pill(c, label, fg, bg, border).AlignSelf(ui.Start)
			}
		})
		if ui.Button(c, l.RecoveryStartMode(mode.Title)).Height(30).FontSize(10.5).
			Disabled(!mode.Enabled).Clicked() {
			if s.workbench.Dirty {
				s.errorText = l.RecoveryUnsavedChanges()
				return
			}
			s.recoveryIntent = mode.ID
			s.recoveryOpen = true
		}
	})
}

func (s *shell) recoveryStatus(id string) (app.ExternalEngineStatusDTO, bool) {
	if s.recoveryStatuses == nil {
		return app.ExternalEngineStatusDTO{}, false
	}
	status, ok := s.recoveryStatuses[id]
	return status, ok
}

func (s *shell) loadRecoveryStatuses() {
	if s.recoveryLoading {
		return
	}
	s.recoveryLoading = true
	go func() {
		statuses := map[string]app.ExternalEngineStatusDTO{
			"mfoc":           s.api.MFoCStatus(),
			"mfcuk-pipeline": s.api.MFCUKStatus(),
			"hardnested":     s.api.HardnestedStatus(),
		}
		s.update(func() {
			s.recoveryStatuses = statuses
			s.recoveryLoading = false
			s.recoveryLoaded = true
		})
	}()
}

func (s *shell) startRecovery(mode string) {
	l := s.locales()
	s.pendingOps++
	go func() {
		var err error
		switch mode {
		case "auto":
			_, err = s.api.StartKeyRecovery(app.KeyRecoveryStartRequestDTO{Authorized: true})
		case "nested":
			_, err = s.api.StartMFoC(app.MFoCStartRequestDTO{Authorized: true})
		case "darkside":
			_, err = s.api.StartMFCUKPipeline(app.MFCUKStartRequestDTO{Authorized: true})
		case "hardnested":
			_, err = s.api.StartHardnested(app.HardnestedStartRequestDTO{Authorized: true})
		}
		s.update(func() {
			s.pendingOps--
			if err != nil {
				s.errorText = l.RecoveryFailed(err.Error())
				return
			}
			s.errorText = ""
			s.toastText = l.RecoveryStarted()
		})
	}()
}

func (s *shell) recoveryDialog(c *ui.Context) {
	if !s.recoveryOpen {
		return
	}
	l := s.locales()
	mode := s.recoveryIntent
	result := ui.AlertDialog(c, &s.recoveryOpen, l.RecoveryConfirmTitle(s.recoveryModeLabel(mode)),
		l.RecoveryConfirmMessage(), l.CommonCancel(), l.RecoveryConfirm())
	if result == -1 {
		return
	}
	s.recoveryIntent = ""
	if result == 1 {
		s.startRecovery(mode)
	}
}

func (s *shell) recoveryModeLabel(mode string) string {
	l := s.locales()
	switch mode {
	case "nested":
		return l.RecoveryNested()
	case "darkside":
		return l.RecoveryDarkside()
	case "hardnested":
		return l.RecoveryHardnested()
	default:
		return l.RecoveryAutomatic()
	}
}
