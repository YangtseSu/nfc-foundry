# Note: DESFire write gates and recovery behavior (26.13)

Status: complete 2026-11-14 on PN532 UART for the changed write paths. The
ACR122U row remains unrecorded. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26m-write-gates.md](../specs/26m-write-gates.md).

## Implementation

- `begin_file_session` is the shared DESFire preflight: ISO-DEP identity,
  application/PICC selection, and optional authentication. Read, write, value,
  record, inspect, file management, application management, key management,
  and the NDEF/FormatPICC sessions use it.
- `Service::desfire_inspect_file` reads file settings without changing state.
  The CLI and GUI use it to show the current settings before a file delete or
  a file-settings change.
- `DesfireWriteRequest` takes an exact `backup_path`. The service writes a
  mode-0600 JSON export with the card UID, application, file ID, type, summary,
  raw settings, and complete data before the first `WriteData`. An unwritable
  path aborts before any write. `DesfireWriteResult.backup_path` returns the
  path, and `write_text` prints it.
- `BackupStore::plan_path` reserves a unique private path. The CLI and GUI plan
  the write backup and the delete manifest path, show it in the dry run or
  confirmation dialog, and pass the exact path to the service.
- File settings changes now require Advanced mode and explicit confirmation.
  The CLI has `--advanced` and `--dry-run`; the GUI uses a read-only inspection
  and a typed `CHANGE` dialog with the current settings and requested mode and
  access word.
- Application and file delete dialogs list the current files/settings, the
  exact manifest path, the key source, and the read-back. The CLI prints the
  same fields before confirmation.
- KeySettings and key changes read the old byte/version before the dialog and
  show both the old and new value; key changes still require an export path or
  the recorded-elsewhere acknowledgement.
- The GTK Activity receives one structured row per DESFire mutation:
  `desfire <operation> · <succeeded|failed> · <target> · <detail>`. The write
  detail includes the backup path; delete details include the manifest path.
  Failures are not logged twice.
- The scripted DESFire transport can swap the card after N APDUs or fail the
  exchange after N APDUs, covering card-change and reader-loss paths.

## Tests

New or updated:

- `crates/nfc-app/tests/desfire_write.rs`: 14 tests, including the JSON backup
  content and mode 0600, an unwritable backup path aborting before `WriteData`,
  a pre-cancelled token sending no command, a card change reported as
  `CardChanged`, and a reader loss reported as an I/O error.
- `crates/nfc-app/tests/desfire_management.rs`: 15 tests, including the
  file-settings Advanced/confirmation gates and the read-only file inspection.
- `crates/nfctl/tests/cli.rs`: file-settings dry run and gates, planned write
  backup output, and the updated delete manifests.
- `crates/gui/src/app.rs`: the DESFire widget test checks the new file-settings
  and delete controls; the reader monitor test checks Advanced reset on card
  removal; the Activity split has a dedicated test.

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, and `reuse lint` pass.

## PN532 UART hardware record

Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6. Card:
MIFARE DESFire EV3 8K, factory DES/2K3DES zero PICC key.

Both runs formatted the card first because earlier fixtures left EEPROM
tombstones.

### Application and file management with the new gate

`hardware_desfire_management` passed on 2026-11-14. It created AES `F0 0D 08`,
created all six file types, changed file `01` to enciphered with the new
access word through `Service::desfire_change_file_settings` (Advanced and
confirmation set), read every setting back, wrote a manifest for each delete,
and removed every file and the application. The log shows the new gate path:

```text
aid F0 0D 08 file 01
before: standard data · plain · read key 0, write key 0, rw key 0, change key 0 · 16 bytes
after: standard data · enciphered · read free, write key 0, rw denied, change key 0 · 16 bytes
change: applied
...
application F0 0D 08 deleted
applications: none
not a byte-for-byte restore
application management passed
```

### Secure-messaging writes with persisted backups

`hardware_desfire_secure_messaging` passed on 2026-11-14 under 3K3DES EV1,
AES EV1, and AES EV2. Each `Service::desfire_write` call supplied a planned
backup path; the test asserts the file exists before reporting the write as
verified. The plain, MACed, enciphered, and Backup files were written and read
back through every channel, and both applications were removed.

```text
formatted the PICC; free memory 8192
3K3DES secure-messaging writes passed
AES secure-messaging writes passed
AES EV2 secure-messaging writes passed
applications: none
```

The NDEF replace/FormatPICC hardware record from 26.12 is unchanged. The CLI
and GUI file-settings plans were exercised on the scripted card; the hardware
run exercises the same service gate.

## Residual notes

- The Activity journal is session-only by the phase 19d contract. A persistent
  journal file was not added; `nfctl` prints the same fields for its single
  operation.
- The ACR122U has no record for the new gate paths. The service and CLI code
  are reader-independent; a run is pending a reader session.
- The data-write backup is a logical file export, not a full-card image, and
  cannot restore application keys or file structure.
