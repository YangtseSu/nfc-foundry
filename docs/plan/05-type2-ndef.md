# Phase 05: Type 2 / NTAG21x and NDEF

## Goal

Add MIFARE Ultralight and NTAG213/215/216 page access, plus committed NDEF
read/write support.

## Work

- Add a Type 2 card module: pages, user-memory range, capability container,
  dynamic/static lock bytes, config pages, PWD/PACK.
- Implement `READ`, `FAST_READ`, `WRITE`, `PWD_AUTH`, and `GET_VERSION` through
  the raw transport.
- Add page dump/restore with a full-image and user-memory view.
- Add NDEF TLV parsing, validation, read, and write.
- Add guarded operations for config/lock/password changes.
- Add UI screens and `nfctl type2`/`nfctl ndef` commands.
- Add fixtures for NTAG213/215/216 and hardware tests with real tags.

## Acceptance

- Tag detection distinguishes NTAG213/215/216 by `GET_VERSION`.
- User-memory read/write works with read-back verification.
- NDEF read/write works on a formatted Type 2 tag.
- Config/lock writes require explicit confirmation and a backup.
- No Classic-specific block/trailer logic leaks into Type 2.

## Non-goals

- No UID-page writing.
- No FeliCa/Type 3 NDEF.

Card emulation is tracked as Phase 09, separate from Type 2/NDEF. This phase
does not implement it, but it does not rule it out for the product.
