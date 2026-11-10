# Design references

The GUI is a GPUI Kit desktop application. This directory records screen
contracts and design decisions; it does not duplicate the GPUI Kit Design
Guides.

## Shell

- One window, persistent left rail, page content, fixed inspector actions.
- The rail keeps reader state, the detected card identity, the task group for
  the detected card family, and the System group. A card-family change replaces
  the task group instead of asking the user to pick a domain. Without a card,
  both task groups stay available so a saved dump can be loaded.
- Top bar: card identity (type, family, UID, detail), reader menu, language
  switch, Settings. There is no separate domain switch or Read button.
- Bottom action bar: status, detail, last backup, Cancel during recovery,
  Scan keys for Classic, and the family-aware Read action. The Read action
  stays in the same place on every screen.
- Presence is scanned in the background. A detected Classic or Type 2 card
  selects the matching task group and routes Read; no card keeps the last
  loaded image available for inspection.
- Reader menu: an overlay above page content with a capped, scrollable list;
  outside clicks dismiss it. USB serial adapters come before platform UARTs.
- Window chrome: client-side decorations with an in-app title bar (brand,
  drag, double-click maximize, minimize/maximize/close). Server-decorated
  windows keep the compositor title bar instead of drawing a duplicate.
- Minimum window size 960x640. Page headers, tables, inspectors, and the reader
  menu flex or scroll within the window instead of overflowing it.
- English is the base language; a complete `zh-CN` catalog is available from
  the in-app switch. The CLI is English-only.

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
- Classic NDEF: MAD/CC/sector mapping from phase 10 is presented as its own
  screen.
- Recovery, Diagnostics, Activity, and Settings are first-class shell screens.
- Icon and desktop entry are part of the installed application identity.

## Prototypes

- [03-workbench.html](03-workbench.html) is the current expanded workbench
  prototype. It covers the reader/card shell, Classic blocks, Classic NDEF,
  diff, keys, native recovery, Type 2 memory, NDEF, protection, diagnostics,
  activity, and typed confirmation flows. It is a design sample, not a second
  implementation. The sample keeps English copy and shows the language-switch
  placement. Phase 18 added the complete `zh-CN` catalog and the persistent
  workbench shell.
- [02-workbench-ndef.html](02-workbench-ndef.html) is the earlier NDEF-only
  reference. It is retained for comparison and does not cover the expanded
  feature set.

The prototype is a new information architecture, not a restyle of the earlier
file. It replaces per-workspace toolbars with one persistent command shell:

- left navigation groups Classic, Type 2, and system surfaces instead of hiding
  features behind a single workspace tab bar;
- reader and card state stay in the shell, while the inspector actions remain
  fixed at the bottom of the pane;
- recovery, diagnostics, and session activity are first-class surfaces;
- destructive and Advanced actions open a confirmation surface that names the
  target, old/new state, backup, and read-back plan;
- the application identity is part of phase 18: a vector icon, PNG exports, and
  a desktop entry with English and `zh_CN` metadata.

## Rules

Follow the GPUI Kit Design Guides:

- Desktop conventions, keyboard access, persistent navigation, resizable panes.
- `Button` for commands; no link-styled internal actions.
- Theme tokens; no raw colors or spacing literals in application code.
- Visible focus, hover, selected, disabled, loading, validation, danger.
- `AlertDialog` for consequential actions; Escape dismisses topmost overlay.
- Bounded 1K/4K block tables use the small Table with stable domain IDs; move
  to a virtualized table when a view can grow beyond a card image.
- Interface copy is complete in English and Simplified Chinese. Protocol
  names, hex, paths, and counts stay unlocalized.
