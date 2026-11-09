# Phase 12 note: MCT-inspired analysis, editing, and clone workflows

Date: 2026-11-09

## Implemented

- `internal/workflow/diff.go` adds a read-only block diff engine. It classifies
  blocks as `equal`, `changed`, or `unknown`, treats any unknown byte as an
  unknown block rather than a zero, records byte certainty and differences, and
  reports trailer Key A/Key B/access/GPB changes plus UID/ATQA/SAK identity and
  completeness. `DumpReadability` is shared by the CLI and GUI.
- `internal/mifare/value.go` adds strict value-block decoding and canonical
  encoding. Every repeated and inverted copy is validated.
- `internal/mifare/access.go` adds read, increment, decrement, and
  trailer-specific permission masks. The GUI access editor uses those masks to
  describe effective permissions in plain language.
- `internal/workflow/clone.go` adds the clone service and factory image builder.
  Clone runs preflight, data writes, trailer writes, an optional manufacturer
  block write, a full read-back, and a diff under one reader lease. The
  manufacturer write reuses the DirectWrite/Gen1A helper extracted from the UID
  service. Factory images preserve block 0, reset trailers to transport
  configuration, and optionally clear data blocks.
- `internal/workflow/restore.go` exposes `executeRestorePlan`, shared by
  restore, clone, and factory format.
- `internal/workflow/uid.go` exposes `writePlanWithReader` and
  `buildBlock0WritePlan` so clone and UID write share the same committed/
  uncertain/reselect rules.
- `app` adds value/trailer previews, dump comparison state, clone and factory
  preflights and tasks, key-file analysis/editor services, and readable-block
  metrics on the workbench DTO. Trailer writes use a single-trailer restore
  plan and require the trailer block number as typed confirmation.
- `internal/cli` adds `classic diff`, `classic value set`,
  `classic trailer set`, `classic clone`, and `classic format`. Dump output now
  reports readable blocks and skipped sectors.
- `internal/desktop` adds a Classic **Diff** tab, value-block editor, access
  condition table and trailer preview, clone and factory dialogs, a line-based
  key-file editor, manufacturer-block BCC calculator, and a partial-read banner
  with an opt-in scan-and-read-again action.
- `internal/keys` adds `AnalyzeDictionary` for per-line editor state while
  keeping `ParseDictionary` as the import path.

## Tested

- Value-block round trips for positive, negative, and zero values; rejection of
  every inconsistent copy and of ordinary data.
- Access permission tables for all eight data and trailer codes.
- Diff classification: changed/equal/unknown counts, unknown zero versus known
  zero, trailer key/access diffs, identity changes, layout mismatch, and
  readability counts.
- Clone service against a mock reader: preflight, data/trailer/block-0 ordering,
  verified read-back diff, read-only block-0 failure, partial source rejection,
  and layout mismatch.
- Factory image preservation of block 0, trailer reset, optional data clearing,
  and key derivation from known trailer bytes.
- App service: value and trailer previews, typed trailer confirmation,
  trailer write through the restore path, dump comparison, key-file preview and
  import, factory format end to end, and clone end to end with a mock reader.
- CLI: diff JSON, value write, trailer typed confirmation, clone restore, and
  factory format with the fake Classic reader.
- Desktop UI: Diff tab and filters, value preview/write, trailer typed
  confirmation, clone confirmation, factory confirmation, key-file editor, and
  partial-read banner.
- `go test ./...` passes.

## Problems and resolutions

- The first diff draft compared raw bytes even when one side was masked. Trailer
  key fields now compare only when both sides know the bytes; otherwise the
  block is `unknown`.
- Clone initially rebuilt the block-0 plan before the restore, which could use
  keys that the restored trailer replaced. The plan is now built after the
  restore from the source image's trailer keys.
- The GUI advanced action row overflowed into the side panel once clone and
  factory actions were added; destructive actions now use a second row.
- The skipped-sector summary initially reused a helper that printed `[3, 7]`
  as `3-7`. A dedicated formatter now compresses only consecutive runs.

## Deviations

- No new hardware test was added. Clone and factory reuse the existing
  dump/restore and Gen1A hardware paths; an optional hardware clone or format
  test would write a target card and needs its own disposable-card gate. The
  code paths are covered by mock-reader tests, and the existing dump/restore
  and Gen1A tests remain the hardware gates.
- The GUI key-file editor is a line-based table, not a free-form code editor, so
  every line has a visible number, an inline state, and a stable row identity.

## Remaining blockers

Clone block-0 behavior on real CUID/Gen2, UFUID, FUID, and Gen1A cards, and
factory format on real cards, still need explicit hardware validation. The
implementation is complete and refuses unverified success, but it is not
advertised as hardware-validated.
