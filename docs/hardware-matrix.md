# Hardware validation matrix

This is the release certificate. A row is `validated` only when a phase note
contains the run and the exact reader record below. An operation marked `not
recorded` is implemented but is not advertised as hardware-validated.

Release: `0.1.0`.

## Reader records

| Reader | Connection | Firmware | Date | Note |
|---|---|---|---|---|
| PN532 UART (CH340 adapter) | `pn532_uart:/dev/ttyUSB0` | PN532 firmware 1.6 | 2026-11-11 | `docs/notes/02-pn532-uart.md`, `04-classic-workflows.md`, `06-recovery-foundation.md`, `07-hardnested.md`, `09-type2-ndef.md`, `10-classic-ndef.md`, `11-type4-ndef.md`, `24c-uid-write.md`, `25a-classic-4k.md`, `25b-ultralight-ev1.md`, `25c-fm11rf08s.md`, `25d-type2-uid.md`, `25e-desfire.md` |
| ACR122U direct USB/CCID | `acr122_usb:001:002` | internal PN532 firmware 1.6 | 2026-11-11 | `docs/notes/05-acr122-usb.md`, `06-recovery-foundation.md`, `07-hardnested.md`, `09-type2-ndef.md`, `24c-uid-write.md`, `24d-restore-with-uid.md`, `26e-secure-messaging.md`, `26f-type4-ndef-read.md` |

`pcscd` was stopped for every ACR122U run. The ACR122U was opened as the
logged-in user through the `uaccess` udev rule.

## Certified operations

`validated` means the operation completed on that reader with the stated card
and no unrecoverable error. `not recorded` means no release run exists.

Each row is a reader/card path, not a surface. `nfctl` and the GTK workbench
call the same `nfc-app` service for every row, so a validated path applies to
both surfaces. GUI file operations (dump load/save, diff, NDEF message export,
card health) add no reader path of their own.

MIFARE Mini (SAK `0x09`) has identity reporting only. No Mini card has been
tested, and no read, write, restore, or recovery operation is claimed for it.

