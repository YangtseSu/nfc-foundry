package desktop

import (
	"strings"

	"github.com/egoist/mygo/ui"
)

// palette holds the workbench colors for one theme. The reviewed mockup is
// the light palette; the dark palette keeps the same hierarchy for systems
// that ask for it.
type palette struct {
	dark bool

	topbar      ui.Color
	topbarText  ui.Color
	topbarMuted ui.Color
	topbarLine  ui.Color

	background ui.Color
	surface    ui.Color
	surfaceAlt ui.Color
	rail       ui.Color
	border     ui.Color
	divider    ui.Color
	header     ui.Color

	text   ui.Color
	muted  ui.Color
	faint  ui.Color
	accent ui.Color

	accentSoft   ui.Color
	accentBorder ui.Color

	success       ui.Color
	successSoft   ui.Color
	successBorder ui.Color

	warning       ui.Color
	warningSoft   ui.Color
	warningBorder ui.Color

	danger       ui.Color
	dangerSoft   ui.Color
	dangerBorder ui.Color
}

func paletteFor(t *ui.Theme) palette {
	if t.Dark {
		return palette{
			dark:        true,
			topbar:      ui.Hex("#0B1B2B"),
			topbarText:  ui.Hex("#F4F8FC"),
			topbarMuted: ui.Hex("#8FA3B7"),
			topbarLine:  ui.RGBA(255, 255, 255, 0.10),

			background: ui.Hex("#0F1216"),
			surface:    ui.Hex("#151A20"),
			surfaceAlt: ui.Hex("#191F27"),
			rail:       ui.Hex("#12171D"),
			border:     ui.Hex("#2A303A"),
			divider:    ui.Hex("#232932"),
			header:     ui.Hex("#12171D"),

			text:   ui.Hex("#E6EDF3"),
			muted:  ui.Hex("#98A2B3"),
			faint:  ui.Hex("#6B7785"),
			accent: ui.Hex("#22C3DD"),

			accentSoft:   ui.Hex("#0E2A33"),
			accentBorder: ui.Hex("#1B4A57"),

			success:       ui.Hex("#3ED6A4"),
			successSoft:   ui.Hex("#0F2A21"),
			successBorder: ui.Hex("#1E4A3A"),

			warning:       ui.Hex("#F5A524"),
			warningSoft:   ui.Hex("#2B2111"),
			warningBorder: ui.Hex("#5A441A"),

			danger:       ui.Hex("#F97066"),
			dangerSoft:   ui.Hex("#2B1514"),
			dangerBorder: ui.Hex("#5C2A26"),
		}
	}
	return palette{
		topbar:      ui.Hex("#0B1B2B"),
		topbarText:  ui.Hex("#F4F8FC"),
		topbarMuted: ui.Hex("#7E95AB"),
		topbarLine:  ui.RGBA(255, 255, 255, 0.11),

		background: ui.Hex("#F7F8FA"),
		surface:    ui.Hex("#FFFFFF"),
		surfaceAlt: ui.Hex("#FBFCFD"),
		rail:       ui.Hex("#F2F5F8"),
		border:     ui.Hex("#E2E7ED"),
		divider:    ui.Hex("#EDF1F5"),
		header:     ui.Hex("#FBFCFD"),

		text:   ui.Hex("#101828"),
		muted:  ui.Hex("#667085"),
		faint:  ui.Hex("#98A2B3"),
		accent: ui.Hex("#0E7490"),

		accentSoft:   ui.Hex("#ECFBFD"),
		accentBorder: ui.Hex("#C8F0F6"),

		success:       ui.Hex("#067647"),
		successSoft:   ui.Hex("#F3FCF8"),
		successBorder: ui.Hex("#D9F3E6"),

		warning:       ui.Hex("#B54708"),
		warningSoft:   ui.Hex("#FFF8EB"),
		warningBorder: ui.Hex("#FDE3B8"),

		danger:       ui.Hex("#B42318"),
		dangerSoft:   ui.Hex("#FEF3F2"),
		dangerBorder: ui.Hex("#FECDCA"),
	}
}

// applyWorkbenchTheme aligns built-in MyGo widgets with the reviewed
// workbench palette while leaving the system light or dark choice intact.
func applyWorkbenchTheme(c *ui.Context, p palette) {
	theme := *c.Theme()
	theme.Background = p.background
	theme.Surface = p.surface
	theme.SurfaceHover = p.surfaceAlt
	theme.SurfacePressed = p.divider
	theme.Border = p.border
	theme.Text = p.text
	theme.TextMuted = p.muted
	if p.dark {
		theme.Accent = ui.RGBA(34, 195, 221, 0.18)
		theme.AccentHover = ui.RGBA(34, 195, 221, 0.24)
		theme.AccentPressed = ui.RGBA(34, 195, 221, 0.30)
	} else {
		theme.Accent = ui.RGBA(14, 116, 144, 0.12)
		theme.AccentHover = ui.RGBA(14, 116, 144, 0.18)
		theme.AccentPressed = ui.RGBA(14, 116, 144, 0.24)
	}
	theme.AccentText = p.text
	theme.Selection = ui.RGBA(14, 116, 144, 0.24)
	theme.Focus = ui.RGBA(14, 116, 144, 0.55)
	theme.Danger = p.danger
	theme.Warning = p.warning
	theme.Success = p.success
	theme.Scrollbar = ui.RGBA(16, 24, 40, 0.22)
	theme.Radius = 7
	c.SetTheme(&theme)
}

