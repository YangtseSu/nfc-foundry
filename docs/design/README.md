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

- Blocks table with sector headers, readability, dirty state, and trailer
  markers.
- Inspector for block bytes, value block, access conditions, BCC.
- Keys: catalog, sets, verification status.
- Recovery: card profile, attack plan, progress, results, review/save.
- Diff: before/after table with changed/unknown/trailer filters.
- Clone/factory/UID/trailer actions are Advanced and typed-confirmed.

## NDEF

- Type 2 memory table with CC, user pages, lock/config pages.
- NDEF message editor: templates, raw, records, exact byte preview.
- Classic NDEF: MAD/CC/sector mapping.
- Protection: lock/config/PWD/PACK/AUTH0/ACCESS are Advanced.
- Destructive actions show target, old/new, backup, read-back plan.

## Rules

Follow the GPUI Kit Design Guides:

- Desktop conventions, keyboard access, persistent navigation, resizable panes.
- `Button` for commands; no link-styled internal actions.
- Theme tokens; no raw colors or spacing literals in application code.
- Visible focus, hover, selected, disabled, loading, validation, danger.
- `AlertDialog` for consequential actions; Escape dismisses topmost overlay.
- Data-heavy views use `DataTable`/`VirtualList` with stable domain IDs.
- English source copy and complete Simplified Chinese translation.
