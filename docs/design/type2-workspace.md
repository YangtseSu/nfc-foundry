# Type 2 / NDEF screens contract

## Task

Read an NTAG/Ultralight image, inspect pages, write NDEF, and manage one-way
protection. The workbench shell owns the reader, tag identity, language, and
status; these screens render the page only.

## Layout

```text
shell sidebar        Memory · NDEF · Protection
top bar              domain switch · tag identity · reader menu · language · Read tag
page header          eyebrow · title · subtitle
file row             dump path · Save · Load · Restore · include options
Memory               metric strip · page table · fixed page inspector
NDEF                 message/capacity · capability container · write/export/format
Protection           lock state · lock form · configuration · password · action plan
status bar           operation · detail · cancel · last backup · version
```

- The page table is the master view; the inspector keeps its write actions
  pinned at the bottom.
- Locked pages, manufacturer pages, and reserved pages cannot be written.
  Configuration, password, and lock pages use the Advanced label.

## States

- No tag: Memory and NDEF show the read/load empty state.
- Partial read: unreadable pages show `—`; warnings stay in the status detail.
- Read-only capability container: NDEF write and format are disabled.
- Locked page: the inspector explains that the page cannot be written.
- PWD/PACK pages never read back; the inspector says so.
- Advanced actions: the inspector and the confirmation dialog name the pages,
  old/new values, backup, and read-back plan.

## Safety

- User memory is the default write scope.
- Lock, configuration, and password actions require the typed dialog
  `I_OWN_THIS_CARD`.
- Restore uses the typed dialog and exposes configuration, capability
  container, and lock-byte include options.
- NDEF format uses the typed dialog. Every mutation keeps the service-level
  backup, read-back verification, and identity check.

## Tests

`crates/gui/nfc-foundry/tests/ui.rs` covers read, page selection, inspector
state, the Advanced page typed dialog, NDEF/Protection screen rendering, and
the shared language switch.
