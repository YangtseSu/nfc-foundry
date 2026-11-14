# CLI and GUI parity

Status: phase 19e audit. Both surfaces call the same `nfc-app` services. A row
is complete only when the GUI path has the same semantics and confirmation
gate as `nfctl`.

Legend: `R` read-only, `C` confirmation dialog (CLI `--yes`), `A` Advanced
mode required, `B` backup plus read-back, `M` manifest export, `W` write plus
read-back.
Hardware status is the release claim from
[hardware-matrix.md](../hardware-matrix.md); `n/r` means not recorded.

## Readers and cards

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Enumerate readers | `nfctl list` | rail reader list with connection state and capabilities; ☰ → Refresh readers; ☰ → Diagnostics | none | PN532, ACR122U |
| Select and read card | `nfctl scan` | header → Read card | R | PN532, ACR122U |
| Card information | `nfctl card info` | rail card facts; Blocks inspector; Type 2 summary | R | PN532, ACR122U |
| Card details and health | `nfctl card health --input` | Classic → Details tab; source, completeness, validation, key coverage, dirty blocks | R | file only |
| MIFARE Mini identity | `card info` reports `MIFARE Mini`; every Classic read/write refuses it | rail card line and Details report `MIFARE Mini`; every Classic write is unavailable | identity only | n/r |
| Watch for card changes | `nfctl scan --watch` | rail reader monitor; removal, reader change, and reader failure invalidate the live image | R | n/r |
| Reader reconnect | automatic poll and backoff; `scan --watch` reports recovery | automatic poll and backoff; rail status shows recovering or error | R | n/r |

`scan --watch` and the GUI reader rail consume the same `nfc-app` monitor:
400 ms polls, two removal misses, card appeared/removed events, and reconnect
with exponential backoff. Normal commands and monitor polls share one exclusive
operation lease, so a poll never touches the reader during an operation. The
GUI shows the current card and invalidates the image when the card is removed,
a different card appears, or the reader fails. No CLI-only card capability is
hidden there.

## Classic

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Read block | `classic read --block` | Blocks → Read block | R | PN532, ACR122U |
| Dump | `classic dump --output --format` | MIFARE Classic → Dumps → Save dump… (format dialog) | R | PN532, ACR |
| Verified key capture | read, `classic keyscan`, `recover` record authenticated keys | read, Key scan, Recovery record authenticated keys | R | file only |
| Write block | `classic write --block --data` | Blocks inspector → Stage edit → Write changes… | C, B | PN532, ACR |
| Write sector trailer | `classic write --confirm-trailer` | Stage the trailer edit, then Write changes… | C, B | PN532, ACR |
| Write preflight | `classic write --dry-run` / `classic restore --dry-run` | Plan dialog before Write changes… / Restore to card | R | PN532, ACR |
| Change UID / manufacturer block 0 | `classic uid --uid --advanced [--dry-run]` | Blocks inspector → Advanced mode → Write UID… → target UID → plan dialog | A, B, C | PN532, ACR |
| Edit working image | — (staged by the GUI; `--dry-run` previews the plan) | Stage edit, Undo edit, Revert edits; dirty rows show `edited` | none | file only |
| Restore | `classic restore --input [--blocks] [--changes-only] [--continue-on-error] [--dry-run]` | MIFARE Classic → Dumps → Restore → Load dump… → mode → Restore to card | C, B | PN532, ACR |
| Restore with UID | `classic restore --input --uid --advanced [--dry-run]` | Restore → target UID + Advanced → Restore to card → plan dialog | A, B, C | PN532, ACR |
| Key scan | `classic keyscan [--save] [--types]` | MIFARE Classic → Keys → Key scan → Start scan; verified keys are captured automatically, Save remains available | R | PN532, ACR |
| Key list | `classic keys list [--builtin]` | Keys table shows stored entries and built-in common keys (version-2 store with card-scoped verifications) | R | file only |
| Key delete | `classic keys delete HEX --sector --key-type` | Keys → select a stored row → Delete selected | explicit | file only |
| Key import | `classic keys import` | Keys → Import .dic / JSON… | explicit | file only |
| Key add | `classic keys add` | Keys → Add key | explicit | file only |
| Key export `.dic` | `classic keys export --format dic` | Keys → Export .dic… | explicit | file only |
| Key export JSON | `classic keys export --format json` | Keys → Save store | explicit | file only |
| Dump diff | `classic diff BEFORE AFTER` | MIFARE Classic → Dumps → Diff → Load before/after → Compare | R | file only |
| Classic NDEF facts | `classic ndef info` | NDEF page facts for the loaded Classic image | R | PN532, ACR |
| Classic NDEF read | `classic ndef read [--output]` | NDEF page records; Export message… writes the raw message | R | PN532, ACR |
| Classic NDEF write | `classic ndef write` | NDEF composer → Write NDEF | C, B | PN532, ACR |
| Classic NDEF format | `classic ndef format` | NDEF → Format NDEF area… | C, B | PN532, ACR |

