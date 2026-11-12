# Hardware validation matrix

This is the release certificate. A row is `validated` only when a phase note
contains the run and the exact reader record below. An operation marked `not
recorded` is implemented but is not advertised as hardware-validated.

Release: `0.1.0`.

## Reader records

| Reader | Connection | Firmware | Date | Note |
|---|---|---|---|---|
| PN532 UART (CH340 adapter) | `pn532_uart:/dev/ttyUSB0` | PN532 firmware 1.6 | 2026-11-11 | `docs/notes/02-pn532-uart.md`, `04-classic-workflows.md`, `06-recovery-foundation.md`, `07-hardnested.md`, `09-type2-ndef.md`, `10-classic-ndef.md`, `11-type4-ndef.md`, `24c-uid-write.md`, `25a-classic-4k.md`, `25b-ultralight-ev1.md`, `25c-fm11rf08s.md`, `25d-type2-uid.md` |
| ACR122U direct USB/CCID | `acr122_usb:001:002` | internal PN532 firmware 1.6 | 2026-11-11 | `docs/notes/05-acr122-usb.md`, `06-recovery-foundation.md`, `07-hardnested.md`, `09-type2-ndef.md`, `24c-uid-write.md`, `24d-restore-with-uid.md` |

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
