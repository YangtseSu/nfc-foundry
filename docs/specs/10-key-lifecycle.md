# Phase 10 specification: key lifecycle, recovery save, and verification

Status: implementation specification.

## Scope

This change makes card-scoped verification self-correcting and turns recovered
keys into a reviewed, named artifact. It covers:

- removal of stale card-scoped verifications when authentication proves them
  wrong;
- version 2 of the internal key store with key names and named key sets;
- a GUI recovery review with Save and Discard;
- an explicit reload-then-verify step after Save;
- optional scan-before-read in the GUI and `classic dump`/`classic restore`;
- CLI commands for names, key sets, and reload.

The internal key store remains authoritative. Dictionary files, key-set export
files, and dumps are exchange artifacts; none of them replaces the store.

## Key store version 2

`keys.json` keeps version 1 compatibility and writes version 2:

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

- Key IDs remain derived from the key value (`key-<hash>`), so version 1
  verifications stay valid.
- `addedAt` is persisted; version 1 files that lack it load with the current
  time.
- Key names are optional, trimmed, and at most 64 runes. Control characters
  are rejected. A name on a built-in key is persisted even though the built-in
  key record itself is not.
- Key-set names are required, trimmed, and at most 80 runes. Control
  characters are rejected. Empty key sets are not persisted.
- Key sets reference key IDs. Missing IDs are pruned on load and reported;
  a set left empty is dropped. Duplicate set IDs or duplicate
  `(name, cardId)` pairs are validation errors.
- Unknown JSON fields are ignored. Version 1 loads normally and the next save
  writes version 2.
- A set ID is assigned at creation and stays stable across renames. The
  `(name, cardId)` pair is unique, so re-creating or re-saving a set with the
  same name updates that record.
- Deleting a key removes it from every set; a set left empty is deleted. The
  key's card-scoped verifications are deleted with it.
- `Store.Reload` replaces the in-memory state atomically from the file, so
  services that already hold the store pointer observe the reloaded data.

Store operations:

```go
RenameKey(id, name string) (Record, error)
KeySets() []KeySet
KeySet(id string) (KeySet, bool)
KeySetByName(name, cardID string) (KeySet, bool)
SetsContaining(keyID string) []KeySet
CreateKeySet(name, cardID string, source KeySetSource) (KeySet, error)
SetKeySetKeys(id string, keyIDs []string) (KeySet, error)
AddKeySetKeys(id string, keyIDs []string) (KeySet, error)
RenameKeySet(id, name string) (KeySet, error)
DeleteKeySet(id string) error
Reload(path string) (LoadReport, error)
```

`LoadReport` reports the source version, pruned key IDs, and dropped set IDs.
`Load` remains the discard-report wrapper for existing callers.

## Verification lifecycle

- A supplied card-scoped verification is removed when a read or block
  operation receives `ErrAuthenticationFailed` for that slot and the sector
  was actually attempted. Unvisited sectors keep their verification.
- Keyscan behavior is unchanged: a slot whose candidates all fail is
  unverified.
- A changed verification is persisted immediately, so a restart cannot
  resurrect a stale claim.
- Reads, writes, and keyscan emit the current sector-key and key-catalog
  snapshots after clearing a verification.
- Verification remains keyed by `UID/ATQA/SAK`. Two physical cards with the
  same selection identity cannot be distinguished; the failure path clears
  the stale claim rather than hiding the ambiguity.

`StartReadCard` takes a request with `ScanKeys bool`. When true, the service
runs the key scan first inside the same device-holding task, emits scan
progress, and stops before reading if the scan fails. The default is false.
The GUI exposes the same option as **Scan keys before reading**, also default
off. `nfctl classic dump --scan` and `nfctl classic restore --scan` use the
same service path. `--scan` together with `--keys` is rejected because
`--keys` already imports and scans.

## Recovery references

Recovery reports only references:

```go
type RecoveredKeyRef struct {
    KeyID   string
    Sector  int
    KeyType nfc.KeyType
    Status  RecoveredKeyStatus // new | replaced | existing
}
```

- `new`: the key was not in the store when the recovery task started.
- `replaced`: the slot had a verification for a different key before the task.
- `existing`: otherwise.

Events and task results carry refs, never key values. The GUI joins refs by ID
against the key catalog. `KeyRecoveryResult`, `MFoC`/`MFCUK` recovery results,
and `TaskEventDetailDTO` expose the refs.

## Save policy

Recovery requests carry a save policy:

| Policy | Use | Behavior |
| --- | --- | --- |
| `persist` (default) | CLI | Merge, apply optional names and key set, write the store |
| `review` | GUI | Merge into memory only; publish a pending review |
| `none` | CLI `--no-save` | Merge into memory only; never write |

A pending review blocks reads, writes, scans, another recovery start, key
mutations, and store reload until the user saves or discards it, so no other
action can persist the unsaved keys. It is also cleared when the card is
removed.

`SaveRecoveryReview`:

1. validates that the card from the review is still present and identical;
2. applies the selected per-key names;
3. removes newly introduced (`new`) keys that the user did not select;
4. creates or updates the named key set with the selected IDs;
5. writes version 2 to disk;
6. reloads the store from that file;
7. authenticates every selected key against the current card;
8. unverifies rejected slots and writes the corrected store;
9. emits key, catalog, and key-set events plus a save result.

