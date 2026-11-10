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
- Content: `Card` and `NDEF` task switcher; the primary action stays in the
  header.
- Status bar: operation text, spinner, and layout/card facts.

## Card

- `GtkColumnView` with `Block`, `Sector`, `Type`, `Status`, and `Data`.
  `Data` expands and shows all 16 bytes; unknown bytes are `??`.
- Selecting a row updates the inspector; the rail repeats the selection.
- Inspector: block number, sector, role, status, error text, hex entry, Read
  block, Write block, and the safety note.
- Empty state before a read: "No card loaded", reader + Read hint.
- Busy state: spinner, disabled reader menu and Read, disabled writes.

## Write flow

- The write button is disabled for block 0, incomplete blocks, unchanged
  bytes, a loaded dump, or no live card.
- Confirmation names the block, sector, old bytes, and new bytes; it states
  that a project backup and read-back happen.
- Sector trailers use the destructive appearance and the service's
  `confirm_trailer` gate.
- Success reports the backup path in the log and a toast; the row and
  inspector update from the written bytes.

## NDEF

- Read a card first.
- Show the MAD sectors, mapping version, read/write state, capacity, message
  bytes, Text/URI decoded records, and warnings.
- Unformatted/no-MAD cards are stated directly.
- No NDEF mutation control exists in this slice.

## Actions menu

- Refresh readers.
- Save dump… (project JSON).
- Load dump… (project JSON; inspection only, writes disabled).

## Copy and states

- English base copy in this slice; `zh-CN` is a completion requirement.
- Hover/focus/disabled/destructive/error states come from Adwaita; no custom
  colors.
- Errors appear in a toast and the session log; partial reads keep their
  warnings visible in the log and card summary.
- Escape dismisses the topmost popover or dialog.
