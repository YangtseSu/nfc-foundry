package app

import "github.com/YangtseSu/nfc-foundry/internal/workflow"

// DumpFormat is re-exported so the GUI, CLI, and services share one format
// vocabulary without depending on workflow internals.
type DumpFormat = workflow.DumpFormat

const (
	DumpFormatProject  = workflow.DumpFormatProject
	DumpFormatRaw      = workflow.DumpFormatRaw
	DumpFormatMCT      = workflow.DumpFormatMCT
	DumpFormatEML      = workflow.DumpFormatEML
	DumpFormatMCTJSON  = workflow.DumpFormatMCTJSON
	DumpFormatProxmark = workflow.DumpFormatProxmark
	DumpFormatKeyArray = workflow.DumpFormatKeyArray
	DumpFormatSidecar  = workflow.DumpFormatSidecar
)

func ParseDumpFormat(name string) (DumpFormat, error) {
	return workflow.ParseDumpFormat(name)
}

func dumpSaveDefaults(format DumpFormat, complete bool) (string, string) {
	if !complete && format != DumpFormatProject {
		format = DumpFormatProject
	}
	switch format {
	case DumpFormatRaw:
		return "card-dump.bin", "*.bin;*.mfd"
	case DumpFormatProxmark:
		return "card-dump.mfc.json", "*.json"
	case DumpFormatKeyArray:
		return "card-keys.bin", "*.bin"
	case DumpFormatMCTJSON:
		return "card-dump.mfcard.json", "*.json"
	case DumpFormatMCT:
		return "card-dump.mct", "*.mct"
	case DumpFormatEML:
		return "card-dump.eml", "*.eml"
	case DumpFormatSidecar:
		return "card-dump.nfcx.json", "*.json"
	default:
		return "card-dump.nfcx.json", "*.json"
	}
}
