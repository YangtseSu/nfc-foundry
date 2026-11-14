# Classic screens contract

## Task

Read a MIFARE Classic 1K or 4K card, inspect blocks, manage keys, compare
images, write NDEF, and recover keys. The workbench shell owns the reader,
card identity, language, and status; these screens render the page only.

## Layout

```text
shell header         NFC Foundry · workspace selector · Read card · ☰ menu
left rail            reader menu · Connect/Disconnect · status · capability profile
                     card identity/size/readability · key-store status · selected block
workspace tabs       Blocks · Details · NDEF · Keys · Recovery · Dumps
Dumps tabs           Diff · Restore
Blocks               sector-grouped block table · fixed block inspector
Classic NDEF         message/capacity · mapping · write/export/format cards
Keys                 Key store · Key scan
Recovery             strategy · advanced options · scrollable log · save keys
system page          Diagnostics · Activity · Settings
status bar           operation text · spinner · layout/card facts
```

- A presence scan detects the card family; the shell shows the Classic task
  group and routes Read card to the Classic workspace automatically.
- The block table is the master view. Rows are grouped under `Sector NN`
  headers that summarize block count, Key A/Key B presence, and unsaved edits;
  headers are not selectable. Block columns show `Data` hex with `??` for
  unknown bytes and `ASCII` with `.` for control or unknown bytes.
- The inspector scrolls independently and keeps its read/write actions pinned
  at the bottom; it repeats the selected block's ASCII on its own line.
- Block 0 is present but read-only. Trailer writes use the danger variant and
  name the target block; writes are disabled while the editor still matches
  the current bytes.
- Save dump and Load dump live on Blocks; Restore and the continue policy live
  under Dumps; Classic NDEF owns NDEF write/format and `.ndef` load/save.
- A Save with an image-only format also writes the `.nfcx.json` sidecar with
  the card identity, read times, and sector keys. Load and Restore apply a
  matching sidecar; a missing or mismatched sidecar never changes the image.
- The block inspector ends with a separate Advanced-mode UID section: a target
  UID entry and a destructive "Write UID…" action. It is disabled unless a live
  four-byte-UID Classic 1K image is loaded and Advanced mode is on.
- The current GTK shell is retained. The old persistent Activity panel, Tag
  health rail, and bottom action bar are not restored; Activity is a hidden
  system page and the status bar carries the current operation.

## States

- No card image: Blocks and Classic NDEF show the read/load empty state.
- Reading, writing, restoring, scanning, or recovering: the status bar names the
  operation and screen actions are disabled.
- Partial dump: unknown blocks show `—`; warnings are kept in the status
  detail.
- No diff: the Diff screen explains that two images are required.
- Empty key store: Keys shows the add/import path.
- Read-only NDEF mapping: write and format actions stay disabled.

## Safety

- Block 0 is read-only in the ordinary editor. The separate Advanced-mode UID
  action requires a live four-byte-UID Classic 1K image, a one-shot plan, a
  destructive confirmation dialog, a sector-0 backup, and post-write reselect
  and verification. An ordinary card refuses the write unchanged.
- A sector trailer opens the confirmation dialog and names the block and sector.
- Restore opens the plan dialog with the image layout and UID; the safe stop
  policy is the default, and the CLI continue policy is exposed as a checkbox.
- Classic NDEF format uses the confirmation dialog and the pre-write backup.
- Recovery authenticates only, exposes cancel, logs every phase and verified key in one scrollable pane, and keeps the save action in the footer.
- The status bar shows the last backup path after a mutation.

## Tests

The gated GTK scenario in `crates/gui/src/app.rs` covers read, sector grouping
for 1K and 4K images, block selection, inspector state, diff after a second
read, key add, screen navigation, the trailer confirmation dialog, the block 0
refusal, settings persistence, and language switching. Run it under Xvfb with
`NFC_FOUNDRY_GUI_TESTS=1`; see [testing](../testing.md).
