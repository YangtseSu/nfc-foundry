# Note: phase 19 GTK4 workbench

## Implemented

- `crates/gui` (`nfc-foundry`) added to the workspace. It uses GTK4-rs and
  libadwaita only; GPUI Kit is not a dependency and the standalone GPUI spike
  was deleted.
- Shell: persistent rail, reader menu, card summary, key status, selected
  block, session log, Card/NDEF task switcher, header `Actions` and `Read
  card`, status bar.
- Classic slice:
  - reader enumeration/selection and `classic_read` through `nfc-app`;
  - real dump table (1K and 4K geometry), selection, inspector, per-block
    read, and verified block write;
  - old/new confirmation, trailer-specific destructive confirmation, backup
    path, read-back through the service, block 0 and unchanged-write guards;
  - save/load project dumps;
  - read-only Classic NDEF mapping/message view;
  - reader changes invalidate the live dump's write permission.
- Worker-thread I/O with `async-channel`; GTK is updated on the main context.
- `classic.rs` unit tests cover 1K/4K row counts, unknown-byte rendering, hex
  parsing, and block kind labels.

## Verification

- `cargo test -p nfc-foundry`: presentation tests passed at this slice; the
  Keys, Diff, Restore, dump-format, and Xvfb UI work landed in phase 19a.
- `cargo clippy -p nfc-foundry --all-targets -- -D warnings`: clean.
- Real hardware on 2026-11-11 through PN532 UART
  `pn532_uart:/dev/ttyUSB0`:
  - `nfctl scan` reported MIFARE Classic 1K, UID `FA B9 CF F4`, ATQA
    `00 04`, SAK `08`.
  - The GUI read 60/64 blocks; sector 5 had no candidate key and was shown as
    a warning in the log. The table showed all 64 blocks.
  - Selecting block 004 updated the inspector and selected rail.
  - Block 0's write action was disabled; clicking it opened no dialog.
  - Changing a trailer byte opened the destructive `Write trailer` dialog with
    old/new bytes and the backup/read-back note. Escape canceled it; no write
    was performed.
  - The NDEF tab showed "Not NDEF formatted" and "No MAD or NFC sector
    mapping" for the same card.
  - The Actions menu opened with Refresh readers, Save dump, and Load dump.
  - After Refresh readers, the card summary changed to "Inspection only · read
    again to write"; the hex editor, Read block, and Write block were disabled.
- The app was rendered on a private `Xvfb :99` display. It needed
  `GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none NO_AT_BRIDGE=1`; with the
  default Vulkan/a11y path it initialized but did not map a window on this
  headless display. This is a headless-environment workaround, not a product
  requirement.

## Problems and fixes

- `GtkComboBoxText` is deprecated since GTK 4.10. Replaced with
  `GtkDropDown` and `GtkStringList`.
- The block table was packed at its natural width and clipped the hex column.
  Replaced the horizontal box with `GtkPaned`, made the data column expand,
  and sized the remaining columns so all 16 bytes fit.
- Reader selection could leave an old dump writable. A reader change now
  clears the live flag and disables writes until a fresh read.
- The GTK file-dialog API required the `v4_10` crate feature.

## Remaining

- Recovery and Advanced UID/factory screens.
- NDEF composer/write/format over the shared document path; Type 2 and Type 4
  screens.
- `zh-CN` catalog and in-app language switch.
- Packaging: GTK dependencies, GUI binary, icon, desktop entry, and release
  tarball. The GUI is not shipped until feature complete.
- Windows/macOS assessment is deferred until after Linux feature parity.

Keys, diff, restore, dump formats, the completed inspector, the write identity
guard, and mock/Xvfb GTK UI tests are implemented in phase 19a; see
[19a-classic-pm3.md](19a-classic-pm3.md).
