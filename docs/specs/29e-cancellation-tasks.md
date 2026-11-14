# Phase 29 S5: Cancellation and unified tasks

Status: complete 2026-11-14.

## Goal

Give every long workbench operation one visible task with a cancel action, and
make cancellation stop at a reader-exchange boundary without hiding a write
that may already have committed.

## Task registry

`crates/gui/src/app.rs` gains a session-only `TaskRegistry`. Every operation
started by the GUI creates one `GuiTask`:

- stable label, such as `Read card`, `Key scan`, `Restore dump`, `Write NDEF`,
  `Format PICC`;
- its own `nfc_core::CancelToken`;
- start time and current phase;
- optional numeric progress;
- terminal outcome (`Succeeded`, `Cancelled`, `Failed`) and result text.

The worker thread receives only the token. Labels, phases, progress, and
outcomes stay on the GTK main thread.

`run_service_task` is the shared path for operations that take the service
mutex, call one service method, and finish on the main context. Operations with
a progress channel (key scan, recovery) register the same task and update its
phase and progress from the existing receiver. A completion handler still owns
the operation-specific widgets; the task registry owns the card and records
the result before the handler runs.

Long operations registered by this slice:

- Classic read, block read, write preflight, write/restore execution, UID
  preflight and UID write;
- Classic NDEF write and format;
- keyscan and recovery;
- Type 2 read (through `Read card`), page write, lock, password, restore, UID
  preflight/execute, NDEF write and format;
- DESFire inventory, file read, write, key verification, authenticated UID
  read, NDEF read/format/delete/replace, PICC format, and the generic
  management/key/value/file runners.

Changing the reader, clearing the activity list (finished tasks only), and
application restart discard task history. An active task keeps its card and
token until the worker answers.

## Activity task card

The Activity page shows one card above the session log:

- task label and current phase;
- elapsed time, refreshed once per second while a task is active;
- a numeric progress bar when the service emits a fraction (keyscan sectors,
  recovery dictionary/sector/verify/brute-force progress), otherwise a
  spinner;
- `Cancel task`, enabled only for an active task that has not requested
  cancellation;
- terminal state: `Succeeded` (`Complete`), `Cancelled`, or `Failed` with the
  error text.

With no task, the card reads `No task running`. The card never repeats the
Activity table and no sidebar is added.

## Cancellation semantics

- `Cancel task` cancels the task token and changes the phase to
  `Cancelling after the current exchange…`; the button becomes insensitive.
  Keyscan and recovery keep their page-level Cancel actions, which route to the
  same token.
- The service checks the token before the next reader exchange. A cancellation
  requested mid-operation therefore stops before the following command.
- A write that may have committed is never reported without its read-back.
  `Operation::write_block_verified` and the Type 2 `write_page_verified` no
  longer check cancellation between the write and the read-back. The
  in-flight page or block is verified, then the next cancellation check stops
  the sequence.
- Cancellation is not reported as an error toast. Each surface reports
  `… cancelled`, logs one Activity row, and leaves the inspected image in
  place. A UID write cancellation invalidates the live image and re-reads the
  card because the UID may already have changed.

## Tests

- `nfc-app` service test: a block write whose hook cancels the token after the
  write is applied still returns success because the read-back ran.
- `nfc-type2` test: the same property for a Type 2 page write.
- GTK scenario:
  - the card's cancel action cancels the token, shows the cancelling phase, and
    reaches `Cancelled`;
  - read, keyscan, recovery, Classic NDEF write, and restore execution are
    cancelled before the first exchange, report `Cancelled` on the card, and
    write one `… cancelled` Activity row;
  - a successful read ends in `Succeeded · Complete`.
