# Phase 26 plan: DESFire workbench

Status: in progress. 26.1–26.5 are complete; 26.6–26.13 are pending, except
that 26.12's ordinary NDEF provisioning and guarded delete landed in phase
25.5. Hardware: PN532 UART first; ACR122U records after a reader is available.

## Goal

Make DESFire a generic workbench card family: inspect it, read it, write it,
provision applications and files, and manage its keys. This is not an
application builder. The product does not implement access control, wallets,
transit, loyalty, membership, ticketing, or issuance workflows; it exposes the
generic DESFire primitives those systems are built from.

The phase builds on the phase 25.5 baseline (commits `f18b7e3`, `d1803c8`,
`114dd43`):

- identity and application/file inventory;
- legacy DES/2K3DES authentication with D40 send mode;
- plain Standard/Backup file read through `nfctl desfire read` and the GTK
  `DESFire` workspace.

## Principles

- Service first: `nfc-app` owns every workflow; `nfctl` and GTK call the same
  method with the same gates.
- Read before write. Every mutation authenticates, validates capacity and
  access rights, exports the current structure/data where possible, checks the
  card identity, writes, and reads back.
- Advanced mode is default off and session-scoped. Format, delete, key, lock,
  configuration, and full-card operations require it plus typed confirmation.
- Keys are session-scoped by default. A key is never logged or exported unless
  the user explicitly exports it. An optional key store is explicit, local, and
  never stores a key without a user action.
- No brute force, default-key dictionary, or key recovery. A wrong key fails
  once.
- A write is complete only when the read-back matches. A partial write is never
  reported as complete.
- Stop on the first unrecoverable failure unless the user explicitly selected a
  continue policy; destructive operations do not offer that policy by default.
- No business templates. No vertical application logic.

## Slices

### 26.1 Baseline (done)

Delivered:

- Legacy DES/2K3DES authentication (`0x0A`) and session key derivation.
- Plain Standard/Backup file read with offset and length.
- CLI `desfire info` and `desfire read`; GTK `DESFire` workspace with
  `Inventory` and `Read`.
- Hardware round trip on the owned EV3 8K: reserved application `F0 0D 01`,
  standard file `01`, write/read/delete, card left with no applications.

### 26.2 File settings decode

Status: complete 2026-11-12. See
[26b-file-settings.md](../notes/26b-file-settings.md).

Deliver:

- A typed `FileSettings` model in `nfc-desfire` for all six file types:
  Standard, Backup, Value, Linear Record, Cyclic Record, Transaction MAC.
- Decode communication mode (Plain/MACed/Enciphered), access rights (read,
  write, read/write, change; key numbers and free/denied), data size, value
  limits, record size/count, and Transaction MAC fields.
- Reject truncated or internally inconsistent settings instead of guessing.
- `nfctl desfire info` and the GUI inventory show decoded fields plus the raw
  settings bytes.
- Unit tests with one valid and one malformed settings vector per file type.

Acceptance:

- The GUI never labels a file type it cannot decode.
- Raw settings remain available for diagnostics.

### 26.3 Value and record read

Status: complete 2026-11-12. See
[26c-value-record-read.md](../notes/26c-value-record-read.md).

Deliver:

- Value file read with `GetValue` (`0x6C`), including signed value and
  limited-credit state.
- Linear and Cyclic Record read with `ReadRecords` (`0xBB`), including record
  size, record count, start record, and explicit partial-read handling.
- CLI and GUI expose the same value/record read.
- File-type checks prevent using the Standard/Backup `ReadData` path against a
  Value or Record file.

Acceptance:

- Value and record reads are hardware-validated on a reserved test
  application, then the application is removed.
- Record reads do not report success when fewer records or bytes were returned
  than requested.

### 26.4 Authentication key families

Status: complete 2026-11-12. See
[26d-auth-key-families.md](../notes/26d-auth-key-families.md).

Deliver:

- 3K3DES authentication (`0x1A`) with the correct 16-byte challenge and session
  key derivation.
- AES authentication for EV1 (`0xAA`) and EV2 (`0x71`), including key type,
  key version, and EV2 session-key derivation.
- Explicit key-type selection in CLI/GUI instead of inferring AES from a
  16-byte legacy key.
- Wrong key, wrong key number, wrong key type, and unsupported card state fail
  once and are reported without retry loops.

