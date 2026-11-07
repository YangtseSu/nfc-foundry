package cli

import (
	"context"
	"encoding/hex"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

var uidFlagSpecs = map[string]flagSpec{"confirm-uid": {takesValue: true}}

func (r *Runner) cmdUID(ctx context.Context, args []string) error {
	if len(args) == 0 || args[0] != "set" {
		return usagef("usage: nfctl uid set <4-byte-hex> --yes [--confirm-uid <4-byte-hex>] [--dry-run]")
	}
	parsed, err := parseCommandArgs(args[1:], uidFlagSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 1 {
		return usagef("usage: nfctl uid set <4-byte-hex> --yes [--confirm-uid <4-byte-hex>] [--dry-run]")
	}
	newUID, err := normalizeUID(parsed.positionals[0])
	if err != nil {
		return err
	}
	confirmation := normalizeHex(parsed.value("confirm-uid"))
	if !r.global.dryRun {
		if !r.global.yes {
			return usagef("uid set is destructive; pass --yes")
		}
		if confirmation == "" {
			return usagef("uid set requires --confirm-uid <4-byte-hex> with the exact target UID")
		}
		if confirmation != newUID {
			return usagef("--confirm-uid does not match the target UID")
		}
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	preflight, err := r.service.PreflightUIDWrite(newUID)
	if err != nil {
		return err
	}
	if !preflight.Valid {
		return nfc.NewError("uid preflight", nfc.CodeInvalidArgument, preflight.Error, nil)
	}
	result := uidResult{DryRun: r.global.dryRun, Preview: preflight.Preview, BackupPath: preflight.BackupPath}
	if r.global.dryRun {
		r.printUID(result)
		r.out.emit("uid.set", r.device, r.card, nil, result)
		return nil
	}
	task, err := r.service.StartUIDWrite(app.UIDWriteStartRequestDTO{Token: preflight.Token, Confirmation: newUID})
	if err != nil {
		return err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return err
	}
	if r.lastProgress.UID != nil {
		result.Result = r.lastProgress.UID
	}
	if r.card != nil && result.Result != nil && result.Result.NewUID != "" {
		updated := *r.card
		updated.UID = result.Result.NewUID
		r.card = &updated
	}
	r.printUID(result)
	r.out.emit("uid.set", r.device, r.card, nil, result)
	return nil
}

func normalizeUID(value string) (string, error) {
	compact := normalizeHex(value)
	if len(compact) != 8 {
		return "", usagef("UID must be 8 hexadecimal digits (4 bytes)")
	}
	decoded, err := hex.DecodeString(compact)
	if err != nil || len(decoded) != 4 {
		return "", usagef("UID must be 8 hexadecimal digits (4 bytes)")
	}
	return compact, nil
}
