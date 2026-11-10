# Phase 19b plan: NDEF write and format

Status: complete 2026-11-11. Implementation record:
[notes/19b-ndef-write.md](../notes/19b-ndef-write.md).

Starting point: `feat/gtk4-ui` at `e5d0995` (clean). Phase 19a completed the
Classic workbench, PM3 interchange, Keys, Diff, Restore, identity checks, and
the mock/Xvfb GTK test entry point.

Goal: close the NDEF write path for MIFARE Classic before Type 2/Type 4
screens. The existing read-only NDEF tab becomes a real composer with a
verified write and an explicit format action. Reuse the same draft model for
Type 2 later.

## Scope

### 1. NDEF draft model

- Add a small presentation model in `crates/gui` for a message being composed:
  - Text record: text and IANA language;
  - URI record;
  - raw NDEF file loaded through `NdefMessage::decode`.
- Encode through `nfc_ndef::{NdefMessage, TextRecord, UriRecord}`. Do not
  duplicate codec logic in the GUI.
- Match the CLI's current behavior: exactly one source (text, URI, or raw
  file) per write. A multi-record composer and the full RTD catalog are a
  later increment.
- Unit-test draft encoding, capacity calculation, and malformed raw input.

### 2. Classic NDEF page

- Keep the current read-only presentation: mapping, sectors, capacity,
  state, records, warnings.
- Add:
  - record type selector: Text / URI / Raw file;
  - language field for Text;
  - text or URI field;
  - file picker for raw NDEF;
  - preview of the encoded record/message and its size;
  - `Write NDEF` action with old/new summary, capacity check, backup, and
    read-back;
  - `Format NDEF area` action with a destructive confirmation and a
    `Read-only NDEF` option matching the CLI;
  - clear handling for unformatted cards: explain that Format is required
    before writing.
- Disable write/format when there is no live card, the image is a loaded
  dump, the NDEF area is read-only, the message exceeds capacity, or the
  operation is busy.
- On success, use the service result's dump/info to refresh the block table,
  NDEF page, card summary, and log. Do not perform a redundant full read.
- On failure, keep the previous image visible and report the service error in
  the toast and log.

### 3. Service flow

- Use the existing `Service::classic_ndef_write` and
  `Service::classic_ndef_format`.
- Pass the loaded image's `CardInfo` as `expected_card`, as the block write and
  restore paths already do.
- Formatting must call `.confirm(true)` and may set `.read_only(true)` when
  the user selected it.
- Run both operations on a worker thread; return through `async-channel` and
  update GTK on the main context.
- The service owns the backup and read-back verification. The GUI reports the
  backup path.

### 4. Tests

- Reuse the phase 19a scripted workbench test. Add a Classic NDEF mock card
  with a formatted NDEF area and an unformatted card.
- Cover:
  - Text and URI encoding;
  - raw file decode;
  - capacity refusal;
  - unformatted state requires Format;
  - read-only NDEF refusal;
  - write confirmation/cancel;
  - format confirmation/cancel;
  - identity mismatch before write;
  - post-write refresh from the service result.
- Keep the `NFC_FOUNDRY_GUI_TESTS=1` + Xvfb gate.
- The normal `cargo test --workspace` path must remain hardware-free and
  headless-safe.

## Order

1. Draft model and unit tests.
2. NDEF page controls and capacity/state presentation.
3. Write flow with confirmation and worker.
4. Format flow with read-only option and destructive confirmation.
5. Mock/Xvfb tests and docs.

## Acceptance

- A Classic NDEF card can be composed and written as Text, URI, or a raw NDEF
  file; the write is backed up and read back by the service.
- A too-large message, read-only area, partial image, loaded dump, or
  identity mismatch cannot write.
- Formatting requires explicit confirmation, can set the area read-only, and
  reports the backup path.
- The block table and NDEF page refresh from the service result.
- `cargo fmt --all --check`, workspace clippy with `-D warnings`, workspace
  tests, the gated GTK scenario, and `reuse lint` pass.
- No hardware write is performed without explicit authorization; mock tests
  cover the write gates.

## Non-goals

- Type 2/Type 4 screens.
- A multi-record composer and vCard/Wi-Fi/Smart Poster/BLE/DPP forms.
- Empty-message erase semantics beyond Format; add a service operation first
  if erase is required.
- `zh-CN`, packaging, and Windows/macOS.

## First files

- `crates/gui/src/app.rs` — NDEF page, draft state, worker calls.
- `crates/gui/src/classic.rs` — draft/format helpers and tests.
- `crates/nfc-app/src/service.rs` — existing write/format entry points.
- `crates/nfctl/src/main.rs` — CLI behavior to mirror.
- `docs/design/workbench.md` — NDEF screen contract.
