# Note: phase 24b editable workbench and selective restore

Status: complete 2026-11-11. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Spec:
[24b-editable-workbench.md](../specs/24b-editable-workbench.md).

## Problem

The rewrite could read, write one block, and restore a whole image, but it had
no working copy. Inspecting a block wrote the card directly, a re-read
overwrote the displayed image, and there was no undo, revert, dirty state, or
per-block restore. Every restore read the whole card and had no plan that a
caller could inspect before authorizing the mutation.

## Implemented

### Workbench model

`nfc-app::classic::ClassicWorkbench` owns a baseline and a working `Dump`.

- `edit_block` and `edit_ascii_block` replace one complete block and record an
  undo entry. Block 0 is not editable.
- `undo`, `revert`, `mark_saved`, and `mark_written` update the working and
  baseline images. `mark_written` marks only read-back blocks as verified.
- Dirty blocks compare data and known masks, not status text, so a status
  refresh never creates a false edit.
- `merge_read` and `merge_block` fill unknown blocks, refresh non-dirty blocks,
  reconcile a dirty edit the card already holds, and keep conflicting dirty
  edits with a report. A merge from a different card identity is refused.
- `validate` reports geometry, trailer access-bit, and manufacturer BCC errors.
  Unknown blocks and unknown access bytes are warnings.
- `dump_fingerprint` hashes layout, card identity, block bytes, and known masks.
  It binds a preflight plan to both the source image and the card backup.

`Service` owns at most one workbench and exposes load, clear, snapshot, dump,
baseline, export, edit, ASCII edit, undo, revert, mark-saved, mark-written,
merge, merge-block, and validate methods. None of them opens a reader.

### Preflight and selective restore

`RestoreSelection` is `All` (default), `Blocks(set)`, or `ChangesOnly`.
`parse_block_list` accepts `1,4,5-7` and rejects empty or reversed ranges.

`ClassicWritePreflight` reads the card once without mutating it, validates the
source and card identity, and builds a `ClassicWritePlan`:

- selected block numbers and the pending write blocks;
- old/new bytes, sector, trailer flag, and the accepted key types;
- unchanged and unknown counts, blocked sectors, and warnings;
- a one-shot `WriteToken`, a 120-second expiry, the source fingerprint, and the
  card-backup fingerprint.

`Service::classic_write_preflight` stores the plan with its baseline. The
execution path consumes the token, rechecks expiry, layout, card identity,
source fingerprint, card-backup fingerprint, and the pending block bytes before
the backup or the first write. A stale token, an edited image, or any card
change outside the planned blocks is refused with `CardChanged` or
`InvalidArgument` and leaves no backup.

The ordinary `ClassicRestore` path uses the same preparation and execution
code. It now carries the selection, the changes-only baseline, and a dry-run
flag. Dry run reports `Planned` block states, no backup, and no writes.

`ClassicRestoreResult` adds `planned_blocks`, `dry_run`, and
`not_selected_blocks`. `RestoreBlockState` adds `Planned` and `NotSelected`.
Block 0 is never written; a selected source block 0 that differs from the card
is an error, and an unselected difference is not consulted.

### CLI

`nfctl classic restore` gains `--blocks`, `--changes-only`, and `--dry-run`.
Restore always builds a preflight plan first and prints it: block, sector,
role, accepted keys, old and new bytes. `--dry-run` does not require `--yes`
and never creates a backup. `nfctl classic write --dry-run` reads the card,
plans the single block, prints the same plan, and exits without writing.

### GUI

- The Blocks inspector now stages edits. "Write changes…" runs the same
  changes-only preflight and shows the plan dialog before executing the token.
  Undo and revert update the table immediately; dirty rows show `edited`.
- Reading a card or loading a dump resets the workbench. A single-block re-read
  and an NDEF write merge into the workbench instead of bypassing it; conflicting
  staged edits are kept and reported.
- The Restore screen has all-known-blocks, changes-only, and selected-blocks
  modes plus the block-list entry. Its confirmation dialog is the preflight
  plan, and execution uses the returned token.
- A stale workbench after the plan dialog is open is refused by the source
  fingerprint check. A card swap is refused by the card fingerprint check.
- New workbench and plan strings are in the English and `zh-CN` catalogs.

## Tests

- `nfc-app` unit tests: edit, ASCII edit, no-op edit, undo, revert, dirty
  tracking, saved/written baselines, merge conflicts, reconciliation, BCC and
  trailer validation, and fingerprint changes.
- `nfc-app` workflow tests: selective restore, changes-only with a baseline and
  empty-plan refusal, dry run, token single use, source change, card change
  outside the plan, expired token, and cross-card merge refusal.
- `nfctl` tests: restore dry run, explicit block set, changes-only refusal,
  write dry run, and the existing restore output contract.
- Gated GTK scenario: staging, dirty row, undo, revert, plan dialog, token
  execution, card-swap refusal, and selective restore.

Commands run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry
reuse lint
```

## Decisions and problems

- The preflight stores both a source fingerprint and a card-backup fingerprint.
  The backup fingerprint catches a card change outside the pending block set,
  which the block list alone would miss.
- `ChangesOnly` with a workbench baseline compares source against that
  baseline. A restore without a baseline compares against the live card, which
  preserves the previous restore behavior; an empty selection is an error.
- Block 0 is excluded from the dirty comparison of the workbench write because
  the editor refuses block 0. A selected block 0 still contributes an
  `unchanged` row for the existing full-restore output.
- The GUI write module changed from direct `ClassicBlockWrite` calls to the
  workbench and token path. The single-block service API remains for callers
  that need one confirmed block without a workbench.
- The plan dialog shows up to 12 blocks because a Classic 4K plan can include
  256 entries. The CLI prints every planned block.
- No hardware rerun was performed in this slice. The reader protocol is
  unchanged; restore now performs a read-only preflight read before the backup
  read and write. The scripted transport covers the token and fingerprint
  gates, and the existing hardware restore remainder is noted in the hardware
  matrix.

## Next

S3a (standalone UID / block-0 write) follows. It reuses the preflight token,
backup, and read-back patterns added here.
