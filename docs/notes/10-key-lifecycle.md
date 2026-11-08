# Phase 10 implementation note: key lifecycle, recovery save, and verification

Date: 2026-11-09

## Implemented

- `keys.json` version 2 keeps key names and named key sets while loading
  version 1 files unchanged. `addedAt`, names, and set membership round trip;
  missing set members are pruned with a report; empty sets are dropped; a name
  on a built-in key is persisted.
- `keys.Store` gained name edits, key-set create/rename/delete/membership
  queries, and an atomic `Reload` that replaces the in-memory state from disk.
  Deleting a key removes it from sets and drops sets left empty.
- Card-scoped verification is now self-correcting:
  - `classic dump --scan`, `classic restore --scan`, and the GUI
    scan-before-read option run a keyscan before reading; the scan is off by
    default.
  - `classic read`/`classic write` clear the verification of a verified key
    that receives `ErrAuthenticationFailed`.
  - A full read clears verifications only for sectors it actually attempted,
    so an interrupted operation leaves untested claims alone.
  - Changed verifications are persisted immediately.
- Recovery reports `RecoveredKeyRef` entries (`keyId`, sector, key type, and
  `new`/`replaced`/`existing`) in task events and results. Key values stay out
  of the reports; the GUI joins IDs against the catalog.
- Recovery requests take a save policy: `persist` (CLI default), `review`
  (GUI), or `none` (CLI `--no-save`). A pending review blocks reads, writes,
  scans, recovery, key mutations, and a manual store reload so unsaved keys
  cannot be persisted by another action. Card removal discards it.
- The GUI recovery panel now opens a review modal with selectable slots,
  catalog values, status pills, a key-set name, and per-key names. Save writes
  names, the set, and verifications, reloads the store from disk, and
  authenticates every selected key against the current card. Rejected slots
  are unverified and the corrected store is written. Discard removes only keys
  the task introduced.
- `ReloadKeyStore` and the Keys tab **Reload key store** action rebuild the
  store and emit key, sector-key, and key-set snapshots.
- The Keys tab gained the **Scan keys before reading** checkbox, key rename,
  and a key-set panel with save-verified, rename, delete, and export actions.
- CLI additions: `--scan` on dump/restore, `--name`/`--key-set`/`--no-save`
  on recovery, and `keys rename`, `keys sets`, `keys set
  create|rename|delete|export`, and `keys reload`.

## Validation

- `go test ./...`
- `go vet ./...`
- `go tool mygo i18n check .` (English and Simplified Chinese complete)
- `reuse lint`
- Focused desktop UI tests for scan-before-read, the recovery review modal,
  discard, and the key-set panel.
- Store fixtures cover version 1 migration, version 2 round trips, pruning,
  duplicate rejection, built-in names, deletion cleanup, and reload.

## Remaining gaps

- The save/reload/verify path and the review UI are validated with mock
  readers and temporary stores. No hardware run has exercised the review flow
  on a physical card yet; the hardware-free behavior is the committed test
  surface.
- Verification remains keyed by `UID/ATQA/SAK`. Two physical cards with the
  same selection identity cannot be distinguished. A failed authentication
  removes the stale claim; it does not identify which physical card is
  present.
