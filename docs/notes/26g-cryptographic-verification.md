# Note: phase 26.7 DESFire cryptographic verification

Status: complete 2026-11-13. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26g-cryptographic-verification.md](../specs/26g-cryptographic-verification.md).

## Implementation

- `nfc-desfire` now returns verification metadata instead of a bare byte
  vector. `VerifiedData` and `VerifiedValue` carry an `IntegrityCheck`:
  `Ev1Cmac`, `Ev1Crc32`, `Ev2Cmac`, or `None`. File, value, and record reads fill
  it only after the existing fail-closed check passes.
- `Desfire::verify_key` authenticates once, records the session family, and
  queries GetKeyVersion. The version query advances the same CMAC chain as
  other management commands. `IllegalCommand` and `PermissionDenied` report an
  unavailable version instead of failing an otherwise valid authentication; a
  response MAC or transport failure still aborts.
- `Desfire::card_uid` uses the existing authenticated session. GetCardUID is
  always an encrypted response under EV1/EV2, not a MACed plaintext response:
  the EV1 ciphertext decrypts to the UID, a CRC32 over `UID || 0x00`, and zero
  padding; the EV2 ciphertext decrypts to the UID and padding and carries a
  truncated response CMAC. The UID is returned only after that check. A legacy
  DES/2K3DES session receives the UID without a response check and reports
  `no response MAC`.
- `nfc-app` adds `desfire_verify_key`, `desfire_card_uid`, shared text
  formatters, and a `protection` field on every read result. CLI and GTK call
  the same methods.
- `nfctl desfire verify-key` and `nfctl desfire uid` are read-only and need no
  Advanced mode. The DESFire read output adds a `protection:` line.
- The GTK DESFire workspace adds a `Verify` tab with the application ID, key,
  key family, key number, `Verify key`, `Read UID`, and a monospace result.
- `nfc-desfire::mock` provides a scripted DESFire card for workflow tests. It
  implements legacy DES/2K3DES, 3K3DES, and AES EV1 authentication,
  GetKeyVersion, and GetCardUID. `nfctl --mock` exposes it as `mock:desfire`.
- No originality-signature read is shipped. Verification needs a separately
  specified trust anchor and algorithm, and an unchecked signature is not
  surfaced.

## Problem found on hardware

The first implementation treated GetCardUID as `UID || CMAC` like a normal
management response. The ACR122U returned 16 bytes with no separate response
MAC: the whole response is ciphertext. Decrypting it as EV1 enciphered data
yields the 7-byte UID, a four-byte CRC32 over `UID || 0x00`, and zero padding.
Sorting the command as plain, the response as enciphered, and verifying the CRC
before returning the UID fixed it. EV2 uses the same encrypted response shape
with an 8-byte response CMAC.

## Tests

- `nfc-desfire` unit tests add EV1 encrypted UID and tampered ciphertext,
  EV2 CMAC UID, legacy plain UID, and GetKeyVersion parsing/refusal. Existing
  read tests now assert the integrity label.
- `nfc-app/tests/desfire_verification.rs` covers legacy DES and AES EV1 verify
  key, AES response-MAC verification, wrong-key single-attempt behavior, UID
  decryption, corrupt-ciphertext rejection, and pre-reader key validation.
- `nfctl/tests/cli.rs` covers `verify-key` output, a wrong-key failure, and
  `uid` against `mock:desfire`.
- The GTK workspace test checks the Verify tab, controls, and input parsing.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt --check`, and `reuse lint` pass.

## Hardware record

2026-11-13, ACR122U `acr122_usb:001:002` (direct USB/CCID, `pcscd` stopped).
The test card had accumulated deleted-application tombstones, so the run used
`NFC_FOUNDRY_HARDWARE_FORMAT=yes`; FormatPICC restored 8192 free bytes. The test
created reserved 3K3DES `F0 0D 04` and AES `F0 0D 05` applications with zero
test keys. For each family it checked authentication, verify-key, and the
authenticated UID:

```text
$ NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
  NFC_FOUNDRY_HARDWARE_FORMAT=yes NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
  cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_auth -- --ignored --nocapture
formatted the PICC; free memory 8192
3K3DES verify_key
3K3DES card_uid
AES verify_key
AES card_uid
EV2 verify_key
EV2 card_uid
test authenticates_with_3k3des_aes_and_ev2_keys ... ok
```

The test was rerun with `NFC_FOUNDRY_HARDWARE_KEEP_APP=1` so the CLI could use
the same fixtures:

```text
$ nfctl --device acr122_usb:001:002 desfire verify-key \
    --aid F00D05 --key <16-byte zero> --key-type aes
aid F0 0D 05 · key 0 · AES
session: established · CMAC secure messaging
key version: 0

$ nfctl --device acr122_usb:001:002 desfire uid \
    --aid F00D05 --key <16-byte zero> --key-type aes
aid F0 0D 05 · key 0 · AES
uid: 04 4D 09 A2 BB 17 90
response: EV1 CRC32 verified

$ nfctl --device acr122_usb:001:002 desfire verify-key \
    --aid F00D04 --key <24-byte zero> --key-type 3k3des
aid F0 0D 04 · key 0 · 3K3DES
session: established · CMAC secure messaging
key version: 0

$ nfctl --device acr122_usb:001:002 desfire uid \
    --aid F00D05 --key <16-byte zero> --key-type ev2
aid F0 0D 05 · key 0 · AES EV2
uid: 04 4D 09 A2 BB 17 90
response: EV2 CMAC verified
```

The fixtures were removed with a normal test run. `nfctl desfire info` then
reported `applications: none`. The PN532 UART reader was enumerated, but no
card was present on it during this run.

## Next

26.8 adds secure-messaging writes and transactions. The read protection labels
and the mock card carry forward so writes can report the same checks.
