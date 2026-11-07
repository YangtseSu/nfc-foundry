package cli

import (
	"context"
	"strconv"
	"strings"
)

func (r *Runner) cmdAPDU(ctx context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, map[string]flagSpec{"raw": {takesValue: true}})
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("apdu takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("apdu does not support --dry-run")
	}
	raw := parsed.value("raw")
	if strings.TrimSpace(raw) == "" {
		return usagef("--raw is required")
	}
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	result, err := r.service.TransmitAPDU(ctx, raw)
	if err != nil {
		return err
	}
	r.out.humanf("ats:     %s (TL %d, FSC %d, FWI %d)\n", result.ATS.Raw, result.ATS.TL, result.ATS.FSC, result.ATS.FWI)
	r.out.humanf("sw:      %s\n", result.SW)
	if result.Data != "" {
		r.out.humanf("data:    %s\n", result.Data)
	}
	r.out.emit("apdu.raw", r.device, r.card, nil, result)
	return nil
}

func (r *Runner) cmdDESFire(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return usagef("usage: nfctl desfire info|apps|files AID|read AID FILE")
	}
	r.command = "desfire." + args[0]
	if _, err := r.connect(ctx); err != nil {
		return err
	}
	if _, err := r.waitForCard(ctx); err != nil {
		return err
	}
	switch args[0] {
	case "info":
		return r.desfireInfo(ctx)
	case "apps":
		return r.desfireApps(ctx)
	case "files":
		if len(args) != 2 {
			return usagef("usage: nfctl desfire files AID")
		}
		return r.desfireFiles(ctx, args[1])
	case "read":
		if len(args) != 3 {
			return usagef("usage: nfctl desfire read AID FILE")
		}
		return r.desfireRead(ctx, args[1], args[2])
	default:
		return usagef("unknown desfire command %q", args[0])
	}
}

func (r *Runner) desfireInfo(ctx context.Context) error {
	info, err := r.service.DESFireInfo(ctx)
	if err != nil {
		return err
	}
	r.out.humanf("vendor:  %02X\n", info.Vendor)
	r.out.humanf("type:    %02X/%02X\n", info.Type, info.Subtype)
	r.out.humanf("version: %s\n", info.Version)
	r.out.humanf("storage: %02X\n", info.Storage)
	r.out.humanf("protocol: %02X\n", info.Protocol)
	r.out.emit("desfire.info", r.device, r.card, nil, info)
	return nil
}

func (r *Runner) desfireApps(ctx context.Context) error {
	apps, err := r.service.DESFireApplications(ctx)
	if err != nil {
		return err
	}
	for _, aid := range apps {
		r.out.humanf("%s\n", aid)
	}
	r.out.emit("desfire.apps", r.device, r.card, nil, struct {
		Applications []string `json:"applications"`
	}{Applications: apps})
	return nil
}

func (r *Runner) desfireFiles(ctx context.Context, aid string) error {
	files, err := r.service.DESFireFiles(ctx, aid)
	if err != nil {
		return err
	}
	r.out.humanf("application: %s\n", files.AID)
	for _, file := range files.Files {
		r.out.humanf("file %02X: %s, communication %d, access %04X", file.ID, file.Type, file.Communication, file.AccessRights)
		if file.Size != 0 {
			r.out.humanf(", size %d", file.Size)
		}
		if file.RecordSize != 0 {
			r.out.humanf(", records %d/%d, record size %d", file.CurrentRecords, file.MaxRecords, file.RecordSize)
		}
		r.out.humanf("\n")
	}
	r.out.emit("desfire.files", r.device, r.card, nil, files)
	return nil
}

func (r *Runner) desfireRead(ctx context.Context, aid, fileValue string) error {
	file, err := strconv.Atoi(strings.TrimSpace(fileValue))
	if err != nil || file < 0 || file > 255 {
		return usagef("FILE must be a byte between 0 and 255")
	}
	data, err := r.service.DESFireReadFile(ctx, aid, byte(file))
	if err != nil {
		return err
	}
	r.out.humanf("application: %s\n", data.AID)
	r.out.humanf("file:        %02X (%d bytes)\n", data.File, data.Length)
	r.out.humanf("hex:         %s\n", data.Hex)
	r.out.humanf("ascii:       %q\n", data.ASCII)
	r.out.emit("desfire.read", r.device, r.card, nil, data)
	return nil
}
