package cli

import (
	"context"

	"github.com/YangtseSu/nfc-foundry/app"
)

var recoverFlagSpecs = map[string]flagSpec{
	"keys":    {takesValue: true},
	"name":    {takesValue: true},
	"key-set": {takesValue: true},
	"no-save": {},
}

func (r *Runner) cmdRecover(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, recoverFlagSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl recover nested|darkside|hardnested --yes [--keys <file>] [--name NAME] [--key-set NAME] [--no-save]")
	}
	mode := parsed.positionals[0]
	switch mode {
	case "nested", "darkside", "hardnested":
	default:
		return usagef("unknown recovery mode %q; use nested, darkside, or hardnested", mode)
	}
	if !r.global.yes {
		return usagef("recover runs an active card attack; pass --yes to authorize it")
	}
	if r.global.dryRun {
		return usagef("recover does not support --dry-run")
	}
	keysPath := parsed.value("keys")
	if mode == "darkside" && keysPath != "" {
		return usagef("recover darkside requires no verified keys and does not accept --keys")
	}
	keyName := parsed.value("name")
	keySetName := parsed.value("key-set")
	noSave := parsed.bool("no-save")
	if noSave && (keyName != "" || keySetName != "") {
		return usagef("--no-save cannot be combined with --name or --key-set")
	}
	if noSave && keysPath != "" {
		return usagef("--no-save cannot be combined with --keys because importing a dictionary persists it")
	}
	policy := app.RecoverySavePersist
	if noSave {
		policy = app.RecoverySaveNone
	}
	r.command = "recover." + mode
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	keyScan, err := r.importKeysAndScan(ctx, keysPath)
	if err != nil {
		return err
	}
	var task app.TaskDTO
	switch mode {
	case "nested":
		task, err = r.service.StartMFoC(app.MFoCStartRequestDTO{
			Authorized: true, SavePolicy: policy, KeyName: keyName, KeySetName: keySetName,
		})
	case "darkside":
		task, err = r.service.StartMFCUKPipeline(app.MFCUKStartRequestDTO{
			Authorized: true, SavePolicy: policy, KeyName: keyName, KeySetName: keySetName,
		})
	case "hardnested":
		task, err = r.service.StartHardnested(app.HardnestedStartRequestDTO{
			Authorized: true, SavePolicy: policy, KeyName: keyName, KeySetName: keySetName,
		})
	}
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	result := recoverResult{
		Mode: mode, KeyScan: keyScan, Recovered: r.lastProgress.Recovered,
		Persisted: !noSave, KeyName: keyName, KeySetName: keySetName,
	}
	if keyName != "" && len(result.Recovered) != 1 {
		result.Warning = "--name applies only when exactly one recovered key is available; no key was renamed"
	}
	if keySetName != "" && len(result.Recovered) == 0 {
		if result.Warning != "" {
			result.Warning += "; "
		}
		result.Warning += "no key was recovered, so the key set was not changed"
	}
	if progress := r.lastProgress; progress.MFoC != nil {
		result.MFoC = progress.MFoC
		result.Engine = progress.Engine
		result.Version = progress.Version
	} else if progress.MFCUK != nil {
		result.MFCUK = progress.MFCUK
		result.Engine = "mfcuk"
		result.Version = progress.MFCUK.MFCUKVersion
	}
	r.printRecover(result)
	var warnings []string
	if result.Warning != "" {
		warnings = append(warnings, result.Warning)
	}
	r.out.emit(r.command, r.device, r.card, warnings, result)
	return nil
}
