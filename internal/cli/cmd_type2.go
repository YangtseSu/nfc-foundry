package cli

import (
	"context"
	"encoding/hex"
	"os"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
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
	case "copy":
		r.command = "type2.copy"
		return r.type2Copy(ctx, args[1:])
	case "erase":
		r.command = "type2.erase"
		return r.type2Erase(ctx, args[1:])
	case "lock":
		r.command = "type2.lock"
		return r.type2Lock(ctx, args[1:])
	case "password":
		return r.type2Password(ctx, args[1:])
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
	"pwd":    {takesValue: true},
	"raw":    {},
	"reveal": {},
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
		r.printNDEF(*dump.NDEF, parsed.bool("reveal"))
	}
	result := ndefReadResult{CC: dump.CC, NDEF: dump.NDEF, Raw: parsed.bool("raw")}
	r.out.emit("ndef.read", r.device, r.card, nil, result)
	return nil
}

var ndefWriteSpecs = map[string]flagSpec{
	"file":     {takesValue: true},
	"hex":      {takesValue: true},
	"template": {takesValue: true},
	"field":    {takesValue: true},
	"pwd":      {takesValue: true},
}

func (r *Runner) ndefWrite(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, ndefWriteSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("ndef write takes no positional arguments")
	}
	file, hexValue, template := parsed.value("file"), parsed.value("hex"), parsed.value("template")
	sources := 0
	for _, source := range []string{file, hexValue, template} {
		if source != "" {
			sources++
		}
	}
	if sources != 1 {
		return usagef("exactly one of --file, --hex, or --template is required")
	}
	if !r.global.dryRun && !r.global.yes {
		return usagef("ndef write is destructive; pass --yes")
	}
	var message []byte
	if template != "" {
		fields, err := parseNDEFTemplateFields(parsed.values["field"])
		if err != nil {
			return err
		}
		message, err = type2.EncodeTemplate(template, fields)
		if err != nil {
			return usagef("invalid NDEF template: %v", err)
		}
	} else {
		message, err = readNDEFMessage(file, hexValue)
		if err != nil {
			return err
		}
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

func parseNDEFTemplateFields(values []string) (map[string]string, error) {
	fields := make(map[string]string, len(values))
	for _, entry := range values {
		key, value, ok := strings.Cut(entry, "=")
		if !ok || strings.TrimSpace(key) == "" {
			return nil, usagef("--field must use key=value, got %q", entry)
		}
		fields[strings.TrimSpace(key)] = value
	}
	return fields, nil
}

var type2CopySpecs = map[string]flagSpec{
	"include-protected": {},
	"same-uid-only":     {},
	"repeat":            {takesValue: true},
	"continue-on-error": {},
	"pwd":               {takesValue: true},
	"confirm-copy":      {},
	"confirm-protected": {},
}

func (r *Runner) type2Copy(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2CopySpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl type2 copy SOURCE [--include-protected] [--same-uid-only] [--repeat N] [--continue-on-error] [--pwd HEX] [--yes] [--confirm-copy] [--confirm-protected] [--dry-run]")
	}
	includeProtected := parsed.bool("include-protected")
	if !r.global.dryRun {
		if !r.global.yes || !parsed.bool("confirm-copy") {
			return usagef("type2 copy overwrites the target; pass --yes and --confirm-copy")
		}
		if includeProtected && !parsed.bool("confirm-protected") {
			return usagef("copying configuration, password, or capability pages requires --confirm-protected")
		}
	}
	repeat, err := parseRepeatCount(parsed.value("repeat"))
	if err != nil {
		return err
	}
	if r.global.dryRun {
		repeat = 1
	}
	source, err := r.service.LoadType2DumpFile(parsed.positionals[0])
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if repeat == 1 {
		if _, err := r.waitForCard(ctx); err != nil {
			return err
		}
		request := app.Type2LifecycleRequestDTO{
			Operation: "copy", Source: &source, IncludeProtected: includeProtected,
			SameUIDOnly: parsed.bool("same-uid-only"), CurrentPassword: parsed.value("pwd"),
			DryRun: r.global.dryRun,
		}
		result, err := r.runType2Lifecycle(ctx, request)
		if err != nil {
			return err
		}
		r.out.emit("type2.copy", r.device, r.card, nil, result)
		return nil
	}
	return r.runRepeatedCopy(ctx, source, app.Type2LifecycleRequestDTO{
		Operation: "copy", Source: &source, IncludeProtected: includeProtected,
		SameUIDOnly: parsed.bool("same-uid-only"), CurrentPassword: parsed.value("pwd"), DryRun: r.global.dryRun,
	}, repeat, parsed.bool("continue-on-error"))
}