Extra service gates on both surfaces: a complete dump, a complete and writable
NFC mapping, no read-only GPB, capacity, and a message different from the
stored TLV. Block 0 is never writable. The GUI exports the same encoded NDEF
message bytes that `nfctl classic ndef read --output` writes.

## Type 2 / NTAG

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Facts | `type2 info` | Type 2 summary, configuration, NDEF facts | R | PN532, ACR122U |
| Read pages | `type2 read [--page] [--type2-password --type2-pack]` | Type 2 table and page inspector; Protected access fields | R | PN532, ACR122U protected n/r |
| Write user page | `type2 write --page --data [--type2-password --type2-pack]` | Page inspector → Write page… with Protected access credentials | C, B | PN532; ACR n/r |
| Write Advanced page | `type2 write --advanced` | Advanced mode → page inspector → Write page… | A, B | n/r |
| Dump | `type2 dump --output` | Memory → Save dump… | R | n/r |
| Restore | `type2 restore --input --include-*` | NFC Forum Type 2 → Dumps → Choose project image… → Restore image… | C, B | n/r |
| Lock pages | `type2 lock --pages` | Protection → One-way locks → Apply lock bits… | A, C, B | n/r |
| Password configuration | `type2 password [--current-pwd --current-pack]` | Protection → Password and access → Apply password configuration… | A, C, B | PN532, NTAG216 |
| Clear protection | `type2 password --auth0 255 --access 00 [--current-pwd --current-pack]` | Protection → Password and access with AUTH0 255 and ACCESS 00 | A, C, B | PN532, NTAG216 |
| Authenticated read/write after AUTH0 | `--type2-password --type2-pack` on Type 2 and NDEF commands | Protected access fields | R, C, B | PN532, NTAG216; ACR n/r |
| UID change (magic clone) | `type2 uid --advanced --uid ... [--dry-run]` | Protection → UID (magic clone) → Change UID… | A, C, B | PN532, 7-byte NTAG213 clone; ACR n/r |
| NDEF read | `ndef read [--output]` | Type 2 NDEF facts; Export message… | R | PN532 |
| NDEF write | `ndef write` | NDEF composer → Write NDEF | C, B | PN532; ACR n/r |
| NDEF format | `ndef format` | NDEF → Format NDEF area… | C, B | PN532; ACR n/r |

Both surfaces refuse a different card identity, a mismatched capability
container, an incomplete NDEF area, a locked data page, a read-only CC, and a
capacity overflow. A page or NDEF area protected by AUTH0 uses the supplied
PWD_AUTH credentials; without them the operation is denied before mutation.
The credentials are per operation, are never saved, and are cleared from the
GUI entries after a successful operation. Restore class selection is the same
four flags/checkboxes in both surfaces. Type 2 UID writes are limited to a
7-byte NTAG213 clone; both surfaces back up pages 0/1/2 first and restore the
original UID if verification fails.

