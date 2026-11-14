# Spec: 29.6 Developer-only raw diagnostics

Status: implemented 2026-11-15.

## Goal

Give bench developers a native PN532 command console and an arbitrary APDU
exchange without adding a supported card path, a GUI entry, or commands that
appear in a release build's help.

## Feature gate

`developer-tools` is a non-default Cargo feature wired through
`nfc-transport`, `nfc-drivers`, `nfc-app`, and `nfctl`. It is:

- off in `default` features, so `cargo build --release -p nfctl` and every
  release/AUR package exclude the commands;
- off for the GUI crate, which has no entry for either command;
- enabled for diagnostics with
  `cargo build -p nfctl --features developer-tools`.

Without the feature the worker code is not compiled: the transport trait, the
PN532 raw-command path, the application diagnostic lease, the clap variants,
and the handlers are all absent. Only the tests that assert the absence are
compiled.

## `nfctl pn532 shell`

- Resolves and opens a reader through the normal selection policy
  (`--device`, `NFC_FOUNDRY_DEVICE`, or a single enumerated reader).
- Human output only. `--json` and `--jsonl` are usage errors (exit 2); the
  command has no `--dry-run` planning path.
- Prints one warning on stderr before the session:
  `developer console: raw PN532 commands bypass card workflows and can leave
  the reader in an unknown state`.
- One input line is one native command frame: hexadecimal command code and
  parameters, with spaces and `:` allowed. `help`/`?` prints the one-line
  usage, `quit`/`exit` and EOF end the session, and blank lines are ignored.
- The driver still supplies framing, the wakeup preamble, ACK handling,
  response-code validation, and the command timeout. The console never enters
  raw or recovery framing mode; a command that needs one is the caller's
  diagnostic choice, not a workflow.
- The reply body prints as space-separated hexadecimal (`response: …`), an
  empty body prints `response: (empty)`, and a command failure prints
  `error: …` without ending the session.
- No card needs to be present. The session holds one exclusive diagnostic
  lease, so the reader stays busy and its cross-process lock stays held until
  the console exits. `--timeout` and SIGINT cancel through the shared token.
- The console never prints key material, the Type 2 password, or the request
  frame.

## `nfctl apdu --raw <HEX>`

- Resolves and opens a reader, selects the card, requires an ISO/IEC 14443-4
  card, and sends one complete ISO 7816-4 command APDU without retry.
- Warning: `raw APDU is a developer diagnostic, not a safe write path`
  (stderr in human mode, the envelope `warnings` array in JSON modes).
- Human result: `data: <hex>` or `data: (none)`, then `sw: XXXX`. JSON result
  fields: `data`, `sw`, and `statusOk`; the envelope `card` reports the
  selected card identity.
- The request APDU is never echoed into the result or the envelope; an
  experimental APDU can carry key material.
- Errors use the normal exit-code table: malformed hex is usage (2), a
  non-ISO-DEP card or a reader without APDU support is hardware/unsupported
  (4), and an exchange or deadline failure is an operation error (1).

## Layer placement

- `nfc-transport`: a feature-gated `RawConsole` capability and
  `Reader::raw_console`. The mock reader implements it and the handle scripts
  replies (`set_raw_response`) and records requests (`raw_commands`), so the
  console tests need no hardware.
- `nfc-drivers`: PN532 UART implements `RawConsole`; ACR122U USB and PC/SC
  keep the default `None` because their command channel is not PN532.
- `nfc-app`: feature-gated `developer::DiagnosticLease` and
  `Service::begin_diagnostics`. The lease owns the reader without selecting a
  card and clears the busy state on drop. `raw_apdu` selects the card and
  rejects every non-ISO-DEP card before the exchange.
- `nfctl`: feature-gated clap variants, names (`pn532.shell`, `apdu.raw`),
  and handlers.

## Separation from phase 13

Phase 13's read-only APDU diagnostic remains a normal product surface when it
lands. This slice adds no `apdu` command to release builds, so the two cannot
be confused; the developer command exits with a diagnostic warning and is
documented as non-supported.

## Tests

- `nfc-app`: a raw console runs before a card is present; a raw APDU on a
  Classic card is refused as unsupported.
- `nfctl` with the feature: help lists `pn532` and `apdu`; the console warns,
  answers, and leaks neither a candidate key nor a Type 2 password; an APDU
  returns `9000` and does not echo the request; machine output is rejected.
- `nfctl` without the feature: help contains neither command and both parse
  as usage errors.
- Both feature modes pass `cargo fmt --check`, clippy, and tests.

## Non-goals

- A GUI raw console or a GUI APDU entry.
- Treating a raw write as a supported card path.
- ACR122U native command console (only the card-level APDU path applies).
- External engines, a private runtime, or a shell launched from the GUI.
