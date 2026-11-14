# Note: 29.5 cancellation and unified tasks

Plan: [29-cli-reader-ownership.md](../plan/29-cli-reader-ownership.md). Spec:
[29e-cancellation-tasks.md](../specs/29e-cancellation-tasks.md).

## Implemented

- `crates/gui/src/app.rs`: `TaskRegistry`/`GuiTask` and the Activity task card.
  Every long operation registers a label, token, start time, phase, optional
  progress, and terminal outcome. `run_service_task` is the shared worker path;
  keyscan and recovery keep their progress channels and update the same task.
- `crates/gui/src/app.rs`: the card shows a spinner or a numeric progress bar,
  elapsed time, `Cancel task`, and `Succeeded`/`Cancelled`/`Failed`.
  `Clear activity` discards finished tasks and keeps an active one.
- `crates/gui/src/i18n.rs`: task-card labels, operation labels, and
  cancellation states in English and Simplified Chinese.
- `crates/nfc-app/src/operation.rs` and `crates/nfc-type2/src/write.rs`: the
  post-write cancellation check before the read-back is removed. A committed
  block or page is always verified; the next exchange observes the token.
- `crates/nfc-transport/src/mock.rs`: `on_write_applied` test hook, used to
  cancel a token exactly inside a write.
- Documentation: the workbench Activity contract, plan and index links, spec,
  and this note.

## Behavior

- The GUI acquires the reader mutex on a worker thread exactly as before. The
  task registry only adds bookkeeping on the main thread.
- Cancellation is requested through the task card, the keyscan Cancel button,
  or the recovery Cancel button; all cancel the same token. The card switches
  to `Cancelling after the current exchange…` and disables its button.
- The service checks the token before the next reader exchange. A read, scan,
  recovery, NDEF write, or restore that is cancelled before its first exchange
  reports `Cancelled` without touching the card.
- A write already issued finishes its read-back. Restore and NDEF paths then
  stop at the next block or page boundary and report `Cancelled` with one
  Activity row. No path reports a write as complete when its read-back did not
  run.
- Cancellation no longer produces an error toast. DESFire operations journal
  `cancelled` in the existing management log instead of `failed`.
- A cancelled UID write invalidates the live image and starts a fresh read,
  because block 0 may already have changed. A cancelled Type 2 UID change asks
  the user to re-read the tag for the same reason.

## Limitations

- Cancellation is cooperative. The reader call currently in flight finishes;
  only the next exchange observes the token. A driver-level abort is out of
  scope for this slice.
- The task card shows the latest task. Operations remain serialized by the
  busy flag and the service reader lease, so two active cards are not expected.
- Task history is session-only and is not written to the Activity log or disk.

## Tested

2026-11-14:

- `cargo fmt --all --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 789 passed, 0 failed, 9 ignored (hardware-gated).
- `NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry --bin
  nfc-foundry app::tests::gtk_workbench_scenarios -- --exact`: pass. The
  scenario covers task-card cancellation for read, keyscan, recovery, Classic
  NDEF write, and restore; the cancel action's token/phase transition; and the
  terminal `Succeeded` state after a read.
- `reuse lint`: 694/694 files compliant.

New unit tests:

- `nfc-app`: `a_classic_write_committed_during_cancellation_is_still_read_back`.
- `nfc-type2`: `a_write_committed_during_cancellation_is_still_read_back`.