## DESFire

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Inventory | `desfire info` | DESFire → Inventory | R | PN532 and ACR122U |
| Read file | `desfire read --aid --file [--key] [--key-type] [--offset] [--length] [--start-record] [--record-count]` | DESFire → Read with the same fields and key-type dropdown | R | PN532 and ACR122U: data, value, linear and cyclic records with legacy DES, 3K3DES/AES EV1, and AES EV2 |
| Write file | `desfire write --aid --file --data|--data-file [--offset] [--key] [--key-type] [--key-number]` | DESFire → Write with a hex/text payload; the backup directory comes from preferences | W, B | PN532 and ACR122U: 3K3DES EV1, AES EV1, and AES EV2 writes with a planned JSON backup path |
| Explicit authentication family | `--key-type des|2k3des|3k3des|aes|ev2` on read/verify/uid/format/delete | Key type dropdown on Read, Verify, and NDEF tabs | explicit key, R for authentication | PN532 and ACR122U: 3K3DES, AES EV1, AES EV2 |
| Verify key | `desfire verify-key --aid --key [--key-type] [--key-number]` | DESFire → Verify → Verify key | R | ACR122U: DES, 3K3DES, AES EV1, AES EV2; key version and session reported |
| Read authenticated UID | `desfire uid --aid --key [--key-type] [--key-number]` | DESFire → Verify → Read UID | R | ACR122U: EV1 CRC32, EV2 CMAC |
| Read NDEF | `desfire ndef-read [--output]` | DESFire → NDEF → Read NDEF | R | PN532 and ACR122U: CC, file facts, records, extra TLVs |
| Format NDEF | `desfire format-ndef --key --advanced [--dry-run]` | DESFire → NDEF → Advanced mode → Format NDEF application… → type `FORMAT` | A, C | PN532 and ACR122U |
| Replace NDEF application | `desfire replace-ndef --key --advanced [--dry-run]` | DESFire → NDEF → Advanced mode → Replace NDEF application… → type `REPLACE` | A, C | PN532 and ACR122U: previous message read, empty NDEF read back |
| Delete NDEF application | `desfire delete-ndef --key --advanced` | DESFire → NDEF → Advanced mode → Delete NDEF application… → type `DELETE` | A, C | PN532 and ACR122U |
| Format PICC | `desfire format-picc --key --advanced [--dry-run]` | DESFire → Manage → Advanced mode → Format PICC (erase all)… → type `ERASE` | A, C | PN532 and ACR122U: scratch application erased, 8192 bytes free, factory key check |
| Free memory | `desfire memory [--aid] [--key]` | DESFire → Manage → Read free memory | R | ACR122U: 8192 bytes before setup, 7648 after deletes |
| Create application | `desfire app-create --aid [--key] --keys --app-key-type` | DESFire → Manage → Create application | W | ACR122U: AES `F0 0D 08`, two keys |
| Delete application | `desfire app-delete --aid [--key] --advanced --yes [--manifest]` | DESFire → Manage → Advanced mode → Delete application… → files, key, manifest path, read-back → type `DELETE` | A, C, M | PN532 and ACR122U 2026-11-14: reserved application removed, planned manifest path shown |
| Create file | `desfire file-create --aid --file --type …` | DESFire → Manage → Create file with the same type fields | W | ACR122U: Standard, Backup, Value, Linear Record, Cyclic Record, Transaction MAC |
| Delete file | `desfire file-delete --aid --file [--key] --advanced --yes [--manifest]` | DESFire → Manage → Advanced mode → Delete file… → current settings, key, manifest path, read-back → type `DELETE` | A, C, M | PN532 and ACR122U 2026-11-14: all six files removed with planned manifests |
| Change file settings | `desfire file-settings --aid --file [--communication] [rights] [--key] --advanced [--dry-run]` | DESFire → Manage → Advanced mode → Change file settings… → current/requested settings and key → type `CHANGE` | A, C | PN532 and ACR122U 2026-11-14: Standard file changed to enciphered with new access rights |
| Value change | `desfire value --aid --file --credit|--debit|--limited-credit N [--key] [--tmac-key]` | DESFire → Values & records → Value credit/debit/limited credit | W | PN532 and ACR122U: credit 20→25, debit 25→22, limited credit 22→25, committed values verified |
| Record change | `desfire record --aid --file --append|--update|--clear [--data] [--record] [--offset] [--key] [--tmac-key]` | DESFire → Values & records → Record append/update/clear | W | PN532 and ACR122U: append, update, and clear read back record by record |

