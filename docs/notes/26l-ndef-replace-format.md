# Note: DESFire NDEF replace and FormatPICC (26.12)

Status: complete 2026-11-14 on PN532 UART and ACR122U. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26l-ndef-replace-format.md](../specs/26l-ndef-replace-format.md).

## Implementation

- `Service::desfire_ndef_replace` reads the current Type 4 NDEF file, deletes
  only the AN11004 application created by this product, rechecks the ISO-DEP
  identity, and calls the phase 25.5 format flow.
- `Service::desfire_format_picc` authenticates the PICC master key, sends
  FormatPICC (`0xFC`), and reads the application list and free EEPROM back.
- `delete_ndef` was split into the layout check and the validated delete, so
  replace and undo share one validation rule.
- The NDEF request gates now share `validate_ndef_request`; the error text for
  format and delete is unchanged.
- CLI: `desfire replace-ndef` and `desfire format-picc`, both Advanced and
  confirmation gated with read-only `--dry-run`.
- GTK: `Replace NDEF application` on the NDEF tab (`REPLACE`) and
  `Format PICC (erase all)` on the Manage tab (`ERASE`). A successful
  FormatPICC clears the Manage key field.
- No NDEF read, inventory, format, or delete path calls FormatPICC.

## Mock card

The `nfc-desfire` scripted card now answers the ISO 7816-4 commands the Type 4
reader uses: SELECT by DF name `D2760000850101`, SELECT FILE for `E103`/`E104`,
READ BINARY, and UPDATE BINARY. It also accepts ISO file IDs in create-file
payloads (the GetFileSettings response still omits them, as on hardware) and
implements FormatPICC: erase every application, restore the free-memory total,
and reset key 0, KeySettings, and the key count to the factory state. The mock
implements FormatPICC for the DES path used by the tests; EV2 management
protection is not simulated.

## Tests

- `crates/nfc-app/tests/desfire_ndef.rs`: 10 tests covering replace gates, a
  missing or foreign NDEF application, an unreadable previous NDEF file,
  message read-back and re-provisioning, a failed second copy after the
  delete, repeated format refusal, FormatPICC gates, a wrong PICC key, and the
  erase/factory-key reset.
- `crates/nfctl/tests/cli.rs`: FormatPICC dry-run, Advanced and confirmation
  refusals, the mock erase result, and the replace refusal without an
  application.
- GTK `desfire_workspace_switches_and_parses_inputs`: the new controls are
  present and Advanced gated.
- `cargo test --workspace` passed without hardware access.

## PN532 UART hardware record

Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6. Card:
MIFARE DESFire EV3 8K, random UID, factory DES/2K3DES zero PICC key.

### NDEF replace

The gated test
`formats_writes_restores_and_deletes_the_ndef_application` passed on
2026-11-14. It formatted `D27600`, read the empty NDEF file, wrote one Text
record, cleared it, wrote the record again, replaced the application, read the
previous message back from the result, verified an empty NDEF file through the
Type 4 reader, deleted the application, and compared the final application
list with the pre-format list (`applications: none`).

```text
formatted aid=[00, 76, D2] cc=15 state=Empty
test formats_writes_restores_and_deletes_the_ndef_application ... ok
```

### FormatPICC

The gated test `format_picc_erases_every_application` passed on 2026-11-14
with `NFC_FOUNDRY_HARDWARE_FORMAT=yes`. It refused FormatPICC without
Advanced mode and without confirmation, created the scratch application
`F0 0D 21`, formatted the PICC, and read back:

```text
applications before the test: []
formatted MIFARE DESFire EV3 (1 applications erased, 8192 bytes free, factory key check)
test format_picc_erases_every_application ... ok
```

The post-reset PICC key check used the factory zero DES key because the owned
card's PICC key was never changed. The mock test covers the reset of a
non-factory key.

### CLI cycle

The CLI paths ran on the same card and reader:

```text
nfctl ... desfire format-ndef --key 0000000000000000 --advanced
  NDEF application formatted · NDEF file: empty
nfctl ... desfire ndef-read
  NDEF file E104 · 255 bytes · read free · write free · empty
nfctl ... desfire replace-ndef --key 0000000000000000 --advanced
  NDEF application D27600 replaced; previous NDEF: empty
nfctl ... desfire delete-ndef --key 0000000000000000 --advanced
  NDEF application D27600 deleted · applications: none
nfctl ... desfire format-picc --key 0000000000000000 --advanced
  MIFARE DESFire EV3 formatted · applications erased: 0
  applications after: none · free memory: 8192 bytes · PICC key: factory state
```

After the cycle `desfire info` reported no applications. The replace run had an
empty previous message; the hardware test above covered a non-empty previous
message.

## ACR122U hardware record

Recorded 2026-11-14 on `acr122_usb:001:002` with `pcscd` stopped, owned EV3
8K. Both gated tests passed:

- `formats_writes_restores_and_deletes_the_ndef_application` formatted
  `D27600`, wrote/cleared/re-wrote a Text record, replaced the application
  with the previous message read back, verified an empty NDEF through Type 4,
  deleted it, and compared the final application list with the pre-format
  list.
- `format_picc_erases_every_application` ran with
  `NFC_FOUNDRY_HARDWARE_FORMAT=yes`, created scratch `F0 0D 21`, erased every
  application, read back 8192 free bytes, and verified the factory DES PICC
  key.

The CLI `format-ndef`/`ndef-read`/`replace-ndef`/`delete-ndef`/`format-picc`
cycle passed on the same reader, leaving `applications: none` and 8192 free
bytes.

## Free-memory behavior

Replace deletes the old application before creating the new one. Deleted
applications keep their EEPROM until FormatPICC, so the card needs room for
both copies. If the second copy does not fit, the service reports that the
application was deleted and provisioning failed. The mock test
`replace_reports_a_failed_reprovision_after_the_delete` covers this path;
FormatPICC is the recovery action.
