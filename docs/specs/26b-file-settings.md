# Spec: phase 26.2 DESFire file settings decode

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md).

## Problem

`GetFileSettings` returns a type-specific byte string. The inventory printed
only the raw bytes, so neither surface showed the communication mode, access
conditions, capacity, value limits, record limits, or Transaction MAC fields.
The GUI must never label a file type it cannot decode.

## Model

`nfc-desfire` adds:

- `FileType`: Standard, Backup, Value, LinearRecord, CyclicRecord,
  TransactionMac.
- `CommunicationMode`: Plain, Maced, Enciphered. Mode 2 is rejected.
- `AccessRight` (`Key(n)`, `Free`, `Denied`) and `AccessRights` with read,
  write, read/write, change, and the raw 16-bit word.
- `FileContents`: `Data { size }`, `Value { lower_limit, upper_limit,
  limited_credit_value, limited_credit, free_get_value }`, `Record {
  record_size, max_records, current_records }`, `TransactionMac { key_option,
  key_version, counter_limit }`. The third value field is the LimitedCredit
  amount, not the current value; `GetValue` is the authoritative current value
  (phase 26.3). The amount is not bounded by the value limits.
- `FileSettings::{parse, raw, summary}`; `FileInfo::decoded`.
- `nfc-app::desfire::inventory_text`, shared by `nfctl` and the GTK inventory.
  It prints the decoded summary and the raw settings for every file; a parse
  error prints `undecoded · <reason>` instead of a guessed file type.

All multi-byte type-specific fields are little-endian 24- or 32-bit values.
The raw response is retained for diagnostics.

## Header and access rights

The first four bytes are file type, file option, and a little-endian 16-bit
access word. The option's low two bits are the communication mode, bit `0x80`
enables additional access rights, and bit `0x20` is valid only for Transaction
MAC files. The access word encodes:

- byte 0: read/write high nibble, change low nibble;
- byte 1: read high nibble, write low nibble;
- nibbles `0x0`-`0xD` are key numbers, `0xE` is free, `0xF` is denied.

When the additional-rights bit is set, one count byte (1..=13) follows the
type-specific fields, then count two-byte access words for keys 1..n.

## Rejections

The parser rejects:

- fewer than four bytes, truncated type fields, or trailing bytes without the
  additional-rights bit;
- unknown file types and communication mode 2;
- the counter-limit bit on a non-Transaction-MAC file;
- a zero data-file size, zero record size, zero maximum record count, or a
  current record count above the maximum;
- value limits that are not increasing or reserved value flags;
- a Transaction MAC counter limit whose declared size is missing;
- an additional-rights count outside 1..=13 or a length that does not match
  the count.

A malformed response is an error; it is never displayed as a known type.

## Transaction MAC layout

After the access word: key option, key version, then an optional counter limit.
The header's `0x20` bit selects whether the counter limit is present. The key
option's `0x02` bit selects a four-byte counter limit; otherwise two bytes are
used. The counter limit is read big-endian, matching the EV3 settings layout.
This layout is covered by unit vectors. A Transaction MAC file created on the
owned EV3 8K on 2026-11-13 reported key option `0x02`, key version 1, and no
counter limit through GetFileSettings.

## Surfaces

- `nfctl desfire info` prints the shared decoded inventory.
- The GTK `DESFire → Inventory` tab prints the same text.
- Raw settings remain visible on both surfaces.
- The NDEF application's `E103`/`E104` standard files were decoded on PN532
  UART on 2026-11-12. Value, Linear Record, Cyclic Record, and Transaction MAC
  vectors are unit-tested only; no hardware record exists for those types yet.

## Non-goals

- Value/GetValue and record reads (26.3).
- File creation for all six types (26.9).
- ChangeFileSettings and access-right editing.
- Value, record, or Transaction MAC writes.
