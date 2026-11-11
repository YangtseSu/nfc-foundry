# Note: phase 24d restore with UID

Status: complete 2026-11-11; validated on PN532 UART and ACR122U with an
owned magic Classic 1K. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Spec:
[24d-restore-with-uid.md](../specs/24d-restore-with-uid.md).

## Problem

S3a could change block 0 and S2 could restore a dump, but there was no single
guarded operation that restored the image and then changed the UID. Two
separate commands leave the card in an intermediate state, and a dump failure
could still be followed by a UID change because nothing tied the two plans to
the same token and card fingerprint.

## Implemented

### Combined plan

- `ClassicWritePreflight` gained `uid_target` and `advanced`. A UID target
  requires Advanced mode and `RestoreSelection::All`; both are checked before
  the live read.
- `ClassicWritePlan` carries `uid: Option<ClassicUidPlan>` under the same
  one-shot token as the restore.
- `uid::plan_for_restore` builds the UID plan from the state the restore will
  leave behind:
  - source block 0 must be complete and equal to the live block 0;
  - the post-restore trailer is the source trailer when it will be written,
    otherwise the live trailer;
  - the trailer access bits decide the ordinary write mask;
  - when the trailer is written, its Key A is the verification key and its
    Key A/B is the write key according to the write mask;
  - when the trailer is unchanged, the key recorded by the live read is the
    surviving key, with a candidate search for the write slot;
  - at least one backend is required, Gen1A first when available;
  - the fingerprint covers the four post-restore sector-0 blocks and the card
    identity.
- The UID plan is built in preflight and stored in the write token; execution
  does not accept a caller-supplied target.

### Execution order

- `restore_with_plan` now takes the Advanced bit and refuses a UID plan without
  it before any write.
- The restore phase uses a collecting execution mode only when a UID plan is
  present. A fatal block or reader error is converted into `failures` while
  keeping the blocks already written; ordinary restores still return the
  error directly.
- The UID phase runs only when the restore result has no failures. It first
  re-reads sector 0 with the planned key and requires the post-restore
  fingerprint to match. A mismatch reports `not attempted` and writes nothing.
- The UID phase reuses the S3a execution with the planned key pinned, so a
  restored trailer key that is not in the key store still authenticates. It
  keeps the S3a sector-0 backup, backend order, reselect, and full block-0,
  BCC, and trailer verification.
- `ClassicRestoreResult.uid` reports `applied`, `unchanged`, `failed`,
  `uncertain`, or `not attempted`, with the backend, final card identity, UID
  backup, detail, and warnings. Dump failures stay in `failures`; the two
  states are never merged.
- An applied or uncertain UID clears a workbench for the old card identity.
  The GUI invalidates the live image and re-reads the card. The CLI exits with
  an error for a failed, uncertain, or skipped UID phase even when the dump
  restore succeeded.

### CLI

`nfctl classic restore --input PATH --uid UID --advanced [--dry-run]`.

- `--uid` requires `--advanced`, and `--advanced` requires `--uid`.
- `--uid` conflicts with `--blocks` and `--changes-only`.
- The confirmation text names the exact UID; `--yes` is the non-interactive
  form and `--dry-run` needs no confirmation.
- The plan prints the post-restore key, backend order, and old/new block 0.
- The result prints restore counts, then the UID phase and the UID backup.

### GUI

- The Restore screen has an optional target UID entry and a session Advanced
  switch.
- A non-empty target requires Advanced mode and the full-restore mode.
- The plan dialog names the target UID, the post-restore key, backends, and
  old/new block 0.
- An applied or uncertain UID invalidates the live image and starts a full
  re-read. A failed UID leaves the restore counts visible and reports the UID
  failure separately.
- The Restore note now states that block 0 is written only when a target UID
  is supplied, after the restore is verified.

## Tests

- `nfc-app` workflow tests:
  - full restore then UID change with the write order `[data block, block 0]`
    and a full read-back;
  - restored trailer supplying a Key B write key that was not the live read
    key;
  - post-restore access bits that deny a UID write and no Gen1A writer;
  - Advanced, full-restore, and confirmation gates;
  - dump block failure collected with the UID phase `not attempted`;
  - dump success followed by a UID failure reported separately.
- `nfctl` mock tests: dry-run plan, all three gates, confirmation text, and a
  successful restore plus UID change.