| Operation | PN532 UART | ACR122U | Evidence |
|---|---|---|---|
| Enumerate, open, firmware check | validated | validated | phase 02, phase 05 |
| Classic 1K select, UID/ATQA/SAK | validated | validated | phase 02, phase 05 |
| Classic Key A authenticate and single-block read | validated | validated | phase 02, phase 05 |
| Classic full dump and per-sector key scan | validated | validated | PN532 phase 04: 64/64 blocks, 16/16 sectors, 37 attempts, 32 verified keys. PN532 2026-11-12: FM11RF32N 4K, 256/256 blocks, 40/40 sectors, 40 attempts, 40 verified Key A. ACR122U 2026-11-11: 64/64 blocks, 16/16 sectors; key scan 46 attempts, 15 Key A hits. |
| Classic temporary data-block write, read-back, restore | validated | validated | PN532 phase 04: block 4. PN532 2026-11-12: FM11RF32N 4K block 4, backup and restore, final dump diff 0 changed. ACR122U 2026-11-11: block 4, backup and restore. |
| Classic sector-trailer write | validated | validated | 2026-11-11: PN532 block 7 and ACR122U block 15, GPB change with Key A preserved, read-back verified, original trailer restored. |
| Classic 4K read/write | validated | not recorded | 2026-11-12: PN532 UART, FM11RF32N `59 98 C9 C5`; 256/256 blocks, 40/40 sectors, key scan 40/40; block 4 write/restore, final dump diff 0 changed, 0 trailer changes. ACR122U 4K not tested. |
| Raw 7-bit REQA with 16-bit ATQA | validated | validated | phase 02, phase 05 |
| Gen1A manufacturer block 0 same-value write | not recorded | validated | phase 05: magic Classic 1K, reselect and read-back |
| Classic UID change through the guarded `nfc-app` service | validated | validated | 2026-11-11: Gen1A magic Classic 1K, UID `FA B9 CF F4` -> `FA B9 CF F5`, reselect and full block 0/BCC verified, original UID restored. PN532 used the ordinary backend; ACR122U used Gen1A. |
| Classic full restore with UID through the guarded `nfc-app` service | validated | validated | phase 24d, 2026-11-11: full dump with one changed data block, UID `FA B9 CF F4` -> `FA B9 CF F5`, full read verified, original data and UID restored and diffed byte for byte on both readers. PN532 used the ordinary backend; ACR122U used Gen1A. |
| Type 2 page read | validated | validated | phase 02, phase 05. PN532 2026-11-12: Ultralight EV1 MF0UL11 `04 43 47 4A 75 22 91`, 20/20 pages including EV1 configuration. |
| Type 2 GET_VERSION product identification | validated | not recorded | 2026-11-12: PN532 UART, MF0UL11 returned storage `0x0B`; the eight-byte descriptor selected the 20-page EV1 layout without a CC. ACR122U not recorded. |
| Type 2 NDEF format, write, read, restore | validated | validated | PN532 phase 09: 135/135 pages, 33-byte Text record, restore. PN532 2026-11-12: EV1 MF0UL11 `04 43 47 4A 75 22 91`, 48-byte user area, OTP CC `E1 10 06 00`, 45-byte NDEF capacity, Text record written/read and cleared to empty; page 3 is OTP and was not restored. ACR122U 2026-11-11: NTAG215 `04 34 4C C2 83 5C 80`, 135/135 pages, 33-byte Text record written and read, original 75-byte vCard NDEF restored. |
| Type 2 one-way lock | not recorded | not recorded | phase 09; no lock bits were set. |
| Type 2 UID change on a UID-writable clone | validated | not recorded | 2026-11-12: PN532 UART, NTAG213 clone `04 00 E3 C2 00 00 02`; same-value page 0/1/2 probe, UID -> `04 00 E3 C2 00 00 03`, reselect and page read-back, restored to the original UID, 45/45 pages, page JSON identical, static lock `00 00`. |
| Type 2 password configuration and authenticated access after AUTH0 | validated | not recorded | phase 24 S6, 2026-11-12: PN532 UART, NTAG216 `04 14 B5 7F DD 2A 81`; wrong PACK refused, 231/231 authenticated read, authenticated format/write/read, cleared to AUTH0=0xFF/ACCESS=0x00, original empty NDEF, CC, locks, and configuration restored; PWD/PACK reset to factory defaults after the run. PN532 2026-11-12: EV1 MF0UL11 `04 43 47 4A 75 22 91`, AUTH0=4/ACCESS=0x80 set, unauthenticated read refused, wrong PACK refused, 20/20 authenticated read, cleared to AUTH0=0xFF/ACCESS=0x00 with VCTID `0x05` preserved. |
| Classic NDEF format, write, read, restore | validated | validated | PN532 phase 10: blank Classic 1K UID `6A 31 5E 28`, byte-for-byte restore. PN532 2026-11-12: FM11RF32N 4K UID `59 98 C9 C5`, 3355-byte capacity, 3 blocks written, restore 47 written/208 unchanged/0 failures, final diff 0 changed. ACR122U 2026-11-11: 715-byte capacity, 3 blocks written, 18 blocks restored, full dump diff 0 changed. |
| Crypto1 parity probe and full software authentication | validated | validated | phase 06 |
| Nested, static-nested, and auto recovery | validated | validated | phase 06: recovered and re-verified target keys |
| Hardnested acquisition and solver | validated | validated | phase 07: 1816 and 1840 frames, recovered and re-verified keys |
| Darkside repeated-nonce collection | no sample | no sample | phase 06: documented reader limitation, not a key result |
| FM11RF08S backdoor nonce acquisition and key recovery | validated | not recorded | 2026-11-12: PN532 UART, FM11RF08S `9C 81 95 D4`; backdoor clear nonce matched the known-key decryption, the 59970-candidate solver list contained the known key, and the full attack recovered 32/32 sector keys with 16/16 sectors read-verified. No card writes. |
| DESFire EV3 identity, inventory, and factory-key authentication | validated | not recorded | 2026-11-12: PN532 UART, MIFARE DESFire EV3 8K (random UID, observed `04 77 1F A2 BB 17 90`); version 51.0, storage `0x1A`, key settings `0F 01`, `applications: none`. Legacy `Authenticate` (`0x0A`) with a DES zero key and D40 send mode completes mutual authentication; AES EV2 and 3K3DES zero keys fail. Read-only; no application or file was created. |
| DESFire EV3 NDEF provision, Type 4 write, delete | validated | validated | phase 25.5, 2026-11-12: PN532 UART, MIFARE DESFire EV3 8K (random UID, observed `04 4D 09 A2 BB 17 90`). Advanced/confirmation gates and a wrong-key refusal were checked; `D27600` with `E103`/`E104` was created, the 15-byte CC and empty NDEF file read back through Type 4, a Text record written/read/cleared, and the application deleted; final application list empty, matching pre-format. The CLI `format-ndef`/`info`/`delete-ndef` cycle passed on the same card. Commit `0x910C` (`NoChanges`) is accepted as success. ACR122U `acr122_usb:001:002` passed the same provisioning/write/restore/delete test and the CLI read cycle. See `docs/notes/25e-desfire.md`. |
| DESFire 3K3DES, AES EV1, and AES EV2 authentication | validated | validated | 2026-11-12: PN532 UART, reserved 3K3DES `F0 0D 04` and AES `F0 0D 05` applications with zero test keys. `0x1A`, `0xAA`, and `0x71` authenticated; 24-byte and 16-byte session keys checked; EV2 ENC/MAC keys differed; wrong key, key number, and family failed once. The PICC was formatted to recover EEPROM tombstones (free memory 8192) with owner approval. Secure-messaging reads were added in phase 26.5. See `docs/notes/26d-auth-key-families.md`. |
| DESFire EV1/EV2 secure-messaging read | validated | validated | 2026-11-12: PN532 UART, reserved 3K3DES `F0 0D 06` and AES `F0 0D 07` applications with zero test keys. Plain, MACed, and enciphered Standard files (48 bytes) and MACed/enciphered Value files read through the direct layer and `nfc-app` under 3K3DES, AES EV1, and AES EV2 plus the same reads through the `nfctl` CLI; response MACs, CRC32, and padding verified; tampered responses rejected in unit tests; applications removed; the PICC was formatted with owner approval to recover EEPROM (free memory 8192). ACR122U `acr122_usb:001:002` passed the same test and all three `nfctl` reads; `pcscd` was stopped. See `docs/notes/26e-secure-messaging.md`. |
| DESFire Standard/Backup file write and transactions | validated | validated | phase 26.8, 2026-11-13: PN532 UART and ACR122U `acr122_usb:001:002` (`pcscd` stopped), reserved 3K3DES `F0 0D 06` and AES `F0 0D 07` applications with Plain, MACed, and Enciphered Standard files plus a Plain Backup file. Wrote a known range through `Service::desfire_write` under 3K3DES EV1, AES EV1, and AES EV2, verified EV1/EV2 response checks, committed the Backup file, read every range back, and removed both applications; `nfctl desfire write`/`read` passed on the kept fixture on both readers. See `docs/notes/26h-generic-write.md`. |
| DESFire application and six file types: create, change, delete | validated | validated | phase 26.9, 2026-11-13: PN532 UART and ACR122U `acr122_usb:001:002` (`pcscd` stopped), owned EV3 8K, PICC formatted first (free memory 8192). Created AES `F0 0D 08` with two keys, then Standard, Backup, Value, Linear Record, Cyclic Record, and Transaction MAC files through `Service::desfire_create_file`; the Transaction MAC create sent the key and version enciphered. Changed the Standard file to enciphered with new access rights, deleted all six files and the application with manifests in a temporary directory, and confirmed `applications: none`. Free memory stayed at 7648 bytes after deletion (DESFire tombstones). The CLI then created `F0 0D 09`, created and changed a Standard file, deleted the file and application with manifests, and reported `applications: none` on both readers. See `docs/notes/26i-management.md`. |
| DESFire value, record, and Transaction MAC writes | validated | not recorded | phase 26.10, 2026-11-14: PN532 UART, owned EV3 8K. AES application `F0 0D 0B` with one AES key and Value (-50..50, initial 20, limited credit and free GetValue), Linear Record (8-byte, 4), Cyclic Record (4-byte, 2), and Transaction MAC (read free, key version 1) files. `nfctl desfire value` credited 20→25, debited 25→22, and limited-credited 22→25; `nfctl desfire record` appended, updated a range of the newest record, and cleared the cyclic file. Creating the Transaction MAC file then credit/append/append/clear/update commits returned TMC 1–5, and each TMV was recomputed from the supplied AppTransactionMACKey and matched both the commit response and the stored Transaction MAC file. A wrong key failed after the commit; an over-limit credit, a record update across the boundary, and clearing an empty file were refused before the card command. All files and the application were deleted; `applications: none`, free memory 6752 (tombstones). The environment-gated `hardware_value_record` test passed on the same reader under AES EV1 and AES EV2 (`NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2`), including Enciphered Value and Cyclic Record files; ClearRecordFile needed the EV2 MACed command and the EV1 Plain command. ACR122U was present but had no card in its field. See `docs/notes/26j-value-record-write.md`. |
| DESFire key verification and authenticated UID | not recorded | validated | 2026-11-13: ACR122U `acr122_usb:001:002`, reserved 3K3DES `F0 0D 04` and AES `F0 0D 05` applications with zero test keys. `verify_key` reported the session family and GetKeyVersion 0, and GetCardUID returned the UID only after EV1 CRC32 or EV2 CMAC verification; no file command ran. `nfctl desfire verify-key`/`uid` passed for DES, 3K3DES, AES EV1, and AES EV2; fixtures removed and `applications: none`. The PN532 UART reader was enumerated but had no card during the run. See `docs/notes/26g-cryptographic-verification.md`. |
| DESFire Type 4 NDEF read | validated | validated | 2026-11-13: PN532 UART (phase 25.5 Type 4 read-back) and ACR122U `acr122_usb:001:002`; `nfctl desfire ndef-read` prints the CC, mapping version, MLe/MLc, file facts, decoded records, proprietary/unknown CC TLVs, and read-only reason. The ACR122U ran `format-ndef`, `ndef-read`, and `delete-ndef`; the application list returned to empty. See `docs/notes/26f-type4-ndef-read.md`. |
| DESFire Value and Linear/Cyclic Record read | validated | validated | 2026-11-12: PN532 UART, reserved AID `F0 0D 03`; value file limits -50..50 with GetValue 7; linear 8-byte records 2/4 with full, offset-1, and latest reads; cyclic 4-byte records 1/2; a count beyond current records was refused before the command; application removed. A 255-byte Standard-file read consumed chained `ReadData` frames. See `docs/notes/26c-value-record-read.md`. |
| DESFire Standard/Backup file-settings decode | validated | validated | 2026-11-12: PN532 UART, DESFire EV3; the NDEF application's `E103` (15 bytes) and `E104` (255 bytes) decoded as standard/plain with the expected access rights and raw settings. Value, Linear Record, Cyclic Record, and Transaction MAC layouts are unit-tested only. ACR122U `acr122_usb:001:002` decoded the phase 26.5 standard and value fixture settings. See `docs/notes/26b-file-settings.md`. |
| DESFire EV3 temporary application/file round trip | validated | not recorded | 2026-11-12: PN532 UART; created reserved AID `F0 0D 01` (1 key, 2K3DES), authenticated app key 0, created plain standard file `01` (32 bytes, key-0 access), wrote and read back `NFC Foundry DESFire` at offset 0, deleted the app, and confirmed `GetApplicationIDs` empty. No other application was touched. |
| Type 4 NDEF read/write | validated | not recorded | phase 11, 2026-11-12: PN532 UART, NTAG 424 DNA `04 86 40 42 22 17 90`; CC 2.0 (23 bytes), MLe 256/MLc 255, NDEF file `E104` with 256-byte capacity, empty state; Text record written and read back, empty state cleared and re-read. An earlier DESFire EV3 8K `04 4D 09 A2 BB 17 90` probe found no NDEF Tag Application (`6A82`). |

