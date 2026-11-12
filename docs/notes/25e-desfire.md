# Note: phase 25.5 DESFire EV3 8K inventory

Status: inventory complete 2026-11-12; factory-key authentication and
primitive file commands validated; NDEF formatting and user-facing writes are
not implemented.
Plan: [25-card-support.md](../plan/25-card-support.md). Spec:
[25e-desfire.md](../specs/25e-desfire.md).

## Card

- MIFARE DESFire EV3 8K.
- Random UID enabled: observed selection UID `04 77 1F A2 BB 17 90`; the last
  four bytes `A2 BB 17 90` are stable. An earlier session observed
  `04 4D 09 A2 BB 17 90`.
- Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6.

## Inventory

`nfctl desfire info`:

```text
MIFARE DESFire EV3 · vendor 04 type 01 subtype 01 version 51.0 storage 1A protocol 05
applications: none
```

`GetKeySettings` (`0x45`) returned `0F 01`, so the PICC has one key.
`GetApplicationIDs` returned an empty list. No authentication or write was
performed.

## Key investigation

- `AuthenticateEV2First` (`0x71`) with an AES zero key returned `91AE`.
- `AuthenticateISO` (`0x1A`) returned an 8-byte challenge with `91AF`, which is
  the DES/2K3DES response shape.
- Follow-up on 2026-11-12: the factory PICC key is legacy DES/2K3DES all zero.
  `Authenticate` (`0x0A`) with an 8-byte zero DES key authenticates both
  directions when the second frame uses D40 send mode (XOR with the previous
  ciphertext, then 3DES decrypt). RndA from the card matches. A 16-byte
  2K3DES zero key also authenticates; `0x1A` 3K3DES and `0x71` AES do not.
- The earlier 54-key dictionary run failed because it used standard CBC for the
  second frame and the wrong command/key type. The card is factory-default, not
  custom-keyed.

No NDEF application was created. The CLI deliberately exposes only
`desfire info`; native authentication and application creation are not exposed
through the CLI or GUI yet. The PICC key is no longer the blocker.

## Implementation

- New `nfc-desfire` crate with `GetVersion`, `GetApplicationIDs`,
  `SelectApplication`, `GetFileIDs`, and `GetFileSettings`.
- Legacy DES/2K3DES authentication (`0x0A`) with D40 send mode, session-key
  derivation, and random RndA.
- Primitive `CreateApplication`, `DeleteApplication`, `CreateStdDataFile`,
  `WriteData`, and `ReadData` commands. They are not exposed through the CLI or
  GUI yet.
- `Service::desfire_inventory` and `nfctl desfire info`.
- No GUI surface is added; this remains a read-only diagnostic.

## Temporary application round trip

The hardware test on 2026-11-12 used the factory DES zero key on PN532 UART:

```text
create application F0 0D 01 (1 key, 2K3DES)
authenticate app key 0
create standard file 01 (plain, 32 bytes, key-0 access)
write 16 bytes at offset 0
read 16 bytes; payload matched
delete application F0 0D 01
GetApplicationIDs: empty
```

The test is idempotent: it removes a leftover reserved AID before starting and
uses the same reader for cleanup. It deletes only `F0 0D 01`.

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-desfire --features hardware --test hardware_legacy -- --ignored
```

The ACR122U row remains unrecorded.

## To continue

1. Expose the authenticated read path through `nfc-app`, `nfctl`, and a
   read-only GUI DESFire workspace; keep keys session-scoped and out of logs.
2. Decode file settings and read Standard/Backup/Value/Record contents, then
   add MACed/enciphered read for EV1/EV2 secure messaging.
3. Add NDEF Type 4 read for DESFire; formatting stays Advanced.
4. Do not add business templates (access control, wallet, transit).

A changed PICC key is not recoverable from the card; record and back it up
before any key change.
