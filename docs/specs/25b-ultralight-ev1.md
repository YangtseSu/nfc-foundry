# Spec: phase 25.2 MIFARE Ultralight EV1

Status: implemented 2026-11-12; MF0UL11 hardware validated on PN532 UART,
MF0UL21 unit-tested only. Plan:
[25-card-support.md](../plan/25-card-support.md).

## Problem

The Type 2 layer selected layouts from the capability container alone. A
48-byte CC mapped to the legacy Ultralight layout (16 pages, no configuration
pages), and a 128-byte CC mapped to `Generic` (no configuration pages). A
blank Ultralight EV1 has no valid CC at all, so `type2 read` failed at the CC
magic before reading any page. The configuration model also assumed NTAG21x
MIRROR/MIRROR_PAGE bytes and cleared EV1 VCTID on a password write.

## Product identification

`Reader::type2_version` sends GET_VERSION (`0x60`) and returns the eight-byte
descriptor. It is implemented by PN532 UART, ACR122U, and the mock reader.
GET_VERSION is optional: a refused or malformed response falls back to the CC
size, preserving legacy Ultralight and NTAG21x behavior.

The parser identifies these exact NXP descriptors:

| Product | Product type | Major | Storage | Layout |
|---|---|---|---|---|
| MF0UL11 | `0x03` | `0x01` | `0x0B` | 20 pages, user 4..15, config 16..19 |
| MF0UL21 | `0x03` | `0x01` | `0x0E` | 41 pages, user 4..35, lock page 36, config 37..40 |
| NTAG213 | `0x04` | `0x01` | `0x0F` | existing 45-page layout |
| NTAG215 | `0x04` | `0x01` | `0x11` | existing 135-page layout |
| NTAG216 | `0x04` | `0x01` | `0x13` | existing 231-page layout |

## Read behavior

- With GET_VERSION, the version layout wins over the CC-size layout.
- A valid CC still supplies the NDEF data-area size and access nibbles. An EV1
  CC whose size differs from the layout user bytes produces a warning.
- An unformatted EV1 (page 3 is not `E1 ...`) is read as a complete page image
  with a default empty CC and the warning
  `no valid Type 2 capability container; the tag memory is unformatted`.
- Configuration pages, PWD, PACK, and the UL21 lock page are included in the
  image. PWD and PACK are masked to zero as on NTAG21x.

## One-way CC format

`format_ndef_area` has two paths:

- A present CC: replace the NDEF data area as before.
- No CC on an EV1: require `allow_otp_cc` / `--otp` plus a separate
  confirmation. Refuse unless page 3 is exactly `00 00 00 00`. Write
  `E1 10 06 00` for MF0UL11 or `E1 10 10 00` for MF0UL21 to page 3, then write
  the empty NDEF TLV `03 00 FE` and zero fill to the user area.

The CC write is verified by read-back. OTP bits cannot be cleared, so a failure
after the CC write leaves a formatted-but-incomplete tag; the caller reports
the failure and does not claim a rollback. A project backup is written first
but cannot restore page 3. Non-EV1 Type 2 tags without a CC remain refused.

## Configuration and password

- `Configuration` carries VCTID and preserves the EV1 MOD bit 0
  (`STRG_MOD_EN`).
- EV1 CFG0 is `[MOD & 0x01, 0, 0, AUTH0]`; CFG1 is
  `[ACCESS & 0xC7, VCTID, 0, 0]`.
- NTAG21x keeps its MIRROR/MIRROR_PAGE behavior.
- PWD and PACK are written before AUTH0/ACCESS, then the new credentials are
  authenticated and both configuration pages are read back.
- Clearing uses `AUTH0=0xFF`, `ACCESS=0x00`, and factory PWD/PACK. CFGLCK,
  AUTHLIM, and lock bits are never set.

## Surfaces

- `nfctl type2 info|read|dump` reads a blank EV1.
- `nfctl ndef format --otp` writes the one-way CC; without `--otp` the format
  is refused before any write.
- The GTK Type 2 format flow shows a dedicated OTP dialog with the exact CC
  bytes and the irreversibility warning. The normal dialog keeps
  `allow_otp_cc=false`.
- `nfctl type2 password` and the GTK Protection form share the same service
  and confirmation.

## Acceptance

- MF0UL11 is identified from GET_VERSION, not from the 48-byte CC: 20/20 pages
  readable, product name correct, no CC reported as unformatted.
- OTP format writes `E1 10 06 00`, NDEF capacity 45 bytes, and an empty NDEF
  state; all pages read back.
- A Text record writes, reads back, and clears to empty.
- Password set with `AUTH0=4`, `ACCESS=0x80`: unauthenticated read fails, the
  correct PWD/PACK reads 20/20 pages, a wrong PACK is refused, and clearing
  restores `AUTH0=FF`, `ACCESS=00`, and VCTID `0x05`.
- MF0UL21 layout and OTP size are covered by unit tests; a physical UL21
  record remains pending.

## Out of scope

- MF0UL21 hardware validation.
- One-way lock bits, CFGLCK, AUTHLIM, and counter commands.
- UID-changeable clones; that is 25.4.
