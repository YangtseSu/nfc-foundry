# Phase 04 note: MIFARE Classic workflows

Date: 2026-11-10

## Implemented

- `nfc-app::classic` owns the Classic workflow layer used by both surfaces.
- `KeyStore`:
  - sector-specific and wildcard entries, Key A / Key B / any type, and
    provenance (`manual`, `key_file`, `key_scan`, `common`);
  - JSON persistence with version and range validation, atomic replacement,
    and parent-directory creation;
  - text key-file import with comments and duplicate suppression;
  - candidate ordering that prefers sector-specific entries, then wildcards,
    with A before B for each entry;
  - merge and removal used by key scan and the GUI.
- Common factory keys are authentication candidates only. A key counts as
  verified after the card accepts it and a trailer read succeeds.
- `BackupStore` writes project-format dumps atomically. The default directory
  is `$XDG_DATA_HOME/nfc-foundry/backups` or
  `$HOME/.local/share/nfc-foundry/backups`. Tests use a temporary directory.
- Full read:
  - authenticates each sector, including sector 0, with the first candidate key
    the card accepts;
  - reads block 0 after sector 0 authentication; if sector 0 does not
    authenticate, block 0 remains unknown;
  - records `AuthFailed` and `ReadFailed` per block and never marks unknown
    bytes as known;
  - returns a partial dump with warnings, and fails only when no block could
    be read or a non-recoverable card error occurs.
- Single-block read authenticates the sector for block 0 like any other
  block.
- Single-block write:
  - rejects manufacturer block 0;
  - requires explicit trailer confirmation at the service boundary;
  - validates the target trailer's access-bit complements;
  - checks the current access conditions and accepts only a key type that the
    sector permits;
  - writes a pre-mutation backup, writes one block, reads it back, and checks
    card identity again;
  - skips the write when the block already matches.
- Restore:
  - requires explicit confirmation;
  - checks the source layout, known card identity, and manufacturer block 0
    before writing anything;
  - refuses to write a block that is unknown in the pre-write backup;
  - writes ordinary data blocks before the sector trailer;
  - stops at the first write failure by default and records failures only when
    the caller selects the continue policy;
  - reports written, unchanged, skipped-unknown, and trailer counts, plus the
    backup path.
- Key scan:
  - tries every candidate key as Key A and Key B for every sector;
  - verifies an accepted key with a trailer read;
  - counts attempts and verified keys and returns a store of sector-specific
    discoveries;
  - checks cancellation between attempts and stops on card loss, timeout, or
    identity change.
- `nfctl`:
  - `classic read --block`, `classic dump`, `classic write`,
    `classic restore`, `classic keyscan`, `classic keys list|import|add`, and
    `classic diff <before> <after>`;
  - global `--key`, `--key-file`, `--key-store`, `--backup-dir`, and
    `--no-common-keys`;
  - stable exit code 2 for errors and human-readable summaries.
- GPUI Kit Classic workspace:
  - reader refresh/selection, card read, dump save/load, restore confirmation,
    block write with trailer confirmation, key scan, key add/remove, and a
    two-dump diff;
  - block table with manufacturer/data/trailer type, status, known-byte count,
    and data;
  - inspector with hex, ASCII, value-block decoding, access condition, and a
    block editor;
  - all card operations run on a background executor through the shared
    `Service`; render performs no I/O.
