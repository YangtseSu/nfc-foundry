package cli

import (
	"context"
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

type valueWriteResult struct {
	Path      string                   `json:"path"`
	DryRun    bool                     `json:"dryRun"`
	Block     int                      `json:"block"`
	OldHex    string                   `json:"oldHex"`
	NewHex    string                   `json:"newHex"`
	Preview   app.ValueBlockPreviewDTO `json:"preview"`
	Preflight *app.WritePreflightDTO   `json:"preflight,omitempty"`
	Restore   *app.RestoreResultDTO    `json:"restore,omitempty"`
}

type trailerWriteResult struct {
	Path      string                 `json:"path"`
	DryRun    bool                   `json:"dryRun"`
	Block     int                    `json:"block"`
	OldHex    string                 `json:"oldHex"`
	NewHex    string                 `json:"newHex"`
	Preview   app.TrailerPreviewDTO  `json:"preview"`
	Preflight *app.WritePreflightDTO `json:"preflight,omitempty"`
	Restore   *app.RestoreResultDTO  `json:"restore,omitempty"`
}

type classicCloneResult struct {
	Path      string                 `json:"path"`
	DryRun    bool                   `json:"dryRun"`
	Preflight *app.ClonePreflightDTO `json:"preflight"`
	Result    *app.CloneResultDTO    `json:"result,omitempty"`
}

type classicFactoryResult struct {
	DryRun    bool                     `json:"dryRun"`
	Preflight *app.FactoryPreflightDTO `json:"preflight"`
	Result    *app.FactoryResultDTO    `json:"result,omitempty"`
}

var classicDiffSpecs = map[string]flagSpec{}

func (r *Runner) classicDiff(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicDiffSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 2 {
		return usagef("usage: nfctl classic diff <before> <after> [--json]")
	}
	if r.global.dryRun {
		return usagef("classic diff does not support --dry-run")
	}
	diff, err := r.service.CompareDumpFiles(parsed.positionals[0], parsed.positionals[1])
	if err != nil {
		return err
	}
	r.printDumpDiff(diff)
	r.out.emit("classic.diff", "", nil, nil, diff)
	return nil
}

func (r *Runner) printDumpDiff(diff app.DumpDiffDTO) {
	r.out.humanf("layout %s; changed %d, equal %d, unknown %d, trailer changes %d\n",
		diff.Layout, diff.ChangedBlocks, diff.EqualBlocks, diff.UnknownBlocks, diff.TrailerChanges)
	if diff.IdentityChanged {
		r.out.humanf("identity before: UID %s ATQA %s SAK %s\n", diff.Before.UID, diff.Before.ATQA, diff.Before.SAK)
		r.out.humanf("identity after:  UID %s ATQA %s SAK %s\n", diff.After.UID, diff.After.ATQA, diff.After.SAK)
	}
	if !diff.Before.Complete || !diff.After.Complete {
		r.out.humanf("completeness before %d/%d blocks, after %d/%d blocks\n",
			diff.Before.ReadableBlocks, diff.Before.TotalBlocks, diff.After.ReadableBlocks, diff.After.TotalBlocks)
	}
	for _, block := range diff.Blocks {
		if block.State == "equal" {
			continue
		}
		r.out.humanf("block %d (sector %d, %s, %s)\n  before %s  %q\n  after  %s  %q\n",
			block.Block, block.Sector, block.Kind, block.State, block.BeforeHex, block.BeforeASCII, block.AfterHex, block.AfterASCII)
		if block.Trailer == nil {
			continue
		}
		if block.Trailer.KeyAChanged {
			r.out.humanf("  Key A %s -> %s\n", block.Trailer.BeforeKeyA, block.Trailer.AfterKeyA)
		}
		if block.Trailer.KeyBChanged {
			r.out.humanf("  Key B %s -> %s\n", block.Trailer.BeforeKeyB, block.Trailer.AfterKeyB)
		}
		if block.Trailer.AccessChanged {
			r.out.humanf("  access %s -> %s\n", block.Trailer.BeforeAccess, block.Trailer.AfterAccess)
		}
		if block.Trailer.GPBChanged {
			r.out.humanf("  GPB %s -> %s\n", block.Trailer.BeforeGPB, block.Trailer.AfterGPB)
		}
		if !block.Trailer.Valid && block.Trailer.Error != "" {
			r.out.humanf("  warning: %s\n", block.Trailer.Error)
		}
	}
}

var classicValueSetSpecs = map[string]flagSpec{
	"block":   {takesValue: true},
	"value":   {takesValue: true},
	"address": {takesValue: true},
	"keys":    {takesValue: true},
	"scan":    {},
}

func (r *Runner) classicValue(ctx context.Context, args []string) error {
	if len(args) == 0 || args[0] != "set" {
		return usagef("usage: nfctl classic value set <file> --block N --value V [--address N] [--keys <file>] [--scan] [--yes] [--dry-run]")
	}
	parsed, err := parseCommandArgs(args[1:], classicValueSetSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl classic value set <file> --block N --value V [--address N] [--keys <file>] [--scan] [--yes] [--dry-run]")
	}
	if !r.global.dryRun && !r.global.yes {
		return usagef("classic value set writes a card; pass --yes")
	}
	block, err := parseBlockNumber(parsed.value("block"))
	if err != nil {
		return err
	}
	if strings.TrimSpace(parsed.value("value")) == "" {
		return usagef("--value is required")
	}
	address := -1
	if raw := parsed.value("address"); raw != "" {
		address, err = strconv.Atoi(raw)
		if err != nil || address < 0 || address > 255 {
			return usagef("--address must be between 0 and 255")
		}
	}
	path := parsed.positionals[0]
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	if err := r.importAndScanKeys(ctx, parsed.value("keys"), parsed.bool("scan")); err != nil {
		return err
	}
	workbench, err := r.service.LoadDumpFile(path, true)
	if err != nil {
		return err
	}
	if block < 0 || block >= len(workbench.Blocks) {
		return usagef("block %d is outside the loaded dump", block)
	}
	oldHex := workbench.Blocks[block].Hex
	preview, err := r.service.PreviewValueBlock(app.ValueBlockPreviewRequestDTO{Block: block, Value: parsed.value("value"), Address: address})
	if err != nil {
		return err
	}
	if _, err := r.service.EditBlockHex(block, preview.Hex); err != nil {
		return err
	}
	preflight, err := r.service.PreflightWrite(app.WritePreflightRequestDTO{Mode: "block", Blocks: []int{block}})
	if err != nil {
		return err
	}
	if !preflight.Valid {
		return nfc.NewError("value write preflight", nfc.CodeInvalidArgument, preflight.Error, nil)
	}
	result := valueWriteResult{Path: path, DryRun: r.global.dryRun, Block: block, OldHex: oldHex, NewHex: preview.Hex, Preview: preview, Preflight: &preflight}
	if r.global.dryRun {
		r.printValueWrite(result)
		r.out.emit("classic.value.set", r.device, r.card, preflight.Warnings, result)
		return nil
	}
	task, err := r.service.StartWrite(app.WriteStartRequestDTO{Token: preflight.Token})
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	if r.lastProgress.Restore != nil {
		result.Restore = r.lastProgress.Restore
	}
	r.printValueWrite(result)
	r.out.emit("classic.value.set", r.device, r.card, preflight.Warnings, result)
	return nil
}

func (r *Runner) printValueWrite(result valueWriteResult) {
	r.out.humanf("block %d value preview: %s -> %s\n", result.Block, result.OldHex, result.NewHex)
	r.out.humanf("decoded value %d, address %d\n", result.Preview.Decoded.Value, result.Preview.Decoded.Address)
	if result.Preview.Warning != "" {
		r.out.humanf("warning: %s\n", result.Preview.Warning)
	}
	if result.Preflight != nil && result.Preflight.BackupPath != "" {
		r.out.humanf("pre-write backup: %s\n", result.Preflight.BackupPath)
	}
	if result.DryRun {
		r.out.humanf("dry run: the card was not modified\n")
		return
	}
	if result.Restore != nil {
		r.out.humanf("value write: %d written, %d verified\n", len(result.Restore.WrittenBlocks), len(result.Restore.VerifiedBlocks))
	}
}

var classicTrailerSetSpecs = map[string]flagSpec{
	"block":   {takesValue: true},
	"key-a":   {takesValue: true},
	"key-b":   {takesValue: true},
	"access":  {takesValue: true},
	"gpb":     {takesValue: true},
	"confirm": {takesValue: true},
	"keys":    {takesValue: true},
	"scan":    {},
}

func (r *Runner) classicTrailer(ctx context.Context, args []string) error {
	if len(args) == 0 || args[0] != "set" {
		return usagef("usage: nfctl classic trailer set <file> --block N [--key-a HEX] [--key-b HEX] [--access G0,G1,G2,G3] [--gpb XX] --yes --confirm N [--keys <file>] [--scan] [--dry-run]")
	}
	parsed, err := parseCommandArgs(args[1:], classicTrailerSetSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl classic trailer set <file> --block N [--key-a HEX] [--key-b HEX] [--access G0,G1,G2,G3] [--gpb XX] --yes --confirm N [--keys <file>] [--scan] [--dry-run]")
	}
	block, err := parseBlockNumber(parsed.value("block"))
	if err != nil {
		return err
	}
	confirm := strconv.Itoa(block)
	if !r.global.dryRun {
		if !r.global.yes {
			return usagef("classic trailer set is destructive; pass --yes")
		}
		if parsed.value("confirm") != confirm {
			return usagef("classic trailer set requires --confirm %s (the trailer block number)", confirm)
		}
	}
	path := parsed.positionals[0]
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	if err := r.importAndScanKeys(ctx, parsed.value("keys"), parsed.bool("scan")); err != nil {
		return err
	}
	workbench, err := r.service.LoadDumpFile(path, true)
	if err != nil {
		return err
	}
	if block < 0 || block >= len(workbench.Blocks) {
		return usagef("block %d is outside the loaded dump", block)
	}
	current := workbench.Blocks[block]
	if current.Kind != "trailer" {
		return usagef("block %d is not a sector trailer", block)
	}
	oldHex := current.Hex
	keyA := parsed.value("key-a")
	if keyA == "" {
		keyA = oldHex[0:12]
	}
	keyB := parsed.value("key-b")
	if keyB == "" {
		keyB = oldHex[20:32]
	}
	groups, err := parseAccessGroups(parsed.value("access"), current.Access)
	if err != nil {
		return err
	}
	preview, err := r.service.PreviewTrailer(app.TrailerPreviewRequestDTO{
		Block: block, KeyA: keyA, KeyB: keyB, GPB: parsed.value("gpb"), Groups: groups,
	})
	if err != nil {
		return err
	}
	if _, err := r.service.EditBlockHex(block, preview.Hex); err != nil {
		return err
	}
	preflight, err := r.service.PreflightWrite(app.WritePreflightRequestDTO{Mode: "trailer", Blocks: []int{block}})
	if err != nil {
		return err
	}
	if !preflight.Valid {
		return nfc.NewError("trailer write preflight", nfc.CodeInvalidArgument, preflight.Error, nil)
	}
	result := trailerWriteResult{Path: path, DryRun: r.global.dryRun, Block: block, OldHex: oldHex, NewHex: preview.Hex, Preview: preview, Preflight: &preflight}
	if r.global.dryRun {
		r.printTrailerWrite(result)
		r.out.emit("classic.trailer.set", r.device, r.card, preflight.Warnings, result)
		return nil
	}
	task, err := r.service.StartWrite(app.WriteStartRequestDTO{Token: preflight.Token, ConfirmTrailers: true, Confirmation: confirm})
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	if r.lastProgress.Restore != nil {
		result.Restore = r.lastProgress.Restore
	}
	r.printTrailerWrite(result)
	r.out.emit("classic.trailer.set", r.device, r.card, preflight.Warnings, result)
	return nil
}

func parseAccessGroups(value string, current *app.AccessSummaryDTO) ([]app.AccessConditionInputDTO, error) {
	if strings.TrimSpace(value) == "" {
		if current == nil || len(current.Groups) != 4 {
			return nil, usagef("--access is required because the current trailer has no decodable access conditions")
		}
		groups := make([]app.AccessConditionInputDTO, 4)
		for index, group := range current.Groups {
			code := group.Code
			if len(code) != 3 {
				return nil, usagef("current trailer access group %d is not decodable", index)
			}
			groups[index] = app.AccessConditionInputDTO{Group: index, C1: code[0] == '1', C2: code[1] == '1', C3: code[2] == '1'}
		}
		return groups, nil
	}
	parts := strings.Split(value, ",")
	if len(parts) != 4 {
		return nil, usagef("--access must contain four 3-bit group codes such as 000,000,000,001")
	}
	groups := make([]app.AccessConditionInputDTO, 4)
	for index, part := range parts {
		part = strings.TrimSpace(part)
		if len(part) != 3 {
			return nil, usagef("--access group %d must contain three bits", index)
		}
		for _, bit := range part {
			if bit != '0' && bit != '1' {
				return nil, usagef("--access group %d must contain only 0 and 1", index)
			}
		}
		groups[index] = app.AccessConditionInputDTO{Group: index, C1: part[0] == '1', C2: part[1] == '1', C3: part[2] == '1'}
	}
	return groups, nil
}

func (r *Runner) printTrailerWrite(result trailerWriteResult) {
	r.out.humanf("trailer %d preview: %s -> %s\n", result.Block, result.OldHex, result.NewHex)
	r.out.humanf("Key A %s, Key B %s, GPB %s\n", result.Preview.KeyA, result.Preview.KeyB, result.Preview.GPB)
	for _, group := range result.Preview.Access.Groups {
		if group.Group == 3 {
			readable := "no"
			if group.KeyBReadable {
				readable = "yes"
			}
			r.out.humanf("group 3 %s: Key A write %s, access write %s, Key B write %s, Key B readable %s\n",
				group.Code, group.TrailerKeyA, group.TrailerAccess, group.TrailerKeyB, readable)
			continue
		}
		r.out.humanf("group %d %s: read %s, write %s, increment %s, decrement %s\n",
			group.Group, group.Code, group.DataReadKeys, group.DataWriteKeys, group.IncrementKeys, group.DecrementKeys)
	}
	if result.Preflight != nil {
		for _, warning := range result.Preflight.Warnings {
			r.out.humanf("warning: %s\n", warning)
		}
		if result.Preflight.BackupPath != "" {
			r.out.humanf("pre-write backup: %s\n", result.Preflight.BackupPath)
		}
	}
	if result.DryRun {
		r.out.humanf("dry run: the card was not modified\n")
		return
	}
	if result.Restore != nil {
		r.out.humanf("trailer write: %d written, %d verified\n", len(result.Restore.WrittenBlocks), len(result.Restore.VerifiedBlocks))
	}
}

var classicCloneSpecs = map[string]flagSpec{
	"keys":    {takesValue: true},
	"scan":    {},
	"confirm": {takesValue: true},
}

func (r *Runner) classicClone(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicCloneSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl classic clone <file> [--keys <file>] [--scan] --yes --confirm <source-uid> [--dry-run]")
	}
	if !r.global.dryRun {
		if !r.global.yes {
			return usagef("classic clone is destructive; pass --yes")
		}
		if strings.TrimSpace(parsed.value("confirm")) == "" {
			return usagef("classic clone requires --confirm <source-uid>")
		}
	}
	path := parsed.positionals[0]
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	if err := r.importAndScanKeys(ctx, parsed.value("keys"), parsed.bool("scan")); err != nil {
		return err
	}
	if _, err := r.service.LoadDumpFile(path, true); err != nil {
		return err
	}
	preflight, err := r.service.PreflightClone()
	if err != nil {
		return err
	}
	if !preflight.Valid {
		return nfc.NewError("clone preflight", nfc.CodeInvalidArgument, preflight.Error, nil)
	}
	result := classicCloneResult{Path: path, DryRun: r.global.dryRun, Preflight: &preflight}
	if r.global.dryRun {
		r.printClone(result)
		r.out.emit("classic.clone", r.device, r.card, preflight.Warnings, result)
		return nil
	}
	if normalizeHex(parsed.value("confirm")) != normalizeHex(preflight.Source.UID) {
		return usagef("--confirm must repeat the source UID %s", preflight.Source.UID)
	}
	task, err := r.service.StartClone(app.CloneStartRequestDTO{Token: preflight.Token, Confirmation: normalizeHex(preflight.Source.UID)})
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	if r.lastProgress.Clone != nil {
		result.Result = r.lastProgress.Clone
	}
	r.printClone(result)
	r.out.emit("classic.clone", r.device, r.card, preflight.Warnings, result)
	return nil
}