Acceptance:

- Legacy DES/2K3DES remains validated.
- AES/3K3DES paths are hardware-validated only on a reserved test application
  with known test keys or on a provisioned card; the PICC master key is not
  changed.
- Keys remain session-scoped and are cleared on card removal.

### 26.5 Secure-messaging read (done)

Status: complete 2026-11-12. See
[26e-secure-messaging.md](../notes/26e-secure-messaging.md) and the
[26e spec](../specs/26e-secure-messaging.md).

Deliver:

- EV1 MACed and enciphered file read using the authenticated session:
  command/response CMAC, IV and counter handling, and CBC encryption.
- EV2 MACed and enciphered file read using AES-CMAC and the EV2 session keys.
- A fail-closed rule: a missing, wrong, or unverified MAC is an error; no data
  is shown as read-authentic.
- EV1/EV2 secure-messaging code lives in `nfc-desfire`; no business logic.

Acceptance:

- Plain file read remains unchanged.
- MACed read verifies the response MAC before returning data.
- Enciphered read decrypts and validates the response before returning data.
- Hardware validation uses a provisioned card or a reserved test application
  with test keys; the owned PICC key is not changed.
- A tampered response is rejected in a unit test.

### 26.6 Type 4 NDEF read

Deliver:

- Read the NFC Forum Type 4 NDEF application on DESFire through the existing
  `nfc-type4` and `nfc-ndef` layers.
- Select the NDEF tag application, read the CC and NDEF file, and expose the
  decoded records in CLI and GUI.
- Keep the existing DESFire `Inventory` and `Read` views; add an NDEF result
  view only after the read path is validated.
- No formatting or write in this slice.

Acceptance:

- A DESFire card with an NDEF application reads CC and NDEF without being
  reported as an unformatted tag.
- Unknown/proprietary TLVs and malformed lengths are reported, not silently
  discarded.
- Hardware validation uses a provisioned NDEF card or a reserved test
  application created by the test harness and removed afterwards.

### 26.7 Cryptographic verification

Deliver:

- An explicit "verify key" operation that reports authentication success, key
  type, key version, and session establishment without reading file data.
- CMAC verification results for MACed reads.
- Authenticated UID read where the card supports it; the UID is shown only
  after the session and any response MAC verify.
- Optional, separately specified originality-signature read/verification.

Acceptance:

- Verification never changes card state or keys.
- No result is presented as proof of a business entitlement; it is only a
  protocol-level cryptographic check.
- Wrong keys and invalid MACs fail once and are not retried.

### 26.8 Generic data write and transactions

Deliver:

- Standard and Backup file write with offset and length in Plain, MACed, and
  Enciphered modes.
- Commit and abort for Backup files and any file whose changes require a
  transaction.
- Capacity and access-right preflight before the first byte is written.
- Export the readable current file content and settings before the write. If
  the file cannot be read, show that explicitly and require an additional
  confirmation; do not invent a backup.
- Read-back verification after every write. Stop on the first mismatch.
- No bulk write, no continue-on-error default.

Acceptance:

- A temporary application/file round trip writes, commits, reads back, and
  deletes on hardware.
- A wrong key, out-of-range offset/length, read-only file, or card change
  aborts before or during the write without reporting success.
- A failed commit leaves no uncommitted data reported as committed.

### 26.9 Application and file management

Deliver:

- Create and delete applications.
- Create, delete, and inspect all six file types: Standard, Backup, Value,
  Linear Record, Cyclic Record, and Transaction MAC.
- Change file settings, access rights, and communication mode where the card
  supports it.
- Show free memory and refuse a create that cannot fit.
- Export an application/file manifest before deletion or format.
- No automatic application templates.

Acceptance:

- Create/delete a reserved application and each file type on hardware, with
  read-back and cleanup.
- Deleting a non-empty or protected application requires typed confirmation
  and an exported manifest.
- Deleting an application does not claim a byte-for-byte restore.

### 26.10 Value, record, and transaction writes

Deliver:

- Value file debit, credit, and limited-credit operations with commit/abort.
- Linear and Cyclic Record append, clear, and update with record-boundary
  checks.
- Transaction MAC file creation and transaction MAC verification where the
  card supports it.
- Transaction Timer configuration where supported.
- All operations expose the exact old/new value or record and require
  read-back.

