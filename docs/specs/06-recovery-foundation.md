# Phase 06 spec: native recovery foundation

Date: 2026-11-10

## Goal

Port the MIFARE Classic recovery foundation from the current
`RfidResearchGroup/proxmark3` tree into `nfc-recover`: Crypto1/LFSR/PRNG
primitives, `mfkey` solvers, parity-disabled nonce acquisition over the Rust
transport, darkside, nested, and static-nested attacks, card/PRNG
classification, an `auto` strategy, and `nfctl recover` commands. Recovery is
read-only: the only card operation is authentication.

## Deliverables

- `nfc-recover`:
  - Crypto1 state, keystream, rollback, filter, `prng_successor`,
    `nonce_distance`, `validate_prng_nonce`, CRC-A, and key conversion from
    PM3 `common/crapto1`.
  - `lfsr_recovery32`, `lfsr_recovery64`, `lfsr_prefix_ks`,
    `lfsr_common_prefix`, and `nonce2key`.
  - `mfkey32`, `mfkey32v2` (moebius), `mfkey32_nested`, and `mfkey64`.
  - Software Crypto1 authentication and nested-authentication acquisition over
    `Reader::transceive_parity`; no PM3 firmware command is ported.
  - Nested and static-nested collection with measured tag-nonce distance,
    parity filtering, two-sample key intersection, and in-process key
    verification.
  - Darkside sample collection from repeated tag nonces, PM3-compatible
    parity/keystream packing, and an explicit no-sample failure when the host
    cannot schedule the PRNG-synchronized frames.
  - Classification of weak PRNG, hard PRNG, and plain static nonce, plus the
    existing `auto` planner.
  - Cancellation checks before every nonce attempt and sample, and one reader
    reopen/retry for transport failures.
- `nfc-transport`:
  - `RecoveryCapabilities`, `ParityFrame`, `Reader::recovery_capabilities`,
    `Reader::set_recovery_mode`, `Reader::transceive_parity`, and
    `Reader::transceive_parity_expect` for known response frame sizes.
- `nfc-drivers`:
  - PN532 UART and ACR122U parity-disabled framing: `CIU_ManualRCV` parity
    disable, caller-supplied parity, 9-bit frame packing/unpacking, and
    trailing-bit handling.
  - Recovery mode mutually exclusive with ordinary and raw byte modes.
- `nfc-app`:
  - `Service::recover` under an exclusive recovery lease, card-identity check,
    and engine-level reader recovery.
- `nfctl`:
  - `recover auto`, `recover darkside`, `recover nested`,
    `recover staticnested` (alias `static-nested`), with exact block and key
    type selection. `hardnested` and `fm11rf08s` remain explicit
    `NotImplemented` errors for phase 07.
- Golden fixtures from PM3's `tools/pm3_tests.sh` and `mfkey_examples.md`:
  `mfkey32`, `mfkey32v2`,
  `mfkey32nested`, `mfkey64`, and `nonce2key`.
- Phase 06 spec and note, plan status, recovery, architecture, testing, and
  hardware documentation updates.

## Non-goals

- No hardnested acquisition, bit-sliced solver, or PM3 tables. Phase 07.
- No FM11RF08S, static encrypted nonce, EV1 signature, or autopwn sequencing
  beyond the phase 06 classifier. Phase 07.
- No external `mfoc`/`mfcuk`/`mfoc-hardnested` process or PM3 firmware
  protocol.
- No card write during recovery. Authentication is the only operation;
  nested/darkside frames that fail remain authentication attempts.
- No GUI recovery screen in this phase. The CLI skeleton is the phase deliverable;
  the shared service is ready for the phase 07 GUI.

## Safety

- The recovery engine receives an `nfc-app` exclusive lease and re-checks card
  identity after an operation.
- Recovered keys are never reported without a fresh authentication. Nested
  verification reselects the card before authenticating.
- Darkside and nested acquisition only send authentication frames. No read,
  write, increment, or trailer command is used.
- Cancellation is checked before each nonce attempt, each sample, and each key
  verification.
- A transport timeout or I/O error triggers one reader reopen and retry; a
  second failure is returned to the caller.
- A card that does not expose a repeated nonce fails with an actionable
  message instead of fabricating a key.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Golden fixtures: PM3 `mfkey32v2`, `mfkey32nested`, `mfkey64`, and
  `nonce2key` vectors reproduce their published keys.
- Scripted tests cover parity frame packing/unpacking, PN532 and ACR122U
  recovery-mode register sequences, exclusivity with raw/ordinary modes, the
  app recovery lease, cancellation, and reader reopen retry.
- Hardware: with an ACR122U and a PN532 UART, partial authentication returns
  a tag nonce, software Crypto1 authentication matches the card, nested and
  static-nested recovery recover and verify a known test key, and darkside
  fails honestly when no repeated nonce sample exists.
