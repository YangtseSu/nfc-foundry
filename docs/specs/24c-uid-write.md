# Spec: phase 24c standalone UID / block-0 write

Status: implemented 2026-11-11, hardware row pending. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Slice: S3a.

## Problem

The Rust workbench refuses every manufacturer-block (0) write. The ACR122U
driver already has a hardware-validated Gen1A sequence, but the application has
no preflight, backup, reselect, or block-0 verification for it, and no ordinary
CUID/Gen2 path. A UID change can make the card unreachable or report a stale
identity, so it needs the same one-shot plan and read-back discipline as the
restore path, plus a stronger confirmation than an ordinary block write.

## Scope

Supported:

- Four-byte UID MIFARE Classic 1K cards only.
- Ordinary CUID / Gen2 block-0 writes where sector 0 access conditions permit a
  data write and the card accepts one.
- ACR122U Gen1A protected block-0 writes through the existing
  `ManufacturerWriter` driver capability.
- The full block 0 is preserved except bytes 0..4 (UID) and byte 4 (BCC), so
  SAK, ATQA, and manufacturer bytes survive the change.

Not in this slice:

- Seven- and ten-byte UIDs, Classic 4K, and cascade BCC layouts.
- Full clone, factory format, and restore-with-UID (S3b).
- A PN532 `ManufacturerWriter`. PN532 has no Gen1A backend and is not advertised
  for one. The ordinary CUID/Gen2 path is reader-independent.
- External `nfc-mfsetuid` or any other process.

## Preflight

`ClassicUidPreflight` requires Advanced mode and a target `[u8; 4]` UID. It
opens a read-only operation and:

1. Requires a selected MIFARE Classic 1K with a four-byte UID.
2. Authenticates sector 0 with the key store and common keys.
3. Reads blocks 0..3 of sector 0.
4. Requires block 0's UID to equal the selected card UID.
5. Requires the current block-0 BCC to equal the XOR of its UID bytes.
6. Decodes the sector-0 trailer access bits, computes the data-write key types
   for group 0, and authenticates a candidate key of an allowed type. The read
   key and the ordinary-write key are stored separately; the ordinary backend
   is offered only when an allowed key authenticates.
7. Builds the new block 0: target UID in bytes 0..4, target BCC in byte 4,
   every other byte preserved.
8. Orders the write backends: Gen1A first when the reader exposes a
   `ManufacturerWriter`, then the ordinary write when the access bits permit
   one. At least one backend must be available.

The plan carries the token, card identity, layout, current and target UID, old
and new block 0, the authenticated sector-0 key, access key types, ordered
backends, warnings, expiry (120 seconds), and a fingerprint of the four
sector-0 blocks plus the card identity. An unchanged target is a valid plan
with `changed == false`; execution reports it without a backup or write.

## Execution

`ClassicUidExecution` consumes the one-shot token and requires Advanced mode
and confirmation. Before any backup it rechecks:

- token existence, expiry, and single use;
- card identity;
- a fresh sector-0 read matching the plan fingerprint (block 0, trailer, and
  the rest of sector 0);
- at least one planned backend is still available on the selected reader.

It then writes a project backup of the four sector-0 blocks and a
`UidBackupRecord` containing the old and new block 0 and both UIDs. Backends
run in plan order:

- Gen1A calls `ManufacturerWriter::write_manufacturer_block`; the driver
  reselects the card internally and sends the 7-bit `0x40` / `0x43` unlock
  before the block-0 write.
- Ordinary re-authenticates with the planned write key, then calls the
  transport write for block 0 without a single-block read-back, because the
  card identity changes with the block.

After every attempt, including a reported failure, the service reselects and
reads block 0 again. Cancellation is ignored for this post-write check so a
cancel cannot hide the card state. The outcome is one of:

- `applied`: reselected UID equals the target, block 0 matches the full planned
  bytes, the BCC is valid, and the sector-0 trailer is unchanged;
- `unchanged`: the reselected UID and block 0 still match the old state, so the
  next backend is tried;
- `different`: anything else, reported as an uncertain write with the backup
  path and no success claim.

A reported write error whose read-back shows `applied` is success with a
warning that the acknowledgement was lost. A transport loss during the
read-back is an uncertain outcome, not a failure that claims the card is
unchanged.

## CLI

`nfctl classic uid --uid <8 hex digits> --advanced [--dry-run]`.

- `--uid` accepts compact hex, spaces, colons, or dashes.
- `--advanced` is required in every mode.
- `--dry-run` prints the plan and exits without a backup or write.
- Execution prints old/new block 0, sector-0 key, access, backend order, result
  backend, reselected UID, and the backup path.
- Interactive execution requires typing the exact target UID. `--yes` is the
  non-interactive confirmation and is accepted only together with
  `--advanced`.

## GUI

The Classic Blocks inspector gains a session Advanced-mode switch and a
"Write UID…" destructive action. The action is available only for a live
four-byte Classic 1K image. The first dialog takes the target UID and refuses an
invalid or unchanged value. The preflight plan is shown in a second dialog that
requires typing the exact target UID. Execution calls the same service methods
as `nfctl`. A successful change invalidates the live image and re-reads the
card, because the card identity, keys, and workbench no longer apply. An error
also invalidates the image; the user must read the card again.

## Safety boundaries

- Block 0 is unreachable from the ordinary workbench, single-block write, and
  restore paths.
- No write without a one-shot preflight from the same card and sector-0 state.
- The current BCC must be valid; a malformed manufacturer block is refused.
- The card is reselected after the attempt; read-back is the authoritative
  success check.
- An ordinary immutable-UID card rejects both backends without changing any
  block; the service reports the refusal and never claims a UID change.
- No PN532 Gen1A claim and no external process.

## Tests

- Service tests: standard CUID success, Gen1A success when access bits deny a
  data write, unchanged UID, wrong sector-0 key, invalid access bits, ignored
  write, lost acknowledgement with a successful read-back, card disappearance
  after the write, cancellation, missing Advanced mode, stale/tampered plan,
  and an ordinary immutable card that changes nothing.
- CLI mock tests: dry run, advance gate, non-interactive confirmation gate,
  successful write with reselect evidence, and invalid UID input.
- Gated GTK scenario: Advanced switch, target dialog, typed UID plan, cancel,
  successful write, and image invalidation.
- Environment-gated hardware test for an owned magic Classic 1K: change the
  UID, reselect, verify block 0 and BCC, then restore the original UID.
- `nfctl` parsing and confirmation unit tests.

## Required checks

`cargo fmt --all --check`, workspace clippy with `-D warnings`, workspace tests,
gated GTK tests for changed screens, and `reuse lint`. The hardware row stays
`not recorded` until the gated run is performed on the owned magic card.
