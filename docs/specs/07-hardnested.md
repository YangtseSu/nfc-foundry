# Phase 07 spec: hardnested and card profile paths

Date: 2026-11-10

Update 2026-11-12: phase 25.3 replaced the detection-only FM11RF08S path with
native backdoor acquisition and one-nonce recovery. The without-backdoor
distance path remains unclaimed. See
[25c-fm11rf08s.md](25c-fm11rf08s.md).

## Goal

Complete the native recovery engine with the hardnested attack, the PM3
hardnested state tables, EV1 originality-signature detection, static encrypted
nonce detection, the FM11RF08S profile path, and `auto` sequencing aligned with
PM3 autopwn. Recovery stays read-only: authentication frames are the only card
commands.

## Deliverables

- `nfc-recover::hardnested`:
  - PM3 table loading from `bitflip_<odd-even>_<mask>_states.bin[.lz4]`,
    including the 99.01% ignore threshold, effective-table merge, and the
    first-byte/second-byte application order.
  - `BitSet24` with PM3 word order, exact AND/OR/popcount, state iteration, and
    the `low20` chunk operation used by `update_allbitflips_array`.
  - Sum-property and partial-sum tables, Sum(a0) filtering, bitflip property
    filtering for first and second nonce bytes, all-bitflip accumulation,
    Sum(a8) probability estimation, and first-byte ordering.
  - Candidate state lists from partial sums and all-byte bitflip invariants.
  - 64-lane bit-sliced Crypto1 brute force from PM3's NOSIMD kernel, including
    test-nonce ordering, parity elimination, full nonce verification, rollback,
    and 48-bit key extraction.
  - Nonce acquisition through `Reader::transceive_parity*` using the PM3
    sequence: fast-select with WUPA and SELECT, known-key authentication, nested
    authentication, dummy-response abort, and card timeout. No PM3 firmware
    command is ported.
  - Cancellation checks during acquisition, filtering, candidate generation,
    and brute force; progress events for acquisition and brute-force
    percentage.
- `nfc-transport`:
  - `Reader::transceive_parity_bits` for 7-bit short frames such as WUPA.
- `nfc-drivers`:
  - PN532 UART and ACR122U support for `transceive_parity_bits`, including raw
    short-frame responses.
- Card profiles:
  - EV1/QL88 originality-signature detection (block 69 Key B), signature read,
    and automatic use of the signature key as a hardnested known key.
  - Static encrypted nonce detection across persistent fast-selected sessions.
  - `RecoveryStrategy::Fm11rf08s` reports the detected profile and an actionable
    transport limitation instead of a generic `NotImplemented`.
  - `auto`: dictionary, EV1 signature key, static encrypted detection, darkside
    for weak PRNG without a known key, nested with a known key, and hardnested
    fallback.
- CLI:
  - `nfctl recover hardnested` with known/target block and key type,
    `--hardnested-tables`, `--max-nonces`, and throttled progress on stderr.
  - `nfctl recover fm11rf08s` profile detection.
- Documentation: phase spec and note, plan status, recovery, architecture,
  testing, hardware, packaging, notices, and REUSE metadata.

## Non-goals

- No PM3 firmware protocol.
- No external hardnested/mfoc process.
- No card writes during recovery.
- No Type 4 or NDEF work.
- FM11RF08S key recovery is not claimed on PN532 UART or ACR122U; the
  static-encrypted nonce profile is detected and fails with the transport
  limitation. PM3's dedicated reader-scheduled FM11RF08S acquisition is not
  available through these transports.

## Safety

- Recovery uses the `nfc-app` exclusive lease and re-checks card identity.
- Every recovered key is verified with a fresh authentication before it is
  returned.
- Acquisition stops on card identity changes, repeated static encrypted
  nonces, cancellation, or a bounded nonce cap.
- Recovery mode is disabled on every acquisition exit path.
- Tables and candidate lists are bounded; the brute force reports tested and
  total state pairs.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Simulated nonce fixtures recover the known key from the bitsliced kernel and
  from the full filtering/solver path.
- PM3 table tests verify the effective table counts and that each table's
  declared state count matches its bitset.
- Hardware: ACR122U and PN532 UART acquire at least 256 distinct nonce first
  bytes with fast select, recover a known target key with hardnested, and verify
  it with a fresh authentication.
