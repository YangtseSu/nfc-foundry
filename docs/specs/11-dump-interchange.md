# Phase 11 spec: Key and dump interchange formats

Date: 2026-11-09

## Goal

Make complete MIFARE Classic images and key dictionaries exchangeable with
MIFARE Classic Tool (MCT), Proxmark3, and RFID Tools without inventing a new
binary format. Keep partial images, block certainty, key provenance, and
verification state in the NFC Foundry project file. Never turn an incomplete
image into a plausible-looking complete external dump.

## Affected packages and interfaces

- `internal/workflow`: new `DumpFormat`, `DumpLoad`, `LoadDumpFile`, and
  `ExportDumpFile` APIs beside the existing project/raw loaders. `DumpResult`
  and `mifare.Dump` stay the in-memory model.
- `app`: dump save-format methods, load warnings on `WorkbenchDTO`, key import
  with an optional key set, the `.keys`/`.dic` key-set artifact change, and
  Proxmark3 raw key-array import/export.
- `internal/cli`: `classic export`, format flags, import warnings, and key
  import/export flags.
- `internal/desktop`: save-format selector, load filters/warnings, key import
  dialog, and key-set format options. New strings are localized.
- No C shim, ABI, packaging, or transport change. No new dependency.

## Safety and failure handling

- Loaders are read-only and never authenticate against the current card or
  register imported keys as verified.
- External exports require a complete image, valid trailers, and a valid BCC
  when a four-byte UID is known.
- Files are written through the existing atomic temporary-file path; a failed
  external export removes the temporary file and does not mark the workbench
  saved.
- Imported partial/corrupt state is rejected with a concrete error before the
  workbench loads it. No silent zero-fill, repair, or truncation.
- **Hardware gates:** none. The interchange layer is file-only and its tests
  run without a reader.

## Packaging and license impact

No packaging change. All new code and documentation is GPL-3.0-or-later under
the repository default REUSE annotation. Upstream MCT and Proxmark3 sources
were consulted for the published format shapes; no upstream files, examples,
or binaries are copied into the repository.

## Format identifiers

`internal/workflow` owns the interchange layer in new files; `DumpResult` and
`mifare.Dump` remain the in-memory model. Format detection is content-based,
not extension-only.

| Identifier | Content | Complete | Metadata |
|---|---|---|---|
| `project` | NFC Foundry `.nfcx.json` | partial allowed | full |
| `raw` | `.bin` / `.mfd` bytes, optionally with `<file>.nfcx.json` | required | sidecar optional |
| `mct` | MCT text `+Sector: N` | required | none |
| `eml` | one 32-hex block per line | required | none |
| `mct-json` | MCT `mfcard` JSON `blocks` map | required | optional `Card` |
| `proxmark3` | Proxmark3 `mfc v2` JSON | required | `Card`, `SectorKeys` |
| `keyarray` | Proxmark3 raw key array, 192/480 bytes | required | none |
| `sidecar` | NFC Foundry metadata for an existing raw file | required | full |

`proxmark3` import also accepts `mfc v3`. Proxmark3 `mfc v3` `Card.ATS` and
`Card.SIGNATURE` have no field in `nfc.CardInfo`; import reports a warning
that they were dropped instead of silently discarding them.

## Detection and validation

1. Read the file once and detect the format from its bytes.
2. JSON is detected by a leading object and its keys:
   - lowercase `version` plus `layout` and an array `blocks` = `project`;
   - `FileType` case-insensitive `mfcard` = `mct-json`;
   - `FileType` case-insensitive `mfc v2` / `mfc v3` = `proxmark3`;
   - an object `blocks` map with no `FileType` = `mct-json`;
   - any other `FileType` is rejected, so an `mfu` or unknown JSON never
     falls through to a Classic importer.
3. Text is detected only when the bytes are printable ASCII plus whitespace:
   - a `+Sector:` header selects `mct`;
   - every non-empty line being exactly 32 hex digits selects `eml`.
4. Remaining bytes are accepted as `raw` only at exactly 64 or 256 blocks and
   only after trailer validation.
5. Every importer rejects unknown sizes, missing sectors/blocks, duplicate
   block keys, non-hex content, and truncated JSON. JSON objects are checked
   for duplicate keys before decoding because Go maps would otherwise hide
   them.
6. All imported images are validated with `Dump.ValidateTrailers`. When the
   source provides a four-byte UID, `Dump.ValidateBCC(4)` must also pass.
7. A raw file uses its `.nfcx.json` sidecar when present and validates the
   sidecar against the raw bytes as before. Without a sidecar the image is
   loaded with empty device/card metadata and a warning that identity and key
   provenance are unavailable.

MCT text layout follows sector geometry: sectors 0-31 contain four blocks and
sectors 32-39 contain sixteen. A file is one layout only; a missing or extra
block, or a sector number outside the layout, is an error. Sector headers may
appear only once; all sectors of the detected layout must be present.

## Export rules

- `project` writes the existing NFCX JSON and preserves partial state.
- `raw` writes the complete image and its sidecar atomically.
- `sidecar` writes only the metadata file for a complete image; a path
  without `.nfcx.json` gets that suffix.
- `mct`, `eml`, `mct-json`, `proxmark3`, and `keyarray` require a complete
  image and pass the same BCC/trailer checks as a raw save. A partial image is
  rejected with a clear error; it is never zero-filled.
