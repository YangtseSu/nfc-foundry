package cli

import (
	"context"
	"encoding/hex"
	"os"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func (r *Runner) cmdType2(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl type2 info|read|write|dump|restore")
	}
	switch args[0] {
	case "info":
		r.command = "type2.info"
		return r.type2Info(ctx, args[1:])
	case "read":
		r.command = "type2.read"
		return r.type2Read(ctx, args[1:])
	case "write":
		r.command = "type2.write"
		return r.type2Write(ctx, args[1:])
	case "dump":
		r.command = "type2.dump"
		return r.type2Dump(ctx, args[1:])
	case "restore":
		r.command = "type2.restore"
		return r.type2Restore(ctx, args[1:])
	default:
		return usagef("unknown type2 command %q", args[0])
	}
}

func (r *Runner) cmdNDEF(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl ndef read|write")
	}
	switch args[0] {
	case "read":
		r.command = "ndef.read"
		return r.ndefRead(ctx, args[1:])
	case "write":
		r.command = "ndef.write"
		return r.ndefWrite(ctx, args[1:])
	default:
		return usagef("unknown ndef command %q", args[0])
	}
}

func (r *Runner) type2Info(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("type2 info takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("type2 info does not support --dry-run")
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	info, err := r.service.Type2Info(ctx)
	if err != nil {
		return err
	}
	r.printType2Info(info)
	r.out.emit("type2.info", r.device, r.card, nil, info)
	return nil
}

var type2ReadSpecs = map[string]flagSpec{
	"page": {takesValue: true},
	"pwd":  {takesValue: true},
	"all":  {},
}

func (r *Runner) type2Read(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2ReadSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("type2 read takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("type2 read does not support --dry-run")
	}
	page, hasPage, err := parseOptionalPage(parsed.value("page"))
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	dump, err := r.service.ReadType2Card(ctx, parsed.value("pwd"))
	if err != nil {
		return err
	}
	if r.global.showKeys {
		dump = r.service.Type2Dump(true)
	}
	if hasPage {
		selected, ok := findType2Page(dump, page)
		if !ok {
			return usagef("page %d is outside the %d-page card", page, len(dump.Pages))
		}
		r.printType2Page(selected)
		r.out.emit("type2.read", r.device, r.card, nil, type2PageResult{Page: selected})
		return nil
	}
	r.printType2Dump(dump, parsed.bool("all"))
	r.out.emit("type2.read", r.device, r.card, nil, dump)
	return nil
}

var type2WriteSpecs = map[string]flagSpec{
	"page": {takesValue: true},
	"data": {takesValue: true},
	"pwd":  {takesValue: true},
}

func (r *Runner) type2Write(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2WriteSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("type2 write takes no positional arguments")
	}
	if !r.global.dryRun && !r.global.yes {
		return usagef("type2 write is destructive; pass --yes")
	}
	page, hasPage, err := parseOptionalPage(parsed.value("page"))
	if err != nil {
		return err
	}
	if !hasPage {
		return usagef("--page is required")
	}
	data, err := parsePageData(parsed.value("data"))
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.service.WriteType2Pages(ctx, app.Type2WriteRequestDTO{
		Writes:   []app.Type2PageWriteDTO{{Page: page, Hex: compactHex(data[:])}},
		Password: parsed.value("pwd"), DryRun: r.global.dryRun,
	})
	if err != nil {
		return err
	}
	r.printType2Write(result, r.global.dryRun)
	r.out.emit("type2.write", r.device, r.card, nil, result)
	return nil
}

var type2DumpSpecs = map[string]flagSpec{"pwd": {takesValue: true}}

func (r *Runner) type2Dump(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2DumpSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl type2 dump <file.json> [--pwd HEX]")
	}
	if r.global.dryRun {
		return usagef("type2 dump does not support --dry-run")
	}
	path := parsed.positionals[0]
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	dump, err := r.service.DumpType2Card(ctx, path, parsed.value("pwd"))
	if err != nil {
		return err
	}
	result := type2DumpFileResult{
		Path: path, Product: dump.Info.Product, ProductName: dump.Info.ProductName,
		PageCount: len(dump.Pages), UserBytes: dump.Info.UserBytes,
	}
	r.out.humanf("saved %s dump to %s (%d pages, %d user bytes)\n", dump.Info.ProductName, path, len(dump.Pages), dump.Info.UserBytes)
	r.out.emit("type2.dump", r.device, r.card, nil, result)
	return nil
}

var type2RestoreSpecs = map[string]flagSpec{
	"pwd":               {takesValue: true},
	"protected":         {},
	"confirm-protected": {},
}

