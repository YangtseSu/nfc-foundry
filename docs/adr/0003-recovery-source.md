# ADR 0003: Recovery is native Rust from current Proxmark3

- Status: accepted
- Date: 2026-11-09

## Context

The old mfoc/mfcuk/mfoc-hardnested tools are stale. Current Proxmark3 is the
active implementation of the recovery behavior, including card classification,
static nonce, static encrypted nonce, EV1, FM11RF08S, hardnested, and autopwn
sequencing.

## Decision

Implement recovery in Rust in `nfc-recover`, using the current
`RfidResearchGroup/proxmark3` tree as the behavioral source. Do not use the old
mfoc family as an algorithm source. Do not integrate PM3 hardware or run PM3
client commands.

The old C engines are differential test oracles only.

## Consequences

- PM3 firmware `CMD_HF_*` cannot be reused; the Rust transport must implement
  equivalent partial-auth and nonce acquisition.
- PM3-derived code and tables remain GPL-3.0-or-later.
- Hardnested tables and solver need porting and golden-fixture validation.
- Recovery has no external process boundary; reader exclusivity is enforced by
  `nfc-app`.
