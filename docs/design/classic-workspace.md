# Classic screens contract

## Task

Read a MIFARE Classic 1K or 4K card, inspect blocks, manage keys, compare
images, write NDEF, and recover keys. The workbench shell owns the reader,
card identity, language, and status; these screens render the page only.

## Layout

```text
shell sidebar        Blocks · Classic NDEF · Diff · Keys · Recovery
top bar              domain switch · card identity · reader menu · language · Read card
page header          eyebrow · title · subtitle · screen actions
file row             dump path · format · Save · Load · Restore · continue policy
Blocks               metric strip · block table · fixed block inspector
Classic NDEF         message/capacity · mapping · write/export/format cards
Diff                 before/after files · summary · changed-block table
Keys                 key store table · scan controls · add/import inspector
Recovery             strategy grid · target · candidates · run/progress/result
status bar           operation · detail · cancel · last backup · version
```

- The block table is the master view. The inspector scrolls independently and
  keeps its read/write actions pinned at the bottom.
- Block 0 is present but read-only. Trailer writes use the danger variant.
- The file row is shared by Blocks and Classic NDEF.

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

- Block 0 writes are disabled.
- A sector trailer opens the typed confirmation dialog and names the block and
  sector.
- Restore opens the typed dialog with the image layout and UID; the safe stop
  policy is the default, and the CLI continue policy is exposed as a checkbox.
- Classic NDEF format uses the typed dialog and the pre-write backup.
- Recovery authenticates only, exposes cancel, and shows the verified key.
- The status bar shows the last backup path after a mutation.

## Tests

`crates/gui/nfc-foundry/tests/ui.rs` covers read, block selection, inspector
state, diff after a second read, key add, screen navigation, the trailer typed
dialog, the block 0 refusal, settings persistence, and language switching.
