# Note: phase 25.1 FM11RN32N Classic 4K validation

Status: complete 2026-11-12. Plan:
[25-card-support.md](../plan/25-card-support.md). Spec:
[25a-classic-4k.md](../specs/25a-classic-4k.md).

## Card

- Fudan FM11RN32N (FM11RF32N, 32 kbit, 4K Classic-compatible).
- UID `59 98 C9 C5`, ATQA `00 02`, SAK `18`.
- Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6.
- Keys: `FFFFFFFFFFFF` Key A on every sector. Key B was not needed.

## Read-only inventory

`nfctl scan` selected `MIFARE Classic 4K`.

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware --test hardware_classic \
  -- --ignored dumps_and_scans_a_classic_card --nocapture
```

Result:

```text
classic_4k uid=59 98 C9 C5 readable=40/40 sectors
keyscan attempts=40 verified=40
```

A full project dump before any write reported:

```text
classic_4k: 256/256 blocks, 40/40 sectors readable
```

The dump was written to
`/tmp/nfc-4k-backups/fm11rn32n-5998c9c5-before.project` (43547 bytes) and the
40 verified keys were captured into the card-scoped key store.

## Temporary write and restore

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BLOCK=4 \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-4k-backups \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored writes_and_restores_one_data_block --nocapture
```

Block 4 changed, read back, restored, and read back again. Both writes produced
a backup record in `/tmp/nfc-4k-backups`. A second full dump reported
`256/256 blocks, 40/40 sectors readable`, and:

```text
classic_4k: 0 changed, 256 equal, 0 unknown
trailer changes: 0
```

## Classic NDEF round trip

The card was unformatted before the run. The read-only test reported:

```text
no MAD; state=Unformatted
```

With both write gates and the full dump present:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-4k-backups \
cargo test -p nfctl --features hardware-write --test hardware_classic_ndef \
  -- --ignored formats_writes_reads_and_restores_a_classic_card --nocapture
```

Result:

```text
formatted: 3355
wrote 3 blocks
restored: 47 written, 208 unchanged, 0 failures
```

The format used MAD2 on the 4K layout, capacity 3355 bytes, and restored the
original unformatted image. A final full dump after the run reported 256/256
blocks, 40/40 sectors, and the diff against the pre-write dump was:

```text
classic_4k: 0 changed, 256 equal, 0 unknown
trailer changes: 0
```

## Decisions

- No product code change was needed; the existing Classic 4K layout, key scan,
  dump, read-back, restore, and MAD2 NDEF handling worked on the first run.
- No block 0 write, no sector-trailer key or access-bit change, and no UID
  write were performed. The NDEF round trip rewrote NFC data and GPB bytes and
  restored the original dump.
- The FM11RF32N did not show the FM11RF08S static encrypted nonce profile:

  ```sh
  nfctl --device pn532_uart:/dev/ttyUSB0 --key FFFFFFFFFFFF --yes \
    recover fm11rf08s --known-block 4 --known-type A --target-type A
  ```

  returned `the card does not return a static encrypted nonce`. No recovery
  claim is added.
- The ACR122U 4K row remains unrecorded.
