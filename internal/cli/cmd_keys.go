package cli

import (
	"context"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
)

func (r *Runner) cmdKeys(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl keys list|add|import|export|rename|sets|set|reload")
	}
	switch args[0] {
	case "list":
		return r.keysList(ctx, args[1:])
	case "add":
		return r.keysAdd(ctx, args[1:])
	case "import":
		return r.keysImport(ctx, args[1:])
	case "export":
		return r.keysExport(ctx, args[1:])
	case "rename":
		return r.keysRename(ctx, args[1:])
	case "sets":
		return r.keysSets(ctx, args[1:])
	case "set":
		return r.keysSet(ctx, args[1:])
	case "reload":
		return r.keysReload(ctx, args[1:])
	default:
		return usagef("unknown keys command %q", args[0])
	}
}

func (r *Runner) keysList(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("keys list takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("keys list does not support --dry-run")
	}
	result := keysResult{Keys: keySummaries(r.service.KeyCatalog(), r.global.showKeys), ValuesIncluded: r.global.showKeys}
	r.printKeys(result)
	r.out.emit("keys.list", "", nil, nil, result)
	return nil
}

func (r *Runner) keysAdd(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys add <A:HEX|B:HEX|HEX>")
	}
	if r.global.dryRun {
		return usagef("keys add does not support --dry-run")
	}
	value := parsed.positionals[0]
	if prefix, rest, ok := strings.Cut(value, ":"); ok {
		switch strings.ToUpper(prefix) {
		case "A", "B":
			value = rest
		default:
			return usagef("key must be A:HEX, B:HEX, or bare HEX")
		}
	}
	compact := normalizeHex(value)
	if _, err := keys.ParseKey(compact); err != nil {
		return usagef("key must contain exactly 12 hexadecimal digits")
	}
	before := len(r.service.KeyCatalog())
	if _, err := r.service.AddKey(compact); err != nil {
		return err
	}
	catalog := r.service.KeyCatalog()
	added := len(catalog) > before
	result := keyAddedResult{Added: added, Total: len(catalog)}
	for _, key := range catalog {
		if normalizeHex(key.Value) == compact {
			summaries := keySummaries([]app.KeyDTO{key}, r.global.showKeys)
			if len(summaries) != 0 {
				result.Key = summaries[0]
			}
			break
		}
	}
	if added {
		r.out.humanf("key added; %d keys in the catalog\n", result.Total)
	} else {
		r.out.humanf("key already present; %d keys in the catalog\n", result.Total)
	}
	r.out.emit("keys.add", "", nil, nil, result)
	return nil
}

var keysImportSpecs = map[string]flagSpec{
	"set":    {takesValue: true},
	"format": {takesValue: true},
}

func (r *Runner) keysImport(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, keysImportSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys import <file> [--set NAME] [--format dic|keys|keyarray]")
	}
	if r.global.dryRun {
		return usagef("keys import does not support --dry-run")
	}
	format := strings.ToLower(strings.TrimSpace(parsed.value("format")))
	if format == "" {
		format = "dic"
	}
	if format != "dic" && format != "keys" && format != "keyarray" {
		return usagef("--format must be dic, keys, or keyarray")
	}
	path := parsed.positionals[0]
	report, err := r.service.ImportKeyFileWithSet(path, format, parsed.value("set"))
	if err != nil {
		return err
	}
	result := keysImportResult{
		Path: path, Format: format, Valid: report.Valid, Added: report.Added, Duplicates: report.Duplicates,
		Ignored: report.Ignored, Issues: report.Issues, Keys: len(report.Keys), KeySet: report.KeySet,
	}
	r.out.humanf("imported %s (%s): %d valid, %d added, %d duplicates, %d ignored, %d catalog keys\n",
		path, format, result.Valid, result.Added, result.Duplicates, result.Ignored, result.Keys)
	if result.KeySet != nil {
		r.out.humanf("key set %q now holds %d keys\n", result.KeySet.Name, result.KeySet.KeyCount)
	}
	for _, issue := range report.Issues {
		r.out.humanf("line %d: %s\n", issue.Line, issue.Message)
	}
	r.out.emit("keys.import", "", nil, nil, result)
	return nil
}

