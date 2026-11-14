# Spec: DESFire write gates and recovery behavior (26.13)

Date: 2026-11-14. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Note:
[26m-write-gates.md](../notes/26m-write-gates.md).

## Scope

Close the DESFire workbench with one set of write gates:

- a shared session preflight for file, application, key, and NDEF mutations;
- read-only inspection so destructive and configuration actions show the old
  state, key source, planned backup or manifest path, and read-back before
  execution;
- structured Activity entries for every mutation;
- cancellation, card-change, and reader-loss behavior that never reports a
  partial mutation as successful.

Classic and Type 2 gates are unchanged. No bulk personalization, persistent
operation log, or continue-on-error default is added.

## Shared preflight

`begin_file_session` is the single preflight for a DESFire file session:

- verify the selected card is still the same ISO-DEP card (ATQA, SAK, stable
  UID suffix);
- select the requested application or the PICC;
- authenticate with the supplied key when one is present.

Read, write, value, record, file inspect, file create/delete/settings,
application create/delete, key settings/key change, and the NDEF/FormatPICC
sessions all use it. Operations still check their own target existence,
access right, capacity, and old state before the first mutation, and read the
result back afterwards. The selected reader is the one held by the operation
lease; a missing or failed reader aborts before the card command.

`Service::desfire_inspect_file(DesfireFileInspectionRequest)` reads settings
without changing state. The CLI and GUI use it for file settings and file
delete plans. Application delete plans use the unauthenticated inventory and
state when the file list is protected.

## Backup and manifest paths

- A data write can take an exact `backup_path`. The service writes a private
  JSON export (`nfc-foundry/desfire-file-backup/1`, mode 0600) before the first
  byte: card UID, application, file ID, type, decoded summary, raw settings,
  and complete data. An unwritable path aborts before `WriteData`.
- Application and file deletes take an exact `manifest_path`. The caller plans
  the path with `BackupStore::plan_path`; the manifest is written before the
  destructive command and the dialog/CLI prints the path before confirmation.
- Value and record writes export their before state in the result
  (`old_value`/`new_value`, `before`/`after` records); they do not create a
  file because the card transaction is the recovery boundary.
- NDEF and FormatPICC have no byte-for-byte backup; their plans state that
  explicitly.

## Gates

| Operation | Advanced | Typed confirmation | Backup / export | Read-back |
|---|---|---|---|---|
| Standard/Backup write | no | no | planned JSON file | written range |
| Value change | no | no | before/after value | committed value |
| Record append/update/clear | no | no | before/after records | every record |
| Create application/file | no | no | none | ID list / settings |
| Change file settings | yes | `CHANGE` | old settings in the plan | settings |
| Delete application/file | yes | `DELETE` | planned manifest path | ID lists |
| KeySettings change | yes | `CHANGE` | old/new byte in the plan | KeySettings |
| Key change | yes | `CHANGE` | export path or recorded-elsewhere gate | version after re-auth |
| NDEF format/replace/delete | yes | `FORMAT` / `REPLACE` / `DELETE` | none, stated | Type 4 CC/NDEF |
| FormatPICC | yes | `ERASE` | none, stated | empty ID list and free memory |

A configuration or destructive action is refused before card I/O when the
gate is absent. Advanced mode is default off, session-scoped, and resets on
card removal, a different card, reader recovery/error, and application
restart. The CLI has no session: `--advanced` defaults off for every command.

## Activity journal

Each DESFire mutation produces one session Activity row:

```text
desfire <operation> · <succeeded|failed> · <target> · <detail>
```

`target` names the application/file, PICC key, or `all applications`. `detail`
contains the written range, old/new value or version, erased count, or the
backup/manifest path. Failures are recorded once and the toast does not add a
second row. Key material, Transaction MAC keys, and NDEF payloads are never
written to the Activity or the CLI output.

The Activity is session-only, as specified for the workbench; the CLI prints
the same fields for its single operation. No persistent journal file is added.

## Cancellation and recovery

- A cancelled token is checked by the operation lease and before each write
  chunk. A pre-cancelled write sends no card command.
- The card identity is checked before the first command and after the last
  read-back. A swap before the final check reports `CardChanged`; the write is
  never reported as verified on the wrong card.
- A reader I/O failure aborts the operation; the last verified state remains
  in the result or error text (value before, records before, backup path, or
  the explicitly deleted NDEF application).
- A failed commit aborts an open Backup transaction. A mismatch after a
  successful commit is reported as a verification failure, not rolled back
  silently.
- No operation retries a wrong key or a failed gate.

## Verification

- `nfc-app` tests: shared session use, file inspection, file-settings gate,
  planned backup path (content, mode 0600), unwritable path abort,
  pre-cancelled token, card change, and reader loss.
- CLI tests: file-settings dry run and gates, planned write backup path,
  manifest path before delete, FormatPICC dry run, NDEF replace refusal.
- GTK tests: new controls, Advanced gating, Advanced reset on card removal,
  and the Activity row split.
- Hardware: the 26.9 management test with the new file-settings gate and the
  26.8 secure-messaging write test with persisted backups passed on PN532 UART
  and ACR122U on 2026-11-14. The 26.12 NDEF tests are recorded in
  [26l](../notes/26l-ndef-replace-format.md).