Both surfaces call the same `nfc-app` service and render inventory through
`inventory_text`. A data write plans its JSON backup path and aborts if the
backup cannot be written; an application or file delete plans its manifest
path and shows it in the confirmation dialog or CLI output before the command.
Every DESFire mutation appends one Activity row with the operation, target,
result, and backup/manifest path; key material is never logged. Value and
record changes use the same preflight, commit, and read-back checks, and a
Transaction MAC is labelled verified only after the supplied
AppTransactionMACKey reproduces the committed value; without the key it is
labelled UNVERIFIED. Each file shows the decoded type, communication mode,
access rights, capacity or limits, and raw settings; a response that does not
parse is marked `undecoded` with the raw bytes, never guessed. Reads report
the effective communication mode, the session key family, and the response
check that passed. Verify key sends no file command; the authenticated UID is
shown only after the EV1 CRC32 or EV2 CMAC check passes, and a legacy DES
session is labelled as having no response MAC. The format path
refuses an existing NDEF application, creates only the AN11004 `D27600`
application and its `E103`/`E104` files, and reads the CC and empty NDEF file
back through the Type 4 adapter. Read NDEF lists the CC, file facts, decoded
records, and proprietary/unknown TLVs without needing the PICC key or Advanced
mode. Delete validates the exact AN11004 layout
before removing it. Keys are session-scoped and never printed. The GUI
Advanced switch resets on card removal, reader change, and restart. The PN532
UART format/write/delete, Standard/Backup decode, value/record reads, and
3K3DES/AES authentication records passed on 2026-11-12. EV1 and EV2
secure-messaging reads pass on PN532 UART and ACR122U. Key verification and
authenticated UID reads pass on ACR122U; the PN532 UART row is pending a card
on that reader.

## Recovery and system

| Operation | CLI | GUI | Gate | Hardware |
|---|---|---|---|---|
| Auto recovery | `recover auto` (confirmation, or `--yes`) | Recovery → Auto → Start recovery → authorization dialog | R | all-sector autopwn on PN532, ACR122U |
| Advanced recovery | `recover nested`, `staticnested`, `hardnested`, `darkside` (confirmation, or `--yes`) | Recovery → Advanced strategy options → authorization dialog | R | developer diagnostics; same engine |
| Darkside | `recover darkside` (confirmation, or `--yes`) | Recovery → Darkside → Start recovery → authorization dialog | R | host-driven MFCUK port; needs a repeating tag nonce |
| Nested | `recover nested` (confirmation, or `--yes`) | Recovery → Nested → authorization dialog | R | PN532, ACR122U |
| Static nested | `recover staticnested` (confirmation, or `--yes`) | Recovery → Static nested → authorization dialog | R | PN532, ACR122U |
| Hardnested | `recover hardnested` (confirmation, or `--yes`) | Recovery → Hardnested → authorization dialog | R | PN532, ACR122U |
| FM11RF08S profile | `recover fm11rf08s` (confirmation, or `--yes`) | Recovery → FM11RF08S → authorization dialog | R | backdoor recovery; without-backdoor path not claimed |
| Save recovered keys | `--key-store` write by the operation | Recovery → Save verified keys | explicit | file only |
| Self-check | `nfctl diag [--json]` | ☰ → Diagnostics (version, commit, build date, platform, drivers, firmware, capabilities, tables, readers) | R (no card I/O) | no card path |

Recovery authenticates only; it never writes the card. A recovered key is
displayed only after the service verifies it. Saving to the key store is a
separate action in both surfaces. Both surfaces require authorization before
the engine starts: `nfctl` prompts unless `--yes` is passed, and the GUI shows
a strategy/target/reader dialog before any worker or reader lease starts.

## Deliberate surface differences

- `--mock`, output formatting, `--device`/`--key`/`--key-file` selection, and
  stdout versus file dialogs are host or presentation differences and do not
  change card semantics.
- `nfctl` stays English under every locale. The GUI is English-base with a
  complete `zh-CN` catalog and an immediate language switch.
- Generic Type 4 NDEF has no CLI command and no GUI page. The DESFire NDEF
  surface provisions and reads the Type 4 application through the DESFire
  service; it does not add a generic Type 4 command.

## Audit result

No shipped `nfctl` card or file operation is missing a GUI path. The audit
added the GUI NDEF message export, which was the only missing file operation.
`scan --watch` and the GUI reader rail are two presentations of the shared
`nfc-app` monitor; neither adds a card path.
