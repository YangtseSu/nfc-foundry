# Phase 00 note: Rust foundation

Date: 2026-11-09

Historical note: the GPUI Kit shell recorded here was removed in phase 19.
The GUI stack is now GTK4-rs (`gtk4` + `libadwaita`, [ADR 0006](../adr/0006-gtk4-rs.md)).

## Implemented

- Rust workspace with `nfc-core`, `nfc-transport`, `nfc-drivers`,
  `nfc-classic`, `nfc-recover`, `nfc-ndef`, `nfc-type2`,
  `nfc-classic-ndef`, `nfc-type4`, `nfc-app`, `nfctl`, and the GPUI Kit
  `nfc-foundry` shell.
- Core card identity, error type, key parsing/redaction, ISO/IEC 14443A
  constants.
- Reader/capability traits and a scripted `MockReader`.
- Classic geometry and domain type skeleton.
- Recovery strategy/card-profile types and the initial `auto` planner.
- NDEF message/record/TLV/template type skeleton and Type 2/Classic/Type 4
  adapters.
- Application device manager/service skeleton.
- CLI command surface for Classic, Type 2, NDEF, recovery, list, scan, and
  diagnostics. Unimplemented operations return `NfcError::NotImplemented`.
- GPUI Kit bootstrap window with the two-domain shell.
- Architecture, hardware, recovery, NDEF, testing, packaging, design, ADR, and
  phase plan documentation.
- GPL-3.0-or-later license, NOTICE, REUSE metadata, and CI workflow.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` (key parsing, Classic geometry, mock transport,
  recovery planner, device selection)
- `cargo check -p nfc-foundry`
- `reuse lint`
- CLI smoke: `nfctl list`, `nfctl diag`, and an unimplemented command returning
  exit code 2.

## Problems and resolutions

- Result aliases now expose a defaulted error parameter, matching the project
  Rust style rule.
- The initial mock reader had no way to observe raw mode in tests; an explicit
  reader query was added.
- Clippy rejected `chunks_exact(2)`, identical strategy branches, and nested
  `if` formatting; the code was simplified rather than suppressed.

## Deviations

- No hardware was touched. PN532 UART and ACR122U drivers return
  `NotImplemented` until phases 02 and 05.
- The GUI is a minimal shell; feature views arrive with their domain phases.

## Remaining blockers

- PN532 UART and ACR122U USB/CCID validation.
- PM3-source recovery port and hardnested tables.
- NDEF codec and Type 2/Classic NDEF workflows.
