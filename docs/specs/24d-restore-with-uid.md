# Spec: phase 24d restore with UID

Status: complete 2026-11-11; validated on PN532 UART and ACR122U. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Slice: S3b.

## Problem

A UID change is useful only after the dump that belongs with the new identity
has been restored. Running the two actions as separate commands leaves a
window where the card holds the old or a partial image, and a failure between
them can make the card look successful when only one half ran. S3a already
provides a one-shot UID plan, backup, reselect, and block-0 verification. S3b
combines that guard with the full-restore preflight so block 0 is written last
and only after the restored sector 0 is verified.

## Scope

Supported:

- A full restore of a complete or partial Classic 1K dump with an optional
  four-byte UID target.
- The ordinary restore selection rules: known source blocks only, data blocks
  before trailers, a project backup, per-block read-back, and the existing stop
  policy.
- Both UID backends from S3a: the Gen1A protected writer when the reader
  exposes one, then an ordinary block-0 write when the post-restore access
  bits permit it.
- `nfctl classic restore --uid --advanced` and the GTK Restore screen.

Not in this slice:

- A UID target with `--blocks` or `--changes-only`. Both stop before I/O.
- Seven-byte UIDs, Classic 4K, or cascade BCC layouts.
- Changing block 0 as part of the dump itself. Source block 0 must match the
  live manufacturer block; the target UID is a separate field.
- Full clone and factory format.

## Combined preflight

`ClassicWritePreflight` carries an optional `uid_target` and the Advanced-mode
bit. A UID target requires Advanced mode and `RestoreSelection::All`; either
violation is refused before the live card is read.

The existing restore preflight reads the card, selects the blocks, and orders
data blocks before trailers. It then builds the UID plan from the state the
restore will leave behind:

1. Source block 0 must be complete and equal to the live block 0, and the live
   BCC must be valid.
2. The post-restore trailer is the source trailer when the restore will write
   it; otherwise it is the live trailer.
3. The post-restore trailer must decode to valid access bits. Its group-0 data
   write mask decides whether an ordinary block-0 write is offered.
4. The verification key is:
   - the key that authenticated the live sector 0 when the trailer is not
     written; it survives the restore unchanged;
   - the restored trailer's Key A when the trailer is written, because that is
     the key the write stores.
5. The ordinary write key is the restored trailer's Key A or B when the
   trailer is written, or a stored candidate allowed by the live access bits
   when it is not.
6. At least one backend must be available. A post-restore trailer that denies
   data writes is refused unless the reader exposes the protected writer.
7. The plan stores the target UID, current and target block 0, post-restore
   trailer, read/write keys, backends, warnings, and the fingerprint of the
   four post-restore sector-0 blocks plus the card identity.

The existing write plan carries the UID plan under the same one-shot token.
The plan is shown by `--dry-run` and by the GUI confirmation dialog; the
confirmation names the exact target UID.

## Execution

The plan rechecks its token, expiry, source fingerprint, card identity, and
current block values exactly as an ordinary restore does. When a UID plan is
present it also requires Advanced mode and confirmation before the first card
write.

The restore phase runs with a collecting stop policy: a fatal block or reader
error is recorded in `failures` with the blocks already written, instead of
discarding the partial state. The UID phase does not run when `failures` is
non-empty.

After the restore:

1. The service re-reads sector 0 with the planned key and requires the four
   blocks to match the post-restore fingerprint. A mismatch reports the UID as
   `not attempted`; no UID write starts.
2. The UID phase consumes the same guard as S3a with the planned key pinned:
   sector-0 backup, backend order, reselect, full block-0/BCC/trailer
   verification, and the "unchanged" / "failed" / "uncertain" distinction.
3. `ClassicRestoreResult.uid` reports `applied`, `unchanged`, `failed`,
   `uncertain`, or `not attempted`, the backend, the final card identity when
   verified, and the UID backup.
4. A changed or uncertain UID clears a workbench that belonged to the old card
   identity. The GUI invalidates the live image and re-reads the card.

A restore without a UID target keeps the existing behavior, including a fatal
error returned directly rather than collected.

## CLI

`nfctl classic restore --input PATH [--continue-on-error] [--dry-run]
[--uid UID --advanced]`.

- `--uid` requires `--advanced`; `--advanced` requires `--uid`.
- `--uid` conflicts with `--blocks` and `--changes-only`.
- The confirmation text includes the exact target UID. `--yes` remains the
  non-interactive form; `--dry-run` needs no confirmation.
- The plan prints the target/current UID, post-restore key, backend order, and
  old/new block 0.
- The result prints restore counts first, then the UID phase and its backup. A
  dump failure and a UID failure are reported separately; either exits with an
  error.

## GUI

The Classic Restore screen gains an optional target UID entry and a session
Advanced-mode switch. A non-empty target requires Advanced mode and the "All
known blocks" mode. The preflight plan dialog names the exact UID and shows the
old/new block 0 before the destructive confirmation.

On success the drawer reports the UID phase separately from the restore counts.
An applied or uncertain UID invalidates the live image and starts a full
re-read; a failed UID leaves the restore counts visible and reports the UID
failure. The CLI and GUI call the same preflight and execution methods.

## Tests

- Workflow tests cover a successful restore followed by a UID change, a
  restored trailer that supplies the next write key, post-restore access bits
  that deny the UID write, Advanced/full-restore/confirmation gates, a dump
  block failure that skips the UID phase, and a UID failure that leaves the
  dump restore successful.
- CLI mock tests cover the dry-run plan, all three gates, the confirmation
  text, and a successful restore followed by a UID change.
- The gated GTK scenario covers the Advanced gate, target entry, plan dialog,
  cancel-free execution, UID change, and automatic re-read.
- An environment-gated hardware test restores a full dump with one changed
  data block, changes the UID, verifies the new UID and every restored block
  with a full read, then restores the original data and UID and verifies the
  original dump. It passed on PN532 UART and ACR122U on 2026-11-11.

## Safety boundaries

- Source block 0 is never written by the restore. The only block-0 write is
  the planned UID phase, after the restore is verified.
- Post-restore access bits and the surviving key are validated before the
  first write.
- A dump failure leaves the UID state untouched and reported as not attempted.
- An uncertain UID result never claims success.
- No external process and no second frontend.