var keysExportSpecs = map[string]flagSpec{
	"set":    {takesValue: true},
	"format": {takesValue: true},
}

func (r *Runner) keysExport(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, keysExportSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys export <file> [--set NAME] [--format dic|keys]")
	}
	if r.global.dryRun {
		return usagef("keys export does not support --dry-run")
	}
	path := parsed.positionals[0]
	format := strings.ToLower(strings.TrimSpace(parsed.value("format")))
	if format == "" {
		format = "dic"
	}
	if format != "dic" && format != "keys" {
		return usagef("--format must be dic or keys")
	}
	result := keysExportResult{Path: path, Format: format, Set: parsed.value("set")}
	if result.Set != "" {
		exported, err := r.service.ExportKeySetFile(result.Set, format, path)
		if err != nil {
			return err
		}
		result.Count = exported.Count
		r.out.humanf("exported %d keys from %q to %s (%s)\n", result.Count, result.Set, path, format)
	} else {
		report, err := r.service.ExportKeyDictionaryFile(path)
		if err != nil {
			return err
		}
		result.Count = report.Count
		r.out.humanf("exported %d keys to %s (%s)\n", result.Count, path, format)
	}
	r.out.emit("keys.export", "", nil, nil, result)
	return nil
}

func (r *Runner) keysRename(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 2 {
		return usagef("usage: nfctl keys rename <key-id> <name>")
	}
	if r.global.dryRun {
		return usagef("keys rename does not support --dry-run")
	}
	id := parsed.positionals[0]
	name := parsed.positionals[1]
	catalog, err := r.service.RenameKey(id, name)
	if err != nil {
		return err
	}
	result := keyRenamedResult{}
	for _, key := range catalog {
		if key.ID == id {
			summaries := keySummaries([]app.KeyDTO{key}, r.global.showKeys)
			if len(summaries) != 0 {
				result.Key = summaries[0]
			}
			break
		}
	}
	r.out.humanf("renamed %s to %q\n", id, name)
	r.out.emit("keys.rename", "", nil, nil, result)
	return nil
}

func (r *Runner) keysSets(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("keys sets takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("keys sets does not support --dry-run")
	}
	sets := r.service.KeySets()
	result := keySetsResult{Sets: sets}
	r.printKeySets(result)
	r.out.emit("keys.sets", "", nil, nil, result)
	return nil
}

var keySetSpecs = map[string]flagSpec{
	"key":    {takesValue: true},
	"card":   {takesValue: true},
	"format": {takesValue: true},
	"output": {takesValue: true},
}

func (r *Runner) keysSet(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl keys set create|rename|delete|export ...")
	}
	switch args[0] {
	case "create":
		return r.keysSetCreate(ctx, args[1:])
	case "rename":
		return r.keysSetRename(ctx, args[1:])
	case "delete":
		return r.keysSetDelete(ctx, args[1:])
	case "export":
		return r.keysSetExport(ctx, args[1:])
	default:
		return usagef("unknown keys set command %q", args[0])
	}
}

func (r *Runner) keysSetCreate(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, keySetSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys set create <name> --key <id>... [--card <cardId>]")
	}
	if r.global.dryRun {
		return usagef("keys set create does not support --dry-run")
	}
	keyIDs := parsed.values["key"]
	if len(keyIDs) == 0 {
		return usagef("keys set create requires at least one --key <id>")
	}
	sets, err := r.service.CreateKeySet(parsed.positionals[0], parsed.value("card"), keyIDs)
	if err != nil {
		return err
	}
	result := keySetsResult{Sets: sets}
	r.out.humanf("key set %q now holds %d keys\n", parsed.positionals[0], keySetCount(sets, parsed.positionals[0], parsed.value("card")))
	r.out.emit("keys.set.create", "", nil, nil, result)
	return nil
}

