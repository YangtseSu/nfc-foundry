# Spec: phase 27 NDEF workbench editor

Status: complete 2026-11-12. Plan:
[27-ndef-workbench.md](../plan/27-ndef-workbench.md). Note:
[27-ndef-workbench.md](../notes/27-ndef-workbench.md).

## Surface

The shared NDEF tab renders one message editor for MIFARE Classic and Type 2.

- Header: card NDEF state and a facts block (MAD/CC, capacity, locks,
  warnings, password coverage).
- Capacity: used/total bytes, record count, free bytes, meter, and a validity
  label.
- Records: a table with `#`, `Type`, `Preview`, and `Size`; the selected row
  drives the inspector.
- Inspector: record ordinal, type, TNF, TYPE, ID, encoded size, payload hex,
  validity, Edit/Remove/Move up/Move down.
- Preflight: TLV validity, capacity fit, protected-area note, Validate.
- Raw message: `TLV 0x03` length and hex preview.
- Actions: Load `.ndef…`, Reload from card, New message, Add record…,
  Save `.ndef…`, Format NDEF area…, Leave the NDEF area read-only (Classic),
  Write NDEF.

## Editor model

- `RecordDraft` variants: Text, URI, MIME, Wi-Fi WSC, vCard, Bluetooth LE,
  Wi-Fi DPP, Smart Poster, Raw.
- Decoding tries the typed template decoders; any failure, uncovered TNF, or
  non-empty record ID becomes Raw with the exact TYPE/ID/payload bytes. A typed
  draft is accepted only when re-encoding reproduces the exact record, so
  unexposed template fields stay lossless.
- Text preserves UTF-8/UTF-16. MIME payloads and WSC SSID/key support text or
  hexadecimal input.
- Editing a typed record preserves template fields the form does not expose:
  WSC credential/outer attributes, Bluetooth LE extra AD structures, Smart
  Poster extra titles and nested records.
- The editor baseline is the last card message, loaded file, or formatted-empty
  state. `is_dirty` compares the encoded logical message with that baseline.

## Dialog

- A modal window with a record-type dropdown, one form per kind, feedback, and
  Save/Cancel.
- Save validates the draft through the NDEF codec and keeps the dialog open on
  error.
- Editing replaces the selected record; adding appends and selects it.

## Sync

- Classic read, Type 2 read, loaded project dump, and format reset the editor
  from the new card state even when a draft exists.
- Incidental refreshes do not replace a dirty draft.
- Loading a `.ndef` file detaches the editor from the current card baseline.
- A successful write sets the baseline to the written message and keeps the
  selection; a successful format clears the editor.

## Gates

- Write remains disabled without a live card, a complete image, a formatted
  NDEF area, a writable mapping/CC, no locked/password-protected pages, an
  encodable message with at least one record, capacity headroom, and a message
  different from the card.
- Format remains available for a live complete image and uses the existing
  destructive confirmation.
- The service remains authoritative for backups, read-back, expected card
  identity, and NDEF/TLV preservation.

## Files

- Loading uses `NdefMessage::decode` and rejects trailing octets.
- Saving encodes the editor message, not the last card message.
- `Load .ndef…` keeps the previous message when the file is malformed.

## Tests

- Model tests cover every template round trip, raw fallback, dirty tracking,
  selection/move/remove, TLV preview, `.ndef` load, and hex parsing.
- The gated GTK scenario covers multi-record composition, add/edit/remove/move,
  `.ndef` load, malformed-file retention, capacity refusal, Classic and Type 2
  writes, format, read-only/locked/unformatted gates, and identity refusal.
- Normal workspace tests stay hardware-free and headless-safe.
