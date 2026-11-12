# Note: phase 25.2 MIFARE Ultralight EV1

Status: complete for MF0UL11 on 2026-11-12; MF0UL21 hardware record pending.
Plan: [25-card-support.md](../plan/25-card-support.md). Spec:
[25b-ultralight-ev1.md](../specs/25b-ultralight-ev1.md).

## Card

- MIFARE Ultralight EV1 MF0UL11 (48-byte user memory).
- UID `04 43 47 4A 75 22 91`, ATQA `00 44`, SAK `00`.
- Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6.
- Initial state: page 3 `00 00 00 00`, no capability container, 20 pages
  readable after GET_VERSION support; config page 16 `00 00 00 FF`,
  page 17 `00 05 00 00` (AUTH0 `FF`, ACCESS `00`, VCTID `05`).

## Product identification

Before the change, `type2 read` failed with
`the Type 2 capability container magic 0x00 is not 0xE1`. GET_VERSION returned
the MF0UL11 descriptor and the read path selected the 20-page EV1 layout even
without a CC:

```text
product: MIFARE Ultralight EV1 (MF0UL11)
uid: 04 43 47 4A 75 22 91
pages: 20/20 readable
user memory: pages 4..=15 (48 bytes)
capability container: none (unformatted)
NDEF: unformatted
static lock: 00 00  dynamic lock: none
AUTH0: FF  ACCESS: 00  CFGLCK: false  AUTHLIM: 0
```

A full project dump was written to
`/tmp/nfc-ev1-ul11-0443474a752291.project`.

## OTP capability container and NDEF

After explicit confirmation, `nfctl ndef format --otp` wrote the CC:

```text
NDEF formatted: 2 page(s) written, 11 unchanged
capability container: E1 10 06 00
NDEF data area: 48 bytes
NDEF capacity: 45 bytes
NDEF: empty
```

A Text record then wrote and read back:

```text
NDEF written and verified: 8 page(s) written, 4 unchanged
NDEF: 1 record(s), 28 bytes, capacity 45
record 0: text "NFC Foundry EV1 check" (language en)
```

A second `nfctl ndef format` (without `--otp`, because the CC now exists)
returned the image to an empty NDEF state: 8 pages written, 4 unchanged.
Backups are under `/tmp/nfc-ev1-backups`; page 3 is OTP and was not restored.

## Password configuration

Protection was set with `AUTH0=4`, `ACCESS=0x80`, and a test PWD/PACK. The
unauthenticated read failed with `AuthenticationRequired`; the correct
credentials read 20/20 pages; a wrong PACK failed with
`PWD_AUTH returned a PACK that does not match the expected value`. Clearing
with `AUTH0=FF`, `ACCESS=00`, and factory PWD/PACK restored an unprotected
tag.

Final configuration pages:

```text
page 16  configuration: 00 00 00 FF
page 17  configuration: 00 05 00 00
page 18       password: 00 00 00 00
page 19           PACK: 00 00 00 00
```

VCTID `05` was preserved through the set/clear cycle. No lock bit, CFGLCK, or
AUTHLIM was written.

## Code changes

- `Reader::type2_version` with PN532 UART, ACR122U, and mock implementations.
- `Type2Version` parser and explicit MF0UL11/MF0UL21 layouts.
- Blank-EV1 read through GET_VERSION and a default empty CC.
- `format_unformatted_ev1` with the OTP gate; service field `allow_otp_cc`;
  `nfctl ndef format --otp`; GTK OTP confirmation dialog.
- Product-aware configuration pages preserving MOD bit 0 and VCTID.

## Remaining

- No MF0UL21 card was available; the 41-page layout and 128-byte OTP CC are
  unit-tested but not hardware-validated.
- The ACR122U GET_VERSION path is implemented but not recorded on hardware.
- One-way locks and AUTHLIM remain unrecorded and are not used.