## Card records

| Card | UID | Readers | Operations |
|---|---|---|---|
| MIFARE Classic 1K | `FA B9 CF F4` | PN532, ACR122U | select, read, raw REQA, recovery; ACR122U dump, data-block/trailer writes, UID, restore, Classic NDEF |
| MIFARE Classic 1K | `6A 31 5E 28` | PN532 | selection, Classic NDEF format/write/restore |
| Fudan FM11RF08S | `9C 81 95 D4` | PN532 | static encrypted profile, backdoor recovery, 32/32 keys, 16/16 read verification |
| FM11RN32N (FM11RF32N, 4K) | `59 98 C9 C5` | PN532 | 4K select/read, 40-sector key scan, data-block write/restore, Classic NDEF format/write/restore |
| NTAG215 | `04 52 F5 67 D3 2A 81` | PN532 | 135-page read, NDEF write/read/restore |
| MIFARE Ultralight EV1 MF0UL11 | `04 43 47 4A 75 22 91` | PN532 | GET_VERSION, 20-page read, OTP CC format, NDEF write/read/clear, password set/auth/clear |
| UID-changeable NTAG213 clone | `04 00 E3 C2 00 00 02` | PN532 | 45-page read, UID-page probe, UID change/restore, BCC verification |
| MIFARE DESFire EV3 8K | random UID, observed `04 4D 09 A2 BB 17 90` | PN532 | identity and application/file inventory; legacy DES zero-key authentication; reserved temporary application/file write-read-delete round trip; NDEF provisioning, Type 4 Text write/read/clear, and guarded application delete; card left with no applications |
| NTAG 424 DNA | `04 86 40 42 22 17 90` | PN532 | Type 4 read, NDEF write, clear/restore |
| Type 2 tag | `04 34 4C C2 83 5C 80` | ACR122U | page read; NDEF format/write/read/restore |
| Magic Gen1A Classic 1K | `FA B9 CF F4` | PN532, ACR122U | same-value block 0 write; guarded UID change/restore; restore with UID on both |