type type2CopyTarget struct {
	Card   app.CardDTO             `json:"card"`
	Result app.Type2WriteResultDTO `json:"result"`
	Error  string                  `json:"error,omitempty"`
}

func (r *Runner) runRepeatedCopy(ctx context.Context, source type2.Document, request app.Type2LifecycleRequestDTO, repeat int, continueOnError bool) error {
	var targets []type2CopyTarget
	var firstErr error
	for index := 0; index < repeat; index++ {
		if index > 0 {
			if err := r.service.WaitForCardRemoval(ctx); err != nil {
				return err
			}
		}
		if _, err := r.waitForCard(ctx); err != nil {
			return err
		}
		card := app.CardDTO{}
		if r.card != nil {
			card = *r.card
		}
		request.Token = ""
		plan, err := r.service.PreviewType2Lifecycle(ctx, request)
		var result app.Type2WriteResultDTO
		if err == nil {
			request.Token = plan.Token
			r.printType2Plan(plan)
			result, err = r.service.ExecuteType2Lifecycle(ctx, request)
			if err == nil {
				r.printType2Write(result, false)
			}
		}
		target := type2CopyTarget{Card: card, Result: result}
		if err != nil {
			target.Error = err.Error()
			if firstErr == nil {
				firstErr = err
			}
		}
		targets = append(targets, target)
		if err != nil && !continueOnError {
			return err
		}
	}
	if firstErr != nil {
		return firstErr
	}
	r.out.emit("type2.copy", r.device, r.card, nil, map[string]any{"sourceUid": source.UID, "targets": targets})
	return nil
}

func parseRepeatCount(value string) (int, error) {
	if strings.TrimSpace(value) == "" {
		return 1, nil
	}
	repeat, err := strconv.Atoi(value)
	if err != nil || repeat < 1 || repeat > 1000 {
		return 0, usagef("--repeat must be an integer between 1 and 1000")
	}
	return repeat, nil
}

var type2EraseSpecs = map[string]flagSpec{
	"pwd":           {takesValue: true},
	"confirm-erase": {},
}

func (r *Runner) type2Erase(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2EraseSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("type2 erase takes no positional arguments")
	}
	if !r.global.dryRun && (!r.global.yes || !parsed.bool("confirm-erase")) {
		return usagef("type2 erase destroys user memory; pass --yes and --confirm-erase")
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.runType2Lifecycle(ctx, app.Type2LifecycleRequestDTO{
		Operation: "erase", CurrentPassword: parsed.value("pwd"), DryRun: r.global.dryRun,
	})
	if err != nil {
		return err
	}
	r.out.emit("type2.erase", r.device, r.card, nil, result)
	return nil
}

var type2LockSpecs = map[string]flagSpec{
	"static-lock":  {takesValue: true},
	"dynamic-lock": {takesValue: true},
	"cc-readonly":  {},
	"cfglock":      {},
	"pwd":          {takesValue: true},
	"confirm-lock": {},
}

func (r *Runner) type2Lock(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2LockSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("type2 lock takes no positional arguments")
	}
	if parsed.value("static-lock") == "" && parsed.value("dynamic-lock") == "" &&
		!parsed.bool("cc-readonly") && !parsed.bool("cfglock") {
		return usagef("type2 lock needs --static-lock, --dynamic-lock, --cc-readonly, or --cfglock")
	}
	if !r.global.dryRun && (!r.global.yes || !parsed.bool("confirm-lock")) {
		return usagef("type2 lock bits are one-way; pass --yes and --confirm-lock")
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.runType2Lifecycle(ctx, app.Type2LifecycleRequestDTO{
		Operation: "lock", StaticLock: parsed.value("static-lock"), DynamicLock: parsed.value("dynamic-lock"),
		CCReadOnly: parsed.bool("cc-readonly"), ConfigLock: parsed.bool("cfglock"),
		CurrentPassword: parsed.value("pwd"), DryRun: r.global.dryRun,
	})
	if err != nil {
		return err
	}
	r.out.emit("type2.lock", r.device, r.card, nil, result)
	return nil
}

func (r *Runner) type2Password(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl type2 password set|remove")
	}
	switch args[0] {
	case "set":
		r.command = "type2.password.set"
		return r.type2PasswordSet(ctx, args[1:])
	case "remove":
		r.command = "type2.password.remove"
		return r.type2PasswordRemove(ctx, args[1:])
	default:
		return usagef("unknown type2 password command %q", args[0])
	}
}

var type2PasswordSetSpecs = map[string]flagSpec{
	"new-pwd":          {takesValue: true},
	"pack":             {takesValue: true},
	"auth0":            {takesValue: true},
	"protect":          {},
	"no-protect":       {},
	"current-pwd":      {takesValue: true},
	"confirm-password": {},
}

