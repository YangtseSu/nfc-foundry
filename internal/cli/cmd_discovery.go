package cli

import (
	"context"

	"github.com/YangtseSu/nfc-foundry/app"
)

func (r *Runner) cmdList(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("list takes no arguments")
	}
	devices, err := r.listDevices(ctx)
	if err != nil {
		return err
	}
	result := deviceListResult{Devices: devices}
	if len(devices) == 0 {
		r.out.humanf("no readers found\n")
	}
	for _, device := range devices {
		r.out.humanf("%s  %s\n", device.ConnString, device.Name)
	}
	r.out.emit("list", "", nil, nil, result)
	return nil
}

var scanFlagSpecs = map[string]flagSpec{"watch": {}}

func (r *Runner) cmdScan(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, scanFlagSpecs)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("scan takes no positional arguments")
	}
	watch := parsed.bool("watch")
	if watch && r.global.json {
		return usagef("scan --watch streams events; use --jsonl or human output instead of --json")
	}
	if r.global.dryRun {
		return usagef("scan does not support --dry-run")
	}
	device, err := r.connect(ctx)
	if err != nil {
		return err
	}
	if watch {
		r.command = "scan.watch"
		return r.watchCards(ctx, device)
	}
	card, err := r.waitForCard(ctx)
	if err != nil {
		return err
	}
	result := cardResult{Device: device, Card: card}
	r.printCard(device, card)
	r.out.emit("scan", device.ConnString, &card, nil, result)
	return nil
}

