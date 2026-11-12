# Type 2 / NDEF screens contract

## Task

Read an NTAG/Ultralight image, inspect pages, write user memory, write NDEF, and
manage one-way protection. The workbench shell owns the reader, tag identity,
language, and status; these screens render the page only.

## Layout

```text
shell sidebar        reader · card · Memory · Protection · System
top bar              tag identity (product · UID · CC · NDEF) · reader menu · language · settings
page header          eyebrow · title · subtitle · Advanced mode
file row             project path · Save · Load · Restore · include options
Memory               summary · page table · fixed page inspector
Protection           configuration · lock form · password form
action bar           status · detail · last backup · Read card
```

- `Read card` detects the tag family and routes a Type 2 tag to the Type 2 tab.
- The page table is the master view; the inspector holds the write action and
  feedback. User memory is visible by default. CC, static/dynamic lock,
  configuration, and PWD/PACK pages appear only in Advanced mode.
- Manufacturer and reserved pages are read-only through the normal page
  workflow. A separate Advanced UID path handles a confirmed 7-byte
  UID-writable NTAG213 clone. Locked and password-protected pages explain why
  the write is refused.
- Restore include options appear after a project image is loaded and only on
  Memory. User memory is on; configuration, CC, and lock bytes are opt-in.

## States

- No tag: Memory and Protection show the read/load empty state.
- Partial read: unreadable pages show `—`; warnings stay visible.
- Read-only capability container: NDEF write and format are disabled.
- NDEF tab: the shared workbench editor lists the decoded records, capacity,
  preflight, raw TLV, and the selected-record inspector. A card read reloads
  the editor; a dirty draft survives incidental refreshes. Write and format
  use the same gates as the CLI, including locked/password-protected pages.
- Locked page: the inspector explains that the page cannot be written.
- PWD/PACK pages never read back; the inspector says so.
- Advanced actions: the dialog names the card identity, pages, old/new values,
  backup, and read-back plan, and requires an explicit destructive confirmation.

## Safety

- User memory is the default write scope.
- Lock, configuration, password, restore, Advanced page writes, and Type 2 NDEF
  format require Advanced mode and the destructive confirmation dialog.
- Every mutation passes the session `CardInfo` as the expected identity; a card
  change is refused before the backup or card I/O.
- Advanced mode resets on card read, reader change, refresh, and loaded dump.
- NDEF write preserves unknown/proprietary TLVs. Format removes them and is
  destructive.
- The service owns backups, read-back verification, CC-write validation, and
  identity checks. The GUI reports the backup path.

## Tests

`crates/gui/src/app.rs` contains the gated `gtk_workbench_scenarios` entry
point. `crates/gui/src/type2.rs` unit tests cover page rows, roles, lock
previews, parsing, and NDEF gates. Run the GTK scenario under Xvfb with:

```sh
env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 \
  xvfb-run -a env GDK_BACKEND=x11 \
  cargo test -p nfc-foundry gtk_workbench_scenarios
```
