# Phase 10 note: Classic NDEF

Date: 2026-11-10

## Implemented

### Mapping crate

`nfc-classic-ndef` now owns the Classic mapping.

- `mad.rs` parses MAD1 from sector 0 blocks 1 and 2 and MAD2 from sector 0 plus
  sector 16 blocks 0..2. It validates CRC-8/MIFARE-MAD over the info byte and
  stored AIDs, treats `03 E1` as the NFC AID, preserves other AIDs, and
  reports DA/version/read-only errors.
- `layout.rs` maps each NFC sector to its data blocks, reads the per-sector GPB
  mapping version and access conditions, rejects non-contiguous sectors (with
  the MAD2 sector-16 exception), and exposes the logical NDEF area.
- Classic TLV parsing handles NULL, NDEF Message, Proprietary, Terminator, and
  reserved tags with short and three-byte lengths. The mandatory NDEF TLV keeps
  its offset so writes can update the length field in place.
- `encode_ndef_area` preserves bytes before the mandatory TLV, replaces the
  TLV, writes the Terminator, and zero-fills unused space. `format_ndef_area`
  emits `03 00 FE`.
- The 4K data area is 210 blocks: sectors 1..15 and 17..31 have three data
  blocks each, sectors 32..39 have fifteen.

There is no capability-container record in the Classic mapping. The GPB is
reported as the mapping version and access capability; the documentation was
corrected accordingly.

### Application workflows

`nfc-app` adds `classic_ndef_read`, `classic_ndef_inspect`,
`classic_ndef_write`, and `classic_ndef_format`.

- Read performs the normal Classic dump read and parses the mapping from that
  image, so partial reads are explicit.
- Write accepts only a formatted writable mapping with an existing mandatory
  NDEF TLV. It validates every changed block against the current access bits
  and available keys, saves a project backup, writes the length field as zero,
  writes the value and Terminator, writes the final length field, verifies each
  block by read-back, checks card identity, and re-reads the mapping.
- Format requires `I_OWN_THIS_CARD`, a complete read, key availability for every
  data block and trailer, a project backup, and a final full read. Data blocks
  are written before trailers. After each trailer write the workflow
  re-authenticates with the new public Key A before reading the block back, so
  a key change cannot leave the format unverified. 1K formats with MAD1 and 15
  NFC sectors; 4K formats with MAD2 and 38 NFC sectors. The read-only option writes GPB `43`
  and access bits `07 8F 0F`. The current Key B is preserved when it was read;
  a masked Key B falls back to the factory value.
- A manufacturer block is never part of a write or format plan.

### CLI and GUI

`nfctl classic ndef info|read|write|format` shares the same services. Format
requires the typed token; read-only format is a flag. The Text and URI message
sources match the Type 2 `nfctl ndef` command.

The GPUI Kit Classic workspace has a NDEF tab showing mapping, sector list,
capacity, records, a Text write form, and a format action with a danger
confirmation and a read-only checkbox. The tab indices changed from
Blocks/Diff/Keys to Blocks/NDEF/Diff/Keys; existing UI tests were updated.

## Verification

Software checks run in this phase:

- `cargo test -p nfc-classic-ndef`: 10 deterministic tests plus 3 property
  tests, including CRC check vectors, MAD1/MAD2 round trips, bad CRC,
  contiguity, long messages, leading proprietary TLV preservation, read-only
  GPB, partial data areas, and arbitrary contiguous NFC sector subsets.
- `cargo test -p nfc-app --test classic_ndef`: 9 scripted workflows, including
  1K format/write/read, 4K MAD2 format, confirmation, complete-read
  enforcement, read-only refusal, unformatted refusal, bad CRC, gaps, a
  400-byte message across sectors, and Key B preservation across format.
- `cargo test -p nfctl --test cli`: 14 tests, including `classic ndef
  info|read|write|format`, confirmation, and read-only format.
- `cargo test -p nfc-classic-gui --test ui`: 8 tests, including NDEF format,
  NDEF write, and read-only format.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- The gated hardware test is `nfctl/tests/hardware_classic_ndef.rs`. Its
  read-only check reports the mapping; the write path requires both
  `NFCX_HARDWARE_WRITE` and `NFCX_HARDWARE_NDEF_FORMAT` and restores the
  original dump after the round trip.

## Hardware

Hardware validation on 2026-11-10 used the PN532 UART at
`pn532_uart:/dev/ttyUSB0` and an owned MIFARE Classic 1K with UID
`6A 31 5E 28`. The card started blank and factory-configured: no MAD, all data
blocks zero, sector trailers `00 00 00 00 00 00 FF 07 80 69 FF FF FF FF FF FF`.
A complete 64/64-block dump was saved first.

The gated test
`nfctl/tests/hardware_classic_ndef.rs::formats_writes_reads_and_restores_a_classic_card`
passed:

```text
formatted: 715
wrote 3 blocks
restored: 20 written, 43 unchanged, 0 failures
no MAD; state=Unformatted
```

The sequence was:

1. Format the blank card: MAD1, NFC sectors 1..15, public NFC Key A
   `D3F7D3F7D3F7`, read/write access bits `7F 07 88`, GPB `40`.
2. Write a Text NDEF record and read it back through the mapping.
3. Restore the original dump; the restore reported zero failures.
4. Re-read the card: `MAD: absent`, `NDEF: unformatted`, and a fresh dump
   compared byte-for-byte equal to the pre-test dump (0 blocks differing).

Two hardware-only defects were found and fixed during the record:

- The first format attempt failed at the sector 0 trailer with
  `block 3 did not match after the trailer write`. Reading a MIFARE Classic
  trailer masks Key A (and usually Key B), so comparing all 16 read-back bytes
  can never match after a key change. The workflow now verifies the access
  bytes and GPB, proves the new Key A by authenticating with it, and proves
  Key B when the trailer grants Key B writes by authenticating with Key B.
- Format initially wrote `03 00 FE` at the start of every NFC sector instead of
  once at the start of the concatenated data area. The first hardware run
  exposed it as `wrote 17 blocks` for a short Text record. Format now builds
  one logical area, and the regression test asserts that later NFC sector data
  blocks stay zero. The corrected hardware run wrote 3 blocks.

No ACR122U was connected, so the ACR122U Classic NDEF record remains pending.

## Follow-ups

- Hardware format/write/read record on ACR122U (PN532 UART is recorded).
- Key-B management for reformatting a card whose MAD or NFC trailers are
  already protected.
- Optional MAD registration/publisher-sector management.
- Type 4 NDEF remains phase 11.
