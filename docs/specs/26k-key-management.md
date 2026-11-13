# Spec: DESFire key management (26.11)

Date: 2026-11-14. Plan: [26-desfire-workbench.md](../plan/26-desfire-workbench.md).
Note: [26k-key-management.md](../notes/26k-key-management.md).

## Scope

Read and change the DESFire key configuration through the application service,
the `nfctl desfire` CLI, and the GTK `DESFire` workspace:

- GetKeySettings: KeySettings byte, key count, key family, ISO file ID flag.
- GetKeyVersion for every key of the selected application.
- ChangeKeySettings for the PICC or one application.
- ChangeKey for one application key or the PICC master key, followed by
  re-authentication with the new key and a key-version read-back.

The PICC master key is not changed by the test suite. No key recovery,
dictionary, brute force, key diversification, or issuance workflow is added.

## Command encodings

### GetKeySettings (`0x45`)

- Response: KeySettings byte, then an optional second byte whose low nibble is
  the key count, bits 6-7 the key family (`00` 2K3DES, `01` 3K3DES, `10` AES),
  and bit 5 the ISO file ID flag. A one-byte response leaves the optional
  fields unknown instead of guessing.
- Sent plain without a session and with the session's management protection
  when authenticated. A response MAC is verified when present.

### ChangeKeySettings (`0x54`)

- Data: one KeySettings byte.
- Under 3K3DES/AES EV1 and AES EV2 the command uses the enciphered mode: the
  byte and its integrity value are enciphered with the session key, and the
  status is MACed. A plain or MACed request is refused by an EV3 card with
  `LENGTH_ERROR`; this was confirmed on hardware.
- Under DES/2K3DES the D40 session sends the byte plain and accepts the status
  without a MAC.
- The service refuses the command when the current KeySettings byte has the
  configuration-changeable bit clear. The card also enforces it.
- Key count, key family, and ISO file ID are creation-time properties; they
  are displayed, never changed by ChangeKeySettings.

### ChangeKey (`0xC4`)

The frame follows the authenticated channel; only a change within the
session's key family is supported.

| Channel | Key data | Integrity | Command protection | Response |
|---|---|---|---|---|
| DES/2K3DES (D40) | New key with embedded version, XOR old key when the target differs | CRC16 over the key area, then CRC16 over the new key when the target differs | CBC in the D40 send direction (block decrypt) with a zero IV | status only |
| 3K3DES / AES EV1 | New key (3K3DES with embedded version, AES with a trailing version byte), XOR old key when the target differs | CRC32 over `INS \|\| KeyNo \|\| key data`, then CRC32 over the new key value when the target differs | CBC with the session IV | status MAC |
| AES EV2 | Same key data; CRC32 over the new key value only when the target differs | ISO/IEC 9797-1 padding, EV2 command IV, truncated CMAC | status MAC; no MAC when the authenticated key itself changes |

- A change of the authenticated key ends the session. The card returns the
  status without a response MAC; the session is dropped before decoding it.
- The PICC master key encodes the new key family in the upper KeyNo bits.
  Application keys use the bare key number.
- When the target is not the authenticated key, the caller supplies that key's
  current value. For DES/3K3DES the stored key version is read first and
  embedded in the old key before the XOR, because the version lives in the
  parity bits.
- The AES EV2 vectors from AN12343 section 9.2 are unit tests, including the
  exact command APDUs for both cases.

## Service API

`Service::desfire_key_settings(DesfireKeySettingsRequest)`:
- read-only when `new_key_settings` is `None`;
- a change requires the authenticating key, Advanced mode, and confirmation;
- reads before and after, requires the after byte to match, and returns the
  per-key versions when authenticated.

`Service::desfire_change_key(DesfireKeyChangeRequest)`:
- gates: Advanced mode, confirmation, key numbers, key family, target key
  existence, application key family, and a recording decision run before the
  ChangeKey command;
- reads the key version before the change;
- writes the new key to the export path with mode 0600 before the command when
  asked;
- requires `acknowledge_unrecorded` when no export path is given, because a
  lost key cannot be recovered;
- re-authenticates with the new key, reads its version back, and fails closed
  on a version mismatch;
- never prints or logs key material.

The request and result types redact keys from `Debug` or do not implement it.

## CLI

- `nfctl desfire key-settings`: read-only, or with `--new-settings` after
  `--advanced` and confirmation.
- `nfctl desfire change-key`: `--key`, `--key-type`, `--auth-key-number`,
  `--key-number`, `--new-key`, `--new-key-version`, optional `--current-key`,
  `--export`, or `--acknowledge-unrecorded`; Advanced mode and confirmation.
- `--aid` defaults to the PICC.

## GUI

The `Keys` tab exposes the same fields and calls the same service methods.
Changing KeySettings or a key requires Advanced mode on the Manage tab and a
typed `CHANGE` confirmation. The export path and the "recorded elsewhere"
checkbox implement the recording gate. Key material is cleared on card
removal, reader change, and workspace reset.

## Restrictions and blockers

- **Multiple key sets and key rolling are not implemented.** EV2/EV3 support
  up to 16 key sets and the InitializeKeySet (`0x56`), FinalizeKeySet (`0x57`),
  and RollKeySet (`0x55`) commands, but AN12752 documents only that the feature
  exists. The command parameters, key set settings encoding, and
  CreateApplication KeySett3 layout are in the NDA-only functional
  specification; an invented frame could leave the active key set unusable.
  The current key set version is therefore not shown either.
- A PICC master key change between key families (for example DES to AES) is
  refused. Only same-family changes are sent; cross-family frame rules cannot
  be validated without a dedicated test card.
- The ACR122U hardware row is not recorded for this slice.

## Acceptance mapping

| Requirement | Evidence |
|---|---|
| Change PICC/app keys with type, number, version, current and new key | unit vectors, mock integration tests, PN532 hardware EV1 AES + DES and EV2 |
| Change application key settings and access rights | KeySettings read/change on PN532; file access rights already covered by 26.9 |
| Multiple key sets and rolling where supported | blocked and documented; no unverified command |
| Key export only on request, private file, no logging | export test checks mode 0600 and key-free output; result types redact |
| Re-authenticate with the new key and read the version back | service flow and hardware asserts every changed key |
| Key changes on a reserved application first | hardware test creates and removes `F0 0D 0B` (AES) and `F0 0D 0C` (DES) |
| PICC master key test on a dedicated card or explicit confirmation and recorded key | same-family PICC path exists with Advanced + typed confirmation + recording gate; not executed on the owned card |
| A lost new key is reported unrecoverable before the change | service gate and CLI/GUI refusal before any card command |
| No recovery, dictionary, or brute force | no such code path |
