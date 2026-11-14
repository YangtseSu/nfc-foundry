# Spec: 29.1 CLI machine contract

Status: implemented 2026-11-14.

## Goal

Give `nfctl` a stable scripting surface without changing card or file
semantics: global output modes, a typed exit-code table, one JSON envelope,
JSONL events for long operations, a total deadline, and documented secret
rules.

## Global flags

- `--json`: one JSON document on stdout.
- `--jsonl`: compact JSON Lines events plus one terminal result.
- `--timeout <duration>`: total operation deadline parsed as `ms`, `s`, `m`,
  `h`, or a sum (`1m30s`).
- `--verbose`: reader, firmware, and capability detail on stderr.
- `--dry-run`: plan-only mode for mutation commands with a preflight path.

`--json` and `--jsonl` conflict. Options are global, so they work before or
after the subcommand. `diag` no longer owns a private `--json` flag.

## Output separation

- stdout: human result, or exactly the requested machine document/events.
- stderr: warnings (human), notes, progress (human/JSON), reader diagnostics,
  and verbose detail.

A warning is printed on stderr in human mode and collected into
`warnings: []` in JSON modes.

## Exit codes

0 success; 1 operation (I/O, authentication, verification, deadline);
2 usage; 3 validation; 4 hardware/permission/busy; 5 reserved;
6 user cancel; 7 internal.

Clap help/version exit 0. Other clap failures exit 2, and when `--json` or
`--jsonl` was requested the CLI still emits a machine error envelope.

## Typed classification

`CliError` has four variants: `Usage`, `Service(NfcError)`, `Validation`, and
`Internal`. Classification is exhaustive over `NfcError`:

| Variant | Code | Exit |
|---|---|---:|
| `Unsupported` | `unsupported` | 4 |
| `InvalidArgument` | `validation` | 3 |
| `Io` | `io` | 1 |
| `Cancelled` | `cancelled` / `deadline_exceeded` | 6 / 1 |
| `NoCard` | `no_card` | 4 |
| `NoReader` | `no_reader` | 4 |
| `Timeout` | `timeout` | 4 |
| `AuthenticationFailed` | `authentication` | 1 |
| `AuthenticationRequired` | `authentication_required` | 1 |
| `CardChanged` | `card_changed` | 3 |
| `VerificationFailed` | `verification` | 1 |
| `NotImplemented` | `not_implemented` | 7 |

CLI-side argument failures construct `Usage` directly, so malformed hex, a
missing `--yes`, an unknown reader, and unsupported `--dry-run` all exit 2 even
though services sometimes use `InvalidArgument` for their own checks.

## Envelope

`{ok, command, device, card, warnings, error, result}`.

- `command` is a stable leaf name (`classic.read`, `scan.watch`, ...).
- `device` is the exact connstring or null.
- `card` is `{uid, atqa, sak, type}` or null; UID/ATQA/SAK are compact
  uppercase hex.
- `warnings` is always an array.
- `error` is null or `{code, message, exitCode}`.
- `result` is null on failure and a command-specific object on success.

`--jsonl` lines:

- `{"type":"progress", "phase", "message", "elapsed_ms", ...}`;
- `{"type":"card", "event":"appeared|removed", "card": {...}}` for watches;
- one terminal `{"type":"result", ...envelope}`.

Recovery progress is rate-limited (phase transition or 2 s; acquire also
64 frames). Keyscan emits one event per sector. Progress is diagnostic; the
terminal result is authoritative.

## Deadline and cancellation

The CLI creates one `CancelToken` per invocation. `--timeout` starts a
detached watchdog that sets a `deadline_expired` flag and cancels the token.
Every service request receives that token.

- Deadline expiry: exit 1, `deadline_exceeded`.
- Ctrl-C/SIGTERM: cancel, exit 6, `cancelled`; a second signal exits 130.
- Cancellation is checked before the next reader exchange. An exchange in
  flight completes; a write that may have committed still performs read-back.

## Dry run

Accepted only by commands with an existing preflight/plan path:

`classic write`, `classic uid`, `classic restore`, `type2 uid`,
`desfire format-ndef`, `desfire replace-ndef`, `desfire format-picc`,
`desfire file-settings`.

Other commands exit 2 with a usage error. Dry-run results include
`"dryRun": true` plus the plan.

## Secret rules

Machine output never includes key values, Type 2 PWD/PACK, DESFire key material,
or raw dump payloads. `classic keys export` without `--output` is rejected in
machine modes. `desfire change-key --export` reports only the path. Explicit
read/export commands (`desfire read`, `type2 read`, `classic.read`) return the
data the user requested.

## Documentation

`docs/cli.md` is the contract: flags, exit codes, error codes, envelope and
JSONL shape, command names, per-command result fields, secret rules, and
side effects.

## Tests

- `crates/nfctl/src/error.rs`: exhaustive variant-to-code table and deadline
  classification.
- `crates/nfctl/src/runtime.rs`: durations, mode pre-scan.
- `crates/nfctl/tests/cli_contract.rs`: envelope schema and golden values, card
  facts, JSON result families, redaction, exit codes, JSONL sequence, signal
  and timeout cancellation, dry-run rejection/plan, human output separation,
  sandboxed side effects. Existing `crates/nfctl/tests/cli.rs` keeps the
  workflow smoke tests; its contract assertions were updated to the new exit
  codes and the `diag` envelope.

## Non-goals

- Reader auto-selection and `NFC_FOUNDRY_DEVICE` (29.3).
- Cross-process reader lock (29.2).
- GUI task registry (29.5).
- Developer raw commands (29.6).
- `LIBNFC_DEVICE` compatibility.
