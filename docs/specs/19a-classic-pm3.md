# Spec: phase 19a Classic completion and PM3 interchange

Status: pending. Plan: [19a-classic-pm3.md](../plan/19a-classic-pm3.md).

## Dump interchange

| Image state | Default export | Other exports | Never |
|---|---|---|---|
| Complete 1K/4K | PM3 raw `.bin` | PM3 JSON `.mfc.json`, MCT `.mct`, project `.nfc-foundry.json` | — |
| Partial | Project `.nfc-foundry.json` | — | raw/PM3 JSON/MCT (unknown bytes cannot be represented) |

- Raw `.bin` is exactly the PM3 block image size (1024 or 4096 bytes).
- PM3 JSON uses `FileType: "mfc v2"`, `Created: "NFC Foundry"`, `Card`
  (`UID`, `ATQA`, `SAK`), decimal-keyed `blocks`, and `SectorKeys`
  (`KeyA`, `KeyB`, `AccessConditions`, `AccessConditionsText`).
- `auto` format means raw for a complete image and project for a partial
  image. Backups always use project.
- No saved name or content uses `nfcx`; legacy `.nfcx.json` files are not
  produced.

## Key interchange

- `.dic` is the PM3 dictionary format: text, one 12-hex key per line, blank
  lines and `#` comments allowed, optional inline comments.
- Export is unique uppercase keys in candidate order.
- Import adds wildcard `any` keys.
- JSON remains the internal store for sector, key type, and provenance.
  Exporting `.dic` is lossy; do not pretend otherwise.
- CLI: `nfctl classic keys export --format dic|json`; GUI: the same actions in
  the Keys screen.

## Write identity

`ClassicBlockWrite` and the restore/NDEF write requests gain an optional
expected `CardInfo`. The service compares it with `operation.card()` after
opening the operation and before any backup or write. A mismatch returns
`NfcError::CardChanged`; no backup file is created. The GUI supplies the
loaded dump's card; CLI writes may leave it unset.

## Classic screens

- Keys: list/add/import/save/export; sector/type/source visible.
- Inspector: hex, access bits and groups, value block, BCC, trailer warnings.
- Diff: before/after images, changed/unknown classification, trailer detail.
- Restore: confirm, backup, stop policy, read-back, block 0 read-only.
- Dump save/load: complete/partial rules above, format detection and warnings.

## Tests

- Golden PM3 `mfc v2` JSON and `.dic` fixtures; raw 1K/4K; partial project.
- Reject raw/PM3 export for partial images.
- Card-swap refusal before backup.
- GTK UI tests on a mock service and Xvfb for gates, dialogs, selection,
  reader-refresh invalidation, and save-format defaults.
- Required checks: `cargo fmt --all --check`, workspace clippy with
  `-D warnings`, workspace tests, `reuse lint`.
