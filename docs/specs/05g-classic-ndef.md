# Phase 05g spec: NDEF on MIFARE Classic

Date: 2026-11-07

## Goal

Read, decode, write, and initialize NDEF on MIFARE Classic 1K/4K cards through
the MIFARE Application Directory mapping (AN10787, AN1304, AN1305). Reuse the
existing key catalog, dump backup, card-identity, and read-back rules. Do not
share Classic block or trailer logic with the Type 2 module.

## Mapping model

`internal/mifare` owns the directory and mapping; `internal/type2` keeps the
TLV and record codec.

- MAD1 lives in sector 0 blocks 1-2. Block 1 is `CRC, info, AID1..AID7`;
  block 2 is `AID8..AID15`. Each AID is stored big-endian, so NDEF is
  `03 E1`. The CRC is CRC-8 with polynomial `0x1D`, mirrored preset `0xC7`,
  over the info byte and the 30 AID bytes in card order.
- MAD2 additionally uses sector 16; block 0 is `CRC, info, AID17..AID23`,
  block 1 is `AID24..AID31`, block 2 is `AID32..AID39`.
- The MAD sector GPB is byte 9 of its trailer: DA is bit 7, MA is bit 6, and
  the mapping version is bits 1-0 (`01` MAD1, `10` MAD2).
- NDEF sectors are the sectors whose AID is `0x03E1`. They must be contiguous;
  a MAD2 sequence may skip sector 16. The NDEF byte area is the concatenation
  of the data blocks of those sectors in sector order, excluding every sector
  trailer.
- Each NDEF sector trailer must use GPB version 1.0 (bits 7-4 = `0100`), read
  access `00`, and write access `00` or `11`. The standard trailer is
  `D3F7D3F7D3F7 7F 07 88 40`; the standard MAD sector trailer is
  `A0A1A2A3A4A5 78 77 88 C1` (`C2` for MAD2).
- The NDEF TLV starts at byte 0 of the first NDEF sector. A zero-length NDEF
  TLV means the application is initialized but has no message.
- `mifare.ParseMAD` validates the CRC and version. A clear DA bit reports
  "no MAD"; a set DA bit with a bad CRC or unsupported version is an error.

## Public keys and the catalog

The mapping reserves two public Key A values: `A0A1A2A3A4A5` for MAD sectors
and `D3F7D3F7D3F7` for NDEF sectors. Reads try the caller's verified catalog
keys first, then the public key for the sector role. A successful public-key
authentication is registered as read-context verification.

Writes use a verified key whose access condition permits the block write.
Trailer writes require a key that can change Key A, the access bytes, and
Key B together. A secret, unreadable Key B is never guessed; when a trailer
must change and Key B is unknown, the operation refuses before writing.

## Read

`workflow.ClassicNDEFService.Read` selects the expected card, reads sector 0
and the NDEF sector trailers and data blocks, and returns:

- the parsed MAD and its NDEF sector list;
- the concatenated NDEF area and capacity;
- the decoded `type2.NDEF`, including decoded records;
- the sector keys that authenticated.

A card with no MAD or no NDEF AID is a successful read with the application
marked absent. Malformed MAD, malformed TLV, unreadable NDEF sectors, and card
changes are errors.

## Write

A write requires an existing NDEF application and a writable GPB (write access
`00`). The service re-reads the current area, builds the desired area with
`type2.BuildNDEFArea`, and writes only changed 16-byte data blocks. Blocks are
written in ascending order, each with authentication and immediate read-back.
The service stops at the first unrecoverable failure. Sector trailers are
never written by the ordinary message write.

Before the first write the service reads the card with the supplied keys and
saves the resulting project dump under the Classic backup directory. NDEF data
blocks and the MAD must be fully known; unrelated unreadable sectors make the
backup partial but do not block the write.

## Format

Formatting targets a contiguous sector range. It is the Advanced-mode
operation; the GUI requires a typed UID confirmation and the CLI requires
`--yes` and `--confirm-format`.

Two inputs are accepted:

