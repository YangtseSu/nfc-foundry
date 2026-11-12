# Spec: phase 26.7 cryptographic verification

Status: complete 2026-11-13. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Note:
[26g-cryptographic-verification.md](../notes/26g-cryptographic-verification.md).

## Scope

Report the protocol-level result of DESFire authentication, expose the
authenticated card UID, and show which response check protected each file read.
The slice is read-only: it changes no key, file, or application.

## Contract

- Verify key authenticates one application key and returns the key number, key
  family, whether the session uses CMAC secure messaging or the legacy D40
  channel, and the key version from GetKeyVersion.
- GetKeyVersion runs only after authentication. A card that refuses it returns
  `key version: unavailable`; authentication success is still reported.
- A wrong key, wrong key number, mismatched key family, or invalid response
  check fails once. There is no retry loop.
- GetCardUID is available only after authentication. Under EV1 and EV2 secure
  messaging the command returns an encrypted response; the UID is returned only
  after CRC32/padding or CMAC/padding verification. A legacy DES/2K3DES session
  has no response encryption and reports `no response MAC`.
- Every file read reports the effective communication mode, the session key
  family, and the check that passed: EV1 CMAC, EV1 CRC32, EV2 CMAC, or no
  response MAC.
- No result is presented as a business entitlement or an access decision. It is
  only a protocol-level cryptographic check.
- The originality signature is not read or verified in this slice. It needs a
  separate spec that names the trust anchor and verification algorithm; an
  unverified signature is not surfaced.

## Surfaces

- CLI: `nfctl desfire verify-key --aid --key [--key-type] [--key-number]` and
  `nfctl desfire uid --aid --key [--key-type] [--key-number]`. Both are
  read-only, need no Advanced mode, and never print the key.
- GUI: DESFire → Verify with the application ID, key, key family, and key
  number. `Verify key` and `Read UID` share the CLI service methods.
- Read results in both surfaces add the `protection:` line.

## Verification

- Unit tests cover EV1 CMAC and CRC32 reads, EV2 CMAC reads, authenticated UID
  decryption, tampered ciphertext/MAC rejection, GetKeyVersion parsing, and the
  refused-version path.
- Service tests use a scripted DESFire card and assert that verify key sends no
  file command and that a wrong key produces exactly one authentication attempt.
- CLI tests run verify-key and uid against the mock DESFire device, including a
  wrong-key failure.
- GTK tests check the Verify tab and its input parsing.
- Hardware: ACR122U `acr122_usb:001:002` with a reserved AES application and
  the 3K3DES/AES/EV2 authentication test; `nfctl` then ran verify-key and uid
  for DES, 3K3DES, AES EV1, and AES EV2, and the applications were removed.
