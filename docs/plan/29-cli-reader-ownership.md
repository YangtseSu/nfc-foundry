# Phase 29 plan: CLI contract and reader ownership

Status: done 2026-11-15. All slices are complete. Decisions from the parity
review:

- complete the old `nfctl` machine contract in one phase;
- GUI and CLI share a cross-process reader lock;
- support `NFC_FOUNDRY_DEVICE` and single-reader auto-selection;
- add manual GUI Connect/Disconnect and cancel all long operations;
- add a unified task card in the Activity page, not a new sidebar;
- add raw PN532/APDU only behind a non-default developer feature;
- `OpenCommandTerminal` is dropped.

## Goal

Close the automation and device-ownership gap between the old plan and the
current GTK/CLI implementation. After this phase a script can select a reader
without guessing, consume stable JSON, distinguish failure classes, and never
race the GUI for the same reader.

## Slices

### 29.1 CLI machine contract

Done 2026-11-14. Spec: [29a-cli-machine-contract.md](../specs/29a-cli-machine-contract.md).
Note: [29a-cli-machine-contract.md](../notes/29a-cli-machine-contract.md).
Contract: [cli.md](../cli.md).

- Add global flags, applicable before or after the subcommand:
  - `--json`;
  - `--jsonl`;
  - `--timeout <duration>`;
  - `--verbose`;
  - `--dry-run` (mutation commands only; other commands reject it with a usage
    error).
- `--json` and `--jsonl` are mutually exclusive.
- Human-readable output stays on stdout; diagnostics and engine logs go to
  stderr.
- Stable exit codes:

  | Code | Meaning |
  |---:|---|
  | 0 | success |
  | 1 | operation failed (I/O, verification, engine failure) |
  | 2 | usage or argument error |
  | 3 | validation failure (dump layout, BCC, access bits, card mismatch) |
  | 4 | permission, reader busy, or hardware unavailable |
  | 5 | reserved for external-engine validation; unused while native-only |
  | 6 | cancelled by user/signal |
  | 7 | internal error |

- Add a typed CLI error classifier. `NfcError` variants and service validation
  errors map explicitly; do not classify by string matching where a typed
  variant exists.
- JSON envelope:

  ```json
  {
    "ok": true,
    "command": "classic.dump",
    "device": "pn532_uart:/dev/ttyUSB0",
    "card": {"uid": "04AABBCC", "atqa": "0004", "sak": "08", "type": "mifare_classic_1k"},
    "warnings": [],
    "error": null
  }
  ```

  Errors add `code`, `message`, and optional `details`. Command-specific result
  fields are additive. Keys and full dumps are never included unless the
  command explicitly exports them.
- JSONL events for scans, recovery, keyscan, and long operations:
  `type`, `phase`, `message`, optional `progress`, `elapsed_ms`, and terminal
  `ok`/`error`. One JSON object per line, no pretty printing.
- `--timeout` is a total operation deadline, implemented through the existing
  `CancelToken`; it is not a reader command timeout replacement.
- `--verbose` adds reader/driver/retry detail on stderr without card payloads.
- Write `docs/cli.md` with global options, exit codes, per-command JSON shapes,
  JSONL event types, secret rules, and device selection.
- Tests: golden JSON for every command family, error-code table, JSONL
  sequence, secret redaction, timeout/cancel, mutual-exclusion usage errors.

### 29.2 Cross-process reader lock

Done 2026-11-14. Spec: [29b-reader-lock.md](../specs/29b-reader-lock.md).
Note: [29b-reader-lock.md](../notes/29b-reader-lock.md).

- Add a reader lock owned by `nfc-app`, used by both `nfctl` and the GUI.
- Lock file:
  - `$XDG_RUNTIME_DIR/nfc-foundry/reader-<key>.lock`;
  - fallback to a per-user temp directory when `XDG_RUNTIME_DIR` is absent;
  - mode `0600`;
  - POSIX advisory lock (`flock`) held for the lifetime of the connection.
- Lock key uses the best available physical identity:
  - PN532 UART: canonical device path, preferring `/dev/serial/by-id/*` when
    present;
  - ACR122U USB: USB bus/address plus serial when available;
  - ACR122U PC/SC: reader name; if a stable USB/serial identity is resolvable,
    use the same key as the USB transport.
- Add an optional `DeviceInfo::physical_id` (or an equivalent
  `Transport::identity`) populated by drivers; do not hash only the user-facing
  name.
- If direct USB and PC/SC aliases cannot be proven to be the same physical
  reader, keep transport-specific lock keys and rely on the OS claim plus the
  actionable `pcscd`/busy errors. Record this limitation in the phase note.
- GUI behavior:
  - acquire the lock when the monitor connects to a reader;
  - release it on Disconnect, reader change, and application exit;
  - if another process holds the lock, show reader-busy and do not claim the
    device.
- CLI behavior:
  - acquire before opening/selecting the reader;
  - on contention, print an actionable message and exit 4;
  - `--json` includes `code: "reader_busy"`.
- Tests: two processes contend; stale lock file does not block after process
  exit; GUI/CLI simulated contention; disconnect releases the lock.

### 29.3 Device selection

Done 2026-11-14. Spec:
[29c-device-selection.md](../specs/29c-device-selection.md). Note:
[29c-device-selection.md](../notes/29c-device-selection.md).

