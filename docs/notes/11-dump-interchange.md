# Phase 11 note: Key and dump interchange formats

Date: 2026-11-09

## Implemented

- `internal/workflow/dump_exchange.go` adds content-based detection and
  import/export for NFC Foundry projects, raw images with optional sidecars,
  MCT text (`.mct`), EML (`.eml`), MCT `mfcard` JSON, and Proxmark3
  `mfc v2`/`mfc v3` JSON.
- Raw import keeps the existing sidecar validation and reports a warning when
  no sidecar exists. External imports validate trailer geometry and, when the
  source has a four-byte UID, block 0 BCC.
- JSON objects are scanned for duplicate keys before decoding, because Go maps
  would otherwise hide duplicate `blocks` entries.
- External exports are complete-image only. A partial image remains an NFC
  Foundry project and is never zero-filled.
- Proxmark3 import preserves `SectorKeys`, rejects keys or access conditions
  that disagree with the trailer bytes, and warns that v3 `ATS`/`SIGNATURE`
  fields are dropped because `nfc.CardInfo` cannot store them.
- Proxmark3 export writes `mfc v2` with `Card`, `blocks`, `SectorKeys`, and
  access-condition text.
- Proxmark3 raw key-array import/export is separate from text dictionaries:
  192 bytes for 1K or 480 bytes for 4K, Key A for all sectors followed by
  Key B for all sectors. Export requires a complete dump. Import adds every
  key to the catalog and optionally to a named key set. The GUI labels it
  "Proxmark3 raw key array" and offers a format selector in the key import
  dialog and the Classic save selector.
- `nfctl classic export FILE --format ...` was added; `classic dump` infers
  `.bin`, `.mfd`, `.json`, `.mct`, and `.eml`; `classic restore` auto-detects
  every import format and prints import warnings.
- `nfctl keys import FILE --set NAME` creates or extends a global named key
  set. `nfctl keys export FILE [--set NAME] [--format dic|keys]` writes
  value-only text.
- GUI: Classic save-format selector, wider load filters, load warnings in the
  log and toast, key import dialog with an optional key-set name, and DIC/MCT
  keys/JSON key-set export options.
- Locale catalogue regenerated with the new English and Simplified Chinese
  strings.

## Tested

- Workflow round trips: raw+sidecar, MCT text 1K/4K, EML, MCT JSON, Proxmark3
  `mfc v2` 1K/4K, and `mfc v3` warning behavior.
- Malformed inputs: missing/duplicate MCT sectors, short and non-hex EML,
  duplicate JSON keys, unknown JSON `FileType`, truncated JSON, invalid access
  bits, bad BCC, oversized files, and conflicting Proxmark3 `SectorKeys`.
- Partial external exports rejected without creating a file.
- App service: explicit save formats, load warnings, sidecar export,
  raw key-array export, key import into an existing key set, and key-array
  import into a set.
- CLI smoke tests: MCT/EML dump output, Proxmark3/MCT JSON/EML/sidecar export,
  raw key-array export and import, restore dry-run from Proxmark3 JSON,
  restore refusal for identity-less EML, and key import/export formats.
- GUI smoke tests: format selector selects the backend format, incomplete
  dumps refuse external formats, and the key import dialog passes the format
  and set name.
- `make check` passes: `go test ./...`, `go vet ./...`, i18n check, desktop UI
  tests, and `reuse lint`.

## Problems and resolutions

- The first MCT draft inferred 4K from sector 32; the correct boundary is
  sector 16 because a 1K card has sectors 0-15. Layout validation now uses the
  actual `mifare.Layout` geometry.
- Proxmark3 `mfc v3` has no field in `nfc.CardInfo` for `ATS` or `SIGNATURE`.
  The import reports each dropped field as a warning instead of discarding it
  silently.
- CLI interchange tests initially dumped an incomplete image because no key
  scan had run. The tests now use `--scan`, matching the documented default
  that reading does not silently scan.
- The key-set export dialog test cannot see options that only exist inside a
  closed select popup; it checks the rendered selected label instead.

## Deviations

- Static upstream-derived fixture files were not copied into `testdata/`.
  Tests generate complete synthetic images and exercise the exact MCT and
  Proxmark3 JSON shapes from their published format code, avoiding upstream
  example licensing and stale-file maintenance.
- `keys set export --format keys` now writes MCT text, not JSON. The JSON
  key-set exchange object remains available as `--format json`. This follows
  Phase 11's decision that `.keys` is the MCT vocabulary.
- `classic export` is the explicit-format counterpart to `classic dump`; like
  `dump`, it reads the current card rather than converting an existing file.

## Remaining blockers

Real-device interoperability with the MCT Android app and a Proxmark3 client
was not exercised; validation is file-level against their published formats.
No hardware is required for the interchange layer itself.
