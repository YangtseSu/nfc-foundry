# Note: phase 24a verified-key capture and dump key metadata

Status: complete 2026-11-11. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Spec:
[24a-verified-keys.md](../specs/24a-verified-keys.md).

## Problem

The rewrite authenticated keys on every Classic read but discarded the result.
`ClassicReadResult` carried only the dump, the key store had no card-scoped
verification state, and project dumps had no key metadata. Loading a dump lost
the keys that produced it, and a malformed line aborted a key-dictionary
import.

## Implemented

### Card identity

`CardInfo::identity_key()` returns `UID/ATQA/SAK` in uppercase hexadecimal with
no separators inside UID or ATQA. This is the scope key for verifications.
Tests cover 4-, 7-, and 10-byte UIDs.

### Key store v2

- `KeySource` adds `ReadContext`; source labels are stable on disk.
- `KeyVerification` records card, sector, key type, key, source, and Unix
  verification time.
- The JSON store is version 2 with an optional `verifications` array. Version 1
  loads with no verifications; unknown versions are rejected.
- One verification exists per `(card, sector, type)`. Recording a new key for
  the slot replaces the previous verification.
- `candidates_for_card` tries verified keys for the card first, then stored
  sector entries and wildcards, then common keys, deduplicated by type/key.
- `record_verified` adds a usable key entry when needed and updates the
  verification. Common keys used by a read are recorded with source `common`.
- `record_dump_keys` restores verifications from project metadata; unknown
  source labels fall back to `manual`.
- Removing a sector-specific entry removes matching verifications. Deleting a
  wildcard does not remove card-scoped records.

### Import report

`import_text`, `import_text_str`, `import_dic_str`, and `import_json_str` return
`KeyImportReport` (`added`, `duplicates`, `ignored`, `issues`). Malformed lines
are reported with 1-based line numbers and skipped; valid lines continue to
import. CLI and GUI show the counts and issues.

### Read and scan capture

- `ClassicReadResult` adds `keys`. Each authenticated sector contributes one
  `AuthenticatedKey` after at least one block was read; the key is also stored
  in `DumpMetadata.keys`. A denied sector contributes neither.
- `ClassicBlockReadResult` returns the card, data, and authenticating key.
- `ClassicKeyscanResult` returns the card, verified keys, and a `cancelled`
  flag. Cancellation returns the partial result instead of an error, so
  confirmed keys survive.
- `RecoveryResult` carries the card identity, set by `Service::recover` from
  the operation's card. The CLI/GUI record sector keys with source `recovery`.
- CLI read/dump/keyscan/recovery/NDEF and GUI read/scan/recovery/NDEF
  capture keys into the active store and persist atomically. Persistence
  failure is logged as a warning, not a read failure.
- Loading a project dump registers its keys and verifications.

### Dump project v2

`Dump` owns `DumpMetadata`: optional device, start/finish Unix seconds, and
sector keys (`sector`, `type`, `key`, optional source). The project format is
version 2 and writes all fields; version 1 loads with empty metadata. Metadata
is validated against the layout (sector bounds, A/B type, key length, no
duplicate slot) and is ignored by diff, raw export, and image equality semantics
in the codec tests.

`auto` keeps the existing rule: complete image to raw, partial image to project.
Raw, MCT, and Proxmark3 are image-only; preserving keys requires `--format
project`. The CLI warns when raw drops a non-empty key list. No raw sidecar was
added.

### Surfaces

- `nfctl classic read` and `classic dump` capture keys; `classic dump --format
  project` writes metadata.
- `nfctl classic keyscan` captures verified keys even with `--save` unset when a
  store path is available.
- `nfctl classic keys import` reports partial success.
- `nfctl recover` records sector keys after a successful result.
- The GTK read, block read, key scan, recovery, and NDEF paths record keys and
  refresh the Keys table. Keys table sector and source cells are localized.

## Tests

- `nfc-core`: identity formatting.
- `nfc-classic`: project v1 load, v2 metadata round trip, metadata validation,
  and auto format preservation.
- `nfc-app`: store v1/v2, verification replacement and deletion, candidate
  ordering, import reports, read capture, keyscan partial capture, and dump
  metadata registration.
- `nfctl`: read/dump capture and project metadata, tolerant import, and existing
  CLI contracts. The CLI test harness isolates `XDG_CONFIG_HOME` so no test
  writes the user's key store.
- GTK under Xvfb: read capture updates the in-memory store and the on-disk copy;
  the existing keys, keyscan, recovery, navigation, and language scenarios
  still pass.

Commands run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry
reuse lint
```

## Decisions and problems

- `auto` format behavior was left intact. Making metadata force the project
  format would silently change `classic dump` output for scripts; explicit
  project selection plus a warning is safer.
- Common keys are recorded as source `common`, even though a read authenticated
  them. This preserves provenance and lets the verification table scope them.
- The key store does not attempt to import the previous application's
  `NFC Foundry/keys.json` automatically; it remains a separate import path.
- The CLI now accepts a non-existent explicit `--key-store` as an empty store
  for read capture. Management commands still use `management_store` for the
  create/exists distinction.

## Next

S2 (editable workbench and selective restore) is complete; see
[24b-editable-workbench.md](24b-editable-workbench.md). It reused the
verification records and dump metadata added here. S3a (UID / block-0 write)
is next.