The save result reports the set, saved key count, verified/total slots, and
rejected slots. A persist or reload failure leaves the review pending. A
verification failure is reported as "saved, but verification failed", not as a
successful verification.

`DiscardRecoveryReview` deletes only keys that the task introduced (`new`),
leaves pre-existing records and verifications of untouched slots alone, and
does not write the store.

`ReloadKeyStore` reloads the file, clears the pending review, and emits the
current key, sector-key, and key-set snapshots. It reports pruned entries.

## CLI surface

```text
nfctl classic dump FILE [--scan]
nfctl classic restore FILE [--scan]
nfctl recover nested|darkside|hardnested --yes [--name NAME] [--key-set NAME] [--no-save]
nfctl keys list|add|import|export
nfctl keys rename <key-id> <name>
nfctl keys sets
nfctl keys set create <name> --key <id>... [--card CARD]
nfctl keys set rename <name> <new-name>
nfctl keys set delete <name>
nfctl keys set export <name> [--format dic|keys] [--output FILE]
nfctl keys reload
```

- `--name` names the recovered key only when exactly one key reference is
  recovered; otherwise it is ignored with a warning. `--key-set` creates or
  adds to the named set for the current card.
- `--no-save` rejects `--name` and `--key-set`, prints the recovered key IDs
  only with `--show-keys`, and states that nothing was persisted.
- `keys set create` accepts key IDs from `keys list`. Without `--card` the set
  is global; recovery-created sets remain card-scoped.
- `keys set export --format dic` writes one 12-digit key per line.
  `--format keys` writes a JSON key-set artifact with values and names.
  Without `--output`, the artifact goes to stdout; JSON mode includes it.
- Machine output uses the existing result envelope and stays English-only.

## GUI surface

- Keys tab: **Scan keys before reading** checkbox; **Reload key store** action;
  rename-key dialog; key-set list with create-from-verified-keys, rename,
  delete, and export actions.
- Recovery panel passes the `review` policy and opens a review modal on the
  review event. The modal lists selectable refs joined to catalog values,
  sector, Key A/B, new/replaced/existing status, a set-name field, and optional
  per-key name fields. Save starts the save task; Discard is immediate.
- The save task result is shown as saved-and-verified or
  saved-but-verification-failed. Only after the save completes does the GUI
  describe the keys as available to future reads.
- Card removal closes the review modal and clears the pending review through
  the existing card-change reset path.
- All user-visible strings are in the i18n catalogue, English and Simplified
  Chinese complete.

## Affected packages and interfaces

- `internal/keys`: store version 2, names, key sets, reload, load report.
- `internal/workflow`: recovery references, key-set verification service.
- `app`: save policies, review state, reload, key-set operations, read
  request options.
- `internal/cli`: `--scan`, recovery save flags, key lifecycle commands.
- `internal/desktop`: scan-before-read option, recovery review modal, key
  rename, key-set panel, reload action, event handling.
- `locales`: English and Simplified Chinese catalogues.

No card-write protocol or libnfc-ng ABI changes. Recovery still releases the
reader to the external engine and re-verifies in-process. Save verification is
read-only authentication. Packaging, udev rules, and licenses are unchanged;
new files are covered by `REUSE.toml`.

## Failure handling and rollback

- A save that cannot persist or reload leaves the review pending and the
  in-memory store intact; the user may retry.
- A save that persists and reloads but fails authentication is reported as
  saved-with-rejections; rejected slots are unverified and the corrected store
  is written.
- A pending review blocks reads, writes, scans, recovery, other key mutations,
  and manual reload until Save or Discard.
- Card removal outside an operation discards the review and only the keys the
  task introduced. Card removal during an operation is handled by that
  operation's failure path.
- Discard does not restore a verification that recovery replaced; the earlier
  claim was already disproven by authentication.

## Tests

- Store: version 1 fixture loads and saves as version 2; names and sets round
  trip; missing set members prune with a report; empty sets drop; duplicate
  names/IDs fail; deleting a key removes set membership; reload swaps state.
- Workflow: recovery refs classify new, replaced, and existing; key-set
  verification authenticates each check without mutating the store; a
  definitive block/read failure unverifies only the attempted slot.
- Service: scan-before-read runs first and stops on failure; save writes,
  reloads, verifies, and reports rejected slots; discard deletes only new
  keys; pending review blocks a second recovery and reload.
- CLI: `--scan` on dump and restore; `recover --no-save` persists nothing;
  `--name`/`--key-set`; keys rename, sets, set create/rename/delete/export,
  reload; JSON output remains stable.
- GUI: scan-before-read passes the flag; review modal opens from the review
  event, joins values by ID, saves the selection, and discards; key-set panel
  actions; i18n check and `reuse lint`.

## Non-goals

- No claim that identical `UID/ATQA/SAK` cards can be distinguished.
- No key storage in dumps by default.
- No encryption or secret storage for `keys.json`; file permissions remain
  the protection.
- No automatic re-keying, formatting, or factory reset.
- No external-format key store replacement.
