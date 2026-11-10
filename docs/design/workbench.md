# GTK4 workbench contract

This is the current screen contract for `crates/gui`. It replaces the removed
GPUI rules for new UI work.

## Shell

- One window, 1360×880 default, resizable.
- Header: `NFC Foundry` title, `Actions` menu, primary `Read card`. The reader
  selector is not duplicated in the header.
- Left rail (fixed start pane, `GtkPaned`): reader menu with refresh, card
  identity/size/readability, key-store status, selected block, and an
  in-memory task/security log.
- Content: `Card`, `NDEF`, `Keys`, `Diff`, and `Restore` tabs; the primary
  action stays in the header.
- Status bar: operation text, spinner, and layout/card facts.

## Card

- `GtkColumnView` with `Block`, `Sector`, `Type`, `Status`, and `Data`.
  `Data` expands and shows all 16 bytes; unknown bytes are `??`.
- Selecting a row updates the inspector; the rail repeats the selection.
- Inspector: block number, sector, role, status, error text, decoded detail,
  hex entry, Read block, Write block, and the safety note.
- Decoded detail:
  - access group, its block range, C1/C2/C3 condition, and read/write/
    increment/decrement keys;
  - sector-trailer conditions for all four groups, trailer write permissions,
    Key A, Key B, and GPB;
  - value-block value and address when the 16 bytes form a valid value block;
  - manufacturer-block BCC status;
  - warnings for unknown bytes, unknown access bytes, and invalid access-bit
    complements.
- Empty state before a read: "No card loaded", reader + Read hint.
- Busy state: spinner, disabled reader menu and Read, disabled writes.

## Write flow

- The write button is disabled for block 0, incomplete blocks, unchanged
  bytes, a loaded dump, or no live card.
- Confirmation names the block, sector, old bytes, and new bytes; it states
  that a project backup and read-back happen.
- Sector trailers use the destructive appearance and the service's
  `confirm_trailer` gate.
- The GUI passes the loaded dump's card identity to the service. A card swap
  is refused with `CardChanged` before the backup or write.
- Success reports the backup path in the log and a toast; the row and
  inspector update from the written bytes.

## Keys

- `GtkColumnView` with `Sector`, `Type`, `Key`, and `Source`.
- Add one key: 12 hex digits, optional sector, `any`/`A`/`B`.
- Import `.dic` text or JSON; save the JSON store; export a Proxmark3 `.dic`.
- Copy states that `.dic` export is lossy and imported keys apply to every
  sector as `any`.
- Reads and writes use the same in-memory store; there is no second key
  source.

## Diff

- Load before and after images without a reader.
- Classify equal/changed/unknown blocks with `nfc-classic::diff`; unknown
  bytes never count as equal.
- The table lists changed and unknown blocks with byte offsets and trailer
  field changes.
- The summary reports layout, changed/equal/unknown counts, trailer changes,
  and an identity change.

## Restore

- Load a dump, target the selected live reader, confirm, back up, restore, and
  read back.
- The service stops on the first failure unless the continue option is
  selected. Block 0 stays outside the ordinary write path.
- A result table lists every block as written, unchanged, unknown, or failed
  with the failure detail; the summary reports written blocks, trailers,
  unchanged blocks, skipped unknown blocks, the backup path, warnings, and
  failures.
- A source image with a different card identity is refused before the backup.
- After a restore the table is marked stale and writes are disabled until a
  fresh read.

## Dump save and load

- Complete 1K/4K image: default `Auto` resolves to Proxmark3 raw `.bin`.
- Partial image: `Auto` resolves to the project `.nfc-foundry.json`; raw,
  Proxmark3 JSON, and MCT choices are disabled with an explanation.
- Explicit choices: Proxmark3 raw, Proxmark3 `mfc v2` JSON, MCT text, NFC
  Foundry project.
- Default names: `card-dump.bin`, `card-dump.mfc.json`, `card-dump.mct`, and
  `card-dump.nfc-foundry.json`.
- Load detects the format from content and shows format plus warnings. A
  partial read is never reported as complete.

## NDEF

- Read a card first.
- Show the MAD sectors, mapping version, read/write state, capacity, message
  bytes, Text/URI decoded records, and warnings.
- Unformatted/no-MAD cards are stated directly.
- No NDEF mutation control exists in this slice.

## Actions menu

- Refresh readers.
- Save dump… (format chooser, completeness-sensitive default).
- Load dump… (format detection; inspection only, writes disabled).

## Copy and states

- English base copy in this slice; `zh-CN` is a completion requirement.
- Hover/focus/disabled/destructive/error states come from Adwaita; no custom
  colors.
- Errors appear in a toast and the session log; partial reads keep their
  warnings visible in the log and card summary.
- Escape dismisses the topmost popover or dialog.
