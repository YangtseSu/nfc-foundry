package cli

import (
	"context"

	"github.com/YangtseSu/nfc-foundry/app"
)

var recoverFlagSpecs = map[string]flagSpec{"keys": {takesValue: true}}

func (r *Runner) cmdRecover(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, recoverFlagSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl recover nested|darkside|hardnested --yes [--keys <file>]")
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
		task, err = r.service.StartMFoC(app.MFoCStartRequestDTO{Authorized: true})
	case "darkside":
		task, err = r.service.StartMFCUKPipeline(app.MFCUKStartRequestDTO{Authorized: true})
	case "hardnested":
		task, err = r.service.StartHardnested(app.HardnestedStartRequestDTO{Authorized: true})
	}
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	result := recoverResult{Mode: mode, KeyScan: keyScan}
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
	r.out.emit(r.command, r.device, r.card, nil, result)
	return nil
}
