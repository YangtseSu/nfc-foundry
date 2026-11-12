# Spec: phase 25.1 FM11RN32N Classic 4K validation

Status: implemented 2026-11-12; PN532 validated. Plan:
[25-card-support.md](../plan/25-card-support.md).

## Problem

The Classic 4K layout, dump, diff, restore, key scan, and MAD2 NDEF mapping are
implemented, but the hardware matrix records Classic 4K as `not recorded`. The
owned Fudan FM11RN32N (FM11RF32N, 32 kbit) is the first 4K card available. The
slice must prove the existing path on that card and identify any assumption
that only held for 1K.

Known 4K-specific behavior to verify:

- Sectors 0..31 have four blocks; sectors 32..39 have sixteen blocks.
- Block 0 is the manufacturer block; trailer blocks are 3, 7, ..., 127, 143,
  ..., 255.
- MAD2 uses sector 0 data blocks 1 and 2 and identifies the NFC sectors.
- Key scan and dump metadata cover 40 sectors, not 16.
- Restore writes data blocks before trailers and preserves keys and access
  bits.

## Scope

Read-only first:

- Select the card and record UID, ATQA, SAK, and `CardType::MifareClassic4K`.
- Read the full 256-block image with the configured keys and common keys.
- Scan Key A (and Key B when configured) for all 40 sectors.
- Read the Classic NDEF mapping when present; report mapping version, sectors,
  block count, capacity, and state.

Then, only with `NFC_FOUNDRY_HARDWARE_WRITE=yes` and a complete original dump:

- Change one ordinary data block, read it back, restore the original value,
  and read it back again.
- Optionally run the existing Classic NDEF format/write/read/restore test if
  the card is blank and `NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=yes`. This test
  rewrites NFC sectors and their trailers and restores the original dump; it is
  not part of the default 4K acceptance unless the card is owned and blank.

Explicitly out of scope:

- Block 0 writes, UID writes, and sector-trailer access-bit changes.
- Recovery. If the card shows the FM11RF08S static encrypted nonce profile,
  stop after recording the profile and route the card to 25.3.

## Implementation

No product code is expected before the hardware run. The existing
`crates/nfctl/tests/hardware_classic.rs` already accepts both 1K and 4K and
derives the layout from `CardType`. `hardware_classic_ndef.rs` already accepts
both layouts.

If the run exposes a 4K-only defect, fix it at the cause in the same slice:
layout arithmetic, sector/block iteration, dump metadata, key scan bounds,
MAD2 discovery, or restore ordering. Do not add a 4K special case to a 1K
workflow.

## Gates and commands

Read-only inventory and dump:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware --test hardware_classic \
  -- --ignored dumps_and_scans_a_classic_card --nocapture
```

Set `NFC_FOUNDRY_HARDWARE_KEY`, `NFC_FOUNDRY_HARDWARE_KEY_2`, and
`NFC_FOUNDRY_HARDWARE_KEY_3` when the card does not use `FFFFFFFFFFFF`.

Temporary data-block write and restore:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=<sector-key> \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BLOCK=4 \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored writes_and_restores_one_data_block --nocapture
```

Classic NDEF read:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware --test hardware_classic_ndef \
  -- --ignored reads_the_classic_ndef_mapping --nocapture
```

## Acceptance

- The selected card reports 4K with the expected 40-sector/256-block layout.
- The full dump reports 256/256 readable blocks with the supplied keys, or the
  exact unreadable sectors are recorded as a key gap, not a layout failure.
- Key scan covers 40 sectors and reports attempts and verified keys.
- One ordinary data block changes, reads back, restores, and reads back; the
  final full dump differs from the original by zero blocks.
- The NDEF read either reports a valid MAD2 mapping and state or reports that
  the card is unformatted; both are recorded.
- If the static encrypted nonce profile is detected, the slice records it and
  stops without running standard nested/hardnested claims.
- The hardware matrix gains a dated PN532 row and `docs/notes/25a-classic-4k.md`
  records the UID, reader, commands, and result.

## Safety

- No block 0 write.
- No sector-trailer key or access-bit change. The NDEF round trip may rewrite
  GPB bytes in NFC trailers and restores the original dump.
- No UID write.
- The temporary write uses the existing service preflight, backup, read-back,
  and identity check.
- Stop on the first unrecoverable failure; do not continue to other blocks.
