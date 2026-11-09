# Phase 04 spec: MIFARE Classic workflows

Date: 2026-11-09

## Goal

Turn the hardware-free Classic model into reader-backed workflows shared by
`nfctl` and the GPUI Kit workspace: read, dump, block write, restore, key scan,
key store, and pre-write backups. Every write is authenticated, backed up,
read back, and verified. Both surfaces call the same `nfc-app` methods and the
same safety policy.

## Deliverables

- `nfc-app` Classic workflow layer:
  - `KeyStore` with sector and wildcard entries, A/B/any key type, provenance,
    JSON persistence, text key-file import, candidate ordering, and merge.
  - Common factory-key dictionary used only as an authentication candidate.
  - `BackupStore` writing project-format snapshots atomically before mutation,
    with the default XDG data directory and an injectable directory for tests.
  - Full dump: authenticates each sector with the first candidate key that the
    card accepts, reading block 0 with sector 0. Sector 0 authentication is
    required: if it fails, block 0 remains unknown. Authentication failures and
    block read failures are recorded per block; an unreadable sector never
    becomes known data.
  - Single-block read and write. Writes are rejected for block 0, require
    explicit trailer confirmation, validate the target trailer's access bits,
    check the current access conditions, choose a key type that is allowed to
    write, back up the current card, write, read back, and recheck card
    identity.
  - Restore: requires explicit confirmation; source layout and known card
    identity must match; manufacturer block 0 may not change; every block to be
    written must be known in the pre-write backup; data blocks precede sector
    trailers; writes are verified one block at a time; the default policy stops
    at the first failure, with an explicit continue policy that records every
    failure.
  - Key scan: authenticates every candidate key as Key A and Key B per sector,
    verifies a successful authentication with a trailer read, records attempts,
    and returns a key store with the discovered keys. Authentication is the
    only card operation; nothing is written.
  - Cancellation is checked between authentication attempts and writes.
- `nfctl`:
  - `classic read --block`, `classic dump`, `classic write`, `classic restore`,
    `classic keyscan`, `classic keys list|import|add`, and
    `classic diff <before> <after>`.
  - Global `--key`, `--key-file`, `--key-store`, `--backup-dir`, and
    `--no-common-keys` options.
  - Human-readable summaries with partial-read warnings and backup paths.
- GPUI Kit Classic workspace:
  - Reader selection and refresh.
  - Block table with sector, status, known bytes, and data; block inspector
    with hex, ASCII, access bits, and value-block decoding.
  - Card dump, dump save, dump load, restore with confirmation, block write
    with trailer confirmation and backup, key scan, key store add/list.
  - Diff view comparing the last two dumps.
  - GPUI UI integration tests for dump, selection, write, keys, and diff.

## Non-goals

- No key recovery attacks. `nfc-recover` remains untouched.
- No manufacturer-block write, magic-card UID path, clone, factory format,
  value-block increment/decrement, or trailer field editing UI.
- No cross-process reader lock. In-process exclusivity remains the boundary.
- No NDEF or Type 2 workflow.
- No raw/MCT/Proxmark3 export from the GUI; the GUI saves and loads the
  project format and loads any supported format for restore and diff.
- No automatic key retrieval from the network.

## Safety

- Block 0 is never written. A restore whose source block 0 differs from the
  card aborts before any write.
- A trailer write requires explicit confirmation at the service boundary.
- Restore requires explicit confirmation at the service boundary.
- Backups are written before the first mutation and are stored in the project
  format, so partial and unreadable sectors stay explicit.
- A block is written only when the current access bits permit it with the
  selected key type. Data blocks are written before the sector trailer.
- Every write is read back and compared before the operation continues.
- Card identity is checked before authentication and after every write
  sequence; a UID, ATQA, or SAK change aborts the operation.
- Authentication failures during a dump produce a partial dump, never a
  complete-looking image.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Scripted tests prove:
  - dump reads block 0 after sector 0 authentication and leaves it unknown
    when sector 0 does not authenticate;
  - a failed authentication never marks unknown bytes as known;
  - single-block write rejects block 0 and unconfirmed trailer writes, rejects
    a write the current access bits forbid, and catches an ignored write by
    read-back;
  - restore backs up first, writes data blocks before trailers, stops at the
    first failure by default, and honors the continue policy;
  - restore rejects a different card identity and a differing block 0 before
    writing anything;
  - key scan records A and B keys per sector, counts attempts, and stops on
    cancellation;
  - key store JSON and text import round-trip and preserve wildcard entries.
- CLI mock smoke covers dump, read, write, restore, keyscan, keys list, and
  diff.
- The GPUI workspace test covers a dump populating the block table, selecting a
  block, adding a key, and a diff after a card change.
- PN532 UART hardware test: full dump with known keys, temporary block write
  with backup and read-back verification, restore of the written block, and a
  key scan, all environment gated. Hardware validation is recorded in
  `docs/notes/04-classic-workflows.md`.
