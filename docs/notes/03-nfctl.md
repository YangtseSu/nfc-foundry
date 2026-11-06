# Phase 03 implementation note: nfctl

Date: 2026-11-06

## Implemented

- Added `cmd/nfctl` and `internal/cli` with an English-only command surface
  shared with the MyGo GUI through `app.Service`.
- Commands: `list`, `scan [--watch]`, `card info`,
  `classic read|write|dump|restore|keyscan`,
  `recover nested|darkside|hardnested`, `uid set`, `diag`, and
  `keys list|add|import|export`.
- Global options: `--device`, `--json`, `--jsonl`, `--yes`, `--dry-run`,
  `--show-keys`, `--timeout`, `--verbose`, `--version`, and `--help`.
  Options may appear before or after the subcommand.
- Stable exit codes 0-7 and a single JSON envelope with `ok`, `command`,
  `device`, `card`, `warnings`, `error`, and `result`. `--jsonl` streams
  progress/card lines and ends with one result line.
- Key values and raw engine logs appear only with `--show-keys`.
- Added `app.NewServiceWithOptions` so tests and the CLI can inject a mock
  transport, temporary key store, temporary UID backup root, and temporary lock
  directory. `app.NewService` keeps the production libnfc wiring.
- Added path-based application methods that reuse the GUI workflows:
  `WaitForCard`, `ReadClassicBlock`, `WriteClassicBlock`, `LoadDumpFile`,
  `SaveDumpFile`, `ImportKeyDictionaryFile`, `ExportKeyDictionaryFile`, and
  `StartHardnested`.
- Added a cross-process `flock` per exact connstring in `internal/device`.
  `Connect` acquires it; `Disconnect`, failed connects, and shutdown release
  it. Recovery and external-engine handoff keep it. The lock file is mode 0600
  under `$XDG_RUNTIME_DIR/nfc-foundry` (or the per-user temporary directory) and
  records the holder's program and PID.
- Persisted card-scoped key verifications in `keys.json` so a keyscan or
  recovery performed by one `nfctl` process is usable by the next process.
  Missing key records are skipped on load; the schema stays backward
  compatible with existing version-1 files.
- Translated the remaining application-facing Chinese messages in
  `internal/device`, `internal/attack`, and `internal/workflow` to English so
  CLI errors and progress are English-only and the GUI base language is
  consistent.

## Safety behavior

- `classic write` accepts ordinary data blocks only. Block 0 is rejected in
  favor of `uid set`; sector trailers are rejected in favor of the validated
  restore workflow.
- `classic restore` always loads a complete dump, runs the preflight, writes
  data blocks before trailers, verifies every block, and skips block 0. An
  actual restore requires both `--yes` and `--confirm-trailers`.
- `uid set` requires `--yes` and an exact `--confirm-uid`; preflight and
  block-0 backup happen before the write, and verification uses reselection and
  a full block-0 readback.
- Recovery commands require `--yes` as explicit authorization. `darkside`
  rejects `--keys` and requires no verified keys.
- Every write path still uses the existing device manager, key catalog, and
  read-back verification.

## Verification

- `make check` passes: `go test ./...`, `go vet ./...`,
  `go tool mygo i18n check .`, the MyGo workbench smoke test, and `reuse lint`.
- `CGO_ENABLED=1 go test -tags libnfc ./...` passes against the installed
  `libnfc-ng` 1.8.0.
- Built both `nfc-foundry` and `nfctl` with `-tags libnfc`; the GUI and CLI both
  link `libnfcng.so.6`.
- `nfctl list --json` on the host enumerated
  `acr122_usb:001:005`, `acr122_pcsc:ACS ACR122U 00 00`, and
  `pcsc:ACS ACR122U 00 00`.
- Cross-process lock test with the real ACR122U: one `nfctl scan` held the
  reader while waiting for a card; a second process exited 4 with
  `reader "acr122_usb:001:005" is in use by nfctl (pid ...)`. Sending SIGTERM to
  the first process produced exit 6 and released the lock.
- `internal/cli` tests with a mock MIFARE Classic 1K transport cover:
  `list` JSON; `scan` and `card info`; keyscan followed by dump in a separate
  service/process; data-block write with `--yes` and trailer rejection;
  restore dry run, missing-confirmation refusal, and verified full restore;
  keys-list value hiding; UID dry run and verified write; and usage failures for
  `recover` without authorization and unknown commands.
- The packaging script was exercised in a temporary tree: it installs both
  binaries, rejects a missing CLI executable, and sets
  `$ORIGIN/runtime/linux-amd64` on both.

## Hardware status

The attached ACR122U opened through `acr122_usb` and reached card polling.
No card was present on the reader during this session, so the card-dependent
CLI commands could not be repeated on hardware. Phase 02 already recorded
Classic keyscan, complete dump, and verified write acceptance on this reader;
Phase 03 exercises the same `app.Service`/workflow paths through the CLI with
the mock transport. Phase 04 later repeated the card-dependent CLI commands on
this ACR122U with a Gen1A test card: `classic keyscan`, complete `classic
dump`, full `classic restore`, and `uid set` in both directions passed with
verified readback. A PN532 UART CLI repeat remains desirable but is no longer
the only hardware gate.

## Problems encountered

- The first `acr122_usb` open after enumeration returned transient PN532 init
  status-word failures; a second open reached polling. This matches the
  Phase 02 note about PC/SC/USB ownership timing.
- One physical ACR122U enumerates as USB, ACR122 PC/SC, and PC/SC. Without
  `--device`, `nfctl` correctly refuses to guess and prints the choices.
- Key verifications were previously memory-only, which made a separate
  `nfctl classic dump` unable to use keys verified by an earlier process. The
  key store now persists card-scoped verifications.
- Appending `go test` build tags required the installed `libnfc-ng` SDK rather
  than the repository-local toolchain output; both paths are supported by the
  Makefile.

## Deviations from the spec

- Type 2/NDEF, APDU, and the raw PN532 console are not registered in this
  phase. They require the card/transport layers from Phases 05, 07, and 04;
  shipping placeholder commands would advertise capabilities that do not
  exist.
- Recovery is exposed as top-level `recover <mode>` rather than
  `classic recover --mode`, matching the repository command list.
- `classic restore` requires `--confirm-trailers` in addition to `--yes`, and
  `uid set` requires `--confirm-uid`, because a single blanket flag is not a
  specific confirmation for trailer or UID writes.
- `classic write` intentionally supports data blocks only; protected writes go
  through the existing restore/UID workflows.

## Blockers and follow-up

- Final card-dependent CLI acceptance was completed in Phase 04 on the
  attached ACR122U: `classic keyscan`, complete `classic dump`, and `uid set`
  in both directions with verified readback all passed.
- Native Go Gen1A landed in Phase 04. The external `nfc-mfsetuid` fallback,
  diagnostic check, and release metadata entry were removed.
