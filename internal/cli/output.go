package cli

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"
	"sync"

	"github.com/YangtseSu/nfc-foundry/app"
)

type outputMode int

const (
	outputHuman outputMode = iota
	outputJSON
	outputJSONL
)

type errorPayload struct {
	Code     string `json:"code"`
	Message  string `json:"message"`
	ExitCode int    `json:"exitCode"`
}

type envelope struct {
	OK       bool          `json:"ok"`
	Command  string        `json:"command,omitempty"`
	Device   string        `json:"device,omitempty"`
	Card     *app.CardDTO  `json:"card,omitempty"`
	Warnings []string      `json:"warnings"`
	Error    *errorPayload `json:"error"`
	Result   any           `json:"result,omitempty"`
}

type resultLine struct {
	Type string `json:"type"`
	envelope
}

type progressLine struct {
	Type          string `json:"type"`
	Event         string `json:"event,omitempty"`
	TaskID        string `json:"taskId,omitempty"`
	Kind          string `json:"kind,omitempty"`
	Phase         string `json:"phase,omitempty"`
	Progress      int    `json:"progress,omitempty"`
	Sector        int    `json:"sector,omitempty"`
	KeyType       string `json:"keyType,omitempty"`
	Attempted     int    `json:"attempted,omitempty"`
	Found         int    `json:"found,omitempty"`
	Message       string `json:"message,omitempty"`
	Engine        string `json:"engine,omitempty"`
	Version       string `json:"version,omitempty"`
	Stream        string `json:"stream,omitempty"`
	Log           string `json:"log,omitempty"`
	Sequence      uint64 `json:"sequence,omitempty"`
	ExitCode      *int   `json:"exitCode,omitempty"`
	RecoveryError string `json:"recoveryError,omitempty"`
}

type cardLine struct {
	Type   string      `json:"type"`
	Event  string      `json:"event"`
	Device string      `json:"device,omitempty"`
	Card   app.CardDTO `json:"card"`
	Time   string      `json:"time"`
}

type output struct {
	mu       sync.Mutex
	stdout   io.Writer
	stderr   io.Writer
	mode     outputMode
	verbose  bool
	showKeys bool
}

func newOutput(stdout, stderr io.Writer, options globalOptions) *output {
	mode := outputHuman
	if options.json {
		mode = outputJSON
	}
	if options.jsonl {
		mode = outputJSONL
	}
	return &output{stdout: stdout, stderr: stderr, mode: mode, verbose: options.verbose, showKeys: options.showKeys}
}

// humanf prints only in human mode, so callers can write the readable result
// without branching on the output mode.
func (o *output) humanf(format string, args ...any) {
	if o.mode != outputHuman {
		return
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	fmt.Fprintf(o.stdout, format, args...)
}

func (o *output) warnf(format string, args ...any) {
	o.mu.Lock()
	defer o.mu.Unlock()
	fmt.Fprintf(o.stderr, "warning: "+format+"\n", args...)
}

func (o *output) emit(command, device string, card *app.CardDTO, warnings []string, result any) {
	if warnings == nil {
		warnings = []string{}
	}
	if o.mode == outputHuman {
		o.mu.Lock()
		defer o.mu.Unlock()
		for _, warning := range warnings {
			fmt.Fprintf(o.stderr, "warning: %s\n", warning)
		}
		return
	}
	env := envelope{OK: true, Command: command, Device: device, Card: card, Warnings: warnings, Error: nil, Result: result}
	o.mu.Lock()
	defer o.mu.Unlock()
	encoder := json.NewEncoder(o.stdout)
	encoder.SetEscapeHTML(false)
	if o.mode == outputJSON {
		encoder.SetIndent("", "  ")
		_ = encoder.Encode(env)
		return
	}
	_ = encoder.Encode(resultLine{Type: "result", envelope: env})
}

func (o *output) fail(command, device string, card *app.CardDTO, err error) int {
	code, exit := classifyError(err)
	payload := &errorPayload{Code: code, Message: err.Error(), ExitCode: exit}
	env := envelope{OK: false, Command: command, Device: device, Card: card, Warnings: []string{}, Error: payload}
	o.mu.Lock()
	defer o.mu.Unlock()
	switch o.mode {
	case outputHuman:
		fmt.Fprintf(o.stderr, "error: %s\n", err.Error())
	case outputJSON:
		encoder := json.NewEncoder(o.stdout)
		encoder.SetEscapeHTML(false)
		encoder.SetIndent("", "  ")
		_ = encoder.Encode(env)
	case outputJSONL:
		encoder := json.NewEncoder(o.stdout)
		encoder.SetEscapeHTML(false)
		_ = encoder.Encode(resultLine{Type: "result", envelope: env})
	}
	return exit
}

func (o *output) progress(event app.TaskEventDetailDTO) {
	switch o.mode {
	case outputJSONL:
		line := progressLine{
			Type: "progress", Event: event.Type, TaskID: event.TaskID, Kind: event.Kind,
			Phase: event.Phase, Progress: event.Progress, Sector: event.Sector, KeyType: event.KeyType,
			Attempted: event.Attempted, Found: event.Found, Message: event.Message,
			Engine: event.Engine, Version: event.Version, Stream: event.Stream,
			Sequence: event.Sequence, ExitCode: event.ExitCode, RecoveryError: event.RecoveryError,
		}
		if o.showKeys {
			line.Log = event.Log
		}
		o.mu.Lock()
		defer o.mu.Unlock()
		encoder := json.NewEncoder(o.stdout)
		encoder.SetEscapeHTML(false)
		_ = encoder.Encode(line)
	case outputHuman:
		if !o.verbose {
			return
		}
		o.mu.Lock()
		defer o.mu.Unlock()
		if event.Message != "" {
			prefix := event.Phase
			if prefix == "" {
				prefix = event.Type
			}
			fmt.Fprintf(o.stderr, "[%s] %s\n", prefix, event.Message)
		}
		if o.showKeys && strings.TrimSpace(event.Log) != "" {
			fmt.Fprintf(o.stderr, "%s: %s\n", event.Engine, strings.TrimSpace(event.Log))
		}
	case outputJSON:
		if !o.verbose {
			return
		}
		o.mu.Lock()
		defer o.mu.Unlock()
		if event.Message != "" {
			fmt.Fprintf(o.stderr, "[%s] %s\n", event.Phase, event.Message)
		}
	}
}

func (o *output) cardEvent(event app.CardEventDTO, device string) {
	if o.mode == outputHuman {
		o.mu.Lock()
		defer o.mu.Unlock()
		fmt.Fprintf(o.stdout, "card %s: %s UID %s\n", event.Type, event.Card.Type, event.Card.UID)
		return
	}
	line := cardLine{Type: "card", Event: event.Type, Device: device, Card: event.Card, Time: event.Time}
	o.mu.Lock()
	defer o.mu.Unlock()
	encoder := json.NewEncoder(o.stdout)
	encoder.SetEscapeHTML(false)
	_ = encoder.Encode(line)
}
