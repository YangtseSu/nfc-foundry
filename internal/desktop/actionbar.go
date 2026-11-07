package desktop

import (
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func (s *shell) actionBar(c *ui.Context, p palette) {
	l := s.locales()
	busy := s.activeTask != "" || s.pendingOps > 0
	status := l.CommonReady()
	statusColor := p.success
	if busy {
		status = l.CommonWorking()
		statusColor = p.accent
	} else if s.errorText != "" {
		status = l.ConnectionError()
		statusColor = p.danger
	}
	ui.Row(c).Height(66).Shrink(0).Background(p.surface).Padding(0, 20).Gap(12).
		AlignItems(ui.Center).BorderWidth(1, 0, 0, 0).BorderColor(p.border).
		Children(func() {
			dot(c, statusColor, 7)
			ui.Text(c, status).FontSize(11).TextColor(p.text).SingleLine()
			ui.Text(c, s.actionSummary()).FontSize(11).TextColor(p.faint).
				SingleLine()
			ui.Spacer(c)
			if s.activeTask != "" {
				if ui.Button(c, l.CommonCancelTask()).Height(32).FontSize(11).
					Clicked() {
					s.cancelActiveTask()
				}
			}
			classic := s.cardPresent() && s.cardFamily() == familyClassic &&
				s.readerSupports(func(device app.DeviceDTO) bool { return device.ClassicRead })
			type2 := s.cardPresent() && s.cardFamily() == familyType2 &&
				s.readerSupports(func(device app.DeviceDTO) bool { return device.Type2Read })
			readable := classic || type2
			if ui.Button(c, l.CommonScanKeys()).Height(32).FontSize(11).
				Disabled(!classic || busy).Clicked() {
				s.scanKeys()
			}
			if primaryButton(c, p, l.CommonReadCard()).Height(32).
				Disabled(!readable || busy).Clicked() {
				s.readCard()
			}
		})
}

func (s *shell) actionSummary() string {
	l := s.locales()
	var parts []string
	if s.cardFamily() == familyType2 && s.connected() &&
		!s.readerSupports(func(device app.DeviceDTO) bool { return device.Type2Read }) {
		return l.ReaderUnsupportedType2()
	}
	if s.cardFamily() == familyType2 {
		if !s.type2.Loaded {
			return l.Type2NoData()
		}
		parts = append(parts, l.Type2Subtitle(s.type2.Info.ProductName, s.type2.Info.TotalPages, s.type2.Info.UserBytes))
		if s.type2.NDEF != nil && s.type2.NDEF.Present {
			parts = append(parts, l.Type2Records(len(s.type2.NDEF.Records)))
		} else {
			parts = append(parts, l.Type2NoNDEF())
		}
		return strings.Join(parts, " · ")
	}
	if len(s.workbench.Blocks) == 0 {
		parts = append(parts, l.ActionNoDump())
	} else {
		parts = append(parts, l.ActionBlocks(len(s.workbench.Blocks)))
		parts = append(parts, l.ActionKeys(s.verifiedKeyCount()))
	}
	if s.workbench.Dirty {
		parts = append(parts, l.ActionUnsaved(len(s.workbench.DirtyBlocks)))
	} else if s.workbench.Loaded {
		parts = append(parts, l.ActionNoChanges())
	}
	return strings.Join(parts, " · ")
}
