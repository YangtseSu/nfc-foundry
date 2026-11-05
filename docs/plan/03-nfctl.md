# Phase 03: nfctl CLI

## Goal

Expose the shared application services as a stable, English-only CLI.

## Work

- Add `nfctl` as a separate binary with the shared service layer.
- Implement discovery, card info, Classic, Type 2, UID, recovery, diagnostics,
  raw PN532, APDU, keys, and NDEF command groups.
- Add `--device`, `--json`, `--jsonl`, `--yes`, `--dry-run`, `--timeout`, and
  `--verbose`.
- Define stable exit codes and JSON schemas.
- Add a cross-process reader lock keyed by connstring.
- Add command-level tests with a mock transport.
- Document the contract in `docs/cli.md`.

## Acceptance

- `nfctl list`, `scan`, `classic dump`, and `classic restore` work on the
  validated PN532 UART setup.
- Destructive commands refuse to run without `--yes`.
- `--json` output is stable and contains no keys unless explicitly requested.
- A CLI command and the GUI cannot open the same reader concurrently.
- All help, errors, logs, and JSON keys are English.

## Non-goals

- No emulation, relay, DEP, anticol, SAM, or barcode commands.
- No translated CLI output.
