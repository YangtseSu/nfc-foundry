# Phase 05c spec: Classic GUI writes and advanced bulk write

Date: 2026-11-07

## Goal

Connect the existing Classic workbench, restore, and UID services to the MyGo
GUI, and add a guarded advanced bulk write over a complete validated dump. The
protocol and restore pipeline remain unchanged.

## Scope

Normal mode:

- edit and write one ordinary data block;
- save the current dump;
- load a dump for inspection.

Advanced mode:

- write all blocks changed in the loaded complete dump;
- full restore, including trailers, with the existing trailer confirmation;
- UID/block 0 write on supported 4-byte UID Classic 1K cards.

## Application surface

Add DTOs and bindings for:

```go
EditBlockHex(block int, hex string) (WorkbenchDTO, error)
LoadDumpFile(path string, discardDirty bool) (WorkbenchDTO, error)
SaveDumpFile(path string) (SaveResultDTO, error)
PreflightWrite(WritePreflightRequestDTO) (WritePreflightDTO, error)
StartWrite(WriteStartRequestDTO) (TaskDTO, error)
PreflightUIDWrite(uid string) (UIDWritePreflightDTO, error)
StartUIDWrite(UIDWriteStartRequestDTO) (TaskDTO, error)
```

The GUI must not call `internal/workflow` directly.

`WritePreflightRequestDTO` carries an explicit `Blocks` list for the `block`
mode used by the editor. `WritePreflightDTO` returns the planned pre-write
backup path. The dialog-based `LoadDump`/`SaveDump` bindings wrap the existing
native file pickers.

Before an advanced bulk write, full restore, or standalone UID write, read the
current card into a new complete dump and save it as a backup under
`$XDG_CONFIG_HOME/NFC Foundry/classic-backups/`. If the backup cannot be
written, return an error without writing. The preflight token binds the
operation to the loaded dump, the dump's source card identity, the current
card, and the planned blocks.

## Bulk write semantics

- `Write block` uses preflight mode `block` with exactly one ordinary data
  block. The service revalidates that the block is not block 0 or a trailer.
- `Write all changed blocks` uses preflight mode `changes` on the loaded dump.
- Blocks are limited to the dump's dirty blocks; block 0 is removed.
- The existing restore service writes ordinary blocks before trailers.
- Trailers require the full-restore confirmation because they can change keys
  and access conditions.
- The operation stops at the first unrecoverable failure.
- The GUI never offers an empty block range or an arbitrary range input.

The label is `Write changed blocks`, not `Write everything`, to keep the entry
bound to a validated dump. A user who wants a complete image uses `Full
restore`.

## UI

- Block table selection opens a hex editor. Only ordinary data blocks are
  editable in normal mode.
- `Write block` writes one block and relies on the reader's immediate read-back.
- `Advanced operations` toggle reveals:
  - `Write changed blocks`;
  - `Full restore`;
  - `Set UID`.
- The preflight panel lists planned blocks, trailers, access changes, skipped
  block 0, UID ordering, warnings, and the backup path.
- The block editor accepts exactly 16 hexadecimal bytes and is available only
  for ordinary data blocks.
- The UID panel accepts one 4-byte UID and opens the exact-UID confirmation.
- Confirmation dialogs are separate for:
  - ordinary block write;
  - changed-blocks write;
  - full restore with trailers;
  - UID write with exact target UID.
- Task failure keeps the backup path visible in the error banner and activity
  log.

## Safety rules

- The loaded dump must be complete, valid, and carry the source card identity.
- The card UID/ATQA/SAK must match the dump and the card that started the task.
  A dump without metadata or from another card is refused before preflight.
- Block 0 is never written by the bulk action.
- Trailers last; access-bit complements validated before any trailer write.
- UID writes use the existing preflight, Gen1A/CUID path, exact confirmation,
  a complete pre-write backup, the block-0 backup, reselection, and full
  block-0 verification.
- Advanced mode resets on card/device change.
- Failed writes stop; no continue-on-error mode.

## Tests

Hardware-free:

- binding tests for dump load/save, block edit, preflight, and write tasks;
- GUI tests for block editing, dirty state, advanced toggle reset, and
  confirmation paths;
- refusal of block 0 in the bulk action;
- trailer confirmation required for full restore;
- UID exact-confirmation refusal;
- backup failure aborts before writing;
- card-change abort;
- dump/source-card mismatch refusal;
- bulk preflight excludes block 0.

Existing Classic dump/restore and UID hardware tests remain the hardware gate.
No new protocol code is introduced.

## Non-goals

- No arbitrary range or write-all bypass.
- No Classic GUI write to a card that does not match the loaded dump.
- No UID write outside the existing supported-card path.