Acceptance:

- Debit/credit/append/clear/update round trips on a reserved test application.
- A value outside the configured limits or a record outside the file bounds is
  refused before the command.
- Transaction MAC results are verified before being shown as valid.

### 26.11 Key management

Deliver:

- Change PICC master key and application keys: key type, key number, key
  version, current key, and new key.
- Change application key settings and access rights.
- Multiple key sets and key rolling where the card supports it, with explicit
  set selection and version display.
- Key export only when the user explicitly asks; otherwise keys remain
  session-scoped.
- Optional local key store with file-permission protection and no key logging.
- Re-authenticate with the new key immediately after the change and read back
  the key version.

Acceptance:

- Key changes are tested on a reserved test application first; PICC master key
  changes require a dedicated test card or explicit user confirmation and a
  recorded new key.
- A lost/unrecorded new key is reported as unrecoverable before the change, not
  after.
- No key recovery, dictionary, or brute-force path is added.

### 26.12 NDEF provisioning and format

Status: ordinary provisioning, Type 4 read-back, typed confirmation, and the
guarded `DeleteApplication` undo landed in phase 25.5. This slice now covers
the replace path and `FormatPICC`.

Deliver:

- Reuse the phase 25.5 provisioning service: create `D27600` with the ISO DF
  name `D2760000850101` and the CC/NDEF files, then read back the complete
  message through the Type 4 writer.
- Delete only the NDEF application created by this product as the undo path.
- Add the explicit replace path: delete the validated NDEF application and
  provision it again after a separate typed confirmation.
- `FormatPICC` as a separate full-reset action that erases every application;
  it is Advanced, typed-confirmation, and never automatic.
- Refuse NDEF provisioning when a valid NDEF application already exists unless
  the user selected the replace path.

Acceptance:

- A blank owned card can be provisioned, read through the Type 4 path, and have
  its NDEF application deleted, leaving the pre-format application list.
- `FormatPICC` is not used for ordinary NDEF provisioning.
- No formatting is performed by a read or inventory operation.

### 26.13 Advanced write gates and recovery behavior

Deliver:

- A shared `nfc-app` preflight for every write: card identity, selected reader,
  authenticated session, target application/file, access rights, capacity,
  old/new values, backup/export path, and read-back plan.
- Advanced mode default off and session-scoped; reset on card removal, reader
  change, and application restart.
- Typed confirmation for format, delete, key, lock, configuration, and full-card
  actions.
- A single write journal/activity entry with operation, target, result, and
  backup path; never the key.
- Cancellation and card-change handling that stops the operation and preserves
  the last verified state.
- No bulk personalization or continue-on-error default.

Acceptance:

- Every destructive action shows the exact application/files/pages, old/new
  values, key source, backup path, and read-back plan before execution.
- A card change or reader loss aborts and reports the last verified state.
- CLI and GUI expose the same gates and semantics.

## Dependencies

- 26.4 adds `aes` and `cmac` at the latest stable versions, with a spec note
  recording why each is needed.
- 26.5 implements EV1/EV2 CMAC chaining and CBC with the existing `cipher`,
  `aes`, and `des` primitives; it adds no crate. 26.8 reuses them for writes.
- 26.6 and 26.12 reuse `nfc-type4` and `nfc-ndef`; no second NDEF codec.
- No C shim, external reader daemon, or Proxmark3 dependency is added.

## Hardware and test gates

- `cargo test --workspace` must never touch a card.
- Hardware tests stay feature- and environment-gated under `NFC_FOUNDRY_HARDWARE`
  and use the exact `NFC_FOUNDRY_DEVICE` connstring.
- Write tests require `NFC_FOUNDRY_HARDWARE_WRITE=yes` and a reserved test AID.
- Destructive tests use a reserved application, export the manifest, execute,
  read back, and remove the application.
- The owned EV3 8K is factory DES; AES/EV2, key-management, and
  secure-messaging tests need a reserved test application with known test keys
  or a separate test card.
- The ACR122U row remains unrecorded until a reader is available.

## Out of scope

- Access-control, wallet, transit, loyalty, membership, or ticketing templates.
- Card issuance, bulk personalization, key diversification, or key recovery.
- Proxmark3 device control.
- Full-card byte-for-byte backup or restore for DESFire. A logical
  application/file export is not a full-card image.
