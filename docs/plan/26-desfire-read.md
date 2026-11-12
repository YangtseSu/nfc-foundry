# Phase 26 plan: DESFire read-only workbench

Status: in progress. 26.1 is complete; 26.2–26.7 are pending. Generic writes
are deferred. Hardware: PN532 UART first; ACR122U records after a reader is
available.

## Goal

Make DESFire a generic read/inspect/verify card family in NFC Foundry, not an
application builder. A DESFire card is a multi-application secure element; the
workbench exposes its structure and protected data without implementing access
control, wallets, transit, loyalty, or issuance workflows.

The phase builds on the phase 25.5 baseline (commits `f18b7e3`, `d1803c8`,
`114dd43`):

- read-only identity and application/file inventory;
- legacy DES/2K3DES authentication with D40 send mode;
- plain Standard/Backup file read through `nfctl desfire read` and the GTK
  `DESFire` workspace;
- no user-facing writes.

## Principles

- Service first: `nfc-app` owns every workflow; `nfctl` and GTK call the same
  method with the same gates.
- Read-only for this phase. No application/file creation, deletion, key change,
  format, or write is exposed by the product. Test-only provisioning uses a
  reserved AID under `NFC_FOUNDRY_HARDWARE_WRITE=yes` and removes it afterwards.
- Keys are session-scoped. They are never logged, exported, or persisted by
  default, and are cleared on card removal, reader loss, and application
  restart.
- No brute force, default-key dictionary, or key recovery. A supplied key either
  authenticates or the operation fails once.
- A read is complete only when every byte is returned and, for protected
  files, its MAC verifies. A partial read is never reported as complete.
- Every slice adds a spec and a note, updates CLI/GUI parity, and records the
  hardware result.
- No business templates. No vertical application logic.

## Slices

### 26.1 Read-only baseline (done)

Delivered:

- Legacy DES/2K3DES authentication (`0x0A`) and session key derivation.
- Plain Standard/Backup file read with offset and length.
- CLI `desfire info` and `desfire read`; GTK `DESFire` workspace with
  `Inventory` and `Read`.
- Hardware round trip on the owned EV3 8K: reserved application `F0 0D 01`,
  standard file `01`, write/read/delete, card left with no applications.

No further work in this slice.

### 26.2 File settings decode

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
- No new card command is added; this is a decoding slice.

### 26.3 Value and record read

Deliver:

- Value file read with `GetValue` (`0x6C`), including signed value and
  limited-credit state.
- Linear and Cyclic Record read with `ReadRecords` (`0xBB`), including record
  size, record count, start record, and explicit partial-read handling.
- CLI and GUI expose the same value/record read; no debit, credit, append,
  clear, update, or commit operation.
- File-type checks prevent using the Standard/Backup `ReadData` path against a
  Value or Record file.

Acceptance:

- Value and record reads are hardware-validated on a reserved test
  application, then the application is removed.
- Record reads do not report success when fewer records or bytes were returned
  than requested.
- The card is left with no test application.

### 26.4 Authentication key families

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

### 26.5 Secure-messaging read

Deliver:

- EV1 MACed and enciphered file read using the authenticated session:
  command/response CMAC, IV and counter handling, and CBC encryption.
- EV2 MACed and enciphered file read using AES-CMAC and the EV2 session keys.
- A fail-closed rule: a missing, wrong, or unverified MAC is an error; no data
  is shown as read-authentic.
- EV1/EV2 secure-messaging code lives in `nfc-desfire`; no business logic.
- Use the AES/CMAC crates added in 26.4 and add the CBC mode crate; record the
  dependency choice in the slice spec.

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
- No formatting, application creation, or NDEF write in this phase.

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

## Dependencies

- 26.4 adds `aes` and `cmac` at the latest stable versions, with a spec note
  recording why each is needed.
- 26.5 adds the CBC mode crate needed for EV1/EV2 enciphered reads.
- 26.6 reuses `nfc-type4` and `nfc-ndef`; it does not add a second NDEF codec.
- No C shim, external reader daemon, or Proxmark3 dependency is added.

## Deferred writes

Generic file write, NDEF formatting, application/file creation, deletion, key
change, and key-set management are **not exposed by the product in phase 26**.
If added later,
they require a separate approved phase with:

- a full logical backup where possible, or an explicit irreversibility warning;
- typed confirmation for format, delete, key, lock, and configuration actions;
- authentication, old/new values, backup path, and read-back plan shown before
  the change;
- identity checks for UID, ATQA, and SAK;
- no full-card byte-for-byte restore claim for DESFire.

## Hardware and test gates

- `cargo test --workspace` must never touch a card.
- Hardware tests stay feature- and environment-gated under `NFC_FOUNDRY_HARDWARE`
  and use the exact `NFC_FOUNDRY_DEVICE` connstring.
- Test-only provisioning uses the reserved AID `F0 0D 01`, requires
  `NFC_FOUNDRY_HARDWARE_WRITE=yes`, and deletes the application after the test.
- The owned EV3 8K is factory DES; AES/EV2 and secure-messaging tests need a
  provisioned test application or a second card with known test keys.
- The ACR122U row remains unrecorded until a reader is available.

## Out of scope

- Access-control, wallet, transit, loyalty, membership, or ticketing templates.
- Card issuance, bulk personalization, key diversification, or key recovery.
- Changing the PICC master key or application keys.
- Proxmark3 device control.
- Full-card backup or restore for DESFire.
