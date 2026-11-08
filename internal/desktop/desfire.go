package desktop

import (
	"fmt"
	"strconv"
	"strings"

	"github.com/egoist/mygo/ui"
)

func (s *shell) desfireView(c *ui.Context, p palette) {
	l := s.locales()
	ui.Scroll(c).Grow(1).MinHeight(0).Gap(12).Children(func() {
		ui.Column(c).Gap(3).Children(func() {
			ui.Text(c, l.DesfireTitle()).FontSize(12.5).Bold().TextColor(p.text)
			ui.Text(c, l.DesfireIntro()).FontSize(10).TextColor(p.muted).Wrap()
		})
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			if ui.Button(c, l.DesfireIdentify()).Height(28).FontSize(10).
				Disabled(s.desfireLoading).Clicked() {
				s.loadDESFireInfo()
			}
			if ui.Button(c, l.DesfireApplications()).Height(28).FontSize(10).
				Disabled(s.desfireLoading).Clicked() {
				s.loadDESFireApplications()
			}
		})
		if s.desfireError != "" {
			ui.Text(c, l.DesfireFailed(s.desfireError)).FontSize(10.5).TextColor(p.danger).Wrap()
		}
		if s.desfireInfo != nil {
			info := *s.desfireInfo
			ui.Column(c).Padding(14).Gap(7).Background(p.surface).Radius(10).
				Border(1, p.border).Children(func() {
				ui.Text(c, l.DesfireInfo()).FontSize(12).Bold().TextColor(p.text)
				fieldRow(c, p, l.DesfireVersion(), info.Version)
				fieldRow(c, p, l.DesfireVendor(), fmt.Sprintf("0x%02X", info.Vendor))
				fieldRow(c, p, l.DesfireType(), fmt.Sprintf("0x%02X", info.Type))
				fieldRow(c, p, l.DesfireSubtype(), fmt.Sprintf("0x%02X", info.Subtype))
				fieldRow(c, p, l.DesfireStorage(), fmt.Sprintf("0x%02X", info.Storage))
				fieldRow(c, p, l.DesfireProtocol(), fmt.Sprintf("0x%02X", info.Protocol))
			})
		}
		if s.desfireApps != nil {
			ui.Column(c).Padding(14).Gap(6).Background(p.surface).Radius(10).
				Border(1, p.border).Children(func() {
				ui.Text(c, l.DesfireApplications()).FontSize(12).Bold().TextColor(p.text)
				if len(s.desfireApps) == 0 {
					ui.Text(c, l.DesfireNoApplications()).FontSize(10).TextColor(p.muted)
					return
				}
				for _, aid := range s.desfireApps {
					ui.Text(c, aid).Font("monospace").FontSize(10).TextColor(p.text).
						Selectable().SingleLine()
				}
			})
		}
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.DesfireAID()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.desfireAID).Placeholder(l.DesfireAIDPlaceholder()).
				Font("monospace").Width(120)
			if ui.Button(c, l.DesfireFiles()).Height(28).FontSize(10).
				Disabled(s.desfireLoading || strings.TrimSpace(s.desfireAID) == "").Clicked() {
				s.loadDESFireFiles()
			}
		})
		if s.desfireFiles != nil {
			files := *s.desfireFiles
			ui.Column(c).Padding(14).Gap(6).Background(p.surface).Radius(10).
				Border(1, p.border).Children(func() {
				ui.Text(c, l.DesfireFilesTitle(files.AID)).FontSize(12).Bold().TextColor(p.text)
				if len(files.Files) == 0 {
					ui.Text(c, l.DesfireNoFiles()).FontSize(10).TextColor(p.muted)
					return
				}
				for _, file := range files.Files {
					ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
						ui.Textf(c, "%d", file.ID).Font("monospace").FontSize(10).
							TextColor(p.faint).Width(24).Shrink(0)
						ui.Text(c, file.Type).FontSize(10).TextColor(p.text).Width(110).Shrink(0)
						ui.Text(c, l.DesfireFileSize(fmt.Sprintf("%d", file.Size))).FontSize(10).
							TextColor(p.muted).Grow(1)
						if ui.Button(c, l.DesfireRead()).Height(24).FontSize(9.5).
							Disabled(s.desfireLoading).Clicked() {
							s.readDESFireFile(file.ID)
						}
					})
				}
			})
		}
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, l.DesfireFileID()).FontSize(9.5).TextColor(p.muted).Shrink(0)
			ui.TextInput(c, &s.desfireFileID).Placeholder(l.DesfireFileIDPlaceholder()).
				Font("monospace").Width(80)
			if ui.Button(c, l.DesfireReadFile()).Height(28).FontSize(10).
				Disabled(s.desfireLoading || strings.TrimSpace(s.desfireAID) == "" || strings.TrimSpace(s.desfireFileID) == "").Clicked() {
				s.readDESFireFileInput()
			}
		})
		if s.desfireRead != nil {
			data := *s.desfireRead
			ui.Column(c).Padding(14).Gap(7).Background(p.surface).Radius(10).
				Border(1, p.border).Children(func() {
				ui.Text(c, l.DesfireFileData(data.AID, fmt.Sprintf("%d", data.File), fmt.Sprintf("%d", data.Length))).FontSize(12).
					Bold().TextColor(p.text)
				ui.Text(c, spacedHex(data.Hex)).Font("monospace").FontSize(9.5).
					TextColor(p.text).Selectable().Wrap()
				if data.ASCII != "" {
					ui.Text(c, data.ASCII).Font("monospace").FontSize(9).TextColor(p.muted).
						Selectable().Wrap()
				}
			})
		}
	})
}

func (s *shell) loadDESFireInfo() {
	if s.desfireLoading {
		return
	}
	s.desfireLoading = true
	go func() {
		info, err := s.api.DESFireInfo()
		s.update(func() {
			s.desfireLoading = false
			if err != nil {
				s.desfireError = err.Error()
				return
			}
			s.desfireError = ""
			s.desfireInfo = &info
		})
	}()
}

func (s *shell) loadDESFireApplications() {
	if s.desfireLoading {
		return
	}
	s.desfireLoading = true
	go func() {
		apps, err := s.api.DESFireApplications()
		s.update(func() {
			s.desfireLoading = false
			if err != nil {
				s.desfireError = err.Error()
				return
			}
			s.desfireError = ""
			s.desfireApps = apps
		})
	}()
}

func (s *shell) loadDESFireFiles() {
	if s.desfireLoading {
		return
	}
	aid := strings.TrimSpace(s.desfireAID)
	s.desfireLoading = true
	go func() {
		files, err := s.api.DESFireFiles(aid)
		s.update(func() {
			s.desfireLoading = false
			if err != nil {
				s.desfireError = err.Error()
				return
			}
			s.desfireError = ""
			s.desfireFiles = &files
		})
	}()
}

func (s *shell) readDESFireFileInput() {
	value, err := strconv.ParseUint(strings.TrimSpace(s.desfireFileID), 10, 8)
	if err != nil {
		s.desfireError = s.locales().DesfireInvalidFile()
		return
	}
	s.readDESFireFile(byte(value))
}

func (s *shell) readDESFireFile(id byte) {
	if s.desfireLoading {
		return
	}
	aid := strings.TrimSpace(s.desfireAID)
	s.desfireLoading = true
	go func() {
		data, err := s.api.DESFireReadFile(aid, id)
		s.update(func() {
			s.desfireLoading = false
			if err != nil {
				s.desfireError = err.Error()
				return
			}
			s.desfireError = ""
			s.desfireRead = &data
		})
	}()
}
