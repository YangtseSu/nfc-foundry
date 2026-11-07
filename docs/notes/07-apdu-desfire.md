# Phase 07 implementation note: ISO/IEC 14443-4 APDU and DESFire read

Date: 2026-11-07

## Implemented

- `internal/isodep`:
  - RATS `E0 80`, ATS length/CRC validation, FSCI/FWI and optional TA/TB/TC
    parsing.
  - I-block short APDUs with CRC-A, command and response chaining, R(ACK),
    WTX, and block-number validation.
  - Explicit session lifecycle over `nfc.RawTransceiver`; leaving raw mode
    power-cycles the field.
  - Short APDU case 1-4 encoding, status-word preservation, `6C XX` and
    `61 XX` helpers.
- `internal/desfire`:
  - Wrapped native commands `90 INS 00 00 [Lc data] 00`.
  - `GetVersion`, `GetApplicationIDs`, `SelectApplication`, `GetFileIDs`,
    `GetFileSettings`, and standard/backup `ReadData`.
  - `91 AF` additional-frame collection and `91 XX` status errors.
  - Little-endian DESFire AIDs, offsets, lengths, file sizes, and access
    rights.
  - No authentication, key change, formatting, or write command.
- `internal/type4`:
  - NDEF application selection, CC `E103` read/validation, NDEF file read,
    and decoding through the existing `type2` record catalogue.
  - Write with NLEN zeroing, chunked UpdateBinary, final NLEN, and full file
    read-back.
- Application and CLI:
  - `nfctl apdu --raw HEX`.
  - `nfctl desfire info|apps|files AID|read AID FILE`.
  - `nfctl ndef read/write` dispatches Type 4 from `SAK & 0x20`.
  - Type 4 writes save the previous raw message under
    `$XDG_CONFIG_HOME/NFC Foundry/type4-backups/` before writing.
- Card detection:
  - `iso14443_4` for SAK bit 5, outside the Classic/Type 2 signatures.
- Added a mock ISO-DEP reader for CLI-level tests.

## Verification

- `go test ./...` and `go vet ./...` pass.
- `make check` passes: i18n, headless GUI, and REUSE lint.
- ISO-DEP tests cover ATS parsing with optional bytes, CRC/length rejection,
  APDU cases 1-4, command chaining, response chaining, WTX, and wrong block
  numbers.
- DESFire tests cover version, application IDs, file settings, standard file
  read, `91AF` chaining, and status errors.
- Type 4 tests cover CC parsing, NDEF decode, write/read-back, capacity,
  read-only access, oversized NLEN, and select errors.
- CLI tests cover raw APDU, DESFire info/apps/read, Type 4 NDEF read/write,
  backup creation, and card-family dispatch.

## Hardware validation

Command:

```text
make libnfc-apdu-hardware-test \
  LIBNFC_DEVICE=acr122_usb:001:004 \
  NFCX_HARDWARE_DESFIRE_CONFIRM=I_OWN_THIS_CARD \
  NFCX_HARDWARE_TYPE4_CONFIRM=I_OWN_THIS_TAG
```

On 2026-11-07 the available ACR122U held the Classic 1K test card
(`FA B9 CF F4`, SAK `08`). `TestHardwareISODEP` skipped because the card is not
ISO/IEC 14443-4; the DESFire and Type 4 tests also skipped before touching the
card. No Type 4 or DESFire card was available, so DESFire identify/read and
Type 4 NDEF are **not hardware-validated**. Add
`NFCX_HARDWARE_TYPE4_WRITE=I_OWN_THIS_TAG` and
`NFCX_TEST_TYPE4_BACKUP_DIR=/path/to/backups` to authorize the Type 4 write
test.

## Problems and resolutions

- ATS T0 places FSCI in the low nibble and TA/TB/TC presence in bits 5/6/7.
  The parser follows the ISO layout rather than assuming nfcpy's byte offsets.
- Command frames must respect the card FSC, not the reader's maximum frame
  size; the session chunks at `FSC - 3` and handles the chaining ACK.
- Type 4 cards may answer ReadBinary with `6C XX`; the reader retries once
  with the returned Le and clamps it to the CC MLe limit.
- DESFire native fields are little-endian; access rights, file sizes, offsets,
  and lengths are parsed and emitted LSB first.
- Loading a dump no longer marks its metadata keys verified; this is recorded
  in the 05g note and the CLI restore documentation.

## Limitations

- No PPS, bitrate change, CID/NAD, Type B activation, or extended APDU length.
- No DESFire authentication, key management, formatting, writes, or protected
  file reads.
- Type 4 NDEF write requires free write access; no authentication.
- Hardware validation for ISO-DEP, DESFire, and Type 4 is pending.
