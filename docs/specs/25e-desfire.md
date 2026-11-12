# Spec: phase 25.5 DESFire EV3 8K inventory

Status: inventory implemented and hardware-validated 2026-11-12; the factory
legacy DES key authenticates, but NDEF formatting and user-facing writes are
not implemented. Plan: [25-card-support.md](../plan/25-card-support.md).

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
- Legacy DES/2K3DES authentication (`0x0A`, D40 send mode), session-key
  derivation, and random RndA.
- Primitive application/file commands (`CreateApplication`,
  `DeleteApplication`, `CreateStdDataFile`, `WriteData`, `ReadData`) for the
  hardware test; no CLI or GUI surface is added.
- `nfctl desfire info` prints the version and the application/file inventory.
- `nfctl desfire read` authenticates with an optional legacy key and reads one
  standard or backup file. It is a developer diagnostic; the GUI workspace is
  still pending.
- `nfctl desfire info` is read-only. `nfctl desfire read` only reads after an
  optional authentication; neither command writes. The hardware test creates
  and removes a reserved temporary application only when both write gates are
  set.

## Formatting gate

AN11004 formatting requires an authenticated PICC session:

- create application `D27600` with the ISO DF name `D2760000850101`;
- create `E103` (CC) and `E104` (NDEF) files;
- write the 15-byte CC;
- verify through the existing Type 4 reader.

The owned factory-blank card authenticates with legacy `Authenticate` (`0x0A`),
an 8-byte DES zero key, and D40 send mode for the second frame. A 16-byte
2K3DES zero key works identically; AES EV2 (`0x71`) and 3K3DES (`0x1A`) do not.
The earlier default-key dictionary failed on framing, not on the key.

A full `FormatPICC` is only needed to erase existing applications. It is not
required to create the NDEF application on an otherwise blank PICC. No
format/delete command is exposed until:

- legacy authentication and the command session are implemented and
  hardware-validated; and
- the create/undo path is hardware-validated.

## Acceptance

- `nfctl desfire info` reports `MIFARE DESFire EV3`, version 51.0, storage
  `0x1A`, and `applications: none` on the owned card.
- The operation performs no authentication and no write.
- The hardware test authenticates with the factory DES zero key, creates the
  reserved temporary application and a standard file, reads back the written
  payload, deletes the application, and leaves `GetApplicationIDs` empty.
- No DESFire format command exists in the CLI or GUI yet.
