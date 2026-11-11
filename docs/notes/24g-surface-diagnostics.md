# Phase 24 S5 implementation record

Date: 2026-11-12. Status: complete.

## What changed

`nfc-core`: `CardType::MifareMini` for SAK `0x09`/`0x0D` (cascade bit
ignored), `CardType::label()`, and `CardType::classic_layout()`. `nfctl`
reports the type and a note that Mini is identified only; every Classic
layout lookup refuses it with a Mini-specific message.

`nfc-app`:

- `health.rs` with `CardHealth`, `CardIdentityHealth`, `KeyCoverage`, and
  `classic_health`. The model derives completeness from `Dump::readability`,
  errors from geometry, trailer access bits, and `Dump::validate_bcc`, warnings
  from partial images and block errors, coverage from card-scoped key
  verifications, and metadata from `DumpMetadata`.
- `diagnostics.rs` with `SelfCheck`, `AppInfo`, `PlatformInfo`, `DriverInfo`,
  `ReaderDevice`, `ReaderListing`, `TablesStatus`, and `CapabilityReport`.
  `Service::self_check` enumerates readers and snapshots the selected-reader
  state without card I/O.
- `build.rs` embeds the short git commit and a UTC build date, with explicit
  environment overrides and `SOURCE_DATE_EPOCH` support.
- `Service::read_card` refuses a Mini card explicitly.

`nfc-transport` and drivers:

- `Reader::firmware() -> Option<String>` with a default `None`.
- `FirmwareVersion::label()` and implementations for PN532 UART and ACR122U;
  the mock reports `mock reader firmware`.
- `ReaderSnapshot::firmware`; `MonitorState::opened` now takes capabilities and
  firmware, and a reset clears both.

Surfaces:

- `nfctl diag [--json]` uses the shared self-check. The text form keeps the
  existing release/version/reader lines and adds commit, build date, platform,
  firmware, capabilities, and selected-reader status.
- `nfctl card health --input [--format]` prints the shared health model for a
  saved image. `--key-store` and `--key`/`--key-file` apply as everywhere else;
  metadata keys from the image are merged for coverage.
- `nfctl classic keys list [--builtin]`, `classic keys delete`, and the existing
  add/import/export paths share one store.
- `nfctl recover` requires confirmation unless `--yes`; the non-interactive
  path fails with `needs confirmation; pass --yes`.
- The GTK Classic workspace gained a Details tab. It shows the health model,
  image source (`live card` or the loaded path), and live/inspection state.
  The tab is refreshed on read, load, key changes, and workbench edits.
- The key page lists stored entries and built-in common keys, enables Delete
  for stored rows only, persists the store after deletion, and shows the
  last import report with line-numbered issues.
- Diagnostics shows commit, build date, platform, driver prefixes, firmware,
  reader capabilities, tables, and readers.
- Recovery shows an authorization dialog before the worker starts.

## Decisions

- Health is one shared model in `nfc-app`, not GUI formatting. The GUI adds
  only source and live-state labels. This keeps `nfctl card health` and the
  Details tab from drifting.
- Key coverage counts card-scoped verifications. A stored candidate key that
  was never accepted by the card is not coverage, matching the key model.
- Built-in common keys are never inserted into the store to display them. A
  stored key with the same value suppresses the duplicate built-in row.
- `CardHealth` reports identity from the recorded `Dump.card`. An image without
  identity reports no UID/ATQA/SAK rather than decoding block 0, because a
  seven-byte UID uses a different manufacturer-block layout.
- Mini is identity-only. A layout mapping would imply read/write support that
  no framework code implements or validates.
- Firmware is diagnostic text on the transport trait, not a typed PN532 struct,
  so the transport boundary does not depend on driver types.
- The self-check is built from cached GUI reader state so rendering never
  opens a reader. `nfctl diag` enumerates because it owns no monitor.
- The recovery dialog is neutral, not destructive: recovery does not write the
  card, but it does occupy the reader and can take minutes.

## Verification

Run on 2026-11-12:

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — pass.
- `NFC_FOUNDRY_GUI_TESTS=1 cargo test -p nfc-foundry gtk_workbench_scenarios`
  — pass under the session display.
- `reuse lint` — pass.

Covered scenarios:

- `nfc-app` health tests: complete image, partial image with a skipped sector,
  invalid access bits plus BCC mismatch, coverage/dirty sorting, and an image
  without identity.
- `nfc-app` diagnostics tests: stable JSON keys, no card/key/dump fields,
  reader enumeration failure, and table lookup status.
- Service tests: `self_check` reports enumerated readers and open firmware; a
  Mini card is identified and `read_card` is refused before any block read.
- CLI suite: `diag --json` schema and leak check, `card health` values, key
  delete persistence and built-in refusal, Mini refusal, and recovery
  confirmation without `--yes`.
- GTK scenarios: Details values against the dump/key model, dirty update,
  key coverage update, key row delete and built-in refusal, diagnostics
  fields, and the recovery authorization gate.

No hardware run is required. The slice adds no reader command sequence or card
write path; firmware reporting uses the identity the driver obtained at open.
