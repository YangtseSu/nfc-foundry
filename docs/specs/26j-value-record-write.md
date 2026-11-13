# Spec: phase 26.10 DESFire value, record, and transaction writes

Status: complete 2026-11-14 for value, record, and Transaction MAC writes;
Transaction Timer configuration remains blocked on the non-public PDCap2.2
encoding. Plan: [26-desfire-workbench.md](../plan/26-desfire-workbench.md).
Note: [26j-value-record-write.md](../notes/26j-value-record-write.md).

## Scope

- Credit, debit, and limited-credit a Value file, then commit and read the value
  back.
- Append, update, and clear Linear and Cyclic Record files, then commit and read
  every record back.
- Create a Transaction MAC file (already available from 26.9), read its counter
  and value, and verify the Transaction MAC returned by CommitTransaction.
- Refuse value changes outside the declared limits and record changes outside
  the record bounds before the card command.
- CLI and GUI expose the same service calls, gates, and results.

## Value operations

`Service::desfire_value_change` takes the application and file, one operation,
an unsigned amount, the optional application key, and an optional
AppTransactionMACKey.

- Credit and Debit require a positive amount; LimitedCredit accepts zero, which
  the card documents as a reset of the limited-credit pool.
- An amount above `0x7FFFFFFF` is refused.
- GetValue is read first with the communication mode granted by the file:
  free GetValue runs in Plain; a matching Read, Write, or Read/Write key uses
  the file mode; a free right runs in Plain.
- The expected result is computed with 64-bit arithmetic and refused when it
  falls outside the file's lower/upper limits.
- LimitedCredit is refused when the file does not enable it.
- Credit requires the Read/Write access right. Debit is granted by Read, Write,
  or Read/Write. LimitedCredit is granted by Write or Read/Write. A matching
  key uses the file communication mode; a free right runs in Plain.
- The card change is committed with CommitTransaction and the committed value
  must equal the expected value.

## Record operations

`Service::desfire_record_change` takes the application and file, one operation,
the record data, the optional key, and the optional AppTransactionMACKey.

- The complete current record list is read before the mutation. If the same
  session cannot read it, the mutation is refused because it cannot be verified.
- Append requires exactly one complete record of the file's record size. A full
  Linear Record file is refused; a Cyclic Record file overwrites its oldest
  record.
- Update takes a record number where 0 is the newest record, plus a byte range.
  The range must stay inside the record and the data must fit one UpdateRecord
  command (200 bytes). The record number must be below the current record
  count.
- Clear is refused on an already empty file.
- WriteRecord and UpdateRecord require Write or Read/Write; ClearRecordFile
  requires Read/Write. WriteRecord and UpdateRecord use the file communication
  mode; ClearRecordFile is MACed under EV2 and Plain under EV1, independent of
  the file mode.
- The mutation is committed, the record list is read again, and it must equal
  the exact expected chronological list: the appended record, the one updated
  range, or an empty list after clear. The reported current record count must
  match the list length.

## Transaction MAC

A Transaction MAC file stores a 4-byte counter and an 8-byte value. When the
selected application has one, CommitTransaction is sent with option `0x01` and
must return the committed counter and TMV.

Verification steps:

1. The returned counter and TMV are compared with a ReadData read of the
   Transaction MAC file. A mismatch fails the operation.
2. When the AppTransactionMACKey is supplied, the service obtains the card UID
   from the authenticated session and recomputes the TMV over the Transaction
   MAC Input accumulated from every GetValue, ReadData, ReadRecords, WriteData,
   Credit, Debit, LimitedCredit, WriteRecord, UpdateRecord, and
   ClearRecordFile command since application selection.
3. The recomputation derives `SesTMMACKey = AES-CMAC(AppTransactionMACKey,
   5A 00 01 00 80 || TMC || UID)`, then `TMV = even-byte-truncated
   AES-CMAC(SesTMMACKey, TMI)`, with the committed counter in little-endian
   order. A mismatch fails the operation.

Without the AppTransactionMACKey the result is reported as unverified, with the
reason; it is never labelled valid. A Transaction MAC file read is excluded from
the Transaction MAC Input.

## Transaction Timer

Transaction Timer configuration is not exposed. The public comparison note
documents SetConfiguration option `0x05` and PDCap2.2, but the bit layout and
timer unit are only in the NDA-only EV3 data sheet and features note. No
unverified configuration byte is written. See the phase note for the evidence.

## Surfaces

- CLI: `nfctl desfire value --credit|--debit|--limited-credit` and
  `nfctl desfire record --append|--update|--clear`, with the same key,
  Transaction MAC key, and data options.
- GUI: `DESFire → Values & records` with the same operations and fields.
- Both print the old and new value, or the full before/after record lists, and
  the Transaction MAC verification state.

## Tests

- `nfc-desfire` unit tests cover the value, update, and clear command payloads,
  TMI padding, a TMV vector, the CommitTransaction MAC response, and the
  exclusion of Transaction MAC file reads.
- `nfc-app` scripted tests cover credit/debit/limited-credit, limit refusals,
  append/update/clear, full-linear refusal, cyclic overwrite, record-boundary
  refusal, TMAC verification with the correct and a wrong key, and the
  unverified report without a key.
- GTK tests assert the new tab, default fields, and hex parser behaviour.
- Hardware: `nfc-desfire --features hardware --test hardware_value_record`
  creates a reserved AES application with value, linear, cyclic, and Transaction
  MAC files, verifies every commit's TMV, and deletes the application. It needs
  `NFC_FOUNDRY_HARDWARE=1`, `NFC_FOUNDRY_HARDWARE_WRITE=yes`, and
  `NFC_FOUNDRY_DEVICE`; `NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2` runs it under EV2,
  and `NFC_FOUNDRY_HARDWARE_FORMAT=yes` erases tombstones first.

## Out of scope

- Transaction Timer configuration until the encoding is documented.
- Chunking one UpdateRecord across multiple transactions.
- Business value semantics, wallets, transit, loyalty, or ticketing.
- Automatic continuing after a failed operation.
