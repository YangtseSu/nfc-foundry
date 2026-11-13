# Note: phase 26.3 DESFire value and record reads

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26c-value-record-read.md](../specs/26c-value-record-read.md).

## Implementation

- `nfc-desfire` adds `GetValue` (`0x6C`), `ReadRecords` (`0xBB`), and the
  test-scaffolding primitives `CreateValueFile` (`0xCC`),
  `CreateLinearRecordFile` (`0xC1`), `CreateCyclicRecordFile` (`0xC0`), and
  `WriteRecord` (`0x3B`). `ValueFileSpec` and `RecordFileSpec` group the
  create parameters.
- `Service::desfire_read` parses `GetFileSettings` and dispatches to `ReadData`,
  `GetValue`, or `ReadRecords`. A Transaction MAC file is rejected. The old raw
  `data_file_size` path can no longer run against a Value or Record file.
- `DesfireReadRequest` adds `start_record` and `record_count`.
  `DesfireReadResult` is an enum of `Data`, `Value`, and `Records`.
  `DesfireRecordRead` carries the DESFire offset, the absolute index of the
  first returned record, the record size, and the records.
- The CLI and GTK Read tab expose the same fields and use the same service.
  `nfctl` prints absolute record numbers; the GUI and CLI label record offset 0
  as the most recent record.
- The service rejects a `ReadRecords` response whose byte count is not exactly
  `count * record_size`; a unit test covers short and long responses.
- `GetValue`, `ReadData`, and `ReadRecords` consume DESFire additional frames
  (`0x91AF`) before checking the final status. A 255-byte Standard-file read
  through `nfctl desfire read` passed on the NDEF `E104` file; a scripted
  test covers the `ReadData` first-frame-plus-`AF` sequence.

## Command codes and record behavior

Two mistakes were found during hardware work and fixed:

- `WriteRecord` is `0x3B`. `0xDB` is `UpdateRecord`; sending the `0x3B`
  payload to `0xDB` produced a boundary error. The record write used by the
  test fixture now uses `0x3B`.
- A record write must be committed before the next write advances the current
  record count on this card. Without a commit, the second write replaced the
  current record and the count stayed at one.

`ReadRecords` offset 0 is the most recent record, not the first. The returned
records are chronological. The service computes `first_record` as
`current_records - offset - count` so the CLI and GUI can print absolute
indices.

The `GetFileSettings` third value field is not the current value: a value file
created with 7 reported `0`, while `GetValue` returned 7. Corrected
2026-11-13: the field is the LimitedCredit amount and is labelled `limited
credit value`; the current value comes only from `GetValue`.

## Unit tests

- `nfc-desfire` covers the `GetValue` command shape and 4-byte little-endian
  decode, the `ReadRecords` payload, the value/record create payloads, and the
  rejection of invalid limits, flags, sizes, and record counts.
- `nfc-app` covers the exact record-byte check: 16 bytes for two 8-byte
  records passes; 8 or 24 bytes fail.

## PN532 UART hardware record

Run on 2026-11-12 on `pn532_uart:/dev/ttyUSB0` with the DESFire EV3. The test
created reserved application `F0 0D 03`:

- value file `01`: plain, key-0 access, limits -50..50, limited credit and
  free GetValue enabled; `GetValue` returned `7`;
- linear record file `02`: 8-byte records, maximum 4, two records written;
  full read returned both records, offset 1 returned the older record, offset
  0 returned the latest;
- cyclic record file `03`: 4-byte records, maximum 2, one record written and
  read at offset 0.

A count beyond the current records failed before the card command. The test
removed the application; `nfctl desfire info` then reported `applications:
none`.

CLI checks with the fixture kept:

```text
$ nfctl desfire read --aid F00D03 --file 01 --key 0000000000000000
aid F0 0D 03 file 01 value · plain · ... · limits -50..50 · settings value 0 · limited credit enabled · get value free
value: 7

$ nfctl desfire read --aid F00D03 --file 02 --key 0000000000000000 \
    --record-count 2
record offset: 0 · records: 2 × 8 bytes
record 0: 01 02 03 04 05 06 07 08 | ........
record 1: 11 12 13 14 15 16 17 18 | ........

$ nfctl desfire read ... --start-record 1 --record-count 1
record offset: 1 · records: 1 × 8 bytes
record 0: 01 02 03 04 05 06 07 08 | ........

$ nfctl desfire read ... --record-count 3
invalid argument: file 02: requested 3 records but only 2 remain
```

## Open item

A second write to the cyclic file with offset 0 overwrote the first record and
left `current_records` at 1. The read path is valid for a one-record cyclic
file. Phase 26.10 closed this item: the first WriteRecord after a commit opens a
new record, and a full Cyclic Record file overwrites its oldest record. See
[26j-value-record-write.md](26j-value-record-write.md).

## ACR122U record

2026-11-12: `acr122_usb:001:002` (direct USB/CCID, `pcscd` stopped) passed
`reads_value_and_record_files_on_a_reserved_application`: the value file,
linear records, and cyclic records were read through `nfc-app`, the
over-request was refused, and the application was removed.

## Next

Phase 26.4 added 3K3DES and AES authentication key families (complete
2026-11-12). Phase 26.5 added their secure-messaging reads; see
[26e-secure-messaging.md](26e-secure-messaging.md).
