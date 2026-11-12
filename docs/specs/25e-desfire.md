# Spec: phase 25.5 DESFire EV3 8K inventory

Status: inventory implemented and hardware-validated 2026-11-12; NDEF
formatting blocked on the card's PICC key. Plan:
[25-card-support.md](../plan/25-card-support.md).

## Problem

The owned DESFire EV3 8K has no NFC Forum Type 4 NDEF Tag Application, so the
Type 4 adapter stops at `6A82`. Before creating one, the product needs a native
DESFire command layer and a read-only inventory.

## Implemented

- New pure `nfc-desfire` crate over `Reader::transceive_apdu`.
- ISO 7816-4 wrapping for DESFire native commands, including the 5-byte
  no-data APDU form.
- `GetVersion` with additional-frame chaining.
- `GetApplicationIDs`, `SelectApplication`, `GetFileIDs`, `GetFileSettings`.
- `nfctl desfire info` prints the version and the application/file inventory.
  It is a developer diagnostic; no GUI path is added until formatting is
  unblocked.
- The operation is read-only; it never authenticates or writes.

## Formatting gate

AN11004 formatting requires an authenticated PICC session:

- create application `D27600` with the ISO DF name `D2760000850101`;
- create `E103` (CC) and `E104` (NDEF) files;
- write the 15-byte CC;
- verify through the existing Type 4 reader.

The owned card uses a response to `AuthenticateISO` (`0x1A`), not EV2/AES. The
published PM3 default dictionary was tried; 54 candidates, including AES and
DES/2K3DES defaults, all failed. The PICC key is therefore custom or otherwise
unknown.

No format/delete command is exposed until:

- the PICC key is known, or PM3 identifies it; and
- the matching secure channel (EV1/2K3DES or EV2/AES) is implemented and
  hardware-validated; and
- the delete/undo path is hardware-validated.

## Acceptance

- `nfctl desfire info` reports `MIFARE DESFire EV3`, version 51.0, storage
  `0x1A`, and `applications: none` on the owned card.
- The operation performs no authentication and no write.
- No DESFire format command exists in the CLI or GUI while the key is unknown.
