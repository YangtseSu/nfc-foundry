# Spec: phase 24b editable workbench and selective restore

Status: implemented 2026-11-11. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Note:
[24b-editable-workbench.md](../notes/24b-editable-workbench.md).

## Problem

The Rust workbench shows a read or loaded image but cannot edit it as a working
copy. The inspector writes one block directly to the card, there is no undo,
revert, dirty state, or save baseline, and a partial re-read overwrites the
image. Restore always targets the whole image; there is no explicit block set,
no changes-only mode, and no pre-write plan that a caller can inspect and then
authorize.

## Workbench model

`nfc-app` owns `ClassicWorkbench`, a hardware-free edit session over one
`Dump`.

- `baseline` is the image last read from the card or file.
- `working` is the current edit image.
- A block is dirty when its data or known mask differs between `baseline` and
  `working`. Block status and error text do not make a block dirty.
- `edit_block` replaces one complete block with 16 known bytes and records an
  undo entry. The manufacturer block (0) is not editable in this path.
- `edit_ascii_block` accepts exactly 16 ASCII characters and uses the same edit
  path.
- `undo` restores the previous block state. Edits that change nothing do not
  add an undo entry.
- `revert` restores every block from `baseline` and clears undo history.
- `mark_saved` moves `baseline` to `working` and clears the dirty set; it is
  used after a project dump write.
- `mark_written` updates the baseline for blocks the card confirmed by
  read-back, so a successful write does not leave those blocks dirty and a
  failed write does not clear them.
- `merge_read` folds a partial or complete read into the workbench. Complete
  read blocks fill unknown working blocks, refresh non-dirty blocks, and are
  reported as conflicts when a differing working block is dirty. User edits
  are never silently overwritten by a merge.
- `validate` checks geometry, sector-trailer access bits, and manufacturer BCC.
  Unknown blocks and unknown access bytes are warnings; malformed trailers and
  a wrong BCC are errors.
- A workbench fingerprint covers layout, card identity, and every block's data
  and known mask. Write preflight binds the fingerprint of the image it planned.

`WorkbenchSnapshot` carries the working image, the dirty block list, and the
undo depth. The GUI renders snapshots; it does not reach into card I/O.

## Service boundary

`Service` holds at most one active `ClassicWorkbench` and exposes load, clear,
snapshot, edit, edit-ASCII, undo, revert, mark-saved, mark-written, merge, merge
one block, validate, and dump accessors. These operations perform no card I/O.

`ClassicWritePreflight` reads the live card into a backup image, validates the
card identity and layout, computes the write plan, and stores it under a
one-shot `WriteToken`. `ClassicWriteExecution` verifies that token, its
expiry, the source fingerprint, the card identity, and the current bytes of
every planned block before the first write. A stale token, a changed source
image, or a changed card is refused before a backup is written.

The token lifetime is 120 seconds. Expired or already used tokens are refused.
The plan contains:

- target card, layout, and selection mode;
- each pending block, its sector, trailer flag, old and new bytes;
- the access conditions that permit the write and the accepted key types;
- unchanged and unknown block counts;
- blocked sectors under the continue policy;
- warnings and the plan expiry.

Block 0 is never written by this path. A source block 0 that differs from the
card is an error when the plan selects it; an unselected block 0 is not
consulted.

## Selective restore

`RestoreSelection` is one of:

- `All`: every known block, the default.
- `Blocks(set)`: exactly the named block numbers.
- `ChangesOnly`: blocks where the source differs from the supplied baseline.
  The workbench supplies its baseline. A caller without a baseline compares the
  source with the current card. An empty changes-only plan is refused.

The plan is restricted to selected blocks. Unknown selected blocks are
reported as skipped, and an unchanged selected block is reported as unchanged
without a write. Data blocks are written before sector trailers. The ordinary
restore path, the preflight path, and the planned execution path share the same
plan builder.

## Interfaces

CLI `nfctl classic restore` gains:

- `--blocks "1,4,5-7"` for an explicit set;
- `--changes-only`;
- `--dry-run`, which prints the plan and exits without confirmation or write.

The GUI Restore screen gains the same three modes, renders the preflight plan in
its confirmation dialog, and executes the returned token. The Classic Blocks
inspector stages edits in the workbench; it no longer writes a block directly.
Undo and revert update the table, dirty rows are visible, and "Write changes…"
uses the changes-only preflight. A single-block re-read merges into the
workbench instead of bypassing it.

## Safety boundaries

- No write without a confirmed preflight from the same image and card.
- Every write still takes a pre-write project backup and reads every block back.
- Adaptive stop policy, trailer ordering, access checks, and identity checks
  remain unchanged.
- Block 0 remains read-only.
- The workbench itself never talks to a reader.

## Tests

- Workbench edit, ASCII edit, no-op edit, undo, revert, dirty tracking,
  save baseline, written baseline, validation, and merge conflicts.
- Plan selection for all, explicit blocks, changes-only, unknown blocks,
  empty changes, block-0 mismatch, and trailer ordering.
- Preflight token expiry, reuse, source fingerprint change, card identity
  change, and card data change before execution.
- CLI restore dry run, explicit block set, changes-only refusal, and normal
  restore output.
- GUI scenarios for staging, undo, revert, dirty display, plan dialog, token
  execution, and selective restore.

## Required checks

`cargo fmt --all --check`, workspace clippy with `-D warnings`, workspace tests,
gated GTK tests for changed screens, and `reuse lint`.