`FA B9 CF F4` appears twice with different sector keys. The PN532 card uses
`FFFFFFFFFFFF` plus `FBBECDF6304E`; the ACR122U card uses
`000000000000`, `D3F7D3F7D3F7`, and `FFFFFFFFFFFF`. Set
`NFC_FOUNDRY_HARDWARE_KEY`, `NFC_FOUNDRY_HARDWARE_KEY_2`,
`NFC_FOUNDRY_HARDWARE_KEY_3`, and `NFC_FOUNDRY_HARDWARE_TARGET_KEY` for the
card under test.

## Release claims

- PN532 UART: **PASS** for every row marked `validated`.
- ACR122U: **PASS** for every row marked `validated`.
- The matrix is not symmetric. ACR122U Type 2 NDEF, one-way locks, password
  configuration, and authenticated access after AUTH0 are not part of the
  release claim.
- Type 4 NDEF read/write is validated on PN532 UART and not recorded on
  ACR122U. It has no CLI or GUI surface yet, so the path is not advertised.
- Classic 4K read/write is validated on PN532 UART and not recorded on
  ACR122U. The ACR122U 4K path is not part of the release claim.
- Ultralight EV1 MF0UL11 is validated on PN532 UART. MF0UL21 is implemented
  and unit-tested but has no hardware record, so it is not advertised.
