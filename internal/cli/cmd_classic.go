package cli

import (
	"context"
	"encoding/hex"
	"fmt"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func (r *Runner) cmdClassic(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl classic read|write|dump|export|restore|keyscan")
	}
	switch args[0] {
	case "read":
		r.command = "classic.read"
		return r.classicRead(ctx, args[1:])
	case "write":
		r.command = "classic.write"
		return r.classicWrite(ctx, args[1:])
	case "dump":
		r.command = "classic.dump"
		return r.classicDump(ctx, args[1:])
	case "export":
		r.command = "classic.export"
		return r.classicExport(ctx, args[1:])
	case "restore":
		r.command = "classic.restore"
		return r.classicRestore(ctx, args[1:])
	case "keyscan":
		r.command = "classic.keyscan"
		return r.classicKeyscan(ctx, args[1:])
	case "diff":
		r.command = "classic.diff"
		return r.classicDiff(ctx, args[1:])
	case "value":
		if len(args) > 1 && args[1] == "set" {
			r.command = "classic.value.set"
			return r.classicValue(ctx, args[1:])
		}
		return usagef("usage: nfctl classic value set <file> --block N --value V")
	case "trailer":
		if len(args) > 1 && args[1] == "set" {
			r.command = "classic.trailer.set"
			return r.classicTrailer(ctx, args[1:])
		}
		return usagef("usage: nfctl classic trailer set <file> --block N ...")
	case "clone":
		r.command = "classic.clone"
		return r.classicClone(ctx, args[1:])
	case "format":
		r.command = "classic.format"
		return r.classicFormat(ctx, args[1:])
	default:
		return usagef("unknown classic command %q", args[0])
	}
}

var classicReadSpecs = map[string]flagSpec{
	"block": {takesValue: true},
	"key":   {takesValue: true},
}

func (r *Runner) classicRead(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicReadSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("classic read takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("classic read does not support --dry-run")
	}
	block, err := parseBlockNumber(parsed.value("block"))
	if err != nil {
		return err
	}
	override, err := parseKeyOverride(parsed.value("key"))
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.service.ReadClassicBlock(ctx, block, override)
	if err != nil {
		return err
	}
	dto := classicReadResult{
		Block: result.Block, Sector: result.Sector, Kind: result.Kind,
		Hex: compactHex(result.Data[:]), ASCII: blockASCII(result.Data[:]),
	}
	r.out.humanf("block %d (sector %d, %s): %s  %q\n", dto.Block, dto.Sector, dto.Kind, dto.Hex, dto.ASCII)
	r.out.emit("classic.read", r.device, r.card, nil, dto)
	return nil
}

var classicWriteSpecs = map[string]flagSpec{
	"block": {takesValue: true},
	"data":  {takesValue: true},
	"key":   {takesValue: true},
}

func (r *Runner) classicWrite(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicWriteSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("classic write takes no positional arguments")
	}
	if !r.global.dryRun && !r.global.yes {
		return usagef("classic write is destructive; pass --yes")
	}
	block, err := parseBlockNumber(parsed.value("block"))
	if err != nil {
		return err
	}
	data, err := parseBlockData(parsed.value("data"))
	if err != nil {
		return err
	}
	override, err := parseKeyOverride(parsed.value("key"))
	if err != nil {
		return err
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	if r.global.dryRun {
		current, err := r.service.ReadClassicBlock(ctx, block, override)
		if err != nil {
			return err
		}
		if current.Kind == "manufacturer" {
			return usagef("block 0 writes require `nfctl uid set`")
		}
		if current.Kind == "trailer" {
			return usagef("sector trailer writes require `nfctl classic restore`")
		}
		dto := classicWriteResult{
			Block: block, Sector: current.Sector, Hex: compactHex(data[:]), DryRun: true,
			CurrentHex: compactHex(current.Data[:]),
		}
		r.out.humanf("dry run: block %d (sector %d) would be written with %s; current %s\n", block, current.Sector, dto.Hex, dto.CurrentHex)
		r.out.emit("classic.write", r.device, r.card, nil, dto)
		return nil
	}
	written, err := r.service.WriteClassicBlock(ctx, block, data, override)
	if err != nil {
		return err
	}
	dto := classicWriteResult{Block: written.Block, Sector: written.Sector, Hex: compactHex(written.Data[:]), Verified: true}
	r.out.humanf("block %d (sector %d) written and verified: %s\n", dto.Block, dto.Sector, dto.Hex)
	r.out.emit("classic.write", r.device, r.card, nil, dto)
	return nil
}

var classicDumpSpecs = map[string]flagSpec{
	"keys": {takesValue: true},
	"scan": {},
}

