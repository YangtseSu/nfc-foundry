# Note: phase 27 NDEF workbench editor

Plan: [27-ndef-workbench.md](../plan/27-ndef-workbench.md). Spec:
[27-ndef-workbench.md](../specs/27-ndef-workbench.md).

## Source

The previous `../nfc-foundry` repository never implemented NDEF write. Its
`docs/design/02-workbench-ndef.html` / `.png` mockup is the design source for
the multi-record NDEF workbench. This phase ports the mockup's information
architecture into the GTK4 shell and completes the write path for Classic and
Type 2.

## Implemented

### Model

- `crates/gui/src/ndef.rs` adds a GTK-free editor model:
  - `RecordKind` and typed drafts for Text, URI, MIME, Wi-Fi WSC, vCard,
    Bluetooth LE OOB, Wi-Fi DPP, Smart Poster, and raw records;
  - `RecordDraft::from_record` tries the template decoders and falls back to
    raw bytes for unknown, external, empty, or malformed known records; a typed
    draft is accepted only when re-encoding reproduces the exact record, so a
    record ID or an unexposed template field cannot be lost;
  - `NdefEditor` owns ordered records, selection, baseline/dirty tracking,
    add/replace/remove/move, `.ndef` loading, message encoding, TLV preview,
    and table rows;
  - typed edits preserve WSC unknown attributes, Bluetooth LE extra AD
    structures, and extra Smart Poster titles/nested records.
- `NdefEditor::message` rejects an empty record list. A successful write sets
  the baseline to the written message; format resets it to empty.

### NDEF page

- Replaced the single-source composer with the template layout:
  - capacity card with used/total, meter, record count, free bytes, and
    validity;
  - records table (`#`, `Type`, `Preview`, `Size`) with Load `.ndef…`, Reload
    from card, New message, and Add record…;
  - selected-record inspector with TNF/TYPE/ID/size/payload, validity,
    Edit/Remove/Move up/Move down;
  - preflight (`TLV valid`, `Fits user memory`, `Protected areas untouched`,
    `Validate`);
  - raw `TLV 0x03` length/hex preview;
  - actions: Save `.ndef…`, Format NDEF area…, Leave the NDEF area read-only
    (Classic), Write NDEF.
- The page still shows the active card's Classic MAD or Type 2 CC/TLV facts
  and warnings.

### Record dialog

- One modal dialog with a type selector and a form per kind. Save validates
  through the NDEF codec and keeps the dialog open on error.
- Media payloads and WSC SSID/key accept text or hexadecimal. Raw records use
  TNF + TYPE (text/hex) + ID hex + payload hex.
- Wi-Fi authentication/encryption use presets with a custom-bits field; an
  edited record keeps its unknown attributes.
- The record form test covers every kind's fields and encoding.

### Sync and writes

- Classic read, Type 2 read, loaded project dump, and format invalidate the
  editor and reload it from the new card state.
- Incidental refreshes keep a dirty draft. The page header shows
  `Unsaved changes` / `No unsaved changes`. Loading `.ndef`, Reload from card,
  or New message explicitly replaces the draft.
- Write uses the editor message for both `classic_ndef_write` and
  `type2_ndef_write`; the existing capacity, read-only, lock, unformatted,
  partial-image, live-card, password, and expected-card gates are unchanged.
- Export saves the editor message, not the last card message.

### Localization

- Added English/Simplified Chinese catalog entries for the page, dialog,
  record labels, capacity/preflight/raw strings, and Type 2 placeholders.
- The gated language switch scans the visible NDEF page and reports no
  untranslated chrome.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes; the GTK scenario takes its skip branch
  without the gate.
- `NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env GDK_BACKEND=x11 cargo test -p
  nfc-foundry -- --nocapture gtk_workbench_scenarios`: passes.
- `reuse lint`: clean.

The gated scenario covers Text/URI multi-record composition, add/edit/remove/
move, `.ndef` load and malformed-file retention, capacity refusal, dialog
forms for every record kind, Classic and Type 2 writes, format, read-only,
locked, unformatted, partial-image, loaded-dump, and identity gates.

## Limits

- No hardware was touched. Classic and Type 2 writes were exercised only
  against the scripted transport.
- The previous repository's mockup had Text/URI/MIME rows; this phase exposes
  all RTD templates in `nfc-ndef` plus a lossless raw form.
- Type 4 and DESFire NDEF editing remain in their own phases.
- The old mockup's top bar/rail/action bar were not duplicated; the current
  GTK shell owns reader, card, language, and status. `Reload from card`
  replaces the mockup's NDEF-only read action; the global `Read card` performs
  the actual card read.
