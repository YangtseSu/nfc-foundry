# Spec: phase 26.9 DESFire application and file management

Status: complete 2026-11-13; ACR122U recorded, PN532 UART not run (the DESFire
card was on the ACR122U). Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Note:
[26i-management.md](../notes/26i-management.md).

## Scope

Create and delete applications; create, delete, and inspect all six file
types; change file settings, access rights, and communication mode; show free
memory and refuse a create that cannot fit; export a manifest before deletion.
Value/record/transaction writes stay in 26.10 and key management stays in
26.11.

## Protocol contract

- `CreateApplication` (`0xCA`) uses the typed key count and key family:
  KeySettings2 bits 6-7 are 00 for 2K3DES, 01 for 3K3DES, and 10 for AES; the
  low nibble is the key count 1..=14.
- `DeleteApplication` (`0xDA`) is sent while the PICC master application is
  selected and authenticated.
- `DeleteFile` (`0xDF`) takes the file number and is sent while the
  application is selected and authenticated.
- `CreateTransactionMACFile` (`0xCE`) carries a plaintext header
  (`FileNo || FileOption || AccessRights || TMACKeyOption`) and an
  **enciphered** data part (`TMACKey || TMACKeyVersion`). The card updates the
  separate AppTransactionMACKey with this command, so a legacy DES/2K3DES
  session is refused. `TMACKeyOption` is `0x02` (AES-128) and no counter limit
  is created. The access word must set the write right to denied and the
  read/write right to key 0..=4, free, or denied; other combinations are
  refused before the command.
- `ChangeFileSettings` (`0x5F`) carries `FileNo` as the plaintext header and
  `FileOption || AccessRights` as the data part. The file's current Change
  access right selects the protection:
  - `Free`: plain command without authentication;
  - `Key(n)`: enciphered command authenticated with key `n`;
  - `Denied`: refused before the command.
- Standard and Backup files accept a new communication mode. Value, record,
  and Transaction MAC files accept only access-right changes and must keep the
  mode set at creation.
- `GetFreeMemory` (`0x6E`) returns three little-endian bytes.

`GetFileSettings` reports the LimitedCredit amount for a Value file, not the
current value; the current value comes only from `GetValue`. The amount is not
bounded by the value limits.

## Preflight and safety

Before a create:

- The selected card is still the same ISO-DEP card.
- The application or file does not already exist.
- The free EEPROM count is read and a file whose content bytes exceed it is
  refused before the command. The card's per-file overhead is not modelled, so
  a value at the limit can still be refused by the card.
- The new settings are read back and compared with the request. A Value file's
  initial value is read back with `GetValue`.
- The application ID list is read back after an application create.

Before a delete:

- Advanced mode and explicit confirmation are required.
- A JSON manifest is written atomically before the first destructive command.
  It records the card version, free memory, application ID, file IDs, decoded
  settings, and raw settings bytes. It is not file content and cannot restore a
  deleted application or file.
- The application ID list or file ID list is read back afterwards. A refused or
  partially applied deletion is an error.
- Free memory is reported before and after when the card answers; deleted
  DESFire applications keep their EEPROM until `FormatPICC`.

## API

- `Desfire::create_application_from(&ApplicationSpec)`.
- `Desfire::delete_file(file_id)` and `Desfire::change_file_settings(...)`.
- `Desfire::create_transaction_mac_file(&TransactionMacFileSpec)`.
- `Service::desfire_create_application`, `desfire_delete_application`,
  `desfire_create_file`, `desfire_delete_file`,
  `desfire_change_file_settings`, `desfire_free_memory`.
- `AccessRight::parse`, `AccessRights::from_parts`, and
  `AccessRights::from_raw` for the shared CLI/GUI access-word input.

## Surfaces

- CLI: `nfctl desfire memory`, `app-create`, `app-delete`, `file-create`,
  `file-delete`, and `file-settings`.
- GUI: DESFire → Manage with the same fields, a shared Advanced switch, and
  typed `DELETE` confirmation for application and file deletion.
- Deletion output names the manifest path and states that no byte-for-byte
  restore exists. Keys and the Transaction MAC key are never printed.
- Phase 26.13 later requires Advanced mode and a typed `CHANGE` confirmation
  for `file-settings`; the current CLI adds `--advanced`/`--dry-run` and the
  GUI shows the current and requested settings before the command.

## Verification

- `nfc-desfire` unit tests: access-word encoding and parsing, management
  payloads, and an AES-session round trip that creates all six file types,
  changes settings, deletes a file, and deletes the application on the
  scripted card.
- `nfc-app` tests: application create and duplicate/zero-memory refusal, all
  six file types, capacity refusal, settings change and read-back, manifest
  content for file and application deletion, unknown-application refusal, and
  gate ordering.
- CLI tests: application create, free-memory output, file create and capacity
  refusal, free-change settings, manifest paths, and Advanced gating.
- GTK test: the Manage tab, button sensitivity, access-word parsing, and file
  spec parsing for Standard, Value, and Transaction MAC.
- Hardware: PN532 UART and ACR122U `acr122_usb:001:002` created reserved AES
  `F0 0D 08`, created all six file types, changed the Standard file to
  enciphered with new access rights, deleted all six files and the application
  with manifests, and confirmed the application list empty. The CLI cycle also
  passed on both readers.
