# Phase 24 S4 implementation record

Date: 2026-11-12. Status: complete.

## What changed

`nfc-app` gained `reader.rs` with `ReaderStatus`, `ReaderEvent`,
`ReaderSnapshot`, `ReaderUpdate`, `ReaderMonitorConfig`, and the internal
monitor state machine. `DeviceManager` now owns the monitor and the busy flag;
`Service` exposes `poll_reader`, `poll_reader_at`, `reader_state`,
`reader_monitor_config`, `set_reader_monitor_config`, and `retry_reader`.

`nfc-transport` added `Reader::capabilities` with the default empty profile and
extended `ReaderCapabilities` with `recovery_parity`. The separate
`RecoveryCapabilities` type and `Reader::recovery_capabilities` were deleted.
PN532 UART, ACR122U, the mock reader, and both application test readers
populate the profile.

`nfc-app` gates:

- UID preflight, UID execution, and restore-with-UID planning require
  `manufacturer_write` before touching the card.
- `darkside`, `nested`, `staticnested`, `hardnested`, and `fm11rf08s` require
  `recovery_parity` before the engine runs. Auto recovery keeps the dictionary
  first and reports the engine's parity limitation only if it escalates.

Surfaces:

- `nfctl scan --watch` loops on `Service::poll_reader` instead of repeating
  `scan`, and prints card and reader transitions.
- The GTK reader rail gained a lifecycle line and a capability line. A
  background thread polls the service; an `async-channel` delivers updates to
  the main context. Removal, a different card, and reader failures invalidate
  the live image, reset Advanced mode, and disable writes.
- GUI tests disable the background thread and step `poll_reader_once`
  directly.

## Decisions

- The monitor is stepped, not self-scheduled, inside `nfc-app`. Hosts own the
  thread and clock; `poll_reader` uses the wall clock and `poll_reader_at` takes
  an explicit `Instant`.
- One missed no-card answer keeps the image. Two consecutive misses produce
  `CardRemoved`, matching the acceptance criterion and tolerating a single
  missed poll.
- Transient `Timeout`/`Io` failures close the reader and retry with exponential
  backoff. Non-retryable errors enter `Error`; re-selecting the same reader, refreshing
  the device list, or calling `retry_reader` resets the backoff.
- `release_reader` parks polling. Without this, the monitor reopened the reader
  immediately after an explicit release.
- The monitor keeps the last known card through a transient failure but marks
  the status `Recovering`. The GUI invalidates the live image on that status,
  so a stale image cannot write while the reader is unavailable.
- UID writes require `manufacturer_write` even though a CUID/Gen2 card may
  accept an ordinary block-0 write. This makes the acceptance rule explicit:
  a driver without the declared capability cannot reach the UID service.

## Verification

Run on 2026-11-12:

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — pass.
- `NFC_FOUNDRY_GUI_TESTS=1 cargo test -p nfc-foundry gtk_workbench_scenarios`
  — pass under the session display.
- `reuse lint` — pass.

Covered scenarios:

- `nfc-app` unit tests: appearance, removal after two misses, transient timeout
  and reconnect, backoff growth, busy status transitions, paused release, and
  the operation/service lock exclusion.
- Service test: `manufacturer_write` absent refuses `classic_uid_preflight`
  before a block read.
- Recovery test: explicit nested recovery without parity fails with the
  capability error.
- GTK scenario: a live image survives one missed poll, a second miss clears the
  live flag, the status reports the removal, and the write commit is disabled.
- CLI suite: all `--mock` commands, including auto recovery and UID gates.

No hardware run was required. The slice does not change a reader command
sequence, and the capability flags match the already-recorded operations in the
hardware matrix.