- The mock Classic 1K reader now carries a complete BCC-valid image so the CLI
  and GUI smoke tests exercise real dump, write, restore, and key-scan paths.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p nfctl --features hardware-write --all-targets -- -D warnings`
- `cargo clippy -p nfc-drivers --features hardware --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test -p nfc-drivers`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `nfc-app` workflow tests use a scripted transport with per-sector key rules,
  ignored writes, failing writes, and cancellation hooks:
  - partial dump with block 0 read and sector 1 authentication failures;
  - block 0 and unconfirmed trailer writes rejected;
  - access conditions that forbid a write rejected before mutation;
  - backup created before a write and loaded back with the old data;
  - ignored write detected by read-back;
  - restore writes data blocks before the trailer and verifies each block;
  - restore stops at the first failure by default;
  - restore continues only when explicitly selected and records the failure;
  - restore rejects a different UID and a differing manufacturer block 0
    before writing;
  - restore requires confirmation;
  - key scan finds A and B keys, counts attempts, and stops on cancellation;
  - single-block read authenticates the sector before reading block 0;
  - key store JSON and text import round-trip, including wildcard entries.
- `nfctl` integration tests run the built binary against `--mock`:
  - list and scan;
  - dump, read, and diff;
  - write with backup and restore with confirmation;
  - block 0, unconfirmed trailer, and unconfirmed restore rejection;
  - key scan persistence and key list/add/import.
- GPUI UI integration tests:
  - read populates the block table and selecting a block updates the inspector;
  - a second read after a card change produces a one-block diff and renders the
    diff tab;
  - key add and remove update the store and status;
  - block write updates the mock card and the local dump;
  - restore opens a confirmation dialog and does not mutate the card before
    confirmation;
  - key scan populates the store.
- The hardware tests compile with `hardware`/`hardware-write` but were not run:
  - `nfctl/tests/hardware_classic.rs` dumps and scans a card;
  - the gated write test changes one ordinary data block, writes the original
    back, and verifies the final value.

## Problems and resolutions

- The first hardware run failed with `NoCard` after a successful scan.
  `Operation::authenticate` reselects the card before authentication, and the
  PN532 does not answer a second `InListPassiveTarget` while the card is
  ACTIVE. `Pn532Protocol::select_card` now cycles the RF field before a
  repeated selection so the card returns to IDLE. The CLI then read block 4
  and completed a 64-block dump.
- The workflow originally assumed block 0 was readable without authentication.
  MIFARE Classic requires sector authentication for block 0, and the PN532
  path does not expose unauthenticated reads. Block 0 is now read after sector
  0 authentication, and a failed sector 0 leaves it explicitly unknown.
- A write path that chose the first stored key could select a key type the
  current access bits forbid. The workflow now intersects the read/write
  permissions of every pending block, selects only an allowed key type, and
  aborts before the backup when no type is permitted.
- Restore initially could write a block whose current value was unknown, which
  made rollback impossible. It now aborts when a pending block is not known in
  the pre-write backup.
- The GUI originally updated `busy` through `Entity::update` from a button
  listener, which re-entered the entity already being updated and panicked in
  UI tests. Busy state is now set directly by the owning method, or from the
  dialog callback where the entity is not already borrowed.
- The UI test for block 63 asserted visibility, but the row was correctly
  clipped below the scroll viewport. The test now checks the first visible row
  and selects a block in view.
- `gpui-kit` with `test-support` resolves its optional `gpui-fast` graph,
  which requires exact `taffy 0.15.0` and `unicode-properties 0.1.3` while the
  existing lock had newer compatible versions. The lock pins both to the
  versions accepted by both `gpui-pre` and `gpui-fast`.
- The GUI test-support dependency initially caused the workspace resolver to
  fail before the lock was updated. The versions were pinned with the GUI
  member temporarily removed, then the member was restored and the full
  workspace resolved.

## Deviations

- Manufacturer block 0 writes, magic-card UID writes, clone, factory format,
  and value increment/decrement remain out of scope. Block 0 is read-only in
  both surfaces.
- The GUI saves and loads the project format; raw, MCT, and Proxmark3 exchange
  stays in the CLI. Restore and diff accept any format the exchange layer
  detects.
- The GUI exposes the safer default restore policy only. `--continue-on-error`
  is a CLI option, and the GUI reports a failure without continuing.
- Key scan writes nothing to the card and records keys only after a trailer
  read succeeds.
- No cross-process reader lock is added. The in-process reader lease remains
  the exclusivity boundary.
- GUI copy is English in this phase; the application-wide localization pass is
  still outstanding.

## Hardware validation

Run on 2026-11-10 with a PN532 over `/dev/ttyUSB0` and an owned MIFARE Classic
1K card (`FA B9 CF F4`, ATQA `00 04`, SAK `08`):

- Full read with the common-key candidate list: 64/64 blocks and 16/16 sectors
  readable; the project dump was written with mode `0600`.
- Authenticated block 0 read returned `FA B9 CF F4 78 08 04 ...`, with the
  four-byte UID BCC `78`.
- Key scan with the common candidates: all 16 sectors found both Key A and Key
  B; 37 attempts and 32 verified keys. Sector 0 Key A is `A0A1A2A3A4A5`,
  sectors 1-2 Key A is `D3F7D3F7D3F7`, and the remaining sectors use
  `FFFFFFFFFFFF`; Key B is `FFFFFFFFFFFF` on every sector.
- Read-only gated test with only `FFFFFFFFFFFF` as Key A: 13/16 sectors
  readable and 13 verified keys, matching the sectors that use that key.
- Temporary write on block 4 with Key A `D3F7D3F7D3F7`: first byte changed,
  read back, original written back, and read back. Both writes created
  project-format backups with mode `0600`. The final block read matched the
  original `03 2B D1 01 27 54 02 65 6E 4E 46 43 20 46 6F 75`.
- Restore from the full dump after the temporary change: 1 written, 62
  unchanged, 0 unknown skipped, 0 trailer writes, with a pre-write backup.
  The final read matched the original block.
- Restore of the unchanged dump: 0 written, 63 unchanged.

## Remaining blockers

- Cross-process locking and physical-reader identity remain future work.
- Segmentation of a 4K card across the large sectors is covered by scripted
  tests and the Classic core property tests, but not by a hardware 4K run.
- Trailer writes were not exercised on hardware; the temporary write used an
  ordinary data block. The service still requires trailer confirmation.
- The GUI was exercised through headless UI integration tests, not with a
  visible window on the hardware run.
