package cli

import (
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
)

type deviceListResult struct {
	Devices []app.DeviceDTO `json:"devices"`
}

type cardResult struct {
	Device app.DeviceDTO     `json:"device"`
	Card   app.CardDTO       `json:"card"`
	Type2  *app.Type2InfoDTO `json:"type2,omitempty"`
}

type classicReadResult struct {
	Block  int    `json:"block"`
	Sector int    `json:"sector"`
	Kind   string `json:"kind"`
	Hex    string `json:"hex"`
	ASCII  string `json:"ascii"`
}

type classicWriteResult struct {
	Block      int    `json:"block"`
	Sector     int    `json:"sector"`
	Hex        string `json:"hex"`
	Verified   bool   `json:"verified"`
	DryRun     bool   `json:"dryRun,omitempty"`
	CurrentHex string `json:"currentHex,omitempty"`
}

type keyscanResult struct {
	Verified int                `json:"verified"`
	Total    int                `json:"total"`
	Sectors  []app.SectorKeyDTO `json:"sectors"`
}

type dumpResult struct {
	Path             string         `json:"path"`
	Sidecar          string         `json:"sidecar,omitempty"`
	Layout           string         `json:"layout"`
	Complete         bool           `json:"complete"`
	Blocks           int            `json:"blocks"`
	VerifiedKeySlots int            `json:"verifiedKeySlots"`
	KeySlots         int            `json:"keySlots"`
	KeyScan          *keyscanResult `json:"keyScan,omitempty"`
}

type restoreResult struct {
	Path      string                 `json:"path"`
	DryRun    bool                   `json:"dryRun"`
	Preflight *app.WritePreflightDTO `json:"preflight"`
	Restore   *app.RestoreResultDTO  `json:"restore,omitempty"`
	KeyScan   *keyscanResult         `json:"keyScan,omitempty"`
}

type recoverResult struct {
	Mode    string              `json:"mode"`
	Engine  string              `json:"engine,omitempty"`
	Version string              `json:"version,omitempty"`
	MFoC    *app.MFoCResultDTO  `json:"mfoc,omitempty"`
	MFCUK   *app.MFCUKResultDTO `json:"mfcuk,omitempty"`
	KeyScan *keyscanResult      `json:"keyScan,omitempty"`
}

type uidResult struct {
	DryRun     bool                    `json:"dryRun"`
	Preview    *app.UIDWritePreviewDTO `json:"preview"`
	BackupPath string                  `json:"backupPath,omitempty"`
	Result     *app.UIDWriteResultDTO  `json:"result,omitempty"`
}

type keySummary struct {
	ID        string   `json:"id"`
	Value     string   `json:"value,omitempty"`
	Sources   []string `json:"sources"`
	Builtin   bool     `json:"builtin"`
	Removable bool     `json:"removable"`
}

type keysResult struct {
	Keys           []keySummary `json:"keys"`
	ValuesIncluded bool         `json:"valuesIncluded"`
}

type keyAddedResult struct {
	Key   keySummary `json:"key"`
	Total int        `json:"total"`
	Added bool       `json:"added"`
}

type keysImportResult struct {
	Path       string               `json:"path"`
	Valid      int                  `json:"valid"`
	Added      int                  `json:"added"`
	Duplicates int                  `json:"duplicates"`
	Ignored    int                  `json:"ignored"`
	Issues     []app.ImportIssueDTO `json:"issues"`
	Keys       int                  `json:"keys"`
}

type type2PageResult struct {
	Page app.Type2PageDTO `json:"page"`
}

type type2DumpFileResult struct {
	Path        string `json:"path"`
	Product     string `json:"product"`
	ProductName string `json:"productName"`
	PageCount   int    `json:"pageCount"`
	UserBytes   int    `json:"userBytes"`
}

type ndefReadResult struct {
	Family      string             `json:"family"`
	Layout      string             `json:"layout,omitempty"`
	Application bool               `json:"application,omitempty"`
	CC          *app.Type2CCDTO    `json:"cc,omitempty"`
	MAD         *app.ClassicMADDTO `json:"mad,omitempty"`
	Sectors     []int              `json:"sectors,omitempty"`
	Capacity    int                `json:"capacity,omitempty"`
	ReadOnly    bool               `json:"readOnly,omitempty"`
	NDEF        *app.Type2NDEFDTO  `json:"ndef"`
	Raw         bool               `json:"raw"`
	KeyScan     *keyscanResult     `json:"keyScan,omitempty"`
}

type keysExportResult struct {
	Path  string `json:"path"`
	Count int    `json:"count"`
}

func buildKeyscanResult(dashboard app.DashboardDTO, showKeys bool) keyscanResult {
	sectors := make([]app.SectorKeyDTO, len(dashboard.Keys))
	copy(sectors, dashboard.Keys)
	verified := 0
	for index := range sectors {
		if sectors[index].KeyAStatus == "verified" {
			verified++
		}
		if sectors[index].KeyBStatus == "verified" {
			verified++
		}
		if !showKeys {
			sectors[index].KeyA = ""
			sectors[index].KeyB = ""
		}
	}
	return keyscanResult{Verified: verified, Total: len(sectors) * 2, Sectors: sectors}
}

func keySummaries(catalog []app.KeyDTO, showValues bool) []keySummary {
	result := make([]keySummary, 0, len(catalog))
	for _, key := range catalog {
		summary := keySummary{ID: key.ID, Sources: append([]string(nil), key.Sources...), Builtin: key.Builtin, Removable: key.Removable}
		if showValues {
			summary.Value = key.Value
		}
		result = append(result, summary)
	}
	return result
}

func normalizeHex(value string) string {
	replacer := strings.NewReplacer(" ", "", ":", "", "-", "")
	return strings.ToUpper(replacer.Replace(strings.TrimSpace(value)))
}
