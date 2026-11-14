# Spec: DESFire NDEF replace and FormatPICC (26.12)

Date: 2026-11-14. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Note:
[26l-ndef-replace-format.md](../notes/26l-ndef-replace-format.md).

## Scope

Extend the phase 25.5 DESFire NDEF service with two structure changes:

- **Replace**: read the current Type 4 NDEF message, delete only the validated
  AN11004 NDEF Tag Application, and provision it again with the same CC and an
  empty NDEF file.
- **FormatPICC**: authenticate the PICC master key and run the full-card
  reset that erases every application and file.

Ordinary provisioning (`format-ndef`) and the guarded undo (`delete-ndef`)
keep their phase 25.5 behavior. No read or inventory operation formats a card.
No other workflow calls FormatPICC.

## Command encodings

### DeleteApplication (`0xDA`)

- Data: the three native AID bytes `00 76 D2`.
- The replace path deletes only after the exact layout check used by
  `delete-ndef`: file IDs `01`/`02`, plain communication, AN11004 sizes, and
  the exact CC bytes.
- Authentication is a current PICC master-key session. A wrong key fails once.

### FormatPICC (`0xFC`)

- No data. Requires a PICC master-key session; the card erases all
  applications and files and resets the PICC master key and its settings to
  the factory state.
- The service reads the application list and the version before the command,
  then reads `GetApplicationIDs` (`0x6A`) and `GetFreeMemory` (`0x6E`) after
  it. A non-empty application list after the command is a verification
  failure.
- Deleted applications keep their EEPROM tombstones until this command, so
  the free-memory value normally grows to the full card size.
- The ISO-DEP identity check (ATQA, SAK, stable UID suffix) runs before the
  result is reported.

## Service API

`Service::desfire_ndef_replace(DesfireNdefReplaceRequest)`:

- gates: Advanced mode, explicit confirmation, key number `0..=14`, and a
  valid key for the selected family;
- requires the NDEF application to exist and matches the AN11004 layout before
  any mutation;
- reads the current NDEF file through `nfc_type4::read_tag`; a read failure
  aborts before the delete, so an unreadable message is never silently lost;
- deletes the validated application, rechecks the ISO-DEP identity, and calls
  the existing format flow;
- returns the previous `Type4Ndef` and the new CC and empty NDEF;
- if provisioning fails after the delete, the error states that the
  application was deleted and not recreated. A failed format still attempts
  its own rollback of a partially created application.

`Service::desfire_format_picc(DesfireFormatPiccRequest)`:

- gates: Advanced mode, explicit confirmation, key number `0..=14`, and a
  valid key for the selected family;
- returns the application list before the reset, the read-back list (empty),
  the free EEPROM bytes, and the card name;
- never handles a byte-for-byte card backup and never exports a key.

Both request types keep the key private and do not implement `Debug`.

## CLI

- `nfctl desfire replace-ndef --key <HEX> [--key-type T] [--key-number N]
  --advanced [--yes] [--dry-run]`
- `nfctl desfire format-picc --key <HEX> [--key-type T] [--key-number N]
  --advanced [--yes] [--dry-run]`

`--dry-run` is read-only: replace prints the current CC and NDEF state;
FormatPICC lists every application and its file count. `--yes` supplies the
same explicit confirmation as the GUI dialog; the service still verifies the
Advanced and confirmation flags.

## GUI

- `DESFire → NDEF → Replace NDEF application…` uses the `PICC master key`
  fields on that tab and a typed `REPLACE` confirmation. The dialog shows the
  current NDEF state and states that a failure after deletion leaves no NDEF
  application.
- `DESFire → Manage → Format PICC (erase all)` uses the Manage key fields and
  a typed `ERASE` confirmation. The dialog lists the applications to erase and
  states that the PICC key returns to the factory state and that FormatPICC is
  never part of NDEF provisioning.
- Both buttons are disabled unless Advanced mode is on. Both call the same
  service methods as the CLI and refresh the inventory after success. A
  successful FormatPICC clears the Manage key field because the key value on
  the card changed.

## Restrictions

- Replace needs free EEPROM for a second copy of the NDEF application because
  the deleted application keeps its bytes until FormatPICC. When the card
  cannot fit the new copy, replace reports the deletion and the failed
  provisioning; FormatPICC is the recovery path.
- FormatPICC erases every application on the card, including applications
  created by other tools. It is not a selective action and has no undo.
- The reset is not executed on the owned card with a changed PICC key; the
  factory-key check is the only post-reset key evidence on hardware.

## Acceptance mapping

| Requirement | Evidence |
|---|---|
| Reuse the phase 25.5 provisioning service for the read-back | `desfire_ndef_replace` calls `format_ndef`, which reads CC/NDEF through `nfc-type4` |
| Delete only the NDEF application created by this product | layout check before delete; mock test with a foreign layout and hardware guarded delete |
| Explicit replace path after a separate confirmation | service gates + typed `REPLACE` dialog + CLI `--yes`; mock and PN532 tests |
| FormatPICC as a separate full reset | `Service::desfire_format_picc`, CLI `format-picc`, GTK Manage action; no NDEF path calls it |
| Refuse provisioning when a valid NDEF application exists | `ensure_ndef_absent`; repeated format test |
| FormatPICC is not used for ordinary provisioning | replace test asserts zero `0xFC` commands; mock test asserts the NDEF app remains |
| No formatting by read or inventory | read/inventory methods contain no `0xFC` call; command-count tests |
| Hardware record | PN532 UART 2026-11-14: NDEF format/write/clear/replace/delete and FormatPICC with `NFC_FOUNDRY_HARDWARE_FORMAT=yes`; CLI cycle recorded in the note |
