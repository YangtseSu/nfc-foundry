package desktop

import (
	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/locales"
)

type dumpFormatChoice struct {
	ID    string
	Label string
}

func dumpFormatChoices(l locales.Messages) []dumpFormatChoice {
	return []dumpFormatChoice{
		{ID: string(app.DumpFormatProject), Label: l.ClassicDumpFormatProject()},
		{ID: string(app.DumpFormatRaw), Label: l.ClassicDumpFormatRaw()},
		{ID: string(app.DumpFormatProxmark), Label: l.ClassicDumpFormatProxmark3()},
		{ID: string(app.DumpFormatKeyArray), Label: l.ClassicDumpFormatKeyArray()},
		{ID: string(app.DumpFormatMCTJSON), Label: l.ClassicDumpFormatMCTJSON()},
		{ID: string(app.DumpFormatMCT), Label: l.ClassicDumpFormatMCT()},
		{ID: string(app.DumpFormatEML), Label: l.ClassicDumpFormatEML()},
		{ID: string(app.DumpFormatSidecar), Label: l.ClassicDumpFormatSidecar()},
	}
}