- Resolution order for `nfctl`:
  1. explicit `--device`;
  2. `NFC_FOUNDRY_DEVICE`;
  3. exactly one enumerated supported reader: select it;
  4. zero readers: hardware-unavailable error (exit 4);
  5. multiple readers: usage error (exit 2) naming the choices.
- Do not support `LIBNFC_DEVICE`; document the migration.
- GUI keeps its current auto-select-first behavior but shows the resolved exact
  connstring and PC/SC/USB transport.
- Tests: env precedence, single/multiple/zero readers, invalid env target.

### 29.4 GUI Connect / Disconnect

Done 2026-11-14. Spec:
[29d-gui-connect-disconnect.md](../specs/29d-gui-connect-disconnect.md). Note:
[29d-gui-connect-disconnect.md](../notes/29d-gui-connect-disconnect.md).

- Add explicit Connect and Disconnect controls to the reader rail or reader
  menu. The current reader dropdown/refresh remains.
- Disconnect:
  - stop the monitor;
  - release the reader and cross-process lock;
  - invalidate the live flag, reset Advanced mode, and keep the inspected
    bytes with an inspection-only status;
  - disable writes until a new read.
- Connect/Read reopens the selected reader, reacquires the lock, and restarts
  the monitor.
- Show `Disconnected` distinctly from `Connecting`, `Polling`, `Busy`,
  `Recovering`, and `Error`.
- Tests: disconnect stops polling and releases the reader; reconnect after
  disconnect; write gates stay off while disconnected.

### 29.5 Cancellation and unified tasks

Done 2026-11-14. Spec:
[29e-cancellation-tasks.md](../specs/29e-cancellation-tasks.md). Note:
[29e-cancellation-tasks.md](../notes/29e-cancellation-tasks.md).

- Add a GUI task registry for every long operation: read, Classic block write,
  restore, NDEF write/format, UID preflight/execute, Type 2 page/lock/password/
  restore/format, DESFire read, keyscan, recovery.
- Each task stores a `CancelToken`, label, start time, phase/status, elapsed
  time, and terminal result.
- Activity page gets a task card:
  - title and current phase;
  - elapsed time;
  - numeric progress where the service already emits it; indeterminate spinner
    otherwise;
  - `Cancel task`;
  - terminal success/cancel/failure state.
- Cancellation semantics:
  - checked before the next reader exchange;
  - a write displays “Cancelling after the current exchange…” and still
    performs read-back/verification when the write may have committed;
  - never report a write as complete when the read-back did not run.
- Do not add a persistent right sidebar; this is the existing Activity page.
- Tests: cancel read, keyscan, recovery, NDEF write, restore; UI state and log.

### 29.6 Developer-only raw diagnostics

Done 2026-11-15. Spec:
[29f-developer-tools.md](../specs/29f-developer-tools.md). Note:
[29f-developer-tools.md](../notes/29f-developer-tools.md).

- Add a non-default Cargo feature `developer-tools`; release packages and AUR
  builds do not enable it.
- `nfctl pn532 shell`:
  - interactive raw command console;
  - only compiled with `developer-tools`;
  - prints a warning that it bypasses card workflows;
  - no GUI entry.
- `nfctl apdu --raw <HEX>`:
  - arbitrary APDU exchange only with `developer-tools`;
  - warning that it is diagnostic, not a safe write path;
  - never advertised in the normal command list/help build.
- Keep the normal read-only APDU diagnostic from phase 13 separate from this
  developer command.
- Tests: feature on/off build; command absent without the feature; warning and
  no secret logging.

### 29.7 Docs and non-goals

Done 2026-11-15. Spec: [29g-docs-non-goals.md](../specs/29g-docs-non-goals.md).
Note: [29g-docs-non-goals.md](../notes/29g-docs-non-goals.md).

- Add `docs/cli.md`.
- Update README CLI examples and device selection.
- Record `OpenCommandTerminal` as a non-goal: no private runtime, no external
  engine bundle, no shell launch from the GUI.
- Update the plan index and testing docs.

## Acceptance

- `nfctl` supports the global flags and exit-code table; JSON/JSONL output is
  stable and documented.
- Two processes cannot open the same reader; the loser exits 4 with a clear
  message.
- `NFC_FOUNDRY_DEVICE` and single-reader auto-selection work; ambiguity is an
  error.
- GUI Disconnect releases the reader and lock; reconnect works.
- Read/write/restore/NDEF operations are cancellable from the Activity task
  card with honest write-cancel semantics.
- Developer raw commands are absent from release builds.
- `cargo fmt`, clippy, workspace tests, GTK scenario, and `reuse lint` pass.

## Non-goals

- Generic PC/SC readers.
- `LIBNFC_DEVICE` compatibility.
- A persistent Activity sidebar (use the Activity page).
- `OpenCommandTerminal`.
- Making the developer raw tools a supported card path.

## First files

- `crates/nfctl/src/main.rs`.
- `crates/nfc-app/src/service.rs`, `reader.rs`, `operation.rs`.
- `crates/nfc-transport/src/device.rs` (physical identity).
- `crates/gui/src/app.rs` (reader controls, task card, cancellation).
- `docs/cli.md`, `docs/design/workbench.md`, `README.md`.
