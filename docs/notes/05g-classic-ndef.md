# Phase 05g implementation note: NDEF on MIFARE Classic

Date: 2026-11-07

## Implemented

- Added `internal/mifare/mad.go`:
  - MAD1 and MAD2 parsing with the 8-bit CRC (polynomial `0x1D`, mirrored
    preset `0xC7`), DA/MA/version validation, and AID maps.
  - MAD encoding for formatting, including sector 16 for MAD2.
  - NDEF sector discovery with contiguity validation that permits the MAD2
    sector-16 gap.
  - NDEF area block selection that excludes every sector trailer.
  - GPB validation for the mapping version and read/write access fields.
- Added `internal/workflow/classic_ndef.go`:
  - `Read` with catalog keys first, then the public MAD/NDEF Key A and the
    transport default keys. Successful keys are returned for catalog
    verification.
  - `Write` that plans changed data blocks only, saves a project dump backup,
    authenticates each block with a key allowed by the current access bits,
    writes, and verifies by read-back.
  - `PreviewFormat` and `Format` for a contiguous sector range. The formatter
    supports an existing MAD with free/NDEF sectors and a blank transport-state
    card. It writes data blocks, NDEF trailers, MAD blocks, then the MAD sector
    trailer, preserving Key B; a secret unverified Key B refuses before the
    first write.
  - Final verification reselects the card and compares the complete NDEF area
    and NDEF sector list.
- Added `internal/workflow/classic_ndef_backup.go`; project dumps are written
  with directory mode 0700 and file mode 0600.
- Added app DTOs and service methods for Classic NDEF read/write/format,
  generic NDEF write dispatch by card family, and automatic Classic NDEF load
  after `StartReadCard`.
- Added the Classic NDEF tab to the MyGo GUI with:
  - mapping facts, free sectors, capacity, and decoded records;
  - the existing template, raw, and multi-record editors;
  - an Advanced-mode format panel with sector range input, preview, old/new
    values, backup directory, and typed card-UID confirmation.
- Extended `nfctl ndef`:
  - `read`/`write` now select Classic or Type 2 from the card SAK;
  - `--keys` imports and scans a Classic dictionary;
  - `ndef format --sectors N-M` requires `--yes` and `--confirm-format`.
- Added English and Simplified Chinese catalogue entries (507 messages).
- Added `TestHardwareClassicNDEF` and the
  `libnfc-classic-ndef-hardware-test` Make target.

## Mapping details

- The on-card NDEF AID is `03 E1`; the 16-bit value is `0x03E1`.
- MAD1 block 1 is `CRC, info, AID1..AID7`; block 2 is `AID8..AID15`.
- MAD2 sector 16 block 0 is `CRC, info, AID17..AID23`; block 1 and 2 carry
  AID24..AID39.
- The NDEF TLV starts at byte 0 of the first NDEF sector. A zero-length NDEF
  TLV is the initialized state and is not a malformed message.
- The standard sector trailers are `A0A1A2A3A4A5 78 77 88 C1` for MAD1
  (`C2` for MAD2) and `D3F7D3F7D3F7 7F 07 88 40` for an NDEF sector.
- A 4K blank card is formatted as MAD1, so only sectors 1-15 are managed;
  sectors 16-39 stay free. Extending a MAD2 card can use 17-39.

## Safety behavior

- Ordinary NDEF writes never change a sector trailer.
- Format refuses a sector with any AID other than free or NDEF.
- A blank-format target must authenticate sector 0 and every selected sector
  with the default Key A, use the transport trailer `FF 07 80`, and contain
  zero data in sector 0 blocks 1-2 and every selected data block. A card with
  data and no MAD is refused instead of silently overwritten.
- The MAD sector trailer is written last so a partial failure never advertises
  an incomplete NDEF application.
- A masked Key B is never guessed. When a trailer has to change, the current
  Key B must be readable or verified; otherwise the card is not modified.
- Backup failure, card change, malformed MAD/TLV, read-only GPB, and unreadable
  NDEF sectors abort before the first write.
- `nfctl` output remains English-only; all new GUI strings are localized.

## Verification

- `make check` passes: `go test ./...`, `go vet ./...`,
  `go tool mygo i18n check .`, the MyGo headless view tests, and `reuse lint`
  (413/413 files annotated).
- `internal/mifare` tests cover the published `F3` CRC vector, MAD1/MAD2 parse
  and round-trip, bad CRC/version, no-DA, AID byte order, contiguity around
  sector 16, area blocks, and GPB validation.