- `mct` emits every sector in order with `+Sector: N` and uppercase hex.
- `eml` emits one uppercase 32-hex block per line.
- `mct-json` emits `Created: "NFC Foundry"`, `FileType: "mfcard"`, and a
  decimal-keyed `blocks` map. It does not claim a card identity.
- `proxmark3` emits `FileType: "mfc v2"` with `Created: "NFC Foundry"`,
  `Card` (UID/ATQA/SAK), `blocks`, and `SectorKeys` containing Key A, Key B,
  `AccessConditions`, and human-readable access-condition text.
- `keyarray` emits the Proxmark3 raw key array: 192 bytes for 1K or 480 bytes
  for 4K. The first half is Key A for sectors 0..N-1; the second half is Key B
  for the same sectors. It is derived from the trailer blocks of a complete
  dump.
- Proxmark3 `SectorKeys` that disagree with the imported trailer bytes are an
  error. `SectorKeys` are preserved into `DumpResult.Keys`; unrelated trailer
  data is not reinterpreted as verified keys.
- Loading or importing a dump never registers dump keys as verified keys for
  the current card. That rule from Phase 10 is unchanged.

## Key interchange

- `.keys` (MCT) and `.dic` (Proxmark3) are the same text: one 12-hex key per
  line, `#` comments, whitespace tolerated, duplicates removed. The extension
  selects the target tool's vocabulary.
- `keys import FILE [--set NAME] [--format dic|keys|keyarray]` imports into
  the global catalog and, when `--set` is present, also puts every valid
  unique key from the file into a global named key set. An existing set with
  the same name is extended. `keyarray` requires a 192-byte 1K or 480-byte 4K
  Proxmark3 raw key array; text formats use the dictionary parser.
- `keys export FILE [--set NAME] [--format dic|keys]` writes values only.
  `--set` exports that set; without it the complete catalog is written.
  `--format` selects the `.dic` or `.keys` vocabulary and does not change the
  line content. A key set is not a sector map, so raw key arrays are exported
  from a complete dump with `classic export FILE --format keyarray`.
- `keys set export --format dic|keys` writes values only. The previous JSON
  key-set artifact remains available explicitly as `--format json`; names and
  verification state stay authoritative in `keys.json`.
- Exported key files are mode 0600.

## CLI

```text
nfctl classic dump FILE.bin|FILE.mfd|FILE.json|FILE.mct|FILE.eml [--keys F] [--scan]
nfctl classic export FILE [--format raw|project|proxmark3|mct|mct-json|eml|sidecar]
    [--keys F] [--scan]
nfctl classic restore FILE... [--keys F] [--scan] [--yes] [--confirm-trailers] [--dry-run]
nfctl classic restore ... prints every import warning before the preflight.
nfctl keys import FILE [--set NAME]
nfctl keys export FILE [--set NAME] [--format dic|keys]
```

`classic dump` accepts external output paths and infers an unambiguous format
from the extension. `.json` means the NFC Foundry project so existing scripts
keep their meaning. `classic export` reads the card like `classic dump` but
requires an explicit format, resolving the `.json` ambiguity. `sidecar`
writes `<FILE>.nfcx.json` and reports the metadata path. `classic restore`
auto-detects every import format.

## GUI

- The Classic load picker accepts `.bin`, `.mfd`, `.json`, `.mct`, and
  `.eml`.
- The Classic save area gains a format selector:
  NFC Foundry project, raw image, Proxmark3 JSON, MCT JSON, MCT text, EML,
  and metadata sidecar. Selecting a non-project format with an incomplete
  image is refused with the same message as the CLI.
- The save dialog chooses the default file name and filter for the selected
  format.
- Load warnings appear in the log and as a toast, not only in the returned
  DTO.
- Key import opens a small dialog with an optional key-set name; confirming
  then opens the file picker and imports. Key export offers `.keys`, `.dic`,
  and `.txt` filters. The key-set export dialog offers DIC, MCT `.keys`, and
  JSON artifact formats.
- New user-visible strings go through the i18n catalogue in English and
  Simplified Chinese.

## Tests

Hardware-free:

- Round trips: raw+sidecar, MCT text, EML, MCT JSON, and Proxmark3
  `mfc v2`, including card identity and `SectorKeys` where present.
- `mfc v3` import with ATS/SIGNATURE warning.
- Proxmark3 `SectorKeys` preserved and conflicting keys rejected.
- Partial images rejected by every external export.
- Malformed inputs: wrong block count, missing sector, bad trailer, bad BCC,
  duplicate JSON block key, truncated JSON, non-hex key text, oversized file,
  and unknown `FileType`.
- Raw file without a sidecar loads with empty identity and a warning.
- Key import/export: `.dic` and `.keys` content equality, comments,
  duplicates, `--set` creation/extension, file mode 0600, and 1K/4K raw
  key-array layout and import.
- CLI smoke tests for `classic dump` to MCT/EML, `classic export`, and
  `classic restore --dry-run` from external formats.
- GUI smoke tests for the format selector and key import dialog.

## Non-goals

- No partial external dump and no zero-filling unknown blocks.
- No new proprietary Classic binary format.
- No Type 2 `mfu` JSON in this phase.
- No DESFire or FeliCa dump interchange.
