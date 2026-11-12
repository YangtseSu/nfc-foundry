# Phase 27 plan: NDEF workbench editor

Status: complete 2026-11-12. Spec:
[27-ndef-workbench.md](../specs/27-ndef-workbench.md). Note:
[27-ndef-workbench.md](../notes/27-ndef-workbench.md).

Goal: port the reviewed NDEF workbench template from the previous
`../nfc-foundry` repository (`docs/design/02-workbench-ndef.html`) into the GTK4
workbench. The template defines a multi-record message editor, not the
single-record Text/URI/raw composer from phase 19b.

The previous repository never implemented NDEF write; the HTML/PNG mockup is
the design source. This phase ports its information architecture and completes
the write path for both Classic NDEF and Type 2 NDEF.

## Scope

### 1. Editor model

- Add `crates/gui/src/ndef.rs`, a GTK-free model:
  - ordered `RecordDraft` values for Text, URI, MIME, Wi-Fi WSC, vCard,
    Bluetooth LE OOB, Wi-Fi DPP, Smart Poster, and raw records;
  - lossless raw fallback for unknown, proprietary, external, empty, and
    malformed known records;
  - `NdefEditor` with selection, add/replace/remove/move, baseline/dirty
    tracking, `.ndef` load, message encode, TLV preview, and table rows.
- Preserve template extras on edit: WSC unknown attributes, Bluetooth LE
  unknown AD structures, Smart Poster additional titles/nested records.

### 2. NDEF page

Replace the single-record composer with the template layout:

- capacity card: used/total, meter, record count, free bytes, validity;
- record table: `# / Type / Preview / Size`, selection drives the inspector;
- record inspector: type, TNF, TYPE, ID, encoded size, payload, validity,
  Edit/Remove/Move up/Move down;
- preflight: TLV valid, fits user memory, protected areas untouched, Validate;
- raw message: `TLV 0x03` length and hex preview;
- actions: Load `.ndef…`, Reload from card, New message, Add record…,
  Save `.ndef…`, Format, Write NDEF.

### 3. Record editor dialog

- One modal form with a record-type selector and a form per record kind.
- Text: language, UTF-8/UTF-16.
- URI.
- MIME: media type, text/hex payload.
- Wi-Fi: SSID (text/hex), authentication/encryption presets or custom bits,
  optional network key and MAC.
- vCard: formatted name, organization, phone, email, URL.
- Bluetooth LE: address/type, role, name, appearance, flags.
- DPP: bootstrapping URI, URI or media carrier.
- Smart Poster: URI, title/language, action.
- Raw: TNF, TYPE (text/hex), ID hex, payload hex.
- Validate through `nfc-ndef` before changing the editor.

### 4. Card sync and writes

- A Classic read, Type 2 read, loaded dump, or explicit format resets the
  editor from that card.
- Incidental refreshes (write results, staged block edits) keep the draft.
- A draft blocks automatic reload until the user reloads, clears, or opens a
  file.
- Write uses the editor message for both `classic_ndef_write` and
  `type2_ndef_write`; a successful write updates the baseline. Format clears
  the editor.
- Keep the existing capacity, read-only, lock, unformatted, partial-image,
  live-card, and card-identity gates.

### 5. Tests and docs

- Model round-trip tests for every template, raw fallback, dirty tracking,
  `.ndef` load, TLV preview, and hex parsing.
- Gated GTK scenario coverage for multi-record composition, add/edit/remove/
  move, load/save, capacity refusal, Classic and Type 2 writes, format, and
  read-only/locked/identity gates.
- Update the workbench and Type 2 design contracts and the README.

## Order

1. Editor model and tests.
2. NDEF page and record dialog.
3. Card sync and write baselines.
4. Localization, tests, docs.

## Acceptance

- The NDEF page matches the template's capacity, records, inspector,
  preflight, and raw-message sections.
- Text, URI, MIME, Wi-Fi, vCard, Bluetooth LE, DPP, Smart Poster, and raw
  records can be added, edited, removed, and reordered; unknown records survive
  a load/edit/save round trip.
- A multi-record message writes to Classic and Type 2 with backup and read-back.
- `.ndef` files load into the editor and save from it.
- The editor reloads on card reads and format, and preserves unsaved edits on
  incidental refreshes.
- `cargo fmt --all --check`, workspace clippy with `-D warnings`, workspace
  tests, the gated GTK scenario, and `reuse lint` pass.
- No hardware write is performed without explicit authorization; mock tests
  cover the gates.

## Non-goals

- Type 4 and DESFire NDEF editing.
- A second shell or action bar; the current GTK shell owns reader, card,
  language, and status.
- CLI flags for every RTD form. `nfctl` can already write any message from a
  `.ndef` file.
- Automatic payload translation or decoding.

## First files

- `crates/gui/src/ndef.rs` — editor model.
- `crates/gui/src/app.rs` — NDEF page, dialog, sync, writes.
- `crates/gui/src/i18n.rs` — English/Chinese catalogs.
- `docs/design/workbench.md`, `docs/design/type2-workspace.md`.