- Gated GTK scenario: Advanced gate, target entry, plan dialog, execution,
  UID change, and automatic re-read. It passed under Xvfb on 2026-11-11.
- Environment-gated hardware test `restores_a_dump_and_changes_the_uid_last`
  passed on PN532 UART and ACR122U with an owned magic Classic 1K. The test
  restores a full dump with one changed data block, writes a different UID,
  verifies every block and the new UID with a full read, then returns the
  original data and UID through a second restore-with-UID plan and verifies the
  original dump byte for byte.
- Trailer preservation tests cover a single-block trailer write with a masked
  Key A and a restore whose source trailer carries a masked Key A.

## Commands run

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry gtk_workbench_scenarios
reuse lint
cargo test -p nfctl --features hardware-write --test hardware_classic --no-run
```

Hardware, 2026-11-11, PN532 UART `pn532_uart:/dev/ttyUSB0`, magic Gen1A
Classic 1K UID `FA B9 CF F4`, keys `FFFFFFFFFFFF` and `FBBECDF6304E`:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_KEY_2=FBBECDF6304E \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-pn532-final \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored --test-threads=1 --nocapture
```

All five Classic hardware checks passed: full dump and per-sector scan
(16/16 sectors, 17 attempts, 16 verified keys), temporary data-block write,
sector-trailer write, standalone UID change, and restore with UID. The
restore-with-UID run used UID `FA B9 CF F5` and returned to `FA B9 CF F4` with
the original dump verified by a full read.

Hardware, 2026-11-11, ACR122U `acr122_usb:001:002`, magic Gen1A Classic 1K
UID `FA B9 CF F4`, keys `000000000000`, `D3F7D3F7D3F7`, and `FFFFFFFFFFFF`:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
NFC_FOUNDRY_HARDWARE_KEY=000000000000 \
NFC_FOUNDRY_HARDWARE_KEY_2=D3F7D3F7D3F7 \
NFC_FOUNDRY_HARDWARE_KEY_3=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_TRAILER=15 \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-acr-final \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored --test-threads=1 --nocapture
```

All five checks passed. The UID path selected Gen1A. The restore-with-UID run
changed `FA B9 CF F4` to `FA B9 CF F5` through Gen1A and returned to the
original UID and dump.

`pcscd` was stopped before the ACR122U runs and restarted by the user's normal
session policy afterwards.

## Decisions and problems

- The UID target is a separate field, not a replacement for source block 0.
  A dump that disagrees with the live manufacturer block is refused. This
  keeps S3b as restore-then-change and leaves full clone to its own spec.
- The combined path uses the source trailer bytes as the post-restore key
  material when the trailer is written. A masked slot (all zero) is resolved
  from the dump's recorded Key A/B metadata, then from the live read, because
  a trailer read never returns Key A and can mask Key B. When the trailer is
  unchanged, the key recorded by the live read survives and is used instead.
- The hardware run exposed a pre-existing trailer-write bug: a read-modify-write
  of a trailer sent the masked Key A zeros back to the card and destroyed the
  key. `Operation::write_trailer_verified` now fills masked slots from the
  authenticated key, verifies the access bytes and GPB by reading back, and
  proves the new Key A and, when writable, Key B by authenticating with them.
  Restore hydrates masked slots from the dump metadata before the write.
  Single-block trailer writes with a B-only access condition and a masked
  Key A now refuse instead of writing zeros, because the generic block request
  carries no separate Key A field.
- A full read of every block after the restore would duplicate the per-block
  read-back and is not what gates the UID. Sector 0 is re-read against the
  post-restore fingerprint because it is the sector the UID phase depends on.
- The collecting stop policy is limited to the restore-with-UID path. Changing
  the ordinary restore error contract would affect callers that already rely
  on a fatal error.
- `uid::execute` accepts a pinned sector-0 key. Falling back to another stored
  candidate could authenticate a different key and hide that the planned key
  did not survive the trailer write.
- The GUI re-reads after an applied UID because the identity, keys, workbench,
  and NDEF mapping belong to the old UID.

## Remaining

- No hardware gap remains for the four-byte Classic 1K restore-with-UID path.
  Seven-byte UIDs, Classic 4K, and full clone remain separate future work.