1. An existing valid MAD: the selected sectors must be free (`0000`) or
   already NDEF (`03E1`), and no other AID is overwritten. MAD1 permits
   sectors 1-15; MAD2 also permits 17-39.
2. A blank transport-state card: sector 0 authenticates with the default
   `FF...FF` Key A, its trailer access bytes are `FF 07 80`, sector 0 blocks
   1-2 and every selected data block are zero, and every selected sector has
   the same transport trailer. The service creates a MAD1 with the selected
   NDEF AIDs. A 4K card is formatted as MAD1; sectors 16-39
   stay unmanaged.

The formatter writes, in order: NDEF sector data blocks (empty NDEF TLV
`03 00 FE` followed by zeros), NDEF sector trailers with the public NDEF
Key A, access `7F 07 88`, GPB `40`, and the preserved Key B, then MAD blocks
with the updated AIDs and CRC, and finally the sector 0 trailer with the
public MAD Key A, access `78 77 88`, the correct GPB, and the preserved Key B.
The MAD is written last so a failure never advertises an incomplete NDEF
application. Writing every selected sector trailer requires a known Key B;
a masked Key B is reported before the first write.

A full-card dump is read and saved before the first write. The complete card
UID/ATQA/SAK must match before and after the operation. Every changed block is
verified immediately, then the service reselects the card, re-reads the MAD
and NDEF area, and compares the final mapping and TLV area.

## Application and CLI

The CLI uses the same `nfctl ndef` surface for both card families and selects
the implementation from the selected card's SAK:

```text
nfctl ndef read [--keys FILE] [--raw] [--reveal]
nfctl ndef write (--file F|--hex H|--template ID [--field K=V]...) \
    [--keys FILE] [--yes] [--dry-run]
nfctl ndef format --sectors N[-M] [--keys FILE] [--yes] [--confirm-format]
    [--dry-run]
nfctl type2 dump/restore ...
```

`--keys` imports a dictionary and runs the normal key scan before the NDEF
operation. JSON output keeps the existing `ndef.read` / `ndef.write` command
names and adds the MAD summary for Classic. Human output prints the mapping
state, capacity, backup path, and planned/written/verified blocks.

## GUI

The Classic workspace gains an NDEF tab. It shows the MAD version, CRC state,
NDEF sectors, capacity, and decoded records. The existing template, raw, and
multi-record editors drive the Classic message write. An Advanced-mode format
panel selects a contiguous sector range, previews the MAD/NDEF changes and
backup path, and requires the card UID to be typed before execution. Advanced
mode resets on card removal and device change as before.

## Safety

- No block 0 write.
- No trailer write outside format.
- Format never writes a sector with a non-free, non-NDEF AID.
- A card change aborts before the next write.
- Backup failure aborts before writing.
- A failed or partially applied format leaves the backup path visible and
  never reports success.
- `nfctl` stays English-only; GUI strings go through the i18n catalogue.

## Tests

Hardware-free:

- MAD1/MAD2 parsing, CRC, AID order, version/DA validation, contiguity, and
  NDEF sector discovery.
- Read of a formatted 1K card, a 4K MAD2 card, an initialized card, and a card
  without MAD.
- Public-key fallback and catalog-key precedence.
- Message write changes only the required data blocks and verifies read-back.
- Malformed TLV, read-only GPB, missing key, masked Key B during format, and
  card change refusals.
- Format of a blank transport card and of a MAD with free sectors, including
  MAD CRC, sector trailers, and final NDEF verification.
- CLI read/write/format smoke tests with the mock reader.
- GUI NDEF tab render, write confirmation, and Advanced format confirmation.

Hardware, explicit only:

- `TestHardwareClassicNDEF` reads a formatted Classic 1K, writes a text
  message, and verifies it through the in-process reader.
- `NFCX_HARDWARE_CLASSIC_NDEF_FORMAT=I_OWN_THIS_CARD` additionally authorizes
  formatting a disposable blank Classic 1K.

## Non-goals

- No automatic reformatting of arbitrary Classic cards.
- No UID or manufacturer-block write.
- No MAD3/DESFire directory support.
- No Type 4 or FeliCa NDEF.
- No claim that a partially known Key B can be preserved.
