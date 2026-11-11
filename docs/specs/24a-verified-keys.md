# Spec: phase 24a verified-key capture and dump key metadata

Status: implemented 2026-11-11. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Note:
[24a-verified-keys.md](../notes/24a-verified-keys.md).

## Problem

A successful read authenticates a key for every readable sector, but the result
keeps only the block image. The key store never learns which key worked, a
saved project dump has no key context, and loading a dump leaves the user
without the keys that produced it. The previous implementation recorded
per-card verifications and stored sector keys in its dump metadata.

## Key store format

The on-disk key store moves to version 2:

```json
{
  "version": 2,
  "keys": [
    {"sector": 0, "type": "A", "key": "FFFFFFFFFFFF", "source": "read_context"}
  ],
  "verifications": [
    {
      "card": "04A2248A/0004/08",
      "sector": 0,
      "type": "A",
      "key": "FFFFFFFFFFFF",
      "source": "read_context",
      "verifiedAt": 1762819200
    }
  ]
}
```

- `card` is the selection identity `UID/ATQA/SAK`, uppercase hexadecimal, with
  no separators inside UID or ATQA.
- `verifiedAt` is Unix seconds. It is informational; no ordering guarantee is
  required.
- A verification is unique per `(card, sector, type)`. Recording a new key for
  the same slot replaces the old verification.
- `source` is one of `manual`, `key_file`, `key_scan`, `recovery`, `common`,
  or `read_context`. Unknown dump sources fall back to `manual` on import; an
  unknown source in the key store is an error.
- Version 1 stores load unchanged. Saving always writes version 2. A version 2
  file with no verifications is valid.
- Deleting a sector-specific key entry removes verifications that reference the
  same sector, type, and key. Deleting a wildcard does not remove card-scoped
  verifications.

## Capture behavior

- `ClassicReadResult` carries every authenticated sector key with its source.
  `read_dump` fills both the result list and `DumpMetadata.keys`.
- A single-block read returns the data plus the key that authenticated the
  sector, when authentication was required.
- Key scans return verified keys and the card identity. Cancellation returns
  the partial result with `cancelled = true`; already verified keys are kept.
- Recovery returns the card identity. Recovered sector keys are recorded with
  source `recovery`.
- `nfc-app` and both surfaces record captured keys into the active store and
  persist it atomically. The GUI keeps its explicit save actions; automatic
  persistence is silent and must not turn a successful card read into a failed
  operation when the store path is unavailable. A persistence failure is
  reported as a warning.
- Candidate ordering for a card is: verified keys for that card and sector,
  then stored sector-specific entries, then wildcards, then common keys.
  Duplicate `(type, key)` pairs are tried once.

## Dump project format

The project format moves to version 2. Version 1 files remain loadable.

```json
{
  "version": 2,
  "complete": true,
  "layout": "classic_1k",
  "card": {"uid": "04A2248A", "atqa": "0004", "sak": 8},
  "device": "pn532_uart:/dev/ttyUSB0",
  "startedAt": 1762819200,
  "finishedAt": 1762819205,
  "keys": [
    {"sector": 0, "type": "A", "key": "FFFFFFFFFFFF", "source": "read_context"}
  ],
  "blocks": []
}
```

- `device`, `startedAt`, `finishedAt`, and `keys` are optional.
- `keys` is validated against the layout: sector inside the layout, type `A` or
  `B`, valid key, no duplicate `(sector, type)`.
- `Dump` owns a `DumpMetadata` value. Equality, diff, and raw export ignore
  metadata.
- Project export always writes version 2. Project import accepts version 1 and
  version 2; version 1 implies empty metadata.
- No raw sidecar is added. Raw, MCT, and Proxmark3 exports are image-only and
  lossy for metadata. `auto` keeps the established rule: a complete image
  resolves to raw, a partial image to the project format. Preserving keys
  requires the explicit project format; the CLI warns when raw drops metadata.
- Loading a project or backup registers its keys and verifications into the
  active key store.

## Key import report

`KeyStore::import_text`, `import_text_str`, `import_dic_str`, and
`import_json_str` return:

```rust
pub struct KeyImportReport {
    pub added: usize,
    pub duplicates: usize,
    pub ignored: usize,
    pub issues: Vec<KeyImportIssue>,
}

pub struct KeyImportIssue {
    pub line: usize,
    pub message: String,
}
```

- Blank lines and comment-only lines increment `ignored`.
- A malformed line adds an issue and continues; it does not abort the import.
- Duplicate keys increment `duplicates`.
- JSON entries report their 1-based entry index as `line`.
- A file with no parseable keys returns a report with issues, not a hard error;
  the caller decides whether zero added keys is a failure.
- CLI and GUI print/show added, duplicate, ignored, and issue counts.

## Safety boundaries

- This slice is non-destructive. It adds no card write path.
- Key material is never logged by the automatic capture path. Counts and
  sources may be logged.
- A key is recorded only after authentication and, for key scans, a trailer
  read. A read result's key is recorded only for a sector whose blocks were
  read.

## Tests

- Card identity formatting for 4/7/10-byte UIDs.
- Key store v1 load, v2 round trip, verification replacement, deletion, and
  unknown-version rejection.
- Import reports for blanks, comments, duplicates, malformed lines, and JSON.
- Read capture records every authenticated sector key; a partial read records
  only the sectors it read.
- Keyscan cancellation returns verified keys and persists them.
- Recovery records source `recovery` and the card identity.
- Project v1 load, v2 round trip with keys/device/timestamps, invalid key
  metadata rejection, and `auto` choosing project when metadata is present.
- CLI: `classic dump` writes project metadata, `classic keys list` shows the
  store, `classic keys import` reports issues, and `classic keyscan` persists
  results.
- GUI: read and keyscan update the key table without a manual save; loading a
  project repopulates keys; cancel keeps confirmed keys.

## Required checks

`cargo fmt --all --check`, workspace clippy with `-D warnings`, workspace
tests, gated GTK tests for changed screens, and `reuse lint`.