- FM11RF08S backdoor key recovery is validated on PN532 UART and not recorded
  on ACR122U. The without-backdoor distance path is not claimed because
  host-driven timing did not make it reliable.
- Type 2 UID change is validated on PN532 UART for a 7-byte NTAG213 clone and
  not recorded on ACR122U. Genuine Type 2 tags remain read-only at the UID
  pages.
- DESFire EV3 inventory, factory-key authentication, NDEF provisioning, Type 4
  Text write/read/clear, and the guarded delete are validated on PN532 UART.
  Standard/Backup file-settings decode, Value/Linear/Cyclic Record reads,
  3K3DES/AES EV1/AES EV2 authentication, and EV1/EV2 secure-messaging reads are
  validated on the same reader. Transaction MAC decoding is unit-tested only.
  The ACR122U rows above cover NDEF, authentication, secure-messaging reads,
  Type 4 NDEF read, and key verification/authenticated UID; the PN532 UART row
  for key verification is pending a card on that reader.
- Classic sector-trailer writes and the guarded UID service are validated on
  both readers. The ACR122U trailer run used a default-access sector because the
  generic single-block request cannot supply a separate Key A for a B-only
  access condition; restore hydrates that key from dump metadata.

Do not promote a row without adding a dated entry to the matching phase note
and updating this table.

