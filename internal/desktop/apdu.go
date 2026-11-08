package desktop

import (
	"strings"

	"github.com/egoist/mygo/ui"
)

func (s *shell) apduView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Column(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		ui.Column(c).Gap(3).Children(func() {
			ui.Text(c, l.ApduTitle()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Text(c, l.ApduIntro()).FontSize(10).TextColor(p.muted).Wrap()
		})
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			input := ui.TextInput(c, &s.apduInput).Placeholder(l.ApduPlaceholder()).
				Label(l.ApduInput()).Font("monospace")
			if input.Submitted() {
				s.sendAPDU()
			}
			if ui.Button(c, l.ApduSend()).Height(30).FontSize(10.5).
				Disabled(s.apduLoading || strings.TrimSpace(s.apduInput) == "").Clicked() {
				s.sendAPDU()
			}
		})
		if s.apduError != "" {
			ui.Text(c, l.ApduFailed(s.apduError)).FontSize(10.5).TextColor(p.danger).Wrap()
		}
		if s.apduResponse == nil {
			return
		}
		response := *s.apduResponse
		ui.Column(c).Padding(14).Gap(9).Background(p.surface).Radius(10).
			Border(1, p.border).Children(func() {
			ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, l.ApduResponse()).FontSize(12.5).Bold().TextColor(p.text)
				status := l.ApduStatusOK()
				fg, bg, border := p.success, p.successSoft, p.successBorder
				if !response.OK {
					status, fg, bg, border = l.ApduStatusError(), p.warning, p.warningSoft, p.warningBorder
				}
				pill(c, status, fg, bg, border)
			})
			monoFieldRow(c, p, l.ApduSW(), response.SW)
			monoFieldRow(c, p, l.ApduATS(), response.ATS.Raw)
			if response.Data != "" {
				ui.Text(c, l.ApduData()).FontSize(9.5).Bold().TextColor(p.faint)
				ui.Text(c, spacedHex(response.Data)).Font("monospace").FontSize(9.5).
					TextColor(p.text).Selectable().Wrap()
			}
		})
	})
}

func (s *shell) sendAPDU() {
	if s.apduLoading {
		return
	}
	raw := strings.TrimSpace(s.apduInput)
	if raw == "" {
		return
	}
	s.apduLoading = true
	go func() {
		response, err := s.api.TransmitAPDU(raw)
		s.update(func() {
			s.apduLoading = false
			if err != nil {
				s.apduError = err.Error()
				s.apduResponse = nil
				return
			}
			s.apduError = ""
			s.apduResponse = &response
		})
	}()
}
