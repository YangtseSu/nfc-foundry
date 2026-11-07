# Phase 05c implementation note: Classic GUI writes and advanced bulk write

Date: 2026-11-07

## Implemented

- Extended `app.Bindings` with the Classic write surface:
  `EditBlockHex`, `LoadDump`, `LoadDumpFile`, `SaveDump`, `SaveDumpFile`,
  `PreflightWrite`, `StartWrite`, `PreflightUIDWrite`, and `StartUIDWrite`.
- Added preflight mode `block` for exactly one ordinary data block. The service
  rejects block 0, sector trailers, empty selections, and multi-block requests.
- Bound every preflight to the dump's source card identity. A dump without
  metadata or from a different UID/ATQA/SAK is refused before any write.
- Added a complete pre-write backup for changed-block writes, full restores,
  and standalone UID writes. The card is read again with the verified keys; an
  incomplete read, card change, or file error aborts before the first write.
  Backups are JSON projects under
  `$XDG_CONFIG_HOME/NFC Foundry/classic-backups/` with directory mode 0700 and
  file mode 0600.
- Added a Classic Blocks editor for ordinary data blocks, dump load/save
  actions, and a session-scoped `Advanced operations` toggle. The toggle is off
  by default, warns before enabling, and resets on card removal, device change,
  or application restart.
- Added the advanced actions:
  - `Write changed blocks`, with a preflight dialog listing planned blocks,
    trailers, access changes, skipped block 0, and the backup path;
  - `Full restore`, always using the trailer confirmation;
  - `Set UID`, with a 4-byte UID input and exact-UID confirmation.
- Reused the existing restore and UID pipelines. No new protocol or transport
  code was added.
- Added Classic task failure and completion handling. Failures include the
  pre-write backup path in both the error banner and activity log.
- Added English and Simplified Chinese messages; the catalogue has 368
  messages.
- `nfctl classic restore` and `nfctl uid set` now report the pre-write backup
  path; dry runs report the planned path without creating the file.

## Safety behavior

- Ordinary block writes use the restore preflight and immediate read-back;
  they do not create a full-image backup.
- Bulk/full/UID writes read the current card into a new complete dump before
  the first mutation. The selected dump itself is never used as the backup.
- Block 0 is removed from changed-block selections. UID writes remain the only
  block-0 path and keep the Gen1A/CUID preflight, reselection, and full block-0
  verification.
- Trailers are written last by the restore pipeline. The GUI passes the
  trailer confirmation whenever the preflight plan contains one.
- The GUI never offers a free-form block range. Changed-block writes are bound
  to the workbench dirty set; full restore uses the complete loaded dump.
- No continue-on-error option or "do not ask again" switch was added.

## Verification

Hardware-free:

- `make check` passes: `go test ./...`, `go vet ./...`,
  `go tool mygo i18n check .`, the MyGo view tests, and `reuse lint`
  (386/386 files annotated).
- `go test -race ./internal/desktop ./app` passes.
- App tests cover single-block preflight/write, current-card backup contents,
  backup failure before any write, card change after preflight, source-card
  mismatch, block-0 exclusion, protected-block refusal, and dump
  load/save bindings.
- CLI tests cover the restore and UID pre-write backup path, file creation, and
  mode 0600. Dry runs assert that no backup file is created.
- Desktop tests cover the advanced warning and reset, block editing, changed
  blocks with trailers, full restore confirmation, UID exact confirmation,
  invalid UID rejection, backup-path failure reporting, and dump actions.
- A headless tester render at 1360x860 was reviewed for the Blocks editor and
  the changed-block preflight dialog; the block hex input shows all 32 digits.

Hardware:

- No new transport or card command was introduced. The existing Classic
  dump/restore and Gen1A UID hardware tests remain the hardware gate.
- This phase did not run a physical GUI session. The native view tests exercise
  the interaction layer headlessly.

## Follow-up

- Run the GUI write workflow against a disposable Classic card on hardware.
- Phase 05d can reuse the shared Advanced mode toggle and confirmation pattern.
- Type 2 protected writes remain CLI-only until Phase 05d.