func (r *Runner) cmdCard(ctx context.Context, args []string) error {
	if len(args) == 0 || args[0] != "info" {
		return usagef("usage: nfctl card info")
	}
	parsed, err := parseCommandArgs(args[1:], nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("card info takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("card info does not support --dry-run")
	}
	device, err := r.connect(ctx)
	if err != nil {
		return err
	}
	card, err := r.waitForCard(ctx)
	if err != nil {
		return err
	}
	result := cardResult{Device: device, Card: card}
	r.printCard(device, card)
	if card.TypeCode == "nfc_type2" {
		if info, infoErr := r.service.Type2Info(ctx); infoErr == nil {
			result.Type2 = &info
			r.out.humanf("product: %s\n", info.ProductName)
		}
	}
	r.out.emit("card.info", device.ConnString, &card, nil, result)
	return nil
}

func (r *Runner) printCard(device app.DeviceDTO, card app.CardDTO) {
	r.out.humanf("reader: %s\n", device.ConnString)
	r.out.humanf("card:   %s\n", card.Type)
	r.out.humanf("uid:    %s\n", card.UID)
	r.out.humanf("atqa:   %s\n", card.ATQA)
	r.out.humanf("sak:    %s\n", card.SAK)
	if device.Nested || device.Darkside || device.Hardnested {
		r.out.humanf("recovery: nested=%t darkside=%t hardnested=%t\n", device.Nested, device.Darkside, device.Hardnested)
	}
}

func (r *Runner) watchCards(ctx context.Context, device app.DeviceDTO) error {
	r.out.humanf("watching %s; press Ctrl-C to stop\n", device.ConnString)
	for {
		select {
		case <-ctx.Done():
			card := r.refreshCard()
			result := map[string]any{"device": device, "stopped": true}
			if card != nil {
				result["card"] = *card
			}
			r.out.emit("scan.watch", device.ConnString, card, nil, result)
			return nil
		case event := <-r.hub.cards:
			r.out.cardEvent(event, device.ConnString)
		case state := <-r.hub.states:
			if state.Connection.Status == "disconnected" {
				return nil
			}
			if state.Connection.Status == "error" && r.global.verbose {
				r.out.warnf("reader: %s", state.Connection.Detail)
			}
		}
	}
}

func (r *Runner) keySlotLabel(verified bool) string {
	if verified {
		return "verified"
	}
	return "missing"
}

func (r *Runner) printKeyscan(summary keyscanResult) {
	r.out.humanf("verified %d/%d key slots\n", summary.Verified, summary.Total)
	for _, sector := range summary.Sectors {
		if sector.KeyAStatus == "verified" && sector.KeyBStatus == "verified" {
			continue
		}
		r.out.humanf("sector %d: Key A %s, Key B %s\n", sector.Sector, r.keySlotLabel(sector.KeyAStatus == "verified"), r.keySlotLabel(sector.KeyBStatus == "verified"))
	}
}

func (r *Runner) printDump(result dumpResult) {
	r.out.humanf("wrote %s\n", result.Path)
	if result.Sidecar != "" {
		r.out.humanf("metadata %s\n", result.Sidecar)
	}
	completeness := "complete"
	if !result.Complete {
		completeness = "incomplete"
	}
	r.out.humanf("layout %s, %d blocks, %s, %d/%d key slots verified\n", result.Layout, result.Blocks, completeness, result.VerifiedKeySlots, result.KeySlots)
	if result.KeyScan != nil {
		r.printKeyscan(*result.KeyScan)
	}
}

func (r *Runner) printRestore(result restoreResult) {
	preflight := result.Preflight
	if preflight == nil {
		return
	}
	r.out.humanf("restore plan: %d blocks, %d trailers, block 0 %s\n",
		len(preflight.Blocks), len(preflight.TrailerBlocks), skippedLabel(preflight.SkippedBlock0))
	if preflight.BackupPath != "" {
		r.out.humanf("pre-write backup: %s\n", preflight.BackupPath)
	}
	if result.Restore != nil {
		r.out.humanf("restore result: %d written, %d verified, %d unwritten\n",
			len(result.Restore.WrittenBlocks), len(result.Restore.VerifiedBlocks), len(result.Restore.UnwrittenBlocks))
	}
	if result.KeyScan != nil {
		r.printKeyscan(*result.KeyScan)
	}
}

func skippedLabel(skipped bool) string {
	if skipped {
		return "skipped"
	}
	return "written last"
}

func (r *Runner) printRecover(result recoverResult) {
	switch result.Mode {
	case "darkside":
		if result.MFCUK != nil {
			r.out.humanf("darkside: %d candidates, %d verified, %d existing, %d rejected, %d conflicts; MFOC added %d keys\n",
				result.MFCUK.Candidates, result.MFCUK.Verified, result.MFCUK.Existing, result.MFCUK.Rejected, result.MFCUK.Conflicts, result.MFCUK.MFoCAdded)
			r.out.humanf("filled %d dump bytes, %d data conflicts, %d key conflicts\n", result.MFCUK.FilledBytes, result.MFCUK.DataConflicts, result.MFCUK.KeyConflicts)
		}
	default:
		if result.MFoC != nil {
			r.out.humanf("%s: %d claimed, %d verified, %d existing, %d rejected, %d conflicts\n",
				result.Mode, result.MFoC.Claimed, result.MFoC.Added, result.MFoC.Existing, result.MFoC.Rejected, result.MFoC.Conflicts)
			r.out.humanf("filled %d dump bytes, %d data conflicts, %d key conflicts\n", result.MFoC.FilledBytes, result.MFoC.DataConflicts, result.MFoC.KeyConflicts)
		}
	}
	if result.KeyScan != nil {
		r.printKeyscan(*result.KeyScan)
	}
}

func (r *Runner) printUID(result uidResult) {
	if result.Preview != nil {
		r.out.humanf("UID %s -> %s\n", result.Preview.CurrentUID, result.Preview.NewUID)
		r.out.humanf("BCC %s, block 0 %s -> %s\n", result.Preview.BCC, result.Preview.OldBlock0, result.Preview.NewBlock0)
	}
	if result.BackupPath != "" {
		r.out.humanf("pre-write backup: %s\n", result.BackupPath)
	}
	if result.DryRun {
		r.out.humanf("dry run: the card was not modified\n")
		return
	}
	if result.Result != nil {
		r.out.humanf("UID write verified: %t\n", result.Result.Verified)
		if result.Result.BackupPath != "" {
			r.out.humanf("block 0 backup: %s\n", result.Result.BackupPath)
		}
	}
}

func (r *Runner) printKeys(result keysResult) {
	for _, key := range result.Keys {
		value := ""
		if result.ValuesIncluded {
			value = "  " + key.Value
		}
		builtin := ""
		if key.Builtin {
			builtin = "  builtin"
		}
		r.out.humanf("%s%s  %v%s\n", key.ID, value, key.Sources, builtin)
	}
}