func (r *Runner) printClone(result classicCloneResult) {
	if preflight := result.Preflight; preflight != nil {
		r.out.humanf("clone source UID %s -> target UID %s\n", preflight.Source.UID, preflight.Target.UID)
		r.out.humanf("clone plan: %d blocks, %d trailers, block 0 %s\n", len(preflight.Blocks), len(preflight.TrailerBlocks), block0Action(preflight.Block0Write))
		if preflight.BackupPath != "" {
			r.out.humanf("pre-write backup: %s\n", preflight.BackupPath)
		}
	}
	if result.DryRun {
		r.out.humanf("dry run: the card was not modified\n")
		return
	}
	if result.Result != nil {
		r.out.humanf("clone verified: %t\n", result.Result.Verified)
		if result.Result.Diff != nil {
			r.out.humanf("read-back diff: changed %d, equal %d, unknown %d\n",
				result.Result.Diff.ChangedBlocks, result.Result.Diff.EqualBlocks, result.Result.Diff.UnknownBlocks)
		}
	}
}

func block0Action(written bool) string {
	if written {
		return "written through the magic-card path"
	}
	return "already matches"
}

// formatSkippedSectors compresses only consecutive sector runs.
func formatSkippedSectors(sectors []int) string {
	if len(sectors) == 0 {
		return ""
	}
	sorted := append([]int(nil), sectors...)
	sort.Ints(sorted)
	parts := make([]string, 0, len(sorted))
	start, previous := sorted[0], sorted[0]
	flush := func() {
		if start == previous {
			parts = append(parts, strconv.Itoa(start))
			return
		}
		parts = append(parts, fmt.Sprintf("%d-%d", start, previous))
	}
	for _, sector := range sorted[1:] {
		if sector == previous {
			continue
		}
		if sector == previous+1 {
			previous = sector
			continue
		}
		flush()
		start, previous = sector, sector
	}
	flush()
	return strings.Join(parts, ", ")
}

