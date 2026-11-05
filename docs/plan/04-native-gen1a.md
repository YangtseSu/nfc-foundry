# Phase 04: native Go Gen1A

## Goal

Remove the `nfc-mfsetuid` executable and keep Gen1A block 0 writing in-process.

## Work

- Extend the C shim with raw mode, `transceive_bits`, and `transceive_bytes`.
- Implement CRC-A, halt, `0x40`/`0x43` unlock, and block 0 write in Go.
- Validate 4-byte UID, Classic 1K, block 0 BCC, and target card identity.
- Require confirmation, keep a block 0 backup, and verify by re-selecting the
  card and reading block 0.
- Delete the external adapter, runtime locator entry, packaging of
  `nfc-mfsetuid`, and its capability check.
- Add fixture/replay tests and a hardware test with a Gen1A card.

## Acceptance

- No `nfc-mfsetuid` binary is shipped or required.
- Gen1A write succeeds on a compatible card and fails with a specific error on
  a normal card.
- Reader remains in-process; no close/reopen handoff for Gen1A.
- UID and block 0 read-back verification is authoritative.

## Non-goals

- No format-all command.
- No support for FUID/UFUID or non-4-byte UIDs.