func primaryButton(c *ui.Context, p palette, label string) ui.Element {
	return ui.Button(c, "").Label(label).Padding(0, 13).Radius(7).
		Background(p.topbar).Border(1, p.topbar).AlignItems(ui.Center).Justify(ui.Center).
		Children(func() {
			ui.Text(c, label).FontSize(11).TextColor(p.topbarText).SingleLine()
		})
}

func sectionLabel(c *ui.Context, p palette, text string) {
	ui.Text(c, strings.ToUpper(text)).FontSize(9.5).Bold().LetterSpacing(1.3).TextColor(p.faint)
}

func hairline(c *ui.Context, p palette) ui.Element {
	return ui.Row(c).Height(1).Shrink(0).Background(p.divider)
}

func pill(c *ui.Context, text string, fg, bg, border ui.Color) ui.Element {
	return ui.Row(c).Height(20).Padding(0, 7).Gap(4).Radius(999).
		Background(bg).Border(1, border).AlignItems(ui.Center).
		Children(func() {
			ui.Text(c, text).FontSize(9.5).Bold().TextColor(fg).SingleLine()
		})
}

func countPill(c *ui.Context, p palette, text string) ui.Element {
	return pill(c, text, p.accent, p.accentSoft, p.accentBorder)
}

func dot(c *ui.Context, color ui.Color, size float32) ui.Element {
	return ui.Row(c).Width(size).Height(size).Radius(size / 2).Background(color)
}

func iconButton(c *ui.Context, p palette, icon *ui.SVG, label string, color ui.Color) ui.Element {
	button := ui.Button(c, "").Width(28).Height(28).Padding(0).Radius(7).
		Background(ui.Transparent).BorderWidth(0).Label(label)
	button.Children(func() {
		ui.Icon(c, icon).FontSize(15).TextColor(color)
	})
	return button
}

func linkButton(c *ui.Context, p palette, text string) ui.Element {
	return ui.Button(c, text).Height(24).Padding(0, 2).Background(ui.Transparent).
		BorderWidth(0).FontSize(10).TextColor(p.accent)
}

func fieldRow(c *ui.Context, p palette, label, value string) {
	ui.Row(c).Gap(8).AlignItems(ui.Start).Children(func() {
		ui.Text(c, label).Width(78).Shrink(0).FontSize(9.5).TextColor(p.faint)
		ui.Text(c, value).FontSize(10).TextColor(p.text).Grow(1)
	})
}

func monoFieldRow(c *ui.Context, p palette, label, value string) {
	ui.Row(c).Gap(8).AlignItems(ui.Start).Children(func() {
		ui.Text(c, label).Width(78).Shrink(0).FontSize(9.5).TextColor(p.faint)
		ui.Text(c, value).Font("monospace").FontSize(9.5).TextColor(p.text).Grow(1).
			Selectable().SingleLine()
	})
}

func payloadBox(c *ui.Context, p palette, text string) ui.Element {
	return ui.Column(c).Padding(8, 9).Background(p.surface).Border(1, p.border).Radius(7).
		Children(func() {
			ui.Text(c, text).Font("monospace").FontSize(8.5).TextColor(p.text).
				Selectable().Wrap()
		})
}

func loadingDots(c *ui.Context, p palette) {
	ui.Row(c).Gap(3).AlignItems(ui.Center).Children(func() {
		for range 3 {
			dot(c, p.accent, 4)
		}
	})
}

var (
	brandIcon    = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4.6 15.4a8.7 8.7 0 0 1 0-6.8"/><path d="M8.2 13.6a4.9 4.9 0 0 1 0-3.2"/><path d="M11.7 12.2a1.2 1.2 0 0 1 0-.4"/><rect x="15" y="6.7" width="5.8" height="10.6" rx="1.6"/><circle cx="18.1" cy="9.4" r=".7"/></svg>`))
	readerIcon   = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="M4 8.5a12 12 0 0 1 0 7"/><path d="M8 10a7.5 7.5 0 0 1 0 4"/><rect x="12" y="7" width="8" height="10" rx="2"/><path d="M16 17v3"/></svg>`))
	cardIcon     = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M5 9.5a9 9 0 0 1 0 5"/><path d="M8.6 10.7a5 5 0 0 1 0 2.6"/><rect x="12.5" y="7.3" width="6.5" height="9.4" rx="1.5"/></svg>`))
	refreshIcon  = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v6h-6"/></svg>`))
	checkIcon    = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M20 6 9 17l-5-5"/></svg>`))
	warningIcon  = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 9v4"/><path d="M12 17h.01"/><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/></svg>`))
	infoIcon     = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="M12 8v4"/><path d="M12 16h.01"/></svg>`))
	lockIcon     = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><rect x="4" y="10" width="16" height="10" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></svg>`))
	trashIcon    = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="m6 6 1 14h10l1-14"/></svg>`))
	keyIcon      = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="8" cy="15" r="4"/><path d="m10.8 12.2 8.2-8.2"/><path d="m15 4 5 5"/><path d="m17 6 2 2"/></svg>`))
	blocksIcon   = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg>`))
	activityIcon = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M3 12h4l2-6 4 12 2-6h6"/></svg>`))
	arrowIcon    = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="m9 6 6 6-6 6"/></svg>`))
)
