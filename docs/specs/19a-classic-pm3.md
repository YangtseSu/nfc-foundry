# Spec: phase 19a Classic completion and PM3 interchange

Status: implemented 2026-11-11. Plan:
[19a-classic-pm3.md](../plan/19a-classic-pm3.md). Note:
[19a-classic-pm3.md](../notes/19a-classic-pm3.md).

## Dump interchange

| Image state | Default export | Other exports | Never |
|---|---|---|---|
| Complete 1K/4K | PM3 raw `.bin` | PM3 JSON `.mfc.json`, MCT `.mct`, project `.nfc-foundry.json` | — |
| Partial | Project `.nfc-foundry.json` | — | raw/PM3 JSON/MCT (unknown bytes cannot be represented) |

- Raw `.bin` is exactly the PM3 block image size (1024 or 4096 bytes).
- PM3 JSON uses `FileType: "mfc v2"`, `Created: "NFC Foundry"`, `Card`
  (`UID`, `ATQA`, `SAK`), decimal-keyed `blocks`, and `SectorKeys`
  (`KeyA`, `KeyB`, `AccessConditions`, `AccessConditionsText`).
- `auto` means raw for a complete image and project for a partial image. On
  load it detects the format from content. `DumpFormat::can_represent` drives
  the GUI's disabled choices. Backups always use project.
- Default names: `card-dump.bin`, `card-dump.mfc.json`, `card-dump.mct`, and
  `card-dump.nfc-foundry.json`.
- No saved name or content uses `nfcx`; legacy `.nfcx.json` files are not
  produced.

## Key interchange

- `.dic` is the PM3 dictionary format: text, one 12-hex key per line, blank
  lines and `#` comments allowed, optional inline comments.
- Export is unique uppercase keys in candidate order. Import accepts lowercase
  and deduplicates.
- Import adds wildcard `any` keys with `KeyFile` provenance.
- JSON remains the internal store for sector, key type, and provenance.
  Exporting `.dic` is lossy; the GUI and CLI say so.
- CLI: `nfctl classic keys export --format dic|json [--output PATH]`; without
  `--output` the export goes to standard output. GUI: the same actions in the
  Keys screen.
- `nfctl classic dump --format` defaults to `auto`.

## Write identity

`ClassicBlockWrite`, `ClassicRestore`, `ClassicNdefWrite`, and
`ClassicNdefFormat` carry an optional expected `CardInfo`. The service
compares it with `operation.card()` after opening the operation and before any
backup or write. A mismatch returns `NfcError::CardChanged`; no backup file is
created. Restore also rejects a source dump with a different identity. The GUI
supplies the loaded dump's card for block writes and restores; CLI writes may
leave it unset.

## Classic screens

- Keys: list/add/import/save/export; sector/type/source visible.
- Inspector: hex, access bits and groups, value block, BCC, trailer warnings.
- Diff: before/after images, changed/unknown classification, trailer detail.
- Restore: confirm, backup, stop policy, read-back, block 0 read-only, and a
  per-block written/unchanged/unknown/failed result table.
- Dump save/load: complete/partial rules above, format detection and warnings.

## Tests

- Golden PM3 `mfc v2` fixture import and semantic re-export; raw 1K/4K round
  trips; partial project round trip; rejection of raw/PM3/MCT export for
  partial images; `Created: "NFC Foundry"` and default names.
- `.dic` comments, inline comments, blank lines, duplicates, lowercase, and
  malformed lines; CLI dic/json export round trip.
- Card-swap refusal before backup for single-block write and restore.
- GTK scenarios under Xvfb cover selection/inspector updates, block 0,
  unchanged, no-live-card and trailer gates, reader-refresh invalidation,
  write confirm/cancel, identity refusal, restore confirm/cancel, diff
  classification, and keys add/save. The test entry point is
  `NFC_FOUNDRY_GUI_TESTS=1`; it is skipped without that gate.
- Required checks: `cargo fmt --all --check`, workspace clippy with
  `-D warnings`, workspace tests, and `reuse lint`.