func (r *Runner) type2PasswordSet(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2PasswordSetSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("type2 password set takes no positional arguments")
	}
	if parsed.value("new-pwd") == "" || parsed.value("pack") == "" {
		return usagef("type2 password set requires --new-pwd and --pack")
	}
	if !r.global.dryRun && (!r.global.yes || !parsed.bool("confirm-password")) {
		return usagef("type2 password set can lock the tag; pass --yes and --confirm-password")
	}
	auth0, err := parseOptionalAuth0(parsed.value("auth0"))
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.runType2Lifecycle(ctx, app.Type2LifecycleRequestDTO{
		Operation: "password-set", NewPassword: parsed.value("new-pwd"), NewPack: parsed.value("pack"),
		Auth0: auth0, Protect: !parsed.bool("no-protect"), CurrentPassword: parsed.value("current-pwd"),
		DryRun: r.global.dryRun,
	})
	if err != nil {
		return err
	}
	r.out.emit("type2.password.set", r.device, r.card, nil, result)
	return nil
}

var type2PasswordRemoveSpecs = map[string]flagSpec{
	"current-pwd":      {takesValue: true},
	"confirm-password": {},
}

func (r *Runner) type2PasswordRemove(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, type2PasswordRemoveSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("type2 password remove takes no positional arguments")
	}
	if !r.global.dryRun && (!r.global.yes || !parsed.bool("confirm-password")) {
		return usagef("type2 password remove changes configuration; pass --yes and --confirm-password")
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.runType2Lifecycle(ctx, app.Type2LifecycleRequestDTO{
		Operation: "password-remove", CurrentPassword: parsed.value("current-pwd"), DryRun: r.global.dryRun,
	})
	if err != nil {
		return err
	}
	r.out.emit("type2.password.remove", r.device, r.card, nil, result)
	return nil
}

func parseOptionalAuth0(value string) (*int, error) {
	if strings.TrimSpace(value) == "" {
		return nil, nil
	}
	page, err := strconv.Atoi(value)
	if err != nil || page < 0 || page > 255 {
		return nil, usagef("--auth0 must be a page between 0 and 255")
	}
	return &page, nil
}

func (r *Runner) runType2Lifecycle(ctx context.Context, request app.Type2LifecycleRequestDTO) (app.Type2WriteResultDTO, error) {
	plan, err := r.service.PreviewType2Lifecycle(ctx, request)
	if err != nil {
		return app.Type2WriteResultDTO{}, err
	}
	r.printType2Plan(plan)
	if r.global.dryRun {
		return app.Type2WriteResultDTO{
			Planned: pageNumbers(plan.Writes), DryRun: true, Changes: plan.Changes,
			Operation: plan.Operation, SourceUID: plan.SourceUID, Token: plan.Token,
		}, nil
	}
	request.Token = plan.Token
	result, err := r.service.ExecuteType2Lifecycle(ctx, request)
	if err != nil {
		return result, err
	}
	r.printType2Write(result, false)
	return result, nil
}

func pageNumbers(writes []app.Type2PageWriteDTO) []int {
	pages := make([]int, len(writes))
	for index, write := range writes {
		pages[index] = write.Page
	}
	return pages
}

func (r *Runner) printType2Plan(plan app.Type2LifecyclePlanDTO) {
	r.out.humanf("plan:    %s on %s (%s)\n", plan.Operation, plan.CardUID, plan.ProductName)
	if plan.SourceUID != "" {
		r.out.humanf("source:  %s\n", plan.SourceUID)
	}
	for _, change := range plan.Changes {
		oneWay := ""
		if change.OneWay {
			oneWay = " (one-way)"
		}
		r.out.humanf("  %-22s %s -> %s%s\n", change.Target, change.Old, change.New, oneWay)
	}
	r.out.humanf("pages:   %d planned\n", len(plan.Writes))
	if plan.BackupRoot != "" {
		r.out.humanf("backup:  %s\n", plan.BackupRoot)
	}
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
		r.printNDEF(*dump.NDEF, false)
	}
}

func (r *Runner) printType2Page(page app.Type2PageDTO) {
	r.out.humanf("page %03d %-13s %s  %q\n", page.Page, page.Region, page.Hex, page.ASCII)
}

func (r *Runner) printNDEF(ndef app.Type2NDEFDTO, reveal bool) {
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
			value := field.Value
			if field.Sensitive && !reveal {
				value = "••••••"
			}
			r.out.humanf("  %s: %s\n", ndefFieldName(field.Key), value)
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
