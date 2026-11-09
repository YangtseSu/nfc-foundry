# Phase 12 spec: MCT-inspired analysis, editing, and clone workflows

Date: 2026-11-09

## Goal

Close the remaining MIFARE Classic workbench gap against MIFARE Classic Tool
(MCT): compare dumps, decode and edit value blocks, edit sector-trailer access
conditions, clone a full image, reset a card to factory transport
configuration, exchange key files through a line-oriented editor, and report
partial reads honestly. Every operation keeps the existing device lease,
backup, authentication, confirmation, read-back, and card-identity rules.

## Affected packages and interfaces

- `internal/mifare`: value-block codec, BCC/access helpers used by the new
  editors.
- `internal/workflow`: dump diff engine, clone service, factory-format image
  builder, and shared manufacturer-block write helper.
- `internal/keys`: line-oriented dictionary analysis for the key-file editor.
- `app`: DTOs and services for diff, value/trailer previews, clone, factory
  format, key-file editing, and partial-read metrics.
- `internal/cli`: `classic diff`, `classic value set`, `classic trailer set`,
  `classic clone`, `classic format`, and partial-read output.
- `internal/desktop`: Diff tab, value/trailer editors, clone/format dialogs,
  key-file editor, and partial-read summary with a scan-and-read-again action.
- No C shim, ABI, packaging, or transport change. No new dependency.

## Dump diff

`workflow.DiffDumps(left, right DumpResult)` compares two images.

- Both sides must use the same Classic layout. A mismatch is an error.
- Each block is `equal` only when every byte is known on both sides and all
  bytes match. A block with any unknown byte is `unknown`, even when the known
  bytes are zero. A block with all bytes known and at least one difference is
  `changed`.
- A byte-level list records offset, old/new byte, and old/new certainty.
- Trailer blocks additionally report Key A, Key B, and access-condition
  changes. A trailer that fails to decode is reported as invalid, never
  repaired.
- Identity reports UID, ATQA, SAK, layout, and completeness for both sides and
  a single `identityChanged` flag.
- The engine is read-only. It never authenticates or writes.

`nfctl classic diff A B [--json]` loads both files through the Phase 11
detection path and prints the summary and changed/unknown rows.

The GUI adds a Classic **Diff** tab. It holds one comparison image loaded
through a file dialog, compares it with the current workbench on demand, and
shows summary counts, identity changes, and a filterable block list.

## Value block codec

`mifare.DecodeValueBlock` accepts only a structurally consistent MIFARE value
block:

```text
value(4 LE) | ^value(4) | value(4) | addr | ^addr | addr | ^addr
```

Every copy is checked. `EncodeValueBlock(int32, byte)` produces the canonical
16 bytes. A decoded value block is shown in the block inspector; the editor
takes a signed decimal value plus an address byte, previews the exact 16 bytes,
and writes through the ordinary data-block path. Value-block edits do not
bypass authentication or access conditions.

## Access-condition editor

- Four groups are edited as C1/C2/C3 checkboxes. `mifare.EncodeAccessBits`
  produces bytes 6..8 and the redundant complement bytes are validated before
  the preview is accepted.
- The full trailer is assembled from Key A, the encoded access bytes, the GPB
  byte, and Key B. The default GPB is the current trailer byte 9.
- A trailer edit is an Advanced action. The preview shows the old and new
  trailer, both keys, the decoded permissions, the full-image backup path, and
  requires typed confirmation (the trailer block number).
- Writing reuses the restore plan with a single trailer block. Access bits
  that do not decode or whose complements are inconsistent are refused.
- Block 0 remains the existing guarded UID path. The manufacturer-block
  inspector adds a BCC calculator: computed BCC, stored BCC, and validity for
  a four-byte UID.

## Clone workflow

`workflow.CloneService` executes one exclusive reader lease:

1. Preflight requires a complete source image, matching layout, valid
   trailers, valid BCC when the source has a four-byte UID, and target keys
   that permit every planned data and trailer write.
2. The application writes a complete target backup before mutation.
3. Data blocks are written, then trailers, using the restore engine order.
4. Block 0 is written only when the source UID differs from the target UID.
   The full source manufacturer block is written through the existing
   DirectWrite capability with the Gen1A fallback on the same reader. A card
   whose block 0 stays read-only fails and never reports success.
5. The target is read back with the source keys, the diff engine compares it
   with the source image, and the result is verified only when every block is
   `equal`.
6. The source path and target backup path are kept in the result.

Clone stops on the first unrecoverable failure and aborts when UID, ATQA, or
SAK changes mid-operation. The GUI requires the source UID as typed
confirmation when block 0 will be written.

## Factory format

`workflow.BuildFactoryImage` derives the target image from a complete current
backup:

- block 0 stays byte-identical;
- every data block is zeroed unless `keepData` is set;
- every sector trailer becomes Key A `FFFFFFFFFFFF`, access bytes `FF 07 80`,
  GPB `69`, Key B `FFFFFFFFFFFF`.

Preflight reads the complete current card, requires MIFARE Classic 1K/4K, and
builds a single-trailer-per-sector restore plan with the current keys. The
execution task saves the fresh backup, rebuilds the factory image from that
backup, runs the restore, reads the card back with the default keys, and
verifies the diff is equal. Block 0 is never written. The GUI requires the
typed confirmation `FORMAT`; the CLI requires `--yes` and `--confirm FORMAT`.

## Key-file editor

`keys.AnalyzeDictionary` returns one result per line: comment, blank, valid
key, duplicate, or a concrete problem. It follows the existing dictionary
rules: `#` starts a comment, whitespace inside a key is ignored, and a key is
12 hex digits.

The GUI editor shows line numbers, one line per row, inline problem text,
duplicate marking, and summary counts. Open, Save As, import into the catalog,
and import into a named key set use the existing catalog and key-set model.
The editor is an exchange surface, not a second source of truth. CLI
`keys import`/`keys export` keep their format and `--set` options.

## Partial-read UX

`WorkbenchDTO` reports readable blocks, total blocks, readable sectors, total
sectors, and the skipped sector list. A block is readable when all 16 bytes
are known; a sector is readable when every block in it is readable.

The Classic header and block table show `readable / total` and the skipped
sectors. A partial read is never labelled complete. A one-click **Scan keys
and read again** action starts the existing scan-and-read task; it is opt-in
and asks before discarding dirty edits.

## Safety

- Diff is read-only.
- Value edits use the ordinary single-data-block write path.
- Trailer, clone, and factory-format writes are Advanced-only in the GUI and
  require explicit confirmation in the CLI.
- Every destructive action shows target blocks, old/new values, backup path,
  and read-back plan.
- A genuine card never reports a successful UID clone.
- Clone and factory operations stop at the first unrecoverable failure and
  abort on a card identity change.

## Hardware gates

No new hardware test is required for the pure codecs, diff, key-file analysis,
or UI. Clone and factory format are exercised with mock readers; an optional
hardware test reuses the existing `NFCX_WORKFLOW_HARDWARE` dump/restore fixture
and is never enabled by default.

## Packaging and license impact

No packaging change. All new code and documentation is GPL-3.0-or-later under
the repository default REUSE annotation.
