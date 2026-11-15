# Phase 34 plan: NFC hardware candidates

Status: conditional 2026-11-15. No implementation is scheduled. This register
records reader/hardware candidates; it does not accept any of them. The current
release scope remains PN532 UART and ACR122U.

Research notes:
[34a-st25r3916-i2c-feasibility.md](../notes/34a-st25r3916-i2c-feasibility.md)
and [34b-nfc-hardware-candidates.md](../notes/34b-nfc-hardware-candidates.md).

## Candidates

- **ST25R3916 I2C**: raw frontend; deferred. Requires a new I2C transport and
  host Crypto1/ISO-DEP. Only for custom embedded or RF research.
- **ACR1552U**: first if ISO15693/NFC-V, Type 1/Topaz, or broader card coverage
  becomes a product goal. USB CCID/PC/SC; ISO15693/Type 5, Type 1/Topaz,
  Picopass, FeliCa, 848 kbps, 64 KB APDU, card emulation. No raw recovery.
- **ACR1252U**: modern ACR122U replacement with P2P and card emulation; NFC
  Forum certified; no ISO15693; no raw recovery.
- **PN532 I2C/SPI**: link variant of the existing PN532 command layer; same
  capabilities; no new card family; recovery preserved.
- **PN7160**: NCI controller; I2C or SPI plus IRQ; no USB/UART. A desktop needs
  a bridge or MCU, and a Rust NCI layer; Type 1–5 and card emulation; no raw
  recovery. Long-term, not next.
- **Raw frontends** (ST25R3916, PN5180, RC663, TRF7970A): custom embedded/RF
  research only; the host owns Crypto1/ISO-DEP, framing, parity, timing, and
  recovery validation.
- **Not candidates**: MFRC522 (partial reader), generic PC/SC-only readers,
  Sony RC-S380 (niche), and Proxmark3 (not a reader backend).

## Entry criteria

- an accepted ADR for the specific reader or transport, including the PC/SC
  exception or USB bridge, permissions, and physical reader identity;
- a spec with a bounded first slice and explicit `ReaderCapabilities`;
- new card families or modes have their own ADR/spec;
- a hardware record for every advertised operation before the matrix entry.

## Non-goals

- adding a generic PC/SC reader;
- claiming host Crypto1/ISO-DEP work from a complete reader;
- claiming raw recovery parity without a validated raw path;
- claiming a new card family without an adapter and hardware record;
- adding a reader that only duplicates current coverage without a product
  reason.

## Documentation

- Add a conditional entry to `docs/plan/README.md`.
- Keep candidates out of the release support matrix until a phase is accepted.
- New card families, modes, and transports need their own ADR, spec, note, and
  hardware record.
