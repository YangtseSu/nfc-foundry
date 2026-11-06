# Phase 05 spec: Type 2 / NTAG21x and NDEF

Date: 2026-11-06

## Goal

Add MIFARE Ultralight and NTAG213/215/216 page access and committed NDEF
read/write. Type 2 has no Crypto1, no sectors, and no Classic trailers; its
commands, memory model, write rules, and dumps are separate from the Classic
module.

## Transport

Type 2 commands use raw ISO/IEC 14443A framing through the existing
`RawTransceiver` capability. A `type2.Session` enables raw mode, appends CRC-A
to each command, and validates/removes CRC-A from data responses. Raw mode is
left, with an RF power cycle, before any card reselection.

The ordinary libnfc `InDataExchange` path was rejected on the validated ACR122U:
`GET_VERSION` failed with the driver's `Command Code verification failed`
message. The same reader and NTAG215 returned correct `GET_VERSION` and `READ`
frames through raw `InCommunicateThru`, so the product uses the hardware-proven
path. The C shim needs no Type 2-specific call; raw mode and byte transceive
already exist for Gen1A.

A session is explicit because raw mode must persist across a command sequence
and must be disabled before `CardInfo` reselects a card:

```go
session, err := type2.OpenSession(ctx, reader)
defer session.Close(context.WithoutCancel(ctx))
document, err := type2.ReadSession(ctx, session, card, type2.ReadOptions{})
```

`nfc.RawTransceiver` remains the only transport boundary. No card protocol
pointer or libnfc type crosses it.

## Commands

`internal/type2` implements only framed command semantics:

- `READ (0x30)` one page -> four pages (16 bytes);
- `FAST_READ (0x3A)` start/end page -> 4 bytes per page;
- `WRITE (0xA2)` page plus four data bytes -> ACK;
- `PWD_AUTH (0x1B)` four-byte password -> two-byte PACK;
- `GET_VERSION (0x60)` -> eight product bytes.

ACK is `0x0A` in either nibble alignment. A one-byte non-ACK frame is a NAK and
is reported as a protocol error. Data responses must have the expected length;
`FAST_READ` responses must match the requested page count.

Writes are read back immediately. A mismatch is a verification failure, not a
silent success.

## Product and geometry

Detection order:

1. `GET_VERSION`. An NXP NTAG response is recognized by `vendor=0x04`,
   `product=0x04`, `subtype=0x02`, major `0x01`, minor `0x00`, protocol `0x03`,
   and storage size `0x0F`, `0x11`, or `0x13` for NTAG213/215/216.
2. If `GET_VERSION` is answered with a protocol failure, fall back to original
   MIFARE Ultralight: 16 pages, user pages 4-15, page 2 lock bytes, page 3 OTP.
   The fallback is accepted only after page 0/1 UID and BCC validation.

Unknown products are rejected before any write.

| Product | Pages | User pages | User bytes | Dynamic lock | Config | PWD/PACK |
|---|---:|---|---:|---|---|---|
| MIFARE Ultralight | 16 | 4-15 | 48 | - | - | - |
| NTAG213 | 45 | 4-39 | 144 | 40 | 41-42 | 43/44 |
| NTAG215 | 135 | 4-129 | 504 | 130 | 131-132 | 133/134 |
| NTAG216 | 231 | 4-225 | 888 | 226 | 227-228 | 229/230 |

Page 0/1 contain UID bytes and BCC0; page 2 contains BCC1, internal and static
lock bytes; page 3 is the capability container on NTAG or OTP on original
Ultralight. Pages 0-3 are never written.

The dump validates page 0 [0:3] against the selected UID, BCC0
(`0x88 ^ UID0 ^ UID1 ^ UID2`), and for seven-byte UIDs page 1 [0:4] against
UID3-UID6 plus page 2 byte 0 BCC1 (`UID3 ^ UID4 ^ UID5 ^ UID6`).

## Capability container and NDEF

The CC is page 3. A valid formatted CC has magic `0xE1`, version major `0x10`,
size in 8-byte units, and access `0x00` or `0x0F`. The NDEF capacity is
`min(CC size * 8, user bytes)`. NTAG defaults are `E1 10 12 00`, `E1 10 3E 00`,
and `E1 10 6D 00`.

TLV parsing starts at the first user page and is bounded by the CC size:

- NULL `0x00` is skipped;
- Terminator `0xFE` stops parsing;
- NDEF `0x03` uses a one-byte length (0-254) or `0xFF` plus a two-byte
  big-endian length;
- Lock/Memory Control and proprietary TLVs are skipped after length
  validation.

NDEF messages are parsed and validated record by record: MB/ME/CF flags, TNF,
short/long payload length, ID length, type and payload bounds. Malformed NDEF
is reported without writing.

NDEF write replaces the NDEF TLV and terminator/padding inside the CC capacity.
It refuses to write when a non-NULL, non-terminator TLV other than NDEF precedes
the NDEF TLV. It never writes outside the CC capacity or into pages 0-3.

## Safety

- Default writes target user-memory pages only.
- Static lock, dynamic lock, configuration, PWD/PACK, and authentication-
  protected pages are dangerous.
- `type2 write` rejects protected pages; protected changes require
  `type2 restore --protected` with `--yes` and `--confirm-protected`.
- Every write creates a full-image backup first, under
  `$XDG_CONFIG_HOME/NFC Foundry/type2-backups/`.
