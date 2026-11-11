# CLI and GUI parity

Status: phase 19e audit. Both surfaces call the same `nfc-app` services. A row
is complete only when the GUI path has the same semantics and confirmation
gate as `nfctl`.

Legend: `R` read-only, `C` confirmation dialog (CLI `--yes`), `T` typed phrase
(`I_OWN_THIS_CARD`), `A` Advanced mode required, `B` backup plus read-back.
Hardware status is the release claim from
[hardware-matrix.md](../hardware-matrix.md); `n/r` means not recorded.

## Readers and cards

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Enumerate readers | `nfctl list` | rail reader list; Actions → Refresh readers; Diagnostics | none | PN532, ACR122U |
| Select and read card | `nfctl scan` | header → Read card | R | PN532, ACR122U |
| Card information | `nfctl card info` | rail card facts; Blocks inspector; Type 2 summary | R | PN532, ACR122U |
| Watch for card changes | `nfctl scan --watch` | — | R | n/r |

`scan --watch` is a presentation loop that repeats the same read; it is not a
distinct card or file operation. The GUI shows the current card and invalidates
the image when the reader changes. No CLI-only card capability is hidden there.

## Classic

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Read block | `classic read --block` | Blocks → Read block | R | PN532, ACR122U |
| Dump | `classic dump --output --format` | Actions → Save dump… (format dialog) | R | PN532; ACR n/r |
| Write block | `classic write --block --data` | Blocks inspector → Write block | C, B | PN532; ACR n/r |
| Write sector trailer | `classic write --confirm-trailer` | Write block with the trailer button when the block is a trailer | C, B | n/r |
| Restore | `classic restore --input [--continue-on-error]` | Restore → Load dump… → Restore to card | C, B | PN532; ACR n/r |
| Key scan | `classic keyscan [--save] [--types]` | Key scan → Start scan → Save discovered keys | R | PN532; ACR n/r |
| Key list | `classic keys list` | Keys table | R | file only |
| Key import | `classic keys import` | Keys → Import .dic / JSON… | explicit | file only |
| Key add | `classic keys add` | Keys → Add key | explicit | file only |
| Key export `.dic` | `classic keys export --format dic` | Keys → Export .dic… | explicit | file only |
| Key export JSON | `classic keys export --format json` | Keys → Save store | explicit | file only |
| Dump diff | `classic diff BEFORE AFTER` | Diff → Load before/after → Compare | R | file only |
| Classic NDEF facts | `classic ndef info` | NDEF page facts for the loaded Classic image | R | PN532 |
| Classic NDEF read | `classic ndef read [--output]` | NDEF page records; Export message… writes the raw message | R | PN532 |
| Classic NDEF write | `classic ndef write` | NDEF composer → Write NDEF | C, B | PN532; ACR n/r |
| Classic NDEF format | `classic ndef format` | NDEF → Format NDEF area… | C, B | PN532; ACR n/r |

Extra service gates on both surfaces: a complete dump, a complete and writable
NFC mapping, no read-only GPB, capacity, and a message different from the
stored TLV. Block 0 is never writable. The GUI exports the same encoded NDEF
message bytes that `nfctl classic ndef read --output` writes.

## Type 2 / NTAG

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Facts | `type2 info` | Type 2 summary, configuration, NDEF facts | R | PN532, ACR122U |
| Read pages | `type2 read [--page]` | Type 2 table and page inspector | R | PN532, ACR122U |
| Write user page | `type2 write --page --data` | Page inspector → Write page… | C, B | PN532; ACR n/r |
| Write Advanced page | `type2 write --advanced` | Advanced mode → page inspector → Write page… | A, T, B | n/r |
| Dump | `type2 dump --output` | Memory → Save dump… | R | n/r |
| Restore | `type2 restore --input --include-*` | Memory → Choose project image… → Restore image… | T, B | n/r |
| Lock pages | `type2 lock --pages` | Protection → One-way locks → Apply lock bits… | A, T, B | n/r |
| Password configuration | `type2 password` | Protection → Password and access → Apply password configuration… | A, T, B | n/r |
| NDEF read | `ndef read [--output]` | Type 2 NDEF facts; Export message… | R | PN532 |
| NDEF write | `ndef write` | NDEF composer → Write NDEF | C, B | PN532; ACR n/r |
| NDEF format | `ndef format` | NDEF → Format NDEF area… | T, B | PN532; ACR n/r |

Both surfaces refuse a different card identity, a mismatched capability
container, an incomplete NDEF area, a locked data page, a password-protected
data page, a read-only CC, and a capacity overflow. Restore class selection is
the same four flags/checkboxes in both surfaces.

## Recovery and system

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Auto recovery | `recover auto` | Recovery → Auto → Start recovery | R | all-sector autopwn on PN532, ACR122U |
| Darkside | `recover darkside` | Recovery → Darkside → Start recovery | R | host-driven MFCUK port; needs a repeating tag nonce |
| Nested | `recover nested` | Recovery → Nested → Start recovery | R | PN532, ACR122U |
| Static nested | `recover staticnested` | Recovery → Static nested | R | PN532, ACR122U |
| Hardnested | `recover hardnested` | Recovery → Hardnested | R | PN532, ACR122U |
| FM11RF08S profile | `recover fm11rf08s` | Recovery → FM11RF08S | R | detection only |
| Save recovered key | `--key-store` write by the operation | Recovery → Save verified key | explicit | file only |
| Self-check | `nfctl diag` | Diagnostics | R (no card I/O) | no card path |

Recovery authenticates only; it never writes the card. A recovered key is
displayed only after the service verifies it. Saving to the key store is a
separate action in both surfaces.

## Deliberate surface differences

- `--mock`, output formatting, `--device`/`--key`/`--key-file` selection, and
  stdout versus file dialogs are host or presentation differences and do not
  change card semantics.
- `nfctl` stays English under every locale. The GUI is English-base with a
  complete `zh-CN` catalog and an immediate language switch.
- Type 4 NDEF has no CLI command and no GUI page. It stays unadvertised until
  a hardware record exists; a read surface will require the matching `nfctl`
  read command in the same change.

## Audit result

No shipped `nfctl` card or file operation is missing a GUI path. The audit
added the GUI NDEF message export, which was the only missing file operation.
`scan --watch` remains a CLI presentation loop over the shared `scan` read.
