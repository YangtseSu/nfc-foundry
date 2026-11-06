package cli

import (
	"fmt"
	"strconv"
	"strings"
	"time"
)

type usageError struct{ message string }

func (e *usageError) Error() string { return e.message }

func usagef(format string, args ...any) error {
	return &usageError{message: fmt.Sprintf(format, args...)}
}

type flagSpec struct {
	takesValue bool
}

type cmdArgs struct {
	values      map[string][]string
	positionals []string
}

func (a cmdArgs) bool(name string) bool {
	return len(a.values[name]) != 0
}

func (a cmdArgs) value(name string) string {
	values := a.values[name]
	if len(values) == 0 {
		return ""
	}
	return values[len(values)-1]
}

// parseCommandArgs accepts options before, between, and after positional
// arguments. Long options may use "--name value" or "--name=value"; single
// dashes and boolean "=true/false" are accepted for symmetry with flag.
func parseCommandArgs(args []string, specs map[string]flagSpec) (cmdArgs, error) {
	parsed := cmdArgs{values: make(map[string][]string)}
	endOfOptions := false
	for index := 0; index < len(args); index++ {
		arg := args[index]
		if endOfOptions {
			parsed.positionals = append(parsed.positionals, arg)
			continue
		}
		if arg == "--" {
			endOfOptions = true
			continue
		}
		name, value, hasValue, isFlag := splitOption(arg)
		if !isFlag {
			parsed.positionals = append(parsed.positionals, arg)
			continue
		}
		spec, known := specs[name]
		if !known {
			return cmdArgs{}, usagef("unknown option --%s", name)
		}
		if spec.takesValue {
			if !hasValue {
				index++
				if index >= len(args) {
					return cmdArgs{}, usagef("option --%s requires a value", name)
				}
				value = args[index]
			}
			parsed.values[name] = append(parsed.values[name], value)
			continue
		}
		if hasValue {
			enabled, err := strconv.ParseBool(value)
			if err != nil {
				return cmdArgs{}, usagef("option --%s does not take a value", name)
			}
			if !enabled {
				continue
			}
		}
		parsed.values[name] = append(parsed.values[name], "true")
	}
	return parsed, nil
}

func splitOption(arg string) (name, value string, hasValue, isFlag bool) {
	if len(arg) < 2 || arg[0] != '-' {
		return "", "", false, false
	}
	trimmed := strings.TrimLeft(arg, "-")
	if trimmed == "" {
		return "", "", false, false
	}
	name, value, hasValue = strings.Cut(trimmed, "=")
	if name == "" {
		return "", "", false, false
	}
	return name, value, hasValue, true
}

type globalOptions struct {
	device   string
	timeout  time.Duration
	json     bool
	jsonl    bool
	yes      bool
	dryRun   bool
	showKeys bool
	verbose  bool
	version  bool
	help     bool
}

var globalFlagSpecs = map[string]flagSpec{
	"device":    {takesValue: true},
	"timeout":   {takesValue: true},
	"json":      {},
	"jsonl":     {},
	"yes":       {},
	"dry-run":   {},
	"show-keys": {},
	"verbose":   {},
	"version":   {},
	"help":      {},
	"h":         {},
}

// extractGlobal removes recognized global options wherever they appear and
// leaves command options and positionals in place.
func extractGlobal(args []string) (globalOptions, []string, error) {
	var options globalOptions
	rest := make([]string, 0, len(args))
	endOfOptions := false
	for index := 0; index < len(args); index++ {
		arg := args[index]
		if endOfOptions {
			rest = append(rest, arg)
			continue
		}
		if arg == "--" {
			endOfOptions = true
			rest = append(rest, arg)
			continue
		}
		name, value, hasValue, isFlag := splitOption(arg)
		if !isFlag {
			rest = append(rest, arg)
			continue
		}
		if _, known := globalFlagSpecs[name]; !known {
			rest = append(rest, arg)
			continue
		}
		spec := globalFlagSpecs[name]
		if spec.takesValue && !hasValue {
			index++
			if index >= len(args) {
				return options, nil, usagef("option --%s requires a value", name)
			}
			value = args[index]
		}
		if !spec.takesValue && hasValue {
			enabled, err := strconv.ParseBool(value)
			if err != nil {
				return options, nil, usagef("option --%s does not take a value", name)
			}
			if !enabled {
				continue
			}
		}
		switch name {
		case "device":
			options.device = value
		case "timeout":
			duration, err := time.ParseDuration(value)
			if err != nil || duration < 0 {
				return options, nil, usagef("invalid --timeout value %q", value)
			}
			options.timeout = duration
		case "json":
			options.json = true
		case "jsonl":
			options.jsonl = true
		case "yes":
			options.yes = true
		case "dry-run":
			options.dryRun = true
		case "show-keys":
			options.showKeys = true
		case "verbose":
			options.verbose = true
		case "version":
			options.version = true
		case "help", "h":
			options.help = true
		}
	}
	if options.json && options.jsonl {
		return options, nil, usagef("--json and --jsonl are mutually exclusive")
	}
	return options, rest, nil
}
