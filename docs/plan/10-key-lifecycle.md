# Phase 10: Key lifecycle, recovery save, and optional verification

## Goal

Make card-scoped key verification self-correcting and make recovered keys an
explicit, reviewed, named artifact instead of an invisible side effect. A
saved artifact must be reloadable from disk and verified against the current
card before the UI reports success.

## Starting point

- `keyscan` already removes a card-scoped verification when no candidate
  authenticates for a slot (`ff2dee5`). Automatic recovery now sees an
  uncovered sector instead of a stale `verified` claim.
- `keys.json` is version 1 and stores a global key catalog plus card-scoped
  verifications. It has no key names, key sets, or explicit save/reload step.
- `StartKeyRecovery` merges and persists verified keys while it runs. The GUI
  has no review or naming step.
- Reading a card uses the verified keys held in memory; it does not re-read
  `keys.json`.
- Two physical cards with the same `UID/ATQA/SAK` cannot currently be
  distinguished.

## Decisions

- Keep the internal key store authoritative. External key files are exchange
  artifacts, not the source of truth.
- Add key names and named key sets to the internal store. The store remains
  backward compatible with version 1.
- GUI recovery performs an explicit review/save step. CLI recovery keeps its
  current auto-persist behavior by default, with `--name`, `--key-set`, and
  `--no-save` controls.
- A save is only complete after the store is reloaded from disk and the saved
  keys are authenticated against the current card.

## Work

### 1. Verification lifecycle

- Add a store operation that removes a card-scoped verification on an
  authentication failure that is definitive (`ErrAuthenticationFailed`).
- Use it when a read finds that a supplied verified key no longer
  authenticates, matching the keyscan behavior.
- Add a GUI "Scan keys before reading" option, default off. When enabled, run
  `StartKeyScan` before `StartReadCard`, and stop if the scan fails.
- Add a CLI equivalent to `classic dump`/`classic restore` such as `--scan`,
  with the same default-off behavior. `--keys FILE` keeps its existing
  import-and-scan semantics.
- Document the unavoidable clone collision: verification is keyed by
  `UID/ATQA/SAK`; a clone with the same selection identity cannot be
  distinguished. The failure path must clear the stale claim rather than
  hide the ambiguity.

### 2. Named keys and key sets

Extend `keys.json` to version 2 with a backward-compatible loader:

```json
{
  "version": 2,
  "keys": [
    {
      "value": "FBBECDF6304E",
      "sources": ["attack_engine", "read_context"],
      "name": "Sector 5 Key A",
      "addedAt": "2026-11-08T14:58:01Z"
    }
  ],
  "verifications": [
    {
      "cardId": "FAB9CFF4/0004/08",
      "sector": 5,
      "keyType": "A",
      "keyId": "key-...",
      "verifiedAt": "2026-11-08T14:58:01Z"
    }
  ],
  "keySets": [
    {
      "id": "set-...",
      "name": "Cloned access card",
      "cardId": "FAB9CFF4/0004/08",
      "keyIds": ["key-...", "key-..."],
      "createdAt": "2026-11-08T15:10:00Z",
      "source": "recovery"
    }
  ]
}
```

- Key records keep derived IDs (`key-<hash>`) so existing verifications remain
  valid.
- Key names are optional, short, and editable; an empty name keeps the current
  value-only display.
- Key sets reference key IDs, not key values. Prune missing IDs on load and
  report the change instead of failing the store.
- Version 1 files load without migration errors; the next save writes version
  2. Unknown fields are ignored; malformed names/sets are validation errors.
- `keys.Store` gains `RenameKey`, `CreateKeySet`, `RenameKeySet`,
  `DeleteKeySet`, `KeySets`, and membership queries.
- Key deletion removes the key from sets or leaves a reported dangling entry
  that is pruned on the next save; choose one behavior and test it.

### 3. Recovery review and save

- Add `RecoveredKeyRef` entries to the recovery task event/report:
  `{keyId, sector, keyType, status}`. Do not include key values in the report;
  the GUI joins by ID against the catalog.
