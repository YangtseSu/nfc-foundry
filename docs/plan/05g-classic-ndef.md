# Phase 05g: NDEF on MIFARE Classic

## Goal

Read and write NDEF messages stored on MIFARE Classic cards that use the
MIFARE Application Directory and NDEF TLV layout.

## Work

- Read and validate the MIFARE Application Directory.
- Locate the NDEF application and TLV area.
- Read, decode, and write NDEF through the existing Type 2 NDEF decoder and
  template catalogue where the payload is compatible.
- Format an NDEF application only after explicit confirmation and a full
  backup.
- Reuse the Classic key catalog, restore preflight, dump, and verification
  rules.
- Add fixtures for a formatted Classic NDEF card and hardware tests.

## Acceptance

- A formatted Classic NDEF card can be read and decoded.
- Writing a message preserves MAD and unrelated applications.
- Malformed or incomplete NDEF areas are reported without modification.
- Formatting requires explicit confirmation, a full dump backup, and read-back.
- `make check` passes.

## Non-goals

- No automatic reformatting of arbitrary Classic cards.
- No shared authentication bypass; all accesses use the Classic key catalog.
- No Type 4 or FeliCa NDEF; those stay in their own phases.
