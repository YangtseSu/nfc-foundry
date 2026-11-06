# Phase 05c: Classic GUI writes and advanced bulk write

## Goal

Add MIFARE Classic 1K/4K write workflows to the GUI, including a normal block
editor, full-dump restore, UID/block 0 writes, and an advanced bulk action that
writes all blocks changed in a validated dump.

This phase introduces the shared **Advanced operations** mode used by Phase 05d.

## Repository constraints

- Do not add an unconstrained "write any writable block" escape hatch.
- The bulk action must operate on a complete, validated dump and the existing
  Classic restore preflight; it is not a bypass around key, trailer, access-bit,
  or block-0 checks.
- Block 0 is never written by the bulk action. UID writes use the existing
  confirmed `uid set` workflow and supported-card checks.
- Sector trailers are written last and only through the full-restore
  confirmation.

## Advanced operations mode

- A session-scoped toggle in the top bar, default off, labelled
  `Advanced operations`.
- Enabling it shows a warning that configuration, lock, UID, and trailer
  changes can permanently damage a card.
- The mode resets on card removal, device disconnect, and application restart.
- It only reveals controls; every dangerous action still has its own
  preflight and confirmation dialog.
- No "do not ask again" option.

## Work

- Expose Classic workbench operations through `app.Bindings` and the desktop
  `Backend`: block hex edit, preflight, start write, load/save dump, UID
  preflight/start, and task events.
- Add a block editor for ordinary data blocks. Write one block from normal
  mode; block 0 and trailers use their dedicated advanced actions.
- Add a dump panel:
  - load a `.bin`, `.mfd`, or `.json` dump;
  - save the current dump;
  - write only changed blocks (advanced);
  - full restore (advanced), with trailer confirmation.
- Add a UID panel behind advanced mode. It requires the existing exact target
  UID confirmation, supports only 4-byte UID Classic 1K special cards, and
  reports the block-0 backup.
- Before an advanced bulk write or full restore, save the card's current
  complete dump as a backup. Abort if the backup cannot be written.
- Show preflight contents: blocks, trailers, access-condition changes, skipped
  block 0, UID order, and backup path.
- Add task progress, cancellation, and failure reporting with the backup path.
- Add English and Simplified Chinese messages.

## Acceptance

- An ordinary data block can be edited and written from the GUI with read-back
  verification.
- The advanced bulk action writes only blocks changed in the loaded complete
  dump, skips block 0, writes trailers last, and requires the full-restore
  confirmation.
- The advanced mode is off by default and resets on card/device change.
- A UID write requires advanced mode, the existing exact target confirmation,
  a block-0 backup, and verified reselection.
- Invalid dumps, incomplete dumps, unverified keys, and card changes stop
  before any write.
- `make check` passes.

## Non-goals

- No arbitrary block-range writer outside a validated dump.
- No block-0 write through the bulk action.
- No bypass of trailer/access-bit validation.
- No GUI write for unsupported UID lengths or non-Classic cards.
- No Type 2 protected writes; those are Phase 05d.