- Static lock bytes and the capability container live in pages 0-3 and are
  never written. The protected restore path can change dynamic lock,
  configuration, PWD, and PACK pages.
- Every written page is read back. Restore stops on the first unrecoverable
  failure.
- Before and after a write, reselect the card and abort on a UID, ATQA, or SAK
  change.
- PWD_AUTH is optional. A supplied password is used for read/write; it is never
  echoed in CLI or GUI output.
- PWD and PACK pages are masked in CLI and GUI output unless `--show-keys` is
  set. Dump files contain them for full restore and are written mode 0600.
- A full restore writes ordinary user pages first, then PWD/PACK, configuration
  pages, and lock bytes. Pages 0-3 are excluded.
- A dump only restores onto the same product and UID.

## Backup, failure handling, and rollback

- A full-image backup is written before the first page update. A backup write
  failure aborts the operation before any tag change.
- Each page is verified immediately. A write that is acknowledged but does not
  read back reports the pages already written and the backup path.
- A failure stops the operation; there is no continue-on-error mode.
- If the card changes after a write, the operation reports `card_changed` and
  leaves the backup for manual recovery.
- A backup restores through `type2 restore`; protected pages still require the
  confirmed path.
- The operation never writes pages 0-3, so UID and lock/capability metadata
  cannot be corrupted by an ordinary page write.

## Packaging and license impact

- No new runtime dependency. The Type 2 module uses the existing libnfc-ng
  transport and Go standard library.
- No libnfc-ng change or utility is added.
- New source files are covered by `REUSE.toml`'s default GPL-3.0-or-later
  annotation; `reuse lint` must pass.
- Type 2 fixtures under `testdata/type2/` are synthetic and contain no real
  user data.

## Dump format

Type 2 dumps are JSON, format `nfc-foundry/type2`, version 1:

```json
{
  "format": "nfc-foundry/type2",
  "formatVersion": 1,
  "product": "ntag213",
  "productName": "NTAG213",
  "uid": "04AABBCCDDEEFF",
  "atqa": "0044",
  "sak": "00",
  "version": "0004040201000F03",
  "totalPages": 45,
  "userPages": {"start": 4, "end": 39},
  "pages": ["...45 four-byte hex strings..."],
  "cc": {"valid": true, "raw": "E1101200", "magic": 225, "version": 16, "size": 18, "access": 0, "capacity": 144},
  "protection": {"staticLock": "0000", "dynamicLock": "000000", "auth0": 255, "access": 0, "protect": false, "configLocked": false, "authLimit": 0, "passwordSet": false, "packSet": false},
  "ndef": {"present": true, "length": 6, "raw": "D1010354...", "records": []}
}
```

`ndef` is informational; restore derives the written bytes from `pages`. CLI and
GUI read output use the same DTOs.

## CLI

```text
nfctl type2 info
nfctl type2 read [--page N] [--pwd HEX] [--all]
nfctl type2 write --page N --data HEX [--pwd HEX] [--yes] [--dry-run]
nfctl type2 dump <file.json> [--pwd HEX]
nfctl type2 restore <file.json> [--pwd HEX] [--protected] [--yes] \
    [--confirm-protected] [--dry-run]
nfctl ndef read [--raw] [--pwd HEX]
nfctl ndef write (--file <path>|--hex <hex>) [--pwd HEX] [--yes] [--dry-run]
```

Results use stable keys `product`, `pages`, `page`, `userPages`, `cc`, `ndef`,
`protection`, `path`, `backup`, `written`, `verified`, `skipped`, `dryRun`.
Writes require `--yes`; protected restores additionally require
`--confirm-protected`. `--dry-run` performs identification and planning without
writing.

## GUI

The Type 2 tabs become live after `Read card`:

- Memory: page table with region, hex and ASCII; system pages included but not
  editable.
- NDEF: capability container, message length, record summary, text/URI payload,
  or an explicit unformatted/malformed message.
- Protection: static/dynamic lock bytes, AUTH0, ACCESS flags, and whether a
  password is set. Password and PACK bytes are masked.

The GUI only reads Type 2 cards in this phase. Destructive Type 2 and NDEF
writes remain CLI-only until a confirmation dialog design exists.

## Tests

Hardware-free:

- CRC-A vectors from the Gen1A path;
- exact CRC-A READ/FAST_READ/WRITE/PWD_AUTH/GET_VERSION frames;
- ACK/NAK handling and response-length validation;
- NTAG213/215/216 and Ultralight geometry;
- BCC0/BCC1, CC, TLV, NDEF record parsing, long lengths and malformed input;
- dump/restore JSON validation and protected-page refusal;
- mock-tag workflow and CLI tests including read-back mismatch and card change;
- raw-session CRC-A validation and ACK/NAK handling;
- dry-run behaviour that must not create a backup or modify the tag;
- desktop view tests for the three Type 2 tabs.

Explicit hardware:

- `NFCX_WORKFLOW_HARDWARE=1`, `NFCX_HARDWARE_TYPE2_CONFIRM=I_OWN_THIS_TAG`,
  `LIBNFC_DEVICE`, and an optional `NFCX_TEST_TYPE2_PWD`.
- Read identity, full dump, and NDEF; write an NDEF message only when the
  confirmation variable is set, then restore the original user memory and
  verify.

## Non-goals

- No UID/page 0-3 writes.
- No FeliCa/Type 3 NDEF.
- No generic "write everything writable".
- No GUI write flow in this phase.
