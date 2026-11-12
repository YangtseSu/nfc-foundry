# Spec: phase 26.3 DESFire value and record reads

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md).

## Problem

The phase 26.2 inventory decodes file types but `nfctl desfire read` only used
`ReadData`, which is valid for Standard and Backup files. Value files and
Linear/Cyclic Record files need `GetValue` (`0x6C`) and `ReadRecords`
(`0xBB`). The command must follow the file type from `GetFileSettings`, and a
short or long record response must not be reported as success.

## Dispatch

`Service::desfire_read` selects the application, authenticates when a key is
supplied, reads and parses the file settings, then dispatches:

- Standard/Backup: `ReadData` with byte offset and length.
- Value: `GetValue`, four signed little-endian bytes; the result is checked
  against the decoded limits.
- Linear/Cyclic Record: `ReadRecords` with the record offset and record count;
  the response must be exactly `count * record_size` bytes.
- Transaction MAC: no data read path; the service returns `Unsupported`.

This replaces the previous raw `data_file_size` check, so `ReadData` can never
run against a Value or Record file.

## Record semantics

DESFire `ReadRecords` uses a record offset where 0 is the most recent record,
not the first record. The returned records are in chronological order. For a
file with `current_records` and a request at offset `o` with count `c`, the
absolute index of the first returned record is:

```text
first_record = current_records - o - c
```

The result carries `start_record` (the DESFire offset), `first_record`, the
record size, and the records. The CLI and GUI label records with their
absolute index. The offset must be below the current count, the count must be
positive and no greater than the remaining records, and an empty file accepts
only offset 0 with no count or a zero count and returns an empty record list.

## Rejections

- `GetValue` shorter or longer than four bytes.
- A value outside the decoded limits.
- A record offset at or beyond the current count.
- A record count of zero or beyond the remaining records.
- A `ReadRecords` response that is not exactly the requested byte count.
- A Transaction MAC file read.

`GetValue`, `ReadData`, and `ReadRecords` consume DESFire additional frames
(`0x91AF`) and require a final success status. A read that returns additional
frames without the final status is an error.

## Surfaces

- `nfctl desfire read --aid --file [--key] [--key-number] [--offset]
  [--length] [--start-record] [--record-count]` prints the decoded settings and
  the data, value, or records.
- The GTK `DESFire → Read` tab has the same fields and dispatch.
- `--start-record`/`Start record` is the DESFire record offset; 0 means the
  most recent record. `--record-count`/`Record count` defaults to the rest.
- Offset/length apply to Standard/Backup files; start record/count apply to
  record files.

## Hardware validation

The gated test in `nfctl/tests/hardware_desfire_files.rs` creates a reserved
application (`F0 0D 03`) with:

- a value file, limits -50..50, limited credit and free GetValue enabled;
- a linear record file with 8-byte records and four slots, two records
  written;
- a cyclic record file with 4-byte records and two slots, one record written.

It reads the value, the full linear range, a single linear record at offset 1,
the latest linear record at offset 0, and the cyclic record, then rejects a
count beyond the current records. The application is removed afterwards. Set
`NFC_FOUNDRY_HARDWARE_KEEP_APP=1` to keep the fixture for manual CLI/GUI
checks.

## Non-goals

- Value credit/debit/limited-credit and record writes (26.10).
- `ClearRecordFile`.
- Secure messaging for MACed or enciphered file communication.
- Transaction MAC read/write.
