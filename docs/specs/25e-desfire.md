# Spec: phase 25.5 DESFire EV3 8K NDEF enablement

Status: inventory, factory-key authentication, NDEF provisioning, undo, the
CLI/GTK surfaces, and the PN532 UART hardware record are complete 2026-11-12.
The ACR122U row remains unrecorded. Plan:
[25-card-support.md](../plan/25-card-support.md).

## Problem

The owned DESFire EV3 8K has no NFC Forum Type 4 NDEF Tag Application, so the
Type 4 adapter stops at `6A82`. The product needs a native DESFire command
layer, a guarded AN11004 bootstrap, and a read-only inventory before it can
use the existing Type 4 NDEF path.

## Implemented

- New pure `nfc-desfire` crate over `Reader::transceive_apdu`.
- ISO 7816-4 wrapping for DESFire native commands, including the 5-byte
  no-data APDU form.
- `GetVersion` with additional-frame chaining.
- `GetApplicationIDs`, `SelectApplication`, `GetFileIDs`, `GetFileSettings`.
- Legacy DES/2K3DES authentication (`0x0A`, D40 send mode), session-key
  derivation, and random RndA.
- Application and file creation with ISO file IDs and an ISO 7816-4 DF name.
- `CommitTransaction` (`0xC7`) and `AbortTransaction` (`0xA7`).
- NDEF layout constants: application AID `D27600` (native bytes `00 76 D2`),
  app ISO ID `E110`, DF name `D2760000850101`, CC file `01`/`E103`, NDEF file
  `02`/`E104`, and the 15-byte mapping-version 2.0 CC.
- `nfctl desfire info` and `nfctl desfire read`.
- `nfctl desfire format-ndef` and `nfctl desfire delete-ndef`, both requiring
  `--advanced`; the CLI still asks for `--yes` or an interactive confirmation.
- A GTK `DESFire` workspace with `Inventory`, `Read`, and `NDEF` tabs. The NDEF
  tab has a session-scoped Advanced switch, a PICC key and key-number field,
  typed `FORMAT`/`DELETE` confirmation dialogs, and no key output.
- Both surfaces call the same `nfc-app` service methods.

## Formatting gate

AN11004 formatting requires an authenticated PICC session:

- create application `D27600` with DF name `D2760000850101` and ISO ID `E110`;
- select the new application and authenticate with its factory zero DES key;
- create `E103` (CC, 15 bytes, read free, write key 0) and `E104` (NDEF, 255
  bytes, read and write free) as plain files with ISO file IDs;
- write the 15-byte CC `00 0F 20 00 3B 00 34 04 06 E1 04 00 FF 00 00` and
  the empty NLEN;
- commit the transaction;
- reselect by DF name and verify the CC and empty NDEF file through the
  existing Type 4 reader.

The service refuses to start when the card already lists the NDEF application
or when the NDEF Tag Application selects successfully. It fails once on a
wrong key without retrying. If a step fails after the application was created,
the service aborts the open transaction and deletes the application as a
rollback. A rollback failure is reported together with the original error.

`DeleteApplication` is the undo path. The service deletes the application only
when its files are exactly `01`/`02`, both are plain, their sizes match the
AN11004 layout, and the CC matches the exact bytes written by this product. It
requires the PICC master key, verifies that the application is gone, and does
not claim a byte-for-byte restore.

The formatting dialog names the created application, both files, the key
source, and the absence of a full-card backup. It requires typing `FORMAT`;
deletion requires typing `DELETE`.

## ISO-DEP identity

The EV3 has random UID enabled, so the first UID bytes change on every
selection while the last four bytes remain stable. The Type 4 and DESFire
workflows use an ISO-DEP identity check that compares ATQA, SAK, and the
stable UID suffix. Classic and Type 2 operations keep the strict UID check.

## Non-goals

- General DESFire file/key management, secure messaging, and business
  templates. Those are tracked in [phase 26](../plan/26-desfire-workbench.md).
- `FormatPICC`; ordinary NDEF provisioning never erases other applications.
- Changing the PICC or application key.
- A user-facing byte-for-byte backup of a DESFire card.

## Acceptance

- `nfctl desfire info` reports the DESFire EV3 version and the current
  application list without authentication.
- `format-ndef` creates a readable Type 4 CC and a writable NDEF file on the
  owned factory-blank card.
- A Text record writes, reads back, and clears to the empty state.
- `delete-ndef` removes only the exact AN11004 application and leaves the
  pre-format application list.
- A wrong PICC key fails once and creates nothing.
- The Advanced and confirmation gates are enforced by the service, CLI, and
  GTK dialog.
- The PN532 UART hardware test passed on 2026-11-12 and is recorded in the
  note; the ACR122U row remains unrecorded until that reader is run.
