# Type 2 / NDEF workspace screen contract

## Task

Read an NFC Forum Type 2 / NTAG tag, inspect its pages, write user memory,
edit NDEF, save or restore a project image, and manage one-way locks and NTAG
password configuration. The card memory is the object; NDEF and protection are
separate tabs over the same image.

## Layout

```text
shell
  domain tabs   MIFARE Classic · Type 2 / NDEF
  workspace
    header       product title, selected reader, Refresh
    device row   one button per enumerated reader
    toolbar      Read tag · Save dump · Load dump · Restore…   path · loaded
    tabs         Memory · NDEF · Protection
    content
      Memory      page table                     page inspector
      NDEF        state/capacity, records       Text write · Format…
      Protection  lock/config summary           lock form · password form
    status       operation, result, error, or backup path
```

- The page table is the master view: page, role, lock state, and data.
- The inspector is a fixed trailing pane with hex, ASCII, the page editor, and
  the write action.
- NDEF and Protection use full-width forms because they are not row-indexed.
- Device buttons and toolbar actions wrap rather than clip.

## States

- No reader: the device row explains how to connect one; actions stay
  disabled.
- No tag image: Memory shows the read empty state.
- Reading, writing, formatting, locking, or restoring: actions are disabled
  and the status line names the operation.
- Partial read: unknown pages show `—`; warnings name the unreadable quads.
- Unformatted/empty NDEF: the NDEF tab shows the state and offers Write or
  Format.
- CC read-only: Write NDEF and Format are disabled.
- No loaded dump: Restore is disabled.
- Errors stay in the status line and keep the previous image visible.

## Safety

- Read tag is the primary action and never mutates the tag.
- Manufacturer pages cannot be written. The write button is disabled for them.
- User-memory writes go through the shared service: backup, write, read-back,
  identity check.
- Lock, configuration, password, and capability-container writes are Advanced.
  The write button opens a dialog that requires typing `I_OWN_THIS_CARD`.
- Lock and password actions always open a typed-confirmation dialog, even when
  the form is complete. Cancel leaves the tag unchanged.
- NDEF format opens a danger dialog; the service writes a backup before the
  first change.
- Restore opens a danger dialog and defaults to user memory only. PWD/PACK are
  never restored.
- The status line reports the backup path after a mutation.
- Locked pages show `locked` and cannot be written from the inspector.

## Interaction

- Tabs are keyboard reachable and use the component selection state.
- Page rows use stable domain IDs (`type2-page-<number>`).
- Selecting a page fills the hex editor with that page.
- NDEF write uses the Text record with a language field, defaulting to `en`.
- Protection shows static/dynamic lock bytes, the locked-page list, and
  AUTH0/ACCESS/CFGLCK/AUTHLIM.
- Long values use the theme monospace family.

## Tests

`cargo test -p nfc-type2-gui --test ui` covers read and page selection, NDEF
write and format, Advanced write confirmation, lock typed confirmation, and
protection controls. A manual window check remains for keyboard traversal,
theme and zoom, focus restoration after dialogs, and real hardware latency.