var classicFormatSpecs = map[string]flagSpec{
	"keep-data": {},
	"keys":      {takesValue: true},
	"scan":      {},
	"confirm":   {takesValue: true},
}

func (r *Runner) classicFormat(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicFormatSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("classic format takes no positional arguments")
	}
	if !r.global.dryRun {
		if !r.global.yes {
			return usagef("classic format is destructive; pass --yes")
		}
		if !strings.EqualFold(strings.TrimSpace(parsed.value("confirm")), "FORMAT") {
			return usagef("classic format requires --confirm FORMAT")
		}
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	if err := r.importAndScanKeys(ctx, parsed.value("keys"), parsed.bool("scan")); err != nil {
		return err
	}
	preflight, err := r.service.PreflightFactoryFormat(parsed.bool("keep-data"))
	if err != nil {
		return err
	}
	if !preflight.Valid {
		return nfc.NewError("factory format preflight", nfc.CodeInvalidArgument, preflight.Error, nil)
	}
	result := classicFactoryResult{DryRun: r.global.dryRun, Preflight: &preflight}
	if r.global.dryRun {
		r.printFactory(result)
		r.out.emit("classic.format", r.device, r.card, preflight.Warnings, result)
		return nil
	}
	task, err := r.service.StartFactoryFormat(app.FactoryStartRequestDTO{Token: preflight.Token, Confirmation: "FORMAT"})
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	if r.lastProgress.Factory != nil {
		result.Result = r.lastProgress.Factory
	}
	r.printFactory(result)
	r.out.emit("classic.format", r.device, r.card, preflight.Warnings, result)
	return nil
}

