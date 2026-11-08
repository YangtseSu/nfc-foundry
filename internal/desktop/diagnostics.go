package desktop

import (
	"fmt"

	"github.com/egoist/mygo/ui"
)

func (s *shell) openDiagnostics() {
	s.diagnosticsOpen = true
	if s.diagnosticsLoading {
		return
	}
	s.diagnosticsLoading = true
	go func() {
		report := s.api.GetDiagnostics()
		s.update(func() {
			s.diagnosticsLoading = false
			s.diagnostics = &report
		})
	}()
}

func (s *shell) diagnosticsModal(c *ui.Context, p palette) {
	if !s.diagnosticsOpen {
		return
	}
	l := s.locales()
	ui.Modal(c, &s.diagnosticsOpen, func() {
		ui.Text(c, l.DiagnosticsTitle()).FontSize(18).Bold().TextColor(p.text)
		if s.diagnosticsLoading || s.diagnostics == nil {
			ui.Text(c, l.DiagnosticsLoading()).FontSize(10.5).TextColor(p.muted)
			return
		}
		report := *s.diagnostics
		status, fg, bg, border := l.DiagnosticsHealthy(), p.success, p.successSoft, p.successBorder
		if !report.Healthy {
			status, fg, bg, border = l.DiagnosticsUnhealthy(), p.warning, p.warningSoft, p.warningBorder
		}
		pill(c, status, fg, bg, border)
		fieldRow(c, p, l.DiagnosticsTitle(), l.DiagnosticsBuild(report.Build.Version, report.Build.Commit))
		fieldRow(c, p, "", l.DiagnosticsGo(report.Build.GoVersion, report.Build.OS, report.Build.Architecture))
		if report.LibNFC.Available {
			fieldRow(c, p, "libnfc-ng", l.DiagnosticsRuntime(report.LibNFC.Version))
		} else {
			fieldRow(c, p, "libnfc-ng", l.DiagnosticsRuntimeMissing(report.LibNFC.Error))
		}
		ui.Text(c, l.DiagnosticsEngines()).FontSize(10).Bold().TextColor(p.muted)
		for _, check := range report.Engines {
			value := check.Name
			if check.Version != "" {
				value += " · " + check.Version
			}
			if check.OK {
				fieldRow(c, p, check.ID, value)
			} else {
				fieldRow(c, p, check.ID, fmt.Sprintf("%s · %s", value, check.Detail))
			}
		}
		manifest := l.DiagnosticsManifestMissing(report.Manifest.Detail)
		if report.Manifest.Verified {
			manifest = l.DiagnosticsManifestVerified(fmt.Sprintf("%d", report.Manifest.Files))
		}
		fieldRow(c, p, l.DiagnosticsManifest(), manifest)
		ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
			if ui.PrimaryButton(c, l.DiagnosticsClose()).Clicked() {
				s.diagnosticsOpen = false
			}
		})
	})
}
