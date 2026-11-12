# Note: phase 26.4 DESFire authentication key families

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26d-auth-key-families.md](../specs/26d-auth-key-families.md).

## Implementation

- `AuthKeyType` selects DES (`0x0A`, 8 bytes), 2K3DES (`0x0A`, 16 bytes),
  3K3DES (`0x1A`, 24 bytes), AES EV1 (`0xAA`, 16 bytes), and AES EV2
  (`0x71`, 16 bytes).
- `Desfire::authenticate` implements all five families; `authenticate_legacy`
  infers DES/2K3DES/3K3DES from the key length, while AES remains explicit.
- 3K3DES uses CBC with the challenge tail as the send IV and the sent tail as
  the receive IV. AES EV1 uses the same rule with 16-byte blocks. AES EV2 uses
  zero-IV AES-CBC and verifies the 32-byte final response (TI, RndA', caps).
- EV2 session ENC/MAC keys use the AN12343 CMAC derivation. Unit tests match
  both zero-key and non-zero-key vectors from the local Proxmark3 test suite.
- `Session` exposes ENC key, MAC key, family, and key version. EV1 and earlier
  use the ENC key as the MAC key; EV2 derives them separately.
- `GetFreeMemory` (`0x6E`) and `FormatPICC` (`0xFC`) were added for test-card
  recovery.

## Read gating

After 3K3DES or AES authentication, the card MACs the `GetFileSettings`
response; the parser sees eight trailing bytes. `Service::desfire_read` now
rejects `Tdes3`, `Aes`, and `AesEv2` with "secure messaging (phase 26.5)"
instead of reporting a malformed response. Plain DES/2K3DES reads are
unchanged.

## Test-card EEPROM

Repeated create/delete fixtures filled the EV3 8K EEPROM: `GetFreeMemory`
reported 32 bytes and `CreateApplication` failed with `0x910E` (Out of EEPROM).
Deleting applications did not reclaim the space; the card keeps tombstones
until FormatPICC. With the owner's approval the test formatted the PICC on
2026-11-12 (free memory returned to 8192). The test now requires
`NFC_FOUNDRY_HARDWARE_FORMAT=yes` to format when free memory is low, or always
when that variable is set. FormatPICC erases every application and resets the
PICC to factory state; it is not a product workflow.

## PN532 UART hardware record

Run on 2026-11-12 on `pn532_uart:/dev/ttyUSB0`:

```text
formatted the PICC; free memory 8192
test authenticates_with_3k3des_aes_and_ev2_keys ... ok
```

The test created reserved 3K3DES `F0 0D 04` (24-byte zero key) and AES
`F0 0D 05` (16-byte zero key) applications, then:

- authenticated `0x1A` and verified a 24-byte ENC and MAC session key;
- authenticated `0xAA` and verified 16-byte ENC and MAC keys;
- authenticated `0x71` and verified 16-byte EV2 ENC and MAC keys differ and
  key version 0;
- rejected a wrong AES key, key number 1, a 3K3DES key length against the AES
  application, and a 2K3DES-family attempt against the AES key.

Both applications were removed afterwards; `applications: none`.

## ACR122U record

2026-11-12: `acr122_usb:001:002` (direct USB/CCID, `pcscd` stopped) passed the
same test on the same reader: `0x1A`, `0xAA`, and `0x71` authenticated, the
session-key and wrong-key checks passed, and both applications were removed.

## Tests

- `nfc-desfire`: EV2 session-key vectors, AES/3DES CBC round trips, key-length
  mapping, key validation, existing legacy tests.
- `nfctl/tests/hardware_desfire_auth.rs` is environment-gated and supports
  `NFC_FOUNDRY_HARDWARE_FORMAT=yes` for test-card recovery.
- The CLI/GUI key-type selection is covered by parsers and the GTK scenario;
  secure-messaging reads are intentionally unavailable until 26.5.

## Next

Phase 26.5 added EV1 and EV2 secure-messaging reads on top of these session
keys; see [26e-secure-messaging.md](26e-secure-messaging.md).
