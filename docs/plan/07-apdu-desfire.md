# Phase 07: ISO14443-4 APDU and DESFire read

## Goal

Add a correct APDU foundation, then read-only DESFire support.

## Work

- Implement RATS/ATS, I-block chaining, status words, and APDU exchange.
- Add a diagnostic `nfctl apdu --raw` path.
- Add DESFire application/file enumeration and read.
- Add ISO14443-4 Type 4 NDEF read/write on top of the APDU transport.
- Keep authentication and write behind a separate guarded phase.
- Add fixtures and hardware tests.

## Acceptance

- APDU transport handles chaining and status words.
- DESFire identify/read works on a validated card.
- Raw APDU is clearly diagnostic and not advertised as a safe write path.

## Non-goals

- No DESFire key changes or formatting.
- No generic "write any card" command.