func (r *Runner) classicDump(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicDumpSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl classic dump <file.bin|file.mfd|file.json|file.mct|file.eml> [--keys <file>] [--scan]")
	}
	if r.global.dryRun {
		return usagef("classic dump does not support --dry-run")
	}
	path := parsed.positionals[0]
	keyScan, err := r.readClassicDump(ctx, parsed.value("keys"), parsed.bool("scan"))
	if err != nil {
		return err
	}
	saved, err := r.service.SaveDumpFile(path)
	if err != nil {
		return err
	}
	result := r.buildDumpResult(path, saved, keyScan)
	warnings := dumpWarnings(result)
	r.printDump(result)
	r.out.emit("classic.dump", r.device, r.card, warnings, result)
	return nil
}

var classicExportSpecs = map[string]flagSpec{
	"keys":   {takesValue: true},
	"scan":   {},
	"format": {takesValue: true},
}

func (r *Runner) classicExport(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicExportSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl classic export <file> --format raw|project|proxmark3|keyarray|mct|mct-json|eml|sidecar [--keys <file>] [--scan]")
	}
	if r.global.dryRun {
		return usagef("classic export does not support --dry-run")
	}
	formatName := parsed.value("format")
	if formatName == "" {
		return usagef("classic export requires --format raw|project|proxmark3|keyarray|mct|mct-json|eml|sidecar")
	}
	format, err := app.ParseDumpFormat(formatName)
	if err != nil {
		return usagef("%v", err)
	}
	path := parsed.positionals[0]
	keyScan, err := r.readClassicDump(ctx, parsed.value("keys"), parsed.bool("scan"))
	if err != nil {
		return err
	}
	saved, err := r.service.SaveDumpFileAs(path, format)
	if err != nil {
		return err
	}
	result := r.buildDumpResult(path, saved, keyScan)
	warnings := dumpWarnings(result)
	r.printDump(result)
	r.out.emit("classic.export", r.device, r.card, warnings, result)
	return nil
}

func (r *Runner) readClassicDump(ctx context.Context, keyFile string, scan bool) (*keyscanResult, error) {
	if scan && keyFile != "" {
		return nil, usagef("--scan is already implied by --keys; pass one of them")
	}
	if _, err := r.connect(ctx); err != nil {
		return nil, err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return nil, err
	}
	keyScan, err := r.importKeysAndScan(ctx, keyFile)
	if err != nil {
		return nil, err
	}
	task, err := r.service.StartReadCard(app.ReadCardStartRequestDTO{DiscardDirty: true, ScanKeys: scan})
	if err != nil {
		return nil, err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return nil, err
	}
	return keyScan, nil
}

func (r *Runner) buildDumpResult(path string, saved app.SaveResultDTO, keyScan *keyscanResult) dumpResult {
	workbench := r.service.Workbench()
	dashboard := r.service.Dashboard()
	result := dumpResult{
		Path: path, Format: saved.Format, Layout: workbench.Layout, Complete: workbench.Complete, Blocks: len(workbench.Blocks),
		ReadableBlocks: workbench.ReadableBlocks, TotalBlocks: workbench.TotalBlocks,
		ReadableSectors: workbench.ReadableSectors, TotalSectors: workbench.TotalSectors,
		SkippedSectors:   append([]int(nil), workbench.SkippedSectors...),
		VerifiedKeySlots: countVerifiedSlots(dashboard.Keys), KeySlots: len(dashboard.Keys) * 2, KeyScan: keyScan,
	}
	if saved.Format == string(app.DumpFormatSidecar) {
		result.Path = filepath.Join(filepath.Dir(path), saved.Path)
	}
	if saved.Sidecar != "" {
		result.Sidecar = filepath.Join(filepath.Dir(path), saved.Sidecar)
	}
	return result
}

var classicRestoreSpecs = map[string]flagSpec{
	"keys":             {takesValue: true},
	"scan":             {},
	"confirm-trailers": {},
}