func (r *Runner) keysSetRename(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 2 {
		return usagef("usage: nfctl keys set rename <name> <new-name>")
	}
	if r.global.dryRun {
		return usagef("keys set rename does not support --dry-run")
	}
	set, err := r.service.ResolveKeySet(parsed.positionals[0])
	if err != nil {
		return err
	}
	sets, err := r.service.RenameKeySet(set.ID, parsed.positionals[1])
	if err != nil {
		return err
	}
	result := keySetsResult{Sets: sets}
	r.out.humanf("renamed key set %s to %q\n", set.ID, parsed.positionals[1])
	r.out.emit("keys.set.rename", "", nil, nil, result)
	return nil
}

func (r *Runner) keysSetDelete(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys set delete <name>")
	}
	if r.global.dryRun {
		return usagef("keys set delete does not support --dry-run")
	}
	if !r.global.yes {
		return usagef("keys set delete removes user data; pass --yes")
	}
	set, err := r.service.ResolveKeySet(parsed.positionals[0])
	if err != nil {
		return err
	}
	sets, err := r.service.DeleteKeySet(set.ID)
	if err != nil {
		return err
	}
	result := keySetsResult{Sets: sets}
	r.out.humanf("deleted key set %q; key records were not removed\n", set.Name)
	r.out.emit("keys.set.delete", "", nil, nil, result)
	return nil
}

func (r *Runner) keysSetExport(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, keySetSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys set export <name> [--format dic|keys|json] [--output <file>]")
	}
	if r.global.dryRun {
		return usagef("keys set export does not support --dry-run")
	}
	name := parsed.positionals[0]
	format := strings.ToLower(parsed.value("format"))
	if format == "" {
		format = "dic"
	}
	if format != "dic" && format != "keys" && format != "json" {
		return usagef("--format must be dic, keys, or json")
	}
	path := parsed.value("output")
	if path == "" && !r.global.showKeys {
		return usagef("key values are secret; pass --output <file> or --show-keys to write them to stdout")
	}
	result := keySetExportResult{Name: name, Format: format, Path: path}
	if path != "" {
		exported, err := r.service.ExportKeySetFile(name, format, path)
		if err != nil {
			return err
		}
		result.Count = exported.Count
		r.out.humanf("exported %d keys from %q to %s\n", result.Count, name, path)
	} else {
		content, set, count, err := r.service.KeySetArtifact(name, format)
		if err != nil {
			return err
		}
		result.Name, result.Count, result.Content = set.Name, count, content
		r.out.humanf("%s", content)
	}
	r.out.emit("keys.set.export", "", nil, nil, result)
	return nil
}

func (r *Runner) keysReload(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("keys reload takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("keys reload does not support --dry-run")
	}
	result, err := r.service.ReloadKeyStore()
	if err != nil {
		return err
	}
	r.out.humanf("reloaded %d keys, %d verifications, and %d key sets from %s\n",
		result.Keys, result.Verifications, result.KeySets, result.Path)
	if len(result.PrunedKeyIDs) != 0 || len(result.DroppedKeySets) != 0 {
		r.out.humanf("pruned %d key references and dropped %d empty key sets\n",
			len(result.PrunedKeyIDs), len(result.DroppedKeySets))
	}
	r.out.emit("keys.reload", "", nil, nil, result)
	return nil
}

func keySetCount(sets []app.KeySetDTO, name, cardID string) int {
	name = strings.TrimSpace(name)
	for _, set := range sets {
		if set.Name == name && set.CardID == cardID {
			return set.KeyCount
		}
	}
	return 0
}

func (r *Runner) printKeySets(result keySetsResult) {
	if len(result.Sets) == 0 {
		r.out.humanf("no key sets\n")
		return
	}
	for _, set := range result.Sets {
		scope := set.CardID
		if scope == "" {
			scope = "all cards"
		}
		r.out.humanf("%s  %q  %d keys  %s  %s\n", set.ID, set.Name, set.KeyCount, scope, set.Source)
	}
}
