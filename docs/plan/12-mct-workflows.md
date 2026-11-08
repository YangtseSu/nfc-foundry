# Phase 12: MCT-inspired analysis, editing, and clone workflows

## Goal

Add the workflows that make MCT practical for day-to-day card work: compare
dumps, edit value blocks, understand and edit access conditions, clone a card
with a visible before/after diff, and restore a card to factory state. These
are Advanced-mode workflows with the same backup and read-back guarantees as
the existing write path.

## What MCT does

From the MCT feature set and source, the relevant behavior is:

- Keys are a dictionary, not a sector map. Every key is tried against every
  Key A/Key B slot; sectors with no matching key are skipped and clearly shown
  as unreadable.
- Dump editing is block-oriented: hex/ASCII views, highlighted bytes, a BCC
  calculator, value-block decode/encode, and an access-condition table.
- Writing supports block-wise writes, dump-wise clone, and factory format.
  Special tags can have block 0 written; genuine cards cannot.
- A diff tool compares two dumps and highlights changed blocks.
- Import/export/convert handles MCT, Proxmark3, and raw formats.

NFC Foundry already covers the core read/write/dump/restore, recovery, UID
write, key scan, and format interop (Phase 11). The remaining gap is the
analysis and editing layer, plus a clone workflow that proves the result.

## Work

### 1. Dump diff

- Add a diff engine in Go that compares two dumps block-wise and reports:
  - changed blocks with sector/block, old/new hex, ASCII, and known status;
  - differences in trailer keys and access conditions;
  - differences in card identity (UID/ATQA/SAK) and completeness;
  - blocks that are unknown on either side, distinct from equal zeros.
- CLI: `nfctl classic diff A B [--json]`.
- GUI: a Diff tab or dialog with side-by-side rows, change count, and filters
  for "changed only", "unknown", "trailers".
- Never treat an unknown block as equal to a zero block.

### 2. Value block editor

- Add a value-block codec matching the MIFARE Classic value format:
  value, address, and the inverted copies. Validate all copies before
  presenting a value.
- Show decoded value/address in the block inspector and a decoded column in
  the editor.
- Let the user edit a value block as a number plus address, then re-encode;
  preview the exact 16 bytes before writing.
- Writes to value blocks use the ordinary data-block write path with the
  existing backup, confirmation, and read-back rules. No new bypass.

### 3. Access-condition editor

- Add a table editor for the four access-condition groups per sector trailer:
  read/write/increment/decrement/key-B-read bits.
- Decode the trailer bytes, validate complement bits, and show the effective
  permissions in plain language.
- A trailer edit is an Advanced action: show old/new trailer, key A/B impact,
  a full-image backup path, and require typed confirmation.
- Refuse to write a trailer whose access bits do not decode or whose
  complement bytes are inconsistent.
- Add a BCC calculator next to the block 0 editor; block 0 writes remain the
  existing guarded UID path.

### 4. Clone workflow

- Add a "Clone card" workflow:
  1. load a complete source dump;
  2. detect the target card and compare UID/ATQA/SAK;
  3. preflight capacity, BCC, access bits, keys, and block-0 policy;
  4. write a full backup of the target;
  5. write data blocks, then trailers, then block 0 only through the existing
     supported magic-card path;
  6. read the target back, run the diff engine, and report differences;
  7. keep the source and target backup paths in the result.
- The workflow must abort on the first unrecoverable write failure and on a
  mid-operation card identity change.
- A genuine card with a read-only block 0 must fail honestly and must not
  report a successful clone.

### 5. Factory format

- Add a Classic factory-format action behind Advanced mode:
  - write default keys (or a user-confirmed key set) and default access bits;
  - optionally clear user data blocks;
  - do not write block 0 unless the card is a validated magic variant and the
    user explicitly confirms it;
  - back up the full card first and verify every write.
- Show exactly which blocks and trailer bytes will change before confirmation.
- Refuse if the card is not Classic 1K/4K or the dump is incomplete.

### 6. Key-file editor parity

- Add a GUI key-file editor with line numbers, one key per line, `#` comments,
  inline validation, duplicate detection, and save/import/export.
- Keep the global catalog and named key sets (Phase 10) as the working model;
  the editor is for exchange files, not a second source of truth.
- Add CLI `keys import`/`keys export` format hints and a `--set` option.

### 7. Partial-read UX

- Make the read result explicit: "64/64 blocks readable" versus
  "48/64 blocks readable; 4 sectors skipped".
- Show unreadable sectors in the block table and a summary in the workbench
  header. Do not call a partial read simply "complete".
- Provide a one-click "scan keys, then read again" action when a read leaves
  sectors unreadable; it must remain opt-in for automatic scans.

## Safety

- All trailer, block 0, factory-format, and clone writes are Advanced actions.
- Every destructive action shows target blocks, old/new values, backup path,
  and read-back plan; trailer and UID writes require typed confirmation.
- The diff engine is read-only; it never writes a card.
- Value-block edits do not bypass access-condition or authentication checks.
- Clone/factory operations stop on the first unrecoverable failure and abort
  if the card identity changes.

## Acceptance

- Dump diff distinguishes changed, equal, and unknown blocks and reports
  trailer/key differences.
- Value-block decode/encode round-trips the MIFARE value format and rejects
  inconsistent copies.
- Access-condition edits are decoded, validated, backed up, and verified.
- Clone and factory-format workflows have preflight, backup, read-back diff,
  and explicit failure reporting.
- A genuine card cannot be reported as a successful UID clone.
- Key-file edits round-trip through `.keys`/`.dic` without changing values.
- GUI and CLI expose equivalent operations with the same safety rules.
- Tests cover pure codecs and workflows; hardware tests are gated by
  environment variables and never write a card by default.
- `go test ./...`, `go vet ./...`, `go tool mygo i18n check .`, UI smoke tests,
  and `reuse lint` pass.

## Non-goals

- No key-cracking implementation in this phase; recovery remains the external
  engine path.
- No generic raw-write command outside the validated block/trailer/clone
  workflows.
- No emulation UI.
- No change to the rule that ordinary cards have a read-only manufacturer
  block.