func (r *Runner) classicRestore(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicRestoreSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl classic restore <file.bin|file.mfd|file.json> [--keys <file>] [--scan] [--yes] [--confirm-trailers] [--dry-run]")
	}
	if parsed.bool("scan") && parsed.value("keys") != "" {
		return usagef("--scan is already implied by --keys; pass one of them")
	}
	if !r.global.dryRun {
		if !r.global.yes {
			return usagef("classic restore is destructive; pass --yes")
		}
		if !parsed.bool("confirm-trailers") {
			return usagef("a full restore replaces sector trailers; pass --confirm-trailers as well")
		}
	}
	path := parsed.positionals[0]
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	keyScan, err := r.importKeysAndScan(ctx, parsed.value("keys"))
	if err != nil {
		return err
	}
	if parsed.bool("scan") {
		task, scanErr := r.service.StartKeyScan(true)
		if scanErr != nil {
			return scanErr
		}
		if _, scanErr := r.waitTask(ctx, task); scanErr != nil {
			return scanErr
		}
		summary := buildKeyscanResult(r.service.Dashboard(), r.global.showKeys)
		keyScan = &summary
	}
	workbench, err := r.service.LoadDumpFile(path, true)
	if err != nil {
		return err
	}
	preflight, err := r.service.PreflightWrite(app.WritePreflightRequestDTO{Mode: "full"})
	if err != nil {
		return err
	}
	if !preflight.Valid {
		return nfc.NewError("classic restore preflight", nfc.CodeInvalidArgument, preflight.Error, nil)
	}
	result := restoreResult{Path: path, DryRun: r.global.dryRun, Preflight: &preflight, KeyScan: keyScan, Warnings: workbench.Warnings}
	if r.global.dryRun {
		r.printRestore(result)
		r.out.emit("classic.restore", r.device, r.card, preflight.Warnings, result)
		return nil
	}
	task, err := r.service.StartWrite(app.WriteStartRequestDTO{Token: preflight.Token, ConfirmTrailers: true})
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	if r.lastProgress.Restore != nil {
		result.Restore = r.lastProgress.Restore
	}
	r.printRestore(result)
	r.out.emit("classic.restore", r.device, r.card, preflight.Warnings, result)
	return nil
}

var classicKeyscanSpecs = map[string]flagSpec{"keys": {takesValue: true}}

func (r *Runner) classicKeyscan(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, classicKeyscanSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("classic keyscan takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("classic keyscan does not support --dry-run")
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	if path := parsed.value("keys"); path != "" {
		if _, err := r.service.ImportKeyDictionaryFile(path); err != nil {
			return fmt.Errorf("import keys: %w", err)
		}
	}
	task, err := r.service.StartKeyScan(true)
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	summary := buildKeyscanResult(r.service.Dashboard(), r.global.showKeys)
	r.printKeyscan(summary)
	r.out.emit("classic.keyscan", r.device, r.card, nil, summary)
	return nil
}

func parseBlockNumber(value string) (int, error) {
	if value == "" {
		return 0, usagef("--block is required")
	}
	block, err := strconv.Atoi(value)
	if err != nil || block < 0 || block > 255 {
		return 0, usagef("invalid block %q", value)
	}
	return block, nil
}

func parseBlockData(value string) ([nfc.BlockSize]byte, error) {
	var data [nfc.BlockSize]byte
	if value == "" {
		return data, usagef("--data is required")
	}
	decoded, err := hex.DecodeString(normalizeHex(value))
	if err != nil || len(decoded) != nfc.BlockSize {
		return data, usagef("--data must be 32 hexadecimal digits")
	}
	copy(data[:], decoded)
	return data, nil
}

func parseKeyOverride(value string) (*app.ClassicKeyOverride, error) {
	if value == "" {
		return nil, nil
	}
	keyType := nfc.KeyTypeA
	hexValue := value
	if prefix, rest, ok := strings.Cut(value, ":"); ok {
		switch strings.ToUpper(prefix) {
		case "A":
			keyType = nfc.KeyTypeA
		case "B":
			keyType = nfc.KeyTypeB
		default:
			return nil, usagef("--key must be A:HEX or B:HEX")
		}
		hexValue = rest
	}
	key, err := keys.ParseKey(normalizeHex(hexValue))
	if err != nil {
		return nil, usagef("--key must contain exactly 12 hexadecimal digits")
	}
	return &app.ClassicKeyOverride{KeyType: keyType, Key: key}, nil
}

func compactHex(data []byte) string {
	return strings.ToUpper(hex.EncodeToString(data))
}

func blockASCII(data []byte) string {
	result := make([]byte, len(data))
	for index, value := range data {
		if value >= 0x20 && value <= 0x7e {
			result[index] = value
			continue
		}
		result[index] = '.'
	}
	return string(result)
}

func countVerifiedSlots(sectors []app.SectorKeyDTO) int {
	count := 0
	for _, sector := range sectors {
		if sector.KeyAStatus == "verified" {
			count++
		}
		if sector.KeyBStatus == "verified" {
			count++
		}
	}
	return count
}

func dumpWarnings(result dumpResult) []string {
	if result.Complete && result.VerifiedKeySlots == result.KeySlots {
		return []string{}
	}
	warnings := make([]string, 0, 2)
	if !result.Complete {
		warnings = append(warnings, fmt.Sprintf("the dump is partial: %d of %d blocks readable, %d sectors skipped; run classic keyscan or recover to fill unknown blocks",
			result.ReadableBlocks, result.TotalBlocks, len(result.SkippedSectors)))
	}
	if result.VerifiedKeySlots < result.KeySlots {
		warnings = append(warnings, fmt.Sprintf("%d of %d key slots are verified; some sectors may be unreadable", result.VerifiedKeySlots, result.KeySlots))
	}
	return warnings
}
