# Spec: 29.7 Docs and non-goals

Status: implemented 2026-11-15.

## Goal

Close phase 29 by making the shipped documentation match the implemented CLI
contract, reader ownership, cancellation, and developer-only diagnostics, and
record the dropped GUI command terminal as a non-goal.

## Deliverables

- `docs/cli.md` is the single CLI contract. It covers global options, reader
  selection and the cross-process lock, the exit-code and error-code tables,
  the JSON envelope, JSONL events, secret rules, every command name, result
  shapes, device and file side effects, developer commands, and compatibility.
- `README.md`:
  - CLI machine-output examples include the total deadline and the JSON
    envelope fields, exit codes, and secret rules;
  - device selection names `--device`, `NFC_FOUNDRY_DEVICE`, and single-reader
    auto-selection, and states that one process owns a reader at a time
    (`reader_busy`, exit 4);
  - the GUI section describes explicit Connect/Disconnect, the inspection-only
    image after disconnect, the Activity task card, and the cancel semantics
    for a write that may already have committed.
- `docs/design/workbench.md`: the main menu has no command terminal entry, and
  the contract states that the GUI never launches a shell, bundles a private
  runtime or external engine, or exposes raw reader/APDU commands.
- `docs/testing.md`: records the phase 29.4-29.6 tests and the commands that
  exercise both sides of a feature gate.
- Plan and index: phase 29 and every slice are marked done.

## Non-goals

- `OpenCommandTerminal`: no GUI terminal, private runtime, external engine
  bundle, or shell launch.
- A persistent Activity sidebar; the existing Activity page carries the task
  card.
- Making `pn532 shell` or `apdu --raw` a supported card path.
- `LIBNFC_DEVICE` compatibility.
- Generic PC/SC readers.

## Acceptance

- Every README CLI example runs against the mock readers or an exact
  connstring as written.
- README device selection names the resolution order and the busy outcome.
- No document advertises a GUI terminal or a release-build developer command.
- `cargo fmt --check`, clippy, workspace tests, the GTK workbench scenario, and
  `reuse lint` pass.
