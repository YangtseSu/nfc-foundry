# Classic workspace screen contract

## Task

Read a MIFARE Classic 1K or 4K card, inspect its blocks, keep a key store, save
or load a dump, and restore or write blocks with backups and read-back
verification. The card is the object; the block table and inspector are the
primary work area.

## Layout

```text
window
  header       product title, selected reader, Refresh
  device row   one button per enumerated reader
  toolbar      Read card · Scan keys        path · Save dump · Load dump · Restore…
  tabs         Blocks · Diff (n) · Keys (n)
  content
    Blocks     block table                 block inspector
    Diff       summary + changed-block table
    Keys       add-key form + key table
  status       operation, result, error, or backup path
```

- The block table is the master view: block, sector, type (manufacturer, data,
  trailer), status, known-byte count, and data.
- The inspector is a fixed trailing pane. It shows hex, ASCII, value-block
  decoding, access conditions, a block editor, and the write action.
- The table scrolls independently; the inspector scrolls independently.
- The window has a usable minimum width; device buttons and toolbar actions
  wrap rather than clipping.

## States

- No reader: device row explains how to connect one; card actions stay
  disabled.
- No card image: Blocks tab shows the read/load empty state.
- Reading, writing, restoring, or scanning: actions are disabled and the
  status line names the operation.
- Partial dump: unknown blocks show `—` and their status; warnings name the
  skipped sectors.
- No diff: the Diff tab explains that two card images are required.
- Empty key store: the Keys tab shows the add/scan path.
- Errors stay in the status line and keep the previous card image visible.

## Safety

- Read card is the primary action. It never mutates the card.
- Block 0 is read-only. The write button is disabled for it.
- Data writes use the shared service: access check, backup, write, read-back,
  identity check.
- A sector trailer opens an AlertDialog that names the block and sector and
  warns about keys and access bits. The write runs only after `Write trailer`.
- Restore opens an AlertDialog with the image layout, UID when known, and the
  backup guarantee. The restore runs only after `Restore`.
- Restore uses the safe default: stop at the first failure. The GUI does not
  expose the CLI continue policy.
- The status line reports the backup path after a completed mutation.

## Interaction

- Tabs are keyboard reachable and use the component's selection state.
- Block rows use stable domain IDs (`classic-block-<number>`); selecting a row
  updates the inspector and the editor value.
- Key add uses a labelled key input, sector input, and A/B/Any radio group. The
  form commits on the Add key button.
- Destructive buttons use the danger variant and the confirmation dialog.
- Long values use the theme monospace family; numeric columns stay aligned.

## Tests

`cargo test -p nfc-classic-gui --test ui` covers read, block selection,
inspector state, diff, key add/remove, write, key scan, and restore
confirmation. A manual window check remains for keyboard traversal, theme and
zoom, focus restoration after dialogs, and real hardware latency.