- Track which keys were added or replaced by the current recovery task, and
  keep their IDs in the task result.
- In the GUI, show a review after recovery:
  - selectable rows for each recovered key (sector, Key A/B, value from the
    catalog, new/replaced/existing status);
  - name field for the key set; default `Recovered <UID> <date>`;
  - optional per-key name field;
  - Save and Discard actions.
- The review is modal or a dedicated panel. Saving writes the key names, key
  set, and verifications together, then reloads and verifies. Discarding
  removes only keys that this task introduced and leaves pre-existing keys
  untouched.
- For the service, separate "merge into memory" from "persist". CLI recovery
  persists by default; GUI recovery defers persistence until Save. Add a
  request field such as `Persist bool` or a save-policy enum. The read
  verification step inside recovery still uses the in-memory keys.
- Extend CLI:
  `nfctl recover nested --yes --name "Sector 5" --key-set "Cloned card"`.
  `--no-save` keeps keys in memory for the invocation and prints them only
  with `--show-keys`; it must state that nothing was persisted.

### 4. Reload and verify after save

- Add `ReloadKeyStore()` to the application service and `desktop.Backend`.
  It reloads the file, rebuilds the store, clears stale in-memory state, and
  emits the key and catalog events.
- After Save, the GUI calls `ReloadKeyStore`, then performs a verification
  read of the current card (or authenticates every saved key for the card).
  The result is shown explicitly:
  - "Saved and verified: 32/32 key slots, 64/64 blocks readable";
  - "Saved, but verification failed: sector 5 Key A rejected".
- Only after successful reload should the GUI claim the keys are available to
  future reads.
- Add a GUI command to reload the store manually, useful after editing the file
  or running `nfctl` externally.

### 5. Optional key reading

- "Scan keys before reading" is opt-in in the GUI and CLI; it is not silently
  enabled for every read.
- Key verification is a separate action in the service layer, so a future
  read-only workflow can use the data without mutating verification state.
- The Keys tab continues to show per-sector verified/unknown status and the
  global catalog; named key sets are displayed separately from individual key
  records.

## CLI surface

```text
nfctl classic dump FILE [--scan]
nfctl classic restore FILE [--scan]
nfctl recover nested|darkside|hardnested --yes [--name NAME] [--key-set NAME] [--no-save]
nfctl keys list|add|import|export
nfctl keys rename <key-id> <name>
nfctl keys sets
nfctl keys set create <name> --key <id>...
nfctl keys set export <name> [--format keys|dic]
nfctl keys reload
```

Exact command names may change; preserve the existing commands and add
subcommands rather than overloading ambiguous flags.

## Acceptance

- A key that fails authentication during a read is no longer reported as
  verified for that card.
- Recovery results expose only key references, not key values, in event
  payloads.
- Recovered keys can be reviewed, selected, named, and saved; discarding does
  not delete pre-existing keys.
- Saving writes version 2 JSON, reloads it from disk, and verifies the selected
  keys against the current card before reporting success.
- Version 1 key stores still load, and the migration is tested with a real v1
  fixture.
- Optional scan-before-read works in GUI and CLI and is off by default.
- Clone collision behavior is documented and tested with a same-identity
  fixture where the stored key no longer authenticates.
- `go test ./...`, UI smoke tests, `go vet ./...`, `go tool mygo i18n check .`,
  and `reuse lint` pass.

## Non-goals

- No claim that two cards with identical `UID/ATQA/SAK` can be distinguished.
- No key storage in dump files by default; key sets remain separate.
- No automatic destructive re-keying or factory reset in this phase.
- No new encryption or secret storage for `keys.json`; it remains a
  user-owned, file-permission-protected local file.

## Completion

Implemented on 2026-11-09. See `docs/notes/10-key-lifecycle.md`. Store
migration, recovery review, save/reload verification, scan-before-read, and
the key-set CLI/GUI surfaces are covered by hardware-free tests. The
review-save flow has not yet been exercised on a physical card.
