# Phase 11: Key and dump interchange formats

## Goal

Make dumps and key files usable by MIFARE Classic Tool (MCT), Proxmark3, and
RFID Tools without losing NFC Foundry's partial-dump and verification
metadata. Use open, already-published formats at the boundary; keep only the
state that no external format can represent in our sidecar.

## Format reference

| Project | Key interchange | Dump interchange |
|---|---|---|
| [MCT](https://github.com/ikarus23/MifareClassicTool) | `.keys` text: one 12-hex key per line, `#` comments, blank lines, no sector binding. Also accepts `.dic`/`.txt` content in its import/export tool. | `.mct` text (`+Sector: N`, 16-byte hex block per line), `.bin`/`.mfd` raw 1024/4096 bytes, `.eml` one 32-hex block per line, `.json` `FileType: mfcard` with a `blocks` object. |
| [Proxmark3 (Iceman)](https://github.com/RfidResearchGroup/proxmark3) | `.dic` text: one hex key per line, `#` comments; key length 4/8/16/24 bytes supported, 6 for MIFARE Classic. Binary `hf-mf-<uid>-key.bin` is a separate raw key array, not `.dic`. | `.bin` raw block image; `.json` `FileType: "mfc v2"` or `"mfc v3"` with `Card`, `blocks`, `SectorKeys` and access-condition text; `.eml` for the emulator. |
| [RFID Tools](https://github.com/RfidResearchGroup/RFIDtools) | Plain key files in the Proxmark3 family. | Accepts `.bin`, `.hex`, `.eml`, `.mfd`, `.mtd`, `.txt`, `.json`, `.dump`; dump editing follows Proxmark3/MCT. |
| NFC Foundry today | Internal `keys.json`; import/export text is one 12-hex key per line with `#` comments, so it already reads MCT `.keys` and Proxmark3 `.dic`. | Raw `.bin`/`.mfd` plus `<name>.nfcx.json`; custom project JSON with `knownMask`, block status, card/device/time, and verified keys. Type 2 uses project JSON and `NFCT2BIN`. |

The raw image bytes are identical across MCT `.mfd`, Proxmark3 `.bin`, libnfc,
and mfoc. The key text formats are also effectively identical. The mismatch is
only in JSON metadata and in the absence of a standard for partial images.

## Decision

1. **Canonical card image:** raw 16-byte block image (`.mfd` or `.bin`,
   1024/4096 bytes for Classic 1K/4K). No custom header, no text conversion.
2. **Complete-dump metadata interchange:** Proxmark3 `mfc v2` JSON. Import
   `mfc v2` and `mfc v3`; export `mfc v2` for the widest compatibility.
   MCT's `mfcard` JSON is also imported and exported.
3. **Partial/internal state:** keep `<name>.nfcx.json`. It is the only place
   that records unknown blocks, `auth_failed`, `knownMask`, key provenance,
   and verification state. Never fabricate zeros to make a partial dump fit an
   external format.
4. **Key interchange:** MCT `.keys` and Proxmark3 `.dic` text. The content is
   the same; choose the extension by target tool. Internal `keys.json` remains
   authoritative for names, key sets, and verifications.
5. **MCT `.mct` and `.eml`:** import and export only; never internal formats.
6. **Type 2:** keep the self-describing `NFCT2BIN` as an internal complete
   image and add Proxmark3 `mfu` JSON import/export in a later step; do not
   force Type 2 through Classic formats.

If one ecosystem must be chosen for all dump metadata, choose **Proxmark3**:
its JSON is the richest machine-readable format, it is actively maintained,
and MCT can import its `blocks` map. MCT remains the preferred key-file
vocabulary because it is the simplest and most widely exchanged text format.

## Work

### 1. Format detection and registry

- Add a `internal/dumpfmt` layer or extend `internal/workflow/dump_file.go`
  with explicit format identifiers, not extension-only checks.
- Detect raw binary by exact 1024/4096 length and valid trailer geometry;
  detect JSON by `FileType`/`blocks`; detect `.mct`/`.eml` by validated text.
- Reject ambiguous or truncated input with a clear error; never zero-fill
  unknown blocks.
- Keep loaders pure: parse into the existing `DumpResult`/`mifare.Dump`
  types, then validate layout, BCC, trailers, and card identity as today.

### 2. Raw image and internal sidecar

- Preserve current behavior: raw complete image plus `<name>.nfcx.json`.
- When importing a raw image without a sidecar, create the metadata from the
  image and mark it as imported with an empty device and no key provenance.
- When exporting a raw image, write the sidecar atomically; a partial image
  remains JSON-only.
- Add an explicit "export sidecar" option when the user copies only the raw
  file.

### 3. MCT text and JSON

- `.mct` import: parse `+Sector: N`; support 4-block and 16-block sectors;
  reject mixed/truncated sector layouts.
- `.mct` export: emit all sectors in order with `+Sector: N` headers and
  uppercase hex; complete images only.
- `.eml` import/export: one 32-hex block per line, 64/256 lines; complete only.
- MCT JSON import: accept `FileType: mfcard` and any object with a `blocks`
  map keyed by decimal block number; validate all 64/256 blocks.
- MCT JSON export: `Created: "NFC Foundry"`, `FileType: "mfcard"`,
  `blocks` map; complete only.

### 4. Proxmark3 JSON

- Import `mfc v2` and `mfc v3`:
  - `Card.UID`, `Card.ATQA`, `Card.SAK` (and `ATS`/`SIGNATURE` for v3 when
    present);
  - `blocks` map keyed by decimal block;
  - `SectorKeys` for Key A/Key B/access conditions;
  - validate completeness and card geometry before loading.
- Export `mfc v2` for complete Classic 1K/4K dumps:
  - `Created: "proxmark3"` (compatibility) or `"NFC Foundry"`;
  - `FileType: "mfc v2"`;
  - `Card`, `blocks`, `SectorKeys` with access-condition text.
- Do not export partial images to Proxmark3 JSON; offer our project JSON or a
  raw image only when complete.
- Preserve v3-only fields internally when importing if the current dump model
  can hold them; otherwise report that they are dropped rather than silently
  discarding them.

### 5. Key files and key sets

- Keep the existing parser and exporter: one 12-hex key per line, `#` comments,
  whitespace tolerated, deduplicate.
- Treat `.keys` and `.dic` as the same content; the GUI file filter offers both.
- Import a key file into the global catalog and optionally create a named key
  set from the file name (Phase 10).
- Export a key set by values only; names/verifications stay in `keys.json`.
- Add an optional Proxmark3 binary key-array import/export separately from
  `.dic`; label it "raw key array" so it is not confused with a text
  dictionary.

### 6. GUI and CLI

- GUI file dialogs:
  - Classic load/import: `.bin`, `.mfd`, `.json`, `.mct`, `.eml`;
  - Classic save/export: choose raw image, Proxmark3 JSON, MCT JSON, `.mct`,
    or `.eml`;
  - key import/export: `.keys`, `.dic`, `.txt`.
- CLI:
  ```text
  nfctl classic dump FILE.bin|FILE.mfd|FILE.json|FILE.mct|FILE.eml
  nfctl classic restore FILE...
  nfctl classic export --format raw|proxmark3|mct FILE
  nfctl keys import FILE.keys|FILE.dic [--set NAME]
  nfctl keys export [--set NAME] [--format keys|dic] FILE
  ```
  Auto-detect on load; require an explicit `--format` on ambiguous export.
- Document in `docs/cli.md` and the GUI import/export hints which formats are
  complete-only and which preserve partial state.

### 7. Fixtures and interoperability tests

- Add fixtures derived from the MCT example files and Proxmark3 JSON examples
  in the repositories above.
- Round-trip tests:
  - raw → internal → raw byte equality;
  - MCT `.mct` → internal → `.mct` equality;
  - Proxmark3 `mfc v2` → internal → `mfc v2` block equality;
  - `.keys`/`.dic` → catalog → export value equality;
  - partial internal → same project JSON; external export rejected.
- Add malformed fixtures: wrong block count, bad trailer, bad BCC, duplicate
  block key, truncated JSON, non-hex keys, oversized file.

## Acceptance

- A complete Classic dump saved by NFC Foundry opens in MCT and Proxmark3
  without conversion.
- MCT `.mct`/`.eml` and Proxmark3 `mfc v2`/`mfc v3` JSON import into NFC
  Foundry with card identity, blocks, and sector keys preserved where the
  source format provides them.
- Partial dumps remain loadable by NFC Foundry and are never silently exported
  as complete external images.
- `.keys` and `.dic` key files import/export cleanly; comments and duplicates
  are handled predictably.
- Round-trip and malformed-fixture tests pass without hardware.
- GUI and CLI expose the same import/export formats and warnings.
- `go test ./...`, `go vet ./...`, `go tool mygo i18n check .`, UI smoke tests,
  and `reuse lint` pass.

## Non-goals

- No new proprietary binary format for Classic dumps.
- No attempt to make Proxmark3 JSON represent partial certainty.
- No Android/MCT app integration or emulation.
- No DESFire or Type 2 format work beyond the explicitly listed follow-up.
