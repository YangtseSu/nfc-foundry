# Phase 05c: Classic GUI writes and advanced bulk write

## Goal

Add MIFARE Classic 1K/4K write workflows to the GUI, including a normal block
editor, full-dump restore, UID/block 0 writes, and an Advanced bulk action that
can write all writable blocks from a validated dump or an explicit fill plan.

This phase introduces the shared **Advanced operations** mode used by Phase 05d.

## Policy

- Advanced mode may expose a generic bulk write. It remains a UI permission
  layer, not a protocol bypass.
- The bulk action uses the Classic restore preflight, a full-image backup,
  immediate read-back, and stop-on-first-failure.
- Block 0 and UID are separate operations; the bulk action never writes block 0.
- Sector trailers are written last and require an explicit advanced trailer
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
  - write only changed blocks;
  - full restore;
  - Advanced `Write all writable blocks` with a scope selector: ordinary data
    blocks, data + trailers, or full image;
  - an Advanced fill plan may target all data blocks or all writable blocks with
    an explicit fill value.
- Add a UID panel behind advanced mode. It requires the existing exact target
  UID confirmation, supports only 4-byte UID Classic 1K special cards, and
  reports the block-0 backup.
- Before any bulk write, full restore, fill, or UID operation, save the card's
  current complete dump as a backup. Abort if the backup cannot be written.
- Show preflight contents: blocks, trailers, access-condition changes, skipped
  block 0, UID order, and backup path.
- Add task progress, cancellation, and failure reporting with the backup path.
- Add English and Simplified Chinese messages.

## Acceptance

- An ordinary data block can be edited and written from the GUI with read-back
  verification.
- The Advanced bulk action can write all writable blocks from a validated dump
  or a fill plan, skips block 0, writes trailers last when selected, and
  requires the advanced confirmation.
- The advanced mode is off by default and resets on card/device change.
- A UID write requires advanced mode, the existing exact target confirmation,
  a block-0 backup, and verified reselection.
- Invalid dumps, incomplete dumps, unverified keys, and card changes stop
  before any write.
- `make check` passes.

## Non-goals

- No block-0 write through the bulk action; UID uses the dedicated path.
- No bypass of authentication, capacity, BCC, access-bit, or card-identity
  validation.
- No bypass of trailer/access-bit validation.
- No GUI write for unsupported UID lengths or non-Classic cards.
- No Type 2 protected writes; those are Phase 05d.