func (r *Runner) type2Restore(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2RestoreSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl type2 restore <file.json> [--pwd HEX] [--protected] [--yes] [--confirm-protected] [--dry-run]")
	}
	protected := parsed.bool("protected")
	if !r.global.dryRun {
		if !r.global.yes {
			return usagef("type2 restore is destructive; pass --yes")
		}
		if protected && !parsed.bool("confirm-protected") {
			return usagef("a protected restore replaces lock, configuration, or password pages; pass --confirm-protected as well")
		}
	}
	document, err := r.service.LoadType2DumpFile(parsed.positionals[0])
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.service.RestoreType2(ctx, app.Type2RestoreRequestDTO{
		Document: document, IncludeProtected: protected, Password: parsed.value("pwd"), DryRun: r.global.dryRun,
	})
	if err != nil {
		return err
	}
	r.printType2Write(result, r.global.dryRun)
	r.out.emit("type2.restore", r.device, r.card, nil, result)
	return nil
}

var ndefReadSpecs = map[string]flagSpec{
	"pwd": {takesValue: true},
	"raw": {},
}

func (r *Runner) ndefRead(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, ndefReadSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("ndef read takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("ndef read does not support --dry-run")
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	dump, err := r.service.ReadType2Card(ctx, parsed.value("pwd"))
	if err != nil {
		return err
	}
	if !dump.CC.Valid {
		return nfc.NewError("read NDEF", nfc.CodeInvalidArgument, "tag has no valid NDEF capability container", nil)
	}
	if dump.NDEFError != "" {
		return nfc.NewError("read NDEF", nfc.CodeInvalidArgument, dump.NDEFError, nil)
	}
	if dump.NDEF == nil {
		return nfc.NewError("read NDEF", nfc.CodeInternal, "NDEF result is missing", nil)
	}
	if parsed.bool("raw") {
		r.out.humanf("%s\n", dump.NDEF.Raw)
	} else {
		r.printNDEF(*dump.NDEF)
	}
	result := ndefReadResult{CC: dump.CC, NDEF: dump.NDEF, Raw: parsed.bool("raw")}
	r.out.emit("ndef.read", r.device, r.card, nil, result)
	return nil
}

var ndefWriteSpecs = map[string]flagSpec{
	"file": {takesValue: true},
	"hex":  {takesValue: true},
	"pwd":  {takesValue: true},
}

func (r *Runner) ndefWrite(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, ndefWriteSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("ndef write takes no positional arguments")
	}
	file, hexValue := parsed.value("file"), parsed.value("hex")
	if (file == "") == (hexValue == "") {
		return usagef("exactly one of --file or --hex is required")
	}
	if !r.global.dryRun && !r.global.yes {
		return usagef("ndef write is destructive; pass --yes")
	}
	message, err := readNDEFMessage(file, hexValue)
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.service.WriteType2NDEF(ctx, app.Type2NDEFWriteRequestDTO{
		Message: message, Password: parsed.value("pwd"), DryRun: r.global.dryRun,
	})
	if err != nil {
		return err
	}
	r.printType2Write(result, r.global.dryRun)
	r.out.emit("ndef.write", r.device, r.card, nil, result)
	return nil
}

func parseOptionalPage(value string) (int, bool, error) {
	if strings.TrimSpace(value) == "" {
		return 0, false, nil
	}
	page, err := strconv.Atoi(value)
	if err != nil || page < 0 || page > 255 {
		return 0, false, usagef("invalid page %q", value)
	}
	return page, true, nil
}

func parsePageData(value string) ([4]byte, error) {
	var data [4]byte
	decoded, err := hex.DecodeString(normalizeHex(value))
	if err != nil || len(decoded) != 4 {
		return data, usagef("--data must be 8 hexadecimal digits")
	}
	copy(data[:], decoded)
	return data, nil
}

func findType2Page(dump app.Type2DumpDTO, page int) (app.Type2PageDTO, bool) {
	if page < 0 || page >= len(dump.Pages) {
		return app.Type2PageDTO{}, false
	}
	return dump.Pages[page], true
}

func readNDEFMessage(file, hexValue string) ([]byte, error) {
	if file != "" {
		info, err := os.Stat(file)
		if err != nil {
			return nil, err
		}
		if info.Size() > 64<<10 {
			return nil, usagef("NDEF file is too large")
		}
		return os.ReadFile(file)
	}
	decoded, err := hex.DecodeString(normalizeHex(hexValue))
	if err != nil || len(decoded) == 0 {
		return nil, usagef("--hex must contain an even number of hexadecimal digits")
	}
	return decoded, nil
}

