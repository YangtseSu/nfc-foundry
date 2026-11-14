# Note: phase 26.10 DESFire value, record, and transaction writes

Date: 2026-11-14. Spec:
[26j-value-record-write.md](../specs/26j-value-record-write.md). Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md).

## Implemented

- `nfc-desfire` adds Credit (`0x0C`), Debit (`0xDC`), LimitedCredit (`0x1C`),
  UpdateRecord (`0xDB`), ClearRecordFile (`0xEB`), and CommitTransaction with
  the optional returned Transaction MAC.
- `Desfire` accumulates the Transaction MAC Input for every command in the
  DESFire list, pads each command to a 16-byte boundary, resets the input on
  application selection, commit, and abort, and excludes reads of the
  Transaction MAC file.
- `transaction_mac` derives the TMAC session key and computes the 8-byte TMV
  from the input, counter, UID, and AppTransactionMACKey.
- `Service::desfire_value_change` and `Service::desfire_record_change` do the
  access-right and bounds preflight, commit, read back, and verify.
- `nfctl desfire value` and `nfctl desfire record` expose the operations; the
  GTK `Values & records` tab calls the same services.
- The scripted card models pending value and record changes, commit/abort,
  record open/append/overwrite, UpdateRecord, ClearRecordFile, and a
  configurable commit MAC response.

## Command and verification decisions

- GetValue is granted by Read, Write, or Read/Write. Credit is granted by
  Read/Write; Debit by Read, Write, or Read/Write; LimitedCredit by Write or
  Read/Write. ClearRecordFile is granted by Read/Write. These follow the
  MF2DL(H)x0 command error tables, and the hardware run agreed.
- Record numbers follow ReadRecords and UpdateRecord: 0 is the newest record.
  The service converts the number to the chronological list index only for the
  expected-result check.
- Append sends one complete record. A record larger than one command is sent as
  consecutive WriteRecord calls in the same transaction, which the card applies
  to the record opened by the first call.
- ClearRecordFile protection depends on the secure channel, not the file mode.
  Under AES EV2 the command is MACed; under EV1 and legacy sessions the command
  is Plain and only the response is MACed. Sending the other mode makes the
  EV3 return a length error (0x917E); both modes were checked on hardware.
- CommitTransaction option `0x01` is sent only when the application has a
  Transaction MAC file. The returned counter and TMV are cross-checked against
  a ReadData read of the file. With the key supplied, the TMV is recomputed and
  a mismatch is a verification failure; without the key the result is shown as
  UNVERIFIED with the reason.
- The Transaction MAC counter is little-endian in the key-derivation vector.
  The TMV uses the even-byte truncation of the AES-CMAC, matching AES secure
  messaging. The PN532 run confirmed both choices against the card.

## PN532 UART hardware record

Run on 2026-11-14 on `pn532_uart:/dev/ttyUSB0` with the owned DESFire EV3 8K.
The environment-gated test
`nfc-desfire --features hardware --test hardware_value_record` passed. It
created AES application `F0 0D 0A` with a zero key, then a Value file, a Linear
Record file, a Cyclic Record file, and a Transaction MAC file, and removed the
application afterwards.

The environment-gated test was run once with AES EV1 and once with AES EV2
(`NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2`); both passed. After repeated runs fill
the test card's EEPROM tombstones, `NFC_FOUNDRY_HARDWARE_FORMAT=yes` erases the
PICC first. The CLI run used AES
application `F0 0D 0B` with one AES key:

- Value file `01`: plain, key-0 rights, limits -50..50, initial 20, flags
  limited credit and free GetValue.
- Linear Record file `02`: 8-byte records, maximum 4.
- Cyclic Record file `03`: 4-byte records, maximum 2.
- Transaction MAC file `0F`: read free, write and read/write denied, change
  key 0, AES key `11…11`, version 1.

Observed results:

```text
credit 5       value 20 -> 25, committed, read back 25, no TMAC file
debit 3        value 25 -> 22, committed, read back 22
limited 3      value 22 -> 25, committed, read back 25
append linear  before 0 records, after 1 x 8 bytes, TMV verified (TMC 1*)
append cyclic  before 0 records, after 1 x 4 bytes
clear cyclic   before 1 record, after 0 records
update linear  record 0 offset 0, before/after lists verified
```

\* The first TMAC-verified commit was a credit of 1 after the Transaction MAC
file was created, so its TMC was 1 and the value went from 25 to 26. Record
appends, the cyclic clear, and the linear update then committed TMC 2, 3, 4,
and 5. Every one reported:

```text
transaction MAC: file 0F · counter N · ... · verified: verified with the
AppTransactionMACKey; matches the stored Transaction MAC file
```

The hardware test also covered a second, Enciphered Value file and an
Enciphered Cyclic Record file: the value credit, record append, and clear read
back correctly, and their TMVs verified under both AES EV1 and AES EV2. The
clear used the EV1 Plain command and the EV2 MACed command, as described above.

An incorrect AppTransactionMACKey was rejected after the commit:

```text
verification failed: the Transaction MAC for counter 6 does not verify with the
supplied AppTransactionMACKey
```

Preflight refusals, with no card command:

```text
credit 100     invalid argument: ... would take the value from 27 to 127,
               outside the limits -50..50
update offset 6 length 3
               invalid argument: update range 6..9 is outside the 8-byte record
clear empty    invalid argument: record file 03 is already empty
```

All four files and the application were deleted with manifests; the final
`nfctl desfire info` reported `applications: none`. Free memory stayed at 6752
bytes after deletion, the DESFire tombstone behaviour already recorded for the
other reserved applications.

## ACR122U record

Recorded 2026-11-14 on `acr122_usb:001:002` with `pcscd` stopped. The
environment-gated `hardware_value_record` test passed under AES EV1 and again
under AES EV2 (`NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2`). It created reserved
`F0 0D 0A`, ran the value credit/debit/limited-credit and record
append/update/clear mutations, cross-checked every Transaction MAC with the
supplied key, deleted the application, and left `applications: none`. The same
test had previously recorded the PN532 UART row; the card is now available on
the ACR122U and the row is validated on both readers.

## Transaction Timer blocker

The EV3 supports the Transaction Timer and the public comparison note documents
that SetConfiguration option `0x05` carries PDCap2.2, which enables and
configures it (AN12752 Table 5 and section 7.4). The bit layout, range, and time
unit are in the NDA-only EV3 data sheet DS487033 and the features note
AN12757; NXP's community answer confirms those documents need a permission
grant. Writing an invented PDCap2.2 byte could leave the card reset before the
disable command completes, so no Transaction Timer write was attempted.

The remaining implementation needs the PDCap2.2 layout. Once available, the
command layer needs only SetConfiguration (`0x5C`) option `0x05` and a
verification that a transaction actually expires.

## Checks

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-desfire --features hardware \
  --test hardware_value_record -- --ignored

NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-desfire --features hardware \
  --test hardware_value_record -- --ignored
```

Then:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
reuse lint
```

All passed. The GNOME/GTK unit test
`app::tests::desfire_workspace_switches_and_parses_inputs` covers the new tab
and hex parser.
