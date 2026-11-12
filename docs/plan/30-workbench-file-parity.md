# Phase 30 plan: workbench and file parity

Status: pending 2026-11-12. Decisions from the parity review:

- restore the old sector-grouped block table and ASCII presentation;
- write/read `.nfcx.json` sidecars for image-only dump exports;
- add an explicit “export all candidates” action that includes built-ins;
- keep the current GTK layout; no old shell resurrection.

## Goal

Close the remaining presentation and file-interchange gaps between the old
workbench and the current GTK workbench without changing card-write semantics.

## Slices

### 30.1 Sector grouping and ASCII

- Replace the flat block table model with sector-aware rows:
  - sector header rows show `Sector NN`, block count, verified Key A/B count,
    and unsaved-change count;
  - block rows remain selectable and keep the current inspector actions;
  - sector headers are not selectable and do not open the block inspector.
- Preferred GTK implementation: `GtkTreeListModel` with sector parents and
  block children, consumed by the existing `ColumnView`; keep the current
  `BlockRow`/`BoxedAnyObject` model for child rows. If the GTK tree-list
  selection model proves unstable with the existing tests, use a sectioned
  list model with a separate sector list and keep the same user contract.
- Add the `ASCII` column:
  - printable ASCII `0x20..=0x7E` renders as-is;
  - other bytes render as `.`;
  - unknown bytes (`known_mask` clear) render as `.` and are never treated as
    zero;
  - the inspector gets a separate ASCII line for the selected block.
- Keep the `Data` column showing all 16 hex bytes and all unknown-byte masks.
- Update `docs/design/workbench.md` and `docs/design/classic-workspace.md`.
- Tests: row/header model, sector counts, unknown-byte ASCII, selection across
  headers, GTK scenario for a 1K and 4K image.

### 30.2 Dump sidecar metadata

- Add `<dump>.nfcx.json` sidecars for image-only exports:
  - `raw` `.bin`;
  - MCT `.mct`;
  - Proxmark3 JSON/.dic interchange.
- Sidecar content is metadata only; the image bytes remain authoritative:

  ```json
  {
    "version": 1,
    "complete": true,
    "layout": "classic1k",
    "card": {"uid": "04AABBCC", "atqa": "0004", "sak": 8},
    "device": "pn532_uart:/dev/ttyUSB0",
    "startedAt": 1760000000,
    "finishedAt": 1760000010,
    "keys": [
      {"sector": 0, "type": "A", "key": "FFFFFFFFFFFF", "source": "read_context"}
    ]
  }
  ```

- Parser compatibility:
  - accept the current project key shape (`type` + `key`);
  - accept the old app shape (`keyA`, `keyB`) and map each to a `DumpKey`;
  - accept and ignore legacy `blocks` metadata after validating its layout and
    length when present;
  - reject an unknown sidecar version with a warning, not a hard failure.
- Write behavior:
  - write the sidecar atomically next to the image;
  - file mode `0600` because it may contain keys;
  - default on; `--no-sidecar` disables it;
  - a failed sidecar write must not invalidate a successful image write; report
    it as a warning.
- Load behavior:
  - project JSON remains authoritative and never consults a sidecar;
  - raw/MCT/Proxmark3 load auto-detects `<path>.nfcx.json`;
  - a missing sidecar is normal;
  - a malformed sidecar warns and loads the image without metadata;
  - a sidecar whose card/layout does not match the image warns and does not
    capture keys;
  - matched keys are recorded through the existing `record_dump_keys` path
    with `read_context`/stored source provenance.
- CLI: extend `classic dump` and `classic restore` documentation;
  `classic dump --no-sidecar`; `classic read`/`dump` expose the sidecar path in
  human output and JSON.
- Tests: raw/MCT/Proxmark3 round trip with keys; missing/corrupt/mismatched
  sidecar; project precedence; legacy `keyA`/`keyB`; file mode/atomicity.

### 30.3 Export all key candidates

- Add a distinct action, not a change to the current export:
  - GUI: `Export all candidates…` next to the existing stored-key export;
  - CLI: `nfctl keys export --all-candidates` (default remains stored-only).
- Output is the unique union of:
  - every stored key;
  - the built-in common-key dictionary.
- Deterministic ordering: uppercase hex, sorted; `.dic` output is one key per
  line. JSON output may include provenance, but keys are never logged.
- Do not write built-in keys into the persistent store as a side effect.
- Update the exported-key counts and docs to distinguish `stored` from
  `all candidates`.
- Tests: union/dedup, built-in-only, stored-only, empty store, deterministic
  order, `.dic`/JSON output, no store mutation.

### 30.4 Documentation

- Update `docs/design/workbench.md`, `docs/design/classic-workspace.md`,
  README CLI examples, and the dump/CLI documentation.
- Record that the current GTK layout is retained; old persistent Activity,
  Tag health rail, and bottom action bar are not restored.

## Acceptance

- The Classic table groups by sector, shows verified-key/unsaved summaries,
  and includes a correct ASCII column/inset.
- Raw/MCT/Proxmark3 exports write sidecars; loads consume them without changing
  image bytes; project JSON remains authoritative.
- Sidecar metadata is never trusted on mismatch, and a corrupt sidecar never
  blocks the image.
- The GUI/CLI can export the complete built-in+stored candidate set without
  mutating the store.
- `cargo fmt`, clippy, workspace tests, GTK scenario, and `reuse lint` pass.

## Non-goals

- Restoring the old MyGo shell layout.
- Changing block-write gates or card mutation semantics.
- Writing keys into a card or changing the store on export.
- A new dump interchange format.

## First files

- `crates/gui/src/classic.rs` — block/ASCII rows.
- `crates/gui/src/app.rs` — tree/sectioned table, sidecar dialogs/actions.
- `crates/nfc-classic/src/exchange.rs` — sidecar encode/decode.
- `crates/nfc-app/src/classic/dump.rs`, `crates/nfctl/src/main.rs` — sidecar
  and candidate export.
- `docs/design/workbench.md`, `docs/design/classic-workspace.md`, README.