func (r *Runner) printFactory(result classicFactoryResult) {
	if preflight := result.Preflight; preflight != nil {
		r.out.humanf("factory format: %d blocks, %d trailers, keep data %t, block 0 not written\n",
			len(preflight.Blocks), len(preflight.TrailerBlocks), preflight.KeepData)
		r.out.humanf("data blocks: %s\ntrailer blocks: %s\n", formatSkippedSectors(preflight.Blocks), formatSkippedSectors(preflight.TrailerBlocks))
		r.out.humanf("trailer bytes: %s\n", preflight.TrailerBytes)
		if preflight.BackupPath != "" {
			r.out.humanf("pre-write backup: %s\n", preflight.BackupPath)
		}
	}
	if result.DryRun {
		r.out.humanf("dry run: the card was not modified\n")
		return
	}
	if result.Result != nil {
		r.out.humanf("factory format verified: %t\n", result.Result.Verified)
		if result.Result.Diff != nil {
			r.out.humanf("read-back diff: changed %d, unknown %d\n", result.Result.Diff.ChangedBlocks, result.Result.Diff.UnknownBlocks)
		}
	}
}

func (r *Runner) importAndScanKeys(ctx context.Context, keyFile string, scan bool) error {
	if scan && keyFile != "" {
		return usagef("--scan is already implied by --keys; pass one of them")
	}
	if keyFile != "" {
		if _, err := r.service.ImportKeyDictionaryFile(keyFile); err != nil {
			return fmt.Errorf("import keys: %w", err)
		}
	}
	if scan {
		task, err := r.service.StartKeyScan(true)
		if err != nil {
			return err
		}
		if _, err := r.waitTask(ctx, task); err != nil {
			return err
		}
	}
	return nil
}