func (r *Runner) printType2Info(info app.Type2InfoDTO) {
	r.out.humanf("product: %s (%s)\n", info.ProductName, info.Product)
	r.out.humanf("pages:   %d (user %d-%d, %d bytes)\n", info.TotalPages, info.UserPages.Start, info.UserPages.End, info.UserBytes)
	if info.Version != "" {
		r.out.humanf("version: %s\n", info.Version)
	}
	r.out.humanf("detected: %s\n", info.DetectedBy)
}

func (r *Runner) printType2Dump(dump app.Type2DumpDTO, all bool) {
	r.printType2Info(dump.Info)
	if dump.CC.Valid {
		r.out.humanf("cc:      %s (capacity %d, %s)\n", dump.CC.Raw, dump.CC.Capacity, map[bool]string{true: "writable", false: "read-only"}[dump.CC.Writable])
	} else if dump.CC.Raw != "" {
		r.out.humanf("cc:      %s (not NDEF formatted)\n", dump.CC.Raw)
	}
	for _, page := range dump.Pages {
		if !all && page.Region != "user" {
			continue
		}
		r.printType2Page(page)
	}
	switch {
	case dump.NDEFError != "":
		r.out.humanf("ndef:    unavailable: %s\n", dump.NDEFError)
	case dump.NDEF == nil || !dump.NDEF.Present:
		r.out.humanf("ndef:    no NDEF message\n")
	default:
		r.out.humanf("ndef:    %d bytes, %d records\n", dump.NDEF.Length, len(dump.NDEF.Records))
		r.printNDEF(*dump.NDEF)
	}
}

func (r *Runner) printType2Page(page app.Type2PageDTO) {
	r.out.humanf("page %03d %-13s %s  %q\n", page.Page, page.Region, page.Hex, page.ASCII)
}

func (r *Runner) printNDEF(ndef app.Type2NDEFDTO) {
	if ndef.Summary != "" {
		r.out.humanf("summary: %s\n", ndef.Summary)
	}
	for index, record := range ndef.Records {
		r.out.humanf("record %d: %s", index+1, record.Kind)
		if record.Summary != "" {
			r.out.humanf(" · %s", record.Summary)
		}
		r.out.humanf("\n")
		if record.Text != "" && len(record.Fields) == 0 {
			r.out.humanf("  %s: %s\n", ndefFieldName("text"), record.Text)
		}
		if record.URI != "" && len(record.Fields) == 0 {
			r.out.humanf("  %s: %s\n", ndefFieldName("uri"), record.URI)
		}
		for _, field := range record.Fields {
			r.out.humanf("  %s: %s\n", ndefFieldName(field.Key), field.Value)
		}
		if len(record.Fields) == 0 && record.Text == "" && record.URI == "" {
			r.out.humanf("  type: %s\n", record.Type)
		}
	}
}

func ndefFieldName(key string) string {
	switch key {
	case "text":
		return "text"
	case "uri":
		return "uri"
	case "search_prefix":
		return "search URL prefix"
	case "query":
		return "search query"
	case "package":
		return "Android package"
	case "to":
		return "to"
	case "subject":
		return "subject"
	case "body":
		return "body"
	case "name":
		return "name"
	case "organization":
		return "organization"
	case "job_title":
		return "job title"
	case "phone":
		return "phone"
	case "email":
		return "email"
	case "address":
		return "address"
	case "url":
		return "URL"
	case "note":
		return "note"
	case "latitude":
		return "latitude"
	case "longitude":
		return "longitude"
	case "label":
		return "label"
	case "ssid":
		return "SSID"
	case "security":
		return "security"
	case "password":
		return "password"
	case "auth":
		return "authentication"
	case "encryption":
		return "encryption"
	case "network_key":
		return "network key"
	case "mac":
		return "MAC address"
	case "bt_address":
		return "Bluetooth address"
	case "bt_name":
		return "device name"
	case "bt_class":
		return "device class"
	case "payload":
		return "payload"
	case "title":
		return "title"
	case "dpp_uri":
		return "DPP bootstrapping URI"
	case "dpp_mac":
		return "DPP MAC"
	case "dpp_version":
		return "DPP version"
	case "dpp_channels":
		return "DPP channels"
	case "dpp_key":
		return "DPP public key"
	case "dpp_info":
		return "DPP information"
	default:
		return key
	}
}

func (r *Runner) printType2Write(result app.Type2WriteResultDTO, dryRun bool) {
	if dryRun {
		r.out.humanf("dry run: planned pages %v; skipped %v\n", result.Planned, result.Skipped)
		return
	}
	if result.Backup != "" {
		r.out.humanf("backup:  %s\n", result.Backup)
	}
	r.out.humanf("written: %v; verified: %v; skipped: %v\n", result.Written, result.Verified, result.Skipped)
}
