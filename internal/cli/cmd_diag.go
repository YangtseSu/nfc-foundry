package cli

import (
	"context"
	"fmt"
)

func (r *Runner) cmdDiag(_ context.Context, args []string) error {
	parsed, err := parseCommandArgs(args, nil)
	if err != nil {
		return err
	}
	if len(parsed.positionals) != 0 {
		return usagef("diag takes no positional arguments")
	}
	if r.global.dryRun {
		return usagef("diag does not support --dry-run")
	}
	report := r.service.Diagnostics()
	if r.out.mode == outputHuman {
		runtime := "unavailable"
		if report.LibNFC.Available {
			runtime = report.LibNFC.Version
			if runtime == "" {
				runtime = "available"
			}
		}
		r.out.humanf("libnfc-ng: %s\n", runtime)
		for _, check := range report.Engines {
			status := "ok"
			if !check.OK {
				status = "FAIL"
			}
			detail := ""
			if check.Detail != "" {
				detail = " (" + check.Detail + ")"
			}
			r.out.humanf("[%s] %s %s%s\n", status, check.Name, check.Version, detail)
		}
		manifest := "missing"
		switch {
		case report.Manifest.Verified:
			manifest = fmt.Sprintf("verified (%d files)", report.Manifest.Files)
		case report.Manifest.Found:
			manifest = "invalid"
		}
		r.out.humanf("manifest: %s\n", manifest)
		r.out.humanf("healthy: %t\n", report.Healthy)
	}
	r.out.emit("diag", "", nil, nil, report)
	return nil
}
