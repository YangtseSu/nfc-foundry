# Design references

The GUI is a GPUI Kit desktop application. This directory records screen
contracts and design decisions; it does not duplicate the GPUI Kit Design
Guides.

## Shell

- One window, persistent left navigation, workspace content, right inspector.
- Top bar: product, reader/card state, language, diagnostics.
- Bottom status: operation, progress, cancel, errors.
- Two domain entries: **Classic** and **NDEF**.

## Classic

The implemented screen contract is [classic-workspace.md](classic-workspace.md).

- Blocks table with sector, type, status, known-byte count, and data.
- Inspector for block bytes, ASCII, value block, access conditions, and BCC.
- Keys: persisted key store with sector, type, and provenance.
- Diff: before/after table with changed/unknown blocks and trailer detail.
- Restore and trailer writes are confirmed; block 0 remains read-only in this
  phase. Clone/factory/UID actions are later Advanced work.

## Type 2 / NDEF

The implemented screen contract is
[type2-workspace.md](type2-workspace.md).

- Type 2 memory table with page role, lock state, and data.
- Page inspector with hex, ASCII, and a verified user-page editor.
- NDEF tab: state, capacity, records, Text write, and Format.
- Protection: static/dynamic locks, AUTH0/ACCESS/CFGLCK/AUTHLIM, lock form,
  and password form.
- Type 2 Advanced writes, locks, password configuration, and NDEF format
  require confirmation; lock/config/password use typed confirmation.
- Classic NDEF: MAD/CC/sector mapping remains phase 10.

## Rules

Follow the GPUI Kit Design Guides:

- Desktop conventions, keyboard access, persistent navigation, resizable panes.
- `Button` for commands; no link-styled internal actions.
- Theme tokens; no raw colors or spacing literals in application code.
- Visible focus, hover, selected, disabled, loading, validation, danger.
- `AlertDialog` for consequential actions; Escape dismisses topmost overlay.
- Bounded 1K/4K block tables use the small Table with stable domain IDs; move
  to a virtualized table when a view can grow beyond a card image.
- Interface copy is English in this phase. The application-wide localization
  pass is a release requirement.
