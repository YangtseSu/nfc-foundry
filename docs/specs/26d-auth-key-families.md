# Spec: phase 26.4 DESFire authentication key families

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md).

## Problem

The native layer authenticated only DES/2K3DES (`0x0A`). 3K3DES and AES
applications could not be opened, and a 16-byte key was always treated as
2K3DES. AES had to be selectable explicitly.

## Key families

`AuthKeyType` selects the command, key length, challenge length, and session
key derivation:

| Family | Command | Key | Nonce | Session key |
|---|---|---|---|---|
| DES | `0x0A` | 8 | 8 | RndA[0..4] + RndB[0..4] |
| 2K3DES | `0x0A` | 16 | 8 | RndA[0..4] + RndB[0..4] + RndA[4..8] + RndB[4..8] |
| 3K3DES | `0x1A` | 24 | 16 | RndA[0..4] + RndB[0..4] + RndA[6..10] + RndB[6..10] + RndA[12..16] + RndB[12..16] |
| AES EV1 | `0xAA` | 16 | 16 | RndA[0..4] + RndB[0..4] + RndA[12..16] + RndB[12..16] |
| AES EV2 | `0x71` | 16 | 16 | AN12343 CMAC derivation, separate ENC and MAC keys |

`AuthKeyType::from_key` maps 8/16/24 bytes to DES/2K3DES/3K3DES; a 16-byte AES
key needs an explicit `Aes` or `AesEv2` selection.

## Protocol

- DES/2K3DES keep D40 send mode and CBC receive mode.
- 3K3DES uses CBC with the last challenge block as the send IV and the last
  sent block as the receive IV.
- AES EV1 uses AES-CBC with the same chaining rule as 3K3DES.
- AES EV2 uses AES-CBC with a zero IV in both directions. The 32-byte final
  response carries TI, RndA', PCDcap2, and PICCcap2; RndA' is verified.
- EV2 derives the session ENC key with `A5 5A` and the MAC key with `5A A5`,
  each over the AN12343 32-byte input with AES-CMAC.
- A leading `0x00` status byte on an authentication response is stripped.
- Key version 0 (current) is used for EV2First and exposed as
  `session_key_version()`.

`authenticate_legacy` keeps the length-based DES/2K3DES/3K3DES behavior.
`authenticate` takes an explicit `AuthKeyType`. Wrong key, key number, or
family fails once; no retry loop is added.

## Reads under the new families

3K3DES and AES authenticated sessions MAC the `GetFileSettings` response
(observed as 8 trailing bytes). Reads under those families therefore require
secure messaging and are rejected with a typed error until phase 26.5. The
authentication itself and the session keys are validated directly.

## Surfaces

- `nfctl desfire read|format-ndef|delete-ndef --key-type
  des|2k3des|3k3des|aes|ev2`.
- The GTK DESFire Read and NDEF tabs have a `Key type` dropdown with `Auto`
  plus the five families. `Auto` keeps the length inference; AES must be
  selected.
- Keys remain session-scoped and are cleared on card removal.

## Test-card recovery

Deleted DESFire applications retain their EEPROM as tombstones until a
FormatPICC. The hardware test therefore requires
`NFC_FOUNDRY_HARDWARE_FORMAT=yes` to format the owned test card when free
memory is low, or always when that variable is set. This is a developer test
recovery, not a product workflow.

## Acceptance

- Legacy DES/2K3DES remains validated.
- 3K3DES, AES EV1, and AES EV2 authenticate on a reserved application with
  zero test keys; session key lengths and EV2 ENC/MAC separation are checked.
- Wrong key, key number, and family fail once.
- The PICC master key is not changed.
- Secure-messaging reads and key changes are out of scope (26.5, 26.11).
- ACR122U remains unrecorded.
