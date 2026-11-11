# Hardware validation matrix

This is the release certificate. A row is `validated` only when a phase note
contains the run and the exact reader record below. An operation marked `not
recorded` is implemented but is not advertised as hardware-validated.

Release: `0.1.0`.

## Reader records

| Reader | Connection | Firmware | Date | Note |
|---|---|---|---|---|
| PN532 UART (CH340 adapter) | `pn532_uart:/dev/ttyUSB0` | PN532 firmware 1.6 | 2026-11-11 | `docs/notes/02-pn532-uart.md`, `04-classic-workflows.md`, `06-recovery-foundation.md`, `07-hardnested.md`, `09-type2-ndef.md`, `10-classic-ndef.md`, `24c-uid-write.md` |
| ACR122U direct USB/CCID | `acr122_usb:001:002` | internal PN532 firmware 1.6 | 2026-11-11 | `docs/notes/05-acr122-usb.md`, `06-recovery-foundation.md`, `07-hardnested.md`, `09-type2-ndef.md`, `24c-uid-write.md`, `24d-restore-with-uid.md` |

`pcscd` was stopped for every ACR122U run. The ACR122U was opened as the
logged-in user through the `uaccess` udev rule.

## Certified operations

`validated` means the operation completed on that reader with the stated card
and no unrecoverable error. `not recorded` means no release run exists.

Each row is a reader/card path, not a surface. `nfctl` and the GTK workbench
call the same `nfc-app` service for every row, so a validated path applies to
both surfaces. GUI file operations (dump load/save, diff, NDEF message export)
add no reader path of their own.

| Operation | PN532 UART | ACR122U | Evidence |
|---|---|---|---|
| Enumerate, open, firmware check | validated | validated | phase 02, phase 05 |
| Classic 1K select, UID/ATQA/SAK | validated | validated | phase 02, phase 05 |
| Classic Key A authenticate and single-block read | validated | validated | phase 02, phase 05 |
| Classic full dump and per-sector key scan | validated | validated | PN532 phase 04: 64/64 blocks, 16/16 sectors, 37 attempts, 32 verified keys. ACR122U 2026-11-11: 64/64 blocks, 16/16 sectors; key scan 46 attempts, 15 Key A hits. |
| Classic temporary data-block write, read-back, restore | validated | validated | PN532 phase 04: block 4. ACR122U 2026-11-11: block 4, backup and restore. |
| Classic sector-trailer write | validated | validated | 2026-11-11: PN532 block 7 and ACR122U block 15, GPB change with Key A preserved, read-back verified, original trailer restored. |
| Classic 4K read/write | not recorded | not recorded | No 4K card was present on 2026-11-11; the two test cards are 1K (SAK 08). |
| Raw 7-bit REQA with 16-bit ATQA | validated | validated | phase 02, phase 05 |
| Gen1A manufacturer block 0 same-value write | not recorded | validated | phase 05: magic Classic 1K, reselect and read-back |
| Classic UID change through the guarded `nfc-app` service | validated | validated | 2026-11-11: Gen1A magic Classic 1K, UID `FA B9 CF F4` -> `FA B9 CF F5`, reselect and full block 0/BCC verified, original UID restored. PN532 used the ordinary backend; ACR122U used Gen1A. |
| Classic full restore with UID through the guarded `nfc-app` service | validated | validated | phase 24d, 2026-11-11: full dump with one changed data block, UID `FA B9 CF F4` -> `FA B9 CF F5`, full read verified, original data and UID restored and diffed byte for byte on both readers. PN532 used the ordinary backend; ACR122U used Gen1A. |
| Type 2 page read | validated | validated | phase 02, phase 05 |
| Type 2 / NTAG215 NDEF format, write, read, restore | validated | validated | PN532 phase 09: 135/135 pages, 33-byte Text record, restore. ACR122U 2026-11-11: NTAG215 `04 34 4C C2 83 5C 80`, 135/135 pages, 33-byte Text record written and read, original 75-byte vCard NDEF restored. |
| Type 2 one-way lock | not recorded | not recorded | phase 09; no lock bits were set. |
| Type 2 password configuration and authenticated access after AUTH0 | validated | not recorded | phase 24 S6, 2026-11-12: PN532 UART, NTAG216 `04 14 B5 7F DD 2A 81`; wrong PACK refused, 231/231 authenticated read, authenticated format/write/read, cleared to AUTH0=0xFF/ACCESS=0x00, original empty NDEF, CC, locks, and configuration restored; PWD/PACK reset to factory defaults after the run. |
| Classic NDEF format, write, read, restore | validated | validated | PN532 phase 10: blank Classic 1K UID `6A 31 5E 28`, byte-for-byte restore. ACR122U 2026-11-11: 715-byte capacity, 3 blocks written, 18 blocks restored, full dump diff 0 changed. |
| Crypto1 parity probe and full software authentication | validated | validated | phase 06 |
| Nested, static-nested, and auto recovery | validated | validated | phase 06: recovered and re-verified target keys |
| Hardnested acquisition and solver | validated | validated | phase 07: 1816 and 1840 frames, recovered and re-verified keys |
| Darkside repeated-nonce collection | no sample | no sample | phase 06: documented reader limitation, not a key result |
| Type 4 NDEF read/write | not recorded | not recorded | phase 11; not advertised. A DESFire EV3 8K `04 4D 09 A2 BB 17 90` was probed on PN532 on 2026-11-11; it has no NDEF Tag Application (`6A82`) and is not NDEF-formatted. |

## Card records

| Card | UID | Readers | Operations |
|---|---|---|---|
| MIFARE Classic 1K | `FA B9 CF F4` | PN532, ACR122U | select, read, raw REQA, recovery; ACR122U dump, data-block/trailer writes, UID, restore, Classic NDEF |
| MIFARE Classic 1K | `6A 31 5E 28` | PN532 | selection, Classic NDEF format/write/restore |
| NTAG215 | `04 52 F5 67 D3 2A 81` | PN532 | 135-page read, NDEF write/read/restore |
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
- Type 4 NDEF is implemented but has no hardware record; it is not advertised.
- Classic 4K has no hardware record; it is not part of the release claim.
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
