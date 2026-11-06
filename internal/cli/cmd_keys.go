package cli

import (
	"context"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
)

func (r *Runner) cmdKeys(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl keys list|add|import|export")
	}
	switch args[0] {
	case "list":
		r.command = "keys.list"
		return r.keysList(ctx, args[1:])
	case "add":
		r.command = "keys.add"
		return r.keysAdd(ctx, args[1:])
	case "import":
		r.command = "keys.import"
		return r.keysImport(ctx, args[1:])
	case "export":
		r.command = "keys.export"
		return r.keysExport(ctx, args[1:])
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

func (r *Runner) keysImport(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys import <file>")
	}
	if r.global.dryRun {
		return usagef("keys import does not support --dry-run")
	}
	path := parsed.positionals[0]
	report, err := r.service.ImportKeyDictionaryFile(path)
	if err != nil {
		return err
	}
	result := keysImportResult{
		Path: path, Valid: report.Valid, Added: report.Added, Duplicates: report.Duplicates,
		Ignored: report.Ignored, Issues: report.Issues, Keys: len(report.Keys),
	}
	r.out.humanf("imported %s: %d valid, %d added, %d duplicates, %d ignored, %d catalog keys\n",
		path, result.Valid, result.Added, result.Duplicates, result.Ignored, result.Keys)
	for _, issue := range report.Issues {
		r.out.humanf("line %d: %s\n", issue.Line, issue.Message)
	}
	r.out.emit("keys.import", "", nil, nil, result)
	return nil
}

func (r *Runner) keysExport(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl keys export <file>")
	}
	if r.global.dryRun {
		return usagef("keys export does not support --dry-run")
	}
	path := parsed.positionals[0]
	report, err := r.service.ExportKeyDictionaryFile(path)
	if err != nil {
		return err
	}
	result := keysExportResult{Path: path, Count: report.Count}
	r.out.humanf("exported %d keys to %s\n", result.Count, result.Path)
	r.out.emit("keys.export", "", nil, nil, result)
	return nil
}
