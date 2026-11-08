# Phase 07b implementation note: GUI parity with the CLI

Date: 2026-11-08

## Implemented

- Added a Classic **Recovery** tab with automatic recovery and explicit
  nested, darkside, and hardnested modes. The panel shows engine availability
  and version, seed-key and Classic 1K prerequisites, reader capability, and
  requires a confirmation that authorizes active card testing. Recovery runs
  through the existing services and keeps the reader handoff, cancellation,
  key persistence, and verified re-read behavior.
- Added ISO/IEC 14443-4 card detection and three tabs:
  - **APDU**: diagnostic short-APDU exchange with ATS, data, and status words.
  - **DESFire**: read-only identify, application list, file list, and file
    read.
  - **NDEF**: Type 4 capability container, NDEF records, and the shared
    template/raw/records editor for writes.
- Type 4 writes use the existing `StartType4NDEFWrite` task: a full-image
  backup, read-back verification, and an explicit confirmation.
- Added a **Diagnostics** menu action that runs the same report as `nfctl diag`
  and `nfc-foundry --self-check`, including build, libnfc-ng, engine, and
  runtime-manifest checks.
- Generic NDEF previews now return the encoded message, allowing the GUI to
  show the exact length without re-encoding. Generic NDEF writes dispatch to
  Classic, Type 2, or Type 4 based on the selected card.
- Added UI tests for recovery, APDU, and Type 4 write confirmation in
  `internal/desktop/parity_test.go`.

## Validation

- `go test ./...`
- `go vet ./...`
- `go tool mygo i18n check .` (English and Simplified Chinese complete)
- `reuse lint`
- Focused desktop UI tests for the new screens.

## Remaining gaps

- APDU, DESFire, and Type 4 NDEF still need hardware validation on the
  available readers; the implementation and hardware-free fixtures are in
  place, but the README and capability matrix keep the existing caveat.
- CLI-only automation remains intentional: `--json`, `--jsonl`, `--show-keys`,
  `--timeout`, exit codes, and shell pipelines have no GUI equivalent. Their
  underlying operations are available in the GUI.
- Raw APDUs remain diagnostic. The GUI does not add convenience writes or
  claim that arbitrary APDUs are reversible.
