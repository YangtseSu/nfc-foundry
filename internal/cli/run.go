package cli

import (
	"context"
	"encoding/json"
	"fmt"
	"io"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/buildinfo"
)

// Dependencies lets tests replace the service factory while production uses
// the normal libnfc backend and per-user configuration.
type Dependencies struct {
	NewService func(app.EventEmitter) *app.Service
}

type Runner struct {
	service      *app.Service
	hub          *eventHub
	out          *output
	global       globalOptions
	command      string
	device       string
	card         *app.CardDTO
	lastProgress app.TaskEventDetailDTO
}

// Run executes one CLI invocation and returns its process exit code.
func Run(ctx context.Context, args []string, stdout, stderr io.Writer, deps Dependencies) int {
	global, rest, err := extractGlobal(args)
	if err != nil {
		return newOutput(stdout, stderr, global).fail("", "", nil, err)
	}
	if global.version {
		printVersion(stdout, global)
		return exitOK
	}
	if global.help {
		printUsage(stdout)
		return exitOK
	}
	if len(rest) == 0 {
		printUsage(stdout)
		return exitOK
	}

	newService := deps.NewService
	if newService == nil {
		newService = app.NewService
	}
	hub := newEventHub()
	service := newService(hub)
	if service == nil {
		return newOutput(stdout, stderr, global).fail("", "", nil, fmt.Errorf("application service is unavailable"))
	}
	defer service.Shutdown()

	out := newOutput(stdout, stderr, global)
	runner := &Runner{service: service, hub: hub, out: out, global: global}
	if global.timeout > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, global.timeout)
		defer cancel()
	}
	if err := runner.dispatch(ctx, rest); err != nil {
		return out.fail(runner.command, runner.device, runner.card, err)
	}
	return exitOK
}

func (r *Runner) dispatch(ctx context.Context, args []string) error {
	switch args[0] {
	case "list":
		r.command = "list"
		return r.cmdList(ctx, args[1:])
	case "scan":
		r.command = "scan"
		return r.cmdScan(ctx, args[1:])
	case "card":
		r.command = "card.info"
		return r.cmdCard(ctx, args[1:])
	case "classic":
		return r.cmdClassic(ctx, args[1:])
	case "type2":
		return r.cmdType2(ctx, args[1:])
	case "ndef":
		return r.cmdNDEF(ctx, args[1:])
	case "apdu":
		r.command = "apdu.raw"
		return r.cmdAPDU(ctx, args[1:])
	case "desfire":
		return r.cmdDESFire(ctx, args[1:])
	case "recover":
		return r.cmdRecover(ctx, args[1:])
	case "uid":
		r.command = "uid.set"
		return r.cmdUID(ctx, args[1:])
	case "diag":
		r.command = "diag"
		return r.cmdDiag(ctx, args[1:])
	case "keys":
		return r.cmdKeys(ctx, args[1:])
	case "help":
		printUsage(r.out.stdout)
		return nil
	default:
		return usagef("unknown command %q; run `nfctl --help`", args[0])
	}
}

func printVersion(w io.Writer, global globalOptions) {
	info := buildinfo.Current()
	if global.json || global.jsonl {
		encoder := json.NewEncoder(w)
		encoder.SetEscapeHTML(false)
		if global.json {
			encoder.SetIndent("", "  ")
		}
		_ = encoder.Encode(info)
		return
	}
	fmt.Fprintf(w, "nfctl %s (%s, %s)\n", info.Version, info.Commit, info.GoVersion)
}

func printUsage(w io.Writer) {
	fmt.Fprint(w, `nfctl - NFC Foundry command-line workbench

Usage:
  nfctl <command> [options] [arguments]

Commands:
  list                              Enumerate NFC readers
  scan [--watch]                    Select a reader and report the card
  card info                         Print the current card facts
  classic read --block N            Read one Classic block
  classic write --block N --data H  Write one data block (requires --yes)
  classic dump FILE [--scan]        Read the card into a dump file
  classic restore FILE [--scan]     Restore a complete dump (requires --yes)
  classic keyscan                   Verify known keys on the card
  type2 info                        Detect the Type 2 product
  type2 read [--page N] [--all]     Read Type 2 pages
  type2 write --page N --data H     Write one user page (requires --yes)
  type2 dump FILE                   Save a Type 2 JSON or binary dump
  type2 restore FILE                Restore a Type 2 JSON or binary dump (requires --yes)
  type2 copy FILE                   Copy a dump to the current tag (requires --yes)
  type2 erase                       Erase user memory (requires --yes)
  type2 lock                        Apply lock bits (requires --yes)
  type2 password set|remove         Set or remove password protection
  ndef read [--keys F] [--raw] [--reveal]
                                     Read Type 2 or Classic NDEF
  ndef write --file F|--hex H|--template ID --field K=V [--keys F]
                                     Write an NDEF message (requires --yes)
  ndef format --sectors N-M         Format a Classic NDEF application
                                     (requires --yes --confirm-format)
  apdu --raw HEX                    Send one diagnostic ISO-DEP APDU
  desfire info|apps|files AID       Read-only DESFire information
  desfire read AID FILE             Read one standard DESFire file
  recover nested|darkside|hardnested  Run an external key-recovery engine
                                     [--name NAME] [--key-set NAME] [--no-save]
  uid set UID                       Write a 4-byte UID on a supported card
  diag                              Run the release self-check
  keys list|add|import|export|rename|sets|set|reload
                                     Manage keys, names, and named key sets

Global options:
  --device <connstring>   Exact reader selection
  --json                  One JSON document on stdout
  --jsonl                 Stream JSON Lines events and a final result
  --yes                   Confirm a destructive operation
  --dry-run               Stop after preflight/planning where supported
  --show-keys             Allow key values and raw engine logs in output
  --timeout <duration>    Bound the whole command
  --verbose               Progress and diagnostics on stderr
  --version               Print the build version
  --help                  Print this help

The CLI is English-only. See docs/cli.md for the stable contract.
`)
}
