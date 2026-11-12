# Note: phase 25.5 DESFire EV3 8K inventory

Status: inventory complete 2026-11-12; NDEF formatting blocked on the PICC key.
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

- `AuthenticateEV2First` (`0x71`) with the factory AES zero key returned
  `91AE`.
- `AuthenticateISO` (`0x1A`) returned an 8-byte challenge with `91AF`, which is
  the DES/2K3DES response shape.
- The PM3 `mfdes_default_keys.dic` dictionary was tried read-only with fresh
  selections: 54 candidates (DES, 2K3DES, AES defaults, common vendor keys) all
  failed the RndA check with `91AE`.
- The card's PICC key is custom or otherwise unknown. DESFire keys cannot be
  recovered from the card; a known key or PM3-assisted identification is
  required.

No NDEF application was created. The CLI deliberately exposes only
`desfire info`; the format/delete path is withheld until a key and secure
channel are validated.

## Implementation

- New `nfc-desfire` crate with `GetVersion`, `GetApplicationIDs`,
  `SelectApplication`, `GetFileIDs`, and `GetFileSettings`.
- `Service::desfire_inventory` and `nfctl desfire info`.
- No GUI surface is added; this is a read-only diagnostic until formatting is
  unblocked.

## To continue

1. Provide the PICC key (16 hex for DES, 32 hex for 2K3DES, 32 hex for AES), or
2. run PM3 `hf mfdes info` / `hf mfdes detect` on the card at home and share the
   key type/result, or
3. use PM3 to format the card and then validate the Type 4 read path.

A custom key that PM3 cannot detect leaves the card unformattable by any tool.