## Gate commands

PN532 UART, read-only Classic workflow:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware -- --ignored
```

PN532 UART, authorized temporary Classic write and restore:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BLOCK=4 \
cargo test -p nfctl --features hardware-write -- --ignored
```

PN532 UART, authorized NTAG215 NDEF round trip:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-type2-backups \
cargo test -p nfctl --features hardware-write -- --ignored
```

PN532 UART, Type 4 NDEF read (validated 2026-11-12 with an NTAG 424 DNA):

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware --test hardware_type4 -- --ignored
```

PN532 UART, authorized Type 4 NDEF write and restore:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-type4-backups \
cargo test -p nfctl --features hardware-write --test hardware_type4 -- --ignored
```

ACR122U, read-only driver checks:

```sh
sudo systemctl stop pcscd
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

ACR122U, authorized Gen1A manufacturer-block same-value write:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
NFC_FOUNDRY_HARDWARE_KEY=<sector-key> \
NFC_FOUNDRY_HARDWARE_SECTOR0_KEY=<sector-0-key> \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM=yes \
cargo test -p nfc-drivers --features hardware-write -- --ignored
```

PN532 UART, authorized magic Classic 1K UID change and restore (validated
2026-11-11 with a Gen1A card and `FFFFFFFFFFFF` sector-0 Key A):

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_KEY_TYPE=A \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-uid-backups \
cargo test -p nfctl --features hardware-write --test hardware_classic -- --ignored changes_and_restores_the_card_uid --nocapture
```

ACR122U, authorized magic Classic 1K UID change and restore (validated
2026-11-11 on `acr122_usb:001:002`; the card used Gen1A):

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:<bus>:<address> \
NFC_FOUNDRY_HARDWARE_KEY=<sector-0-key> \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_UID=<target-uid> \
cargo test -p nfctl --features hardware-write --test hardware_classic -- --ignored changes_and_restores_the_card_uid --nocapture
```

The test changes the UID, prints the reselected identity and backup path, then
restores the original UID and block 0. The ACR122U run used sector-0 Key A
`A0A1A2A3A4A5` and Key B `000000000000`.

Both readers, full Classic 1K restore with UID (validated 2026-11-11):

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=<pn532_uart:/dev/ttyUSB0|acr122_usb:001:002> \
NFC_FOUNDRY_HARDWARE_KEY=<card-key-1> \
NFC_FOUNDRY_HARDWARE_KEY_2=<card-key-2> \
[NFC_FOUNDRY_HARDWARE_KEY_3=<card-key-3>] \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-s3b-backups \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored restores_a_dump_and_changes_the_uid_last --nocapture
```

Both readers, sector-trailer write and restore (validated 2026-11-11):

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=<reader> \
NFC_FOUNDRY_HARDWARE_KEY=<trailer-key> \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_TRAILER=<trailer-block> \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored writes_and_restores_a_sector_trailer --nocapture
```

ACR122U, Classic NDEF format/write/read/restore (validated 2026-11-11):

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-acr-ndef-backups \
cargo test -p nfctl --features hardware-write --test hardware_classic_ndef \
  -- --ignored formats_writes_reads_and_restores_a_classic_card --nocapture
```

Recovery, read-only and card-owned:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
NFC_FOUNDRY_HARDWARE_KEY=D3F7D3F7D3F7 \
cargo test -p nfc-recover --test hardware_recovery -- --ignored --nocapture
```

Restart `pcscd` after an ACR122U session when other smart-card applications
need the reader.
