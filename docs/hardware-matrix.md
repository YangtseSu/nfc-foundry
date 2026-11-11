# Hardware validation matrix

This is the release certificate. A row is `validated` only when a phase note
contains the run and the exact reader record below. An operation marked `not
recorded` is implemented but is not advertised as hardware-validated.

Release: `0.1.0`.

## Reader records

| Reader | Connection | Firmware | Date | Note |
|---|---|---|---|---|
| PN532 UART (CH340 adapter) | `pn532_uart:/dev/ttyUSB0` | PN532 firmware 1.6 | 2026-11-11 | `docs/notes/02-pn532-uart.md`, `04-classic-workflows.md`, `06-recovery-foundation.md`, `07-hardnested.md`, `09-type2-ndef.md`, `10-classic-ndef.md`, `24c-uid-write.md` |
| ACR122U direct USB/CCID | `acr122_usb:001:017` | internal PN532 firmware 1.6 | 2026-11-10 | `docs/notes/05-acr122-usb.md`, `06-recovery-foundation.md`, `07-hardnested.md` |

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
| Classic full dump and per-sector key scan | validated | not recorded | phase 04: 64/64 blocks, 16/16 sectors, 37 attempts, 32 verified keys |
| Classic temporary data-block write, read-back, restore | validated | not recorded | phase 04: block 4, backup and restore |
| Classic sector-trailer write | not recorded | not recorded | — |
| Classic 4K read/write | not recorded | not recorded | — |
| Raw 7-bit REQA with 16-bit ATQA | validated | validated | phase 02, phase 05 |
| Gen1A manufacturer block 0 same-value write | not recorded | validated | phase 05: magic Classic 1K, reselect and read-back |
| Classic UID change through the guarded `nfc-app` service | validated | not recorded | phase 24c, 2026-11-11: Gen1A magic Classic 1K on PN532 UART, UID `FA B9 CF F4` -> `FA B9 CF F5`, reselect and full block 0/BCC verified, original UID restored. ACR122U service path not recorded; its driver primitive is validated. |
| Type 2 page read | validated | validated | phase 02, phase 05 |
| Type 2 / NTAG215 NDEF format, write, read, restore | validated | not recorded | phase 09: 135/135 pages, 33-byte Text record, restore |
| Type 2 one-way lock and password configuration | not recorded | not recorded | phase 09 |
| Classic NDEF format, write, read, restore | validated | not recorded | phase 10: blank Classic 1K UID `6A 31 5E 28`, byte-for-byte restore |
| Crypto1 parity probe and full software authentication | validated | validated | phase 06 |
| Nested, static-nested, and auto recovery | validated | validated | phase 06: recovered and re-verified target keys |
| Hardnested acquisition and solver | validated | validated | phase 07: 1816 and 1840 frames, recovered and re-verified keys |
| Darkside repeated-nonce collection | no sample | no sample | phase 06: documented reader limitation, not a key result |
| Type 4 NDEF read/write | not recorded | not recorded | phase 11; not advertised |

## Card records

| Card | UID | Readers | Operations |
|---|---|---|---|
| MIFARE Classic 1K | `FA B9 CF F4` | PN532, ACR122U | select, read, raw REQA, recovery |
| MIFARE Classic 1K | `6A 31 5E 28` | PN532 | selection, Classic NDEF format/write/restore |
| NTAG215 | `04 52 F5 67 D3 2A 81` | PN532 | 135-page read, NDEF write/read/restore |
| Type 2 tag | `04 34 4C C2 83 5C 80` | ACR122U | page read |
| Magic Gen1A Classic 1K | `FA B9 CF F4` | PN532, ACR122U | same-value block 0 write; guarded UID change/restore on PN532 |

`FA B9 CF F4` appears twice with different sector keys; set
`NFC_FOUNDRY_HARDWARE_KEY` and `NFC_FOUNDRY_HARDWARE_TARGET_KEY` for the card under test.

## Release claims

- PN532 UART: **PASS** for every row marked `validated`.
- ACR122U: **PASS** for every row marked `validated`.
- The matrix is not symmetric. ACR122U writes for Classic data blocks, Type 2
  NDEF, and Classic NDEF, plus one-way locks and passwords, are not part of the
  release claim.
- Type 4 NDEF is implemented but has no hardware record; it is not advertised.
- Classic 4K and sector-trailer writes are not part of the release claim.
- The guarded UID service is validated on PN532 UART for a Gen1A magic
  Classic 1K. The ACR122U service path is not recorded; the phase 05 record
  covers its driver's same-value block-0 write only.

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
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

ACR122U, authorized Gen1A manufacturer-block same-value write:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
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

ACR122U, same service test (not yet recorded):

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
restores the original UID and block 0.

Recovery, read-only and card-owned:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
NFC_FOUNDRY_HARDWARE_KEY=D3F7D3F7D3F7 \
cargo test -p nfc-recover --test hardware_recovery -- --ignored --nocapture
```

Restart `pcscd` after an ACR122U session when other smart-card applications
need the reader.