- Workflow tests cover formatted read, no-MAD read, write block planning and
  read-back, read-only refusal, card change, blank-card format, other-application
  refusal, unverified Key B refusal, malformed TLV, and backup file permissions.
- CLI tests cover Classic NDEF read/write, format preview and execution, and
  sector-range guards.
- Desktop tests cover the Classic NDEF tab, decoded records, write
  confirmation, format preview, and typed UID confirmation.

## Hardware validation (2026-11-07)

- Reader: ACR122U, `acr122_usb:001:004`.
- Card: one blank MIFARE Classic 1K, 4-byte UID `FA B9 CF F4`, transport
  trailer `FF 07 80 69`, default `FF...FF` Key A/B, all data zero.
- `TestHardwareClassicNDEF` formatted sectors 1-2 (MAD1, 96-byte NDEF
  capacity), wrote a 52-byte Text record, and re-read the card in 16.11s.
  A second run without the format gate wrote another 52-byte record to the
  formatted card and passed in 9.25s. Both runs saved project dumps under the
  configured Classic backup directory.
- CLI validation on the same card and reader:
  - `nfctl ndef read` decoded the 52-byte Text record and the MAD1 NDEF
    sectors 1-2.
  - `nfctl ndef write --template text --field text='CLI hardware NDEF OK'`
    planned/wrote/verified blocks 4, 5, 6, and 8; the read-back was exact.
  - `nfctl ndef format --sectors 1-2 --dry-run` previewed a valid plan with
    no trailer or MAD changes.
  - `nfctl ndef format --sectors 1-2 --yes --confirm-format` wrote and
    verified blocks 4-5, saved a project dump, and left an initialized NDEF
    application with no message.
  - A final `nfctl ndef write` wrote `NFC Foundry Classic NDEF hardware OK`
    (blocks 4-6) and `nfctl ndef read` returned the same text.
- Not tested on hardware: Classic 4K/MAD2, secret unreadable Key B, format of
  a card with another MAD application, and read-only GPB.

The explicit gate remains:

```text
make libnfc-classic-ndef-hardware-test \
  LIBNFC_DEVICE=acr122_usb:001:004 \
  NFCX_HARDWARE_CLASSIC_NDEF_CONFIRM=I_OWN_THIS_CARD \
  NFCX_TEST_CLASSIC_NDEF_BACKUP_DIR=/path/to/backups \
  NFCX_TEST_KEY_A=FFFFFFFFFFFF \
  NFCX_TEST_KEY_B=FFFFFFFFFFFF
```

Set `NFCX_HARDWARE_CLASSIC_NDEF_FORMAT=I_OWN_THIS_CARD` and
`NFCX_HARDWARE_CLASSIC_NDEF_SECTORS=1-2` to format a blank disposable card
before the write test.

## Limitations

- MAD3 and DESFire application directories are not implemented.
- A classic NDEF application with a secret, unverified Key B can be read and
  written through its data blocks, but its trailer cannot be reformatted.
- Formatting a blank 4K card creates MAD1 only; sectors 16-39 remain unmanaged
  until a separate MAD2 extension is implemented.
- No Type 4 or FeliCa NDEF.

## Problems and resolutions

- The MAD CRC preset is documented as `0xE3`, but the reader-side algorithm
  uses the mirrored value `0xC7`. The published `F3` test vector confirmed the
  implementation.
- Hardware masks Key A and, for a secret Key B, masks both key fields when a
  trailer is read. Trailer verification therefore compares only the visible
  access/GPB bytes and, when readable, Key B. The new Key A is proven by the
  final authentication and complete re-read.
- "Trailer already compliant" cannot compare Key A bytes from a trailer read.
  The formatter compares access bytes and GPB, then treats the standardized
  public Key A authentication as the Key A proof. This avoids requiring a
  secret Key B to reformat a card whose trailers are already standard.
- A zero-length NDEF TLV is legal in the Classic initialized state. The shared
  TLV parser now reports it as `Present: false` instead of a malformed message,
  so an initialized card reads cleanly and `ndef format` can verify its result.
- Formatting a blank card with the default key requires distinguishing the
  transport default Key A from the standardized public MAD Key A. The test
  fixture verifies keys; the CLI test imports a one-key dictionary so the
  default key outranks the built-in catalogue.

## Deviations from the spec

- None in the mapping or safety rules. The implementation writes the MAD
  sector trailer last instead of first; the spec records this as the chosen
  failure ordering.
- Hardware validation is recorded above for Classic 1K on an ACR122U. Classic
  4K/MAD2 and secret-Key-B paths remain hardware-test follow-ups.
