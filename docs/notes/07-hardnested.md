# Phase 07 note: hardnested and card profile paths

Date: 2026-11-10

## Implemented

- `nfc-recover::hardnested`:
  - PM3 table loader for
    `bitflip_<odd-even>_<mask>_states.bin[.lz4]`. The 99.01% state-count
    threshold, per-parity effective lists, merged application order, and
    first-byte/second-byte split match PM3. The vendored table directory has
    145 even and 96 odd effective tables (239 distinct masks because two masks
    exist for both parities).
  - `BitSet24` with PM3's MSB-first 24-bit state order, exact AND/OR/popcount,
    ascending state iteration, and PM3's coarse `low20` chunk operation.
  - Sum-property and partial-sum tables, `Sum(a0)` filtering, first- and
    second-byte bitflip filtering, all-bitflip accumulation, `Sum(a8)`
    probability estimates, expected brute-force estimates, and best-first-byte
    ordering.
  - Candidate state lists from partial-sum intersections and
    `all_bitflips_match` invariants.
  - 64-lane bit-sliced brute force ported from PM3's NOSIMD kernel: test-nonce
    common-bit ordering, per-byte parity elimination, full nonce verification,
    state rollback, and key extraction.
  - Acquisition over the transport parity exchange: persistent recovery
    session, known-key authentication, nested authentication, a dummy-byte
    abort with the card authentication timeout, WUPA plus SELECT fast select,
    and a 500 microsecond pre-authentication pause.
  - Cancellation during acquisition, filtering, candidate generation, and
    brute force. Progress reports acquisition frames/nonces/first bytes,
    estimated states, and brute-force percentage.
- `nfc-transport` / `nfc-drivers`:
  - `Reader::transceive_parity_bits` for 7-bit short frames.
  - PN532 UART and ACR122U parity-bit framing. Short-frame exchanges return the
    raw response so a WUPA/ATQA probe does not require parity reconstruction.
- Card profiles:
  - EV1/QL88 originality signature detection at block 69 Key B and a 32-byte
    signature read. `auto` uses the signature key as a known key when no
    supplied candidate authenticates.
  - Static encrypted nonce detection across three fast-selected nested
    acquisitions.
  - `RecoveryStrategy::Fm11rf08s` reports the detected static-encrypted
    profile. FM11RF08S key recovery itself needs reader-scheduled acquisition
    that the PN532 UART and ACR122U transports do not expose; the strategy
    returns that limitation instead of a key or a generic `NotImplemented`.
  - `auto` follows PM3 autopwn ordering: classification, EV1 signature,
    candidate dictionary, static encrypted detection, darkside for weak PRNG
    without a known key, nested, and hardnested fallback. Static-nonce cards go
    directly to staticnested.
- `nfctl`:
  - `recover hardnested` with block/key-type selection, `--hardnested-tables`,
    `--max-nonces`, and throttled progress.
  - `recover fm11rf08s` profile detection.
- Documentation, plan status, recovery/architecture/testing/hardware notes,
  notices, and REUSE metadata for the PM3 tables and derived code.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Simulated nonce fixtures:
  - a 24-nonce fixture reproduces PM3's `simulate_MFplus_RNG` encoding and the
    bit-sliced kernel recovers `A0A1A2A3A4A5`;
  - the ignored full filtering/solver fixture collects until the PM3 stopping
    rule and recovers the same key after the true state survives every filter;
    run it with
    `cargo test --release -p nfc-recover --lib hardnested::tests -- --ignored`.
- Table tests load all 351 vendored files, verify the effective counts, and
  check that every loaded bitset's population count equals its declared state
  count.
- Driver tests cover 7-bit short-frame wrapping and the existing parity
  round trips.
- Hardware on 2026-11-10, ACR122U at `acr122_usb:001:017`, card
  UID `FA B9 CF F4`, block 4 Key A `D3F7D3F7D3F7`:
  - `nfctl recover hardnested --known-block 4 --target-block 0
    --target-type A` collected 1840 frames / 1805 distinct nonces, reached
    256/256 first bytes, estimated 2.309e8 states, tested 729232 state pairs,
    recovered sector 0 Key A `A0A1A2A3A4A5`, and verified it with a fresh
    authentication. The run took 4m3s including table loading.
  - `nfctl recover auto` classified `PrngProfile::Weak`, found the known key,
    and recovered the same key through nested.
  - `nfctl recover fm11rf08s` reported that the normal card does not return a
    static encrypted nonce.
  - The ignored `hardnested_recovers_a_known_test_key` hardware test passed on
    the ACR122U: 1547 nonces, 103270992 tested state pairs, recovered
    `A0A1A2A3A4A5`, verified with a fresh authentication, 206.30s.
- Hardware on 2026-11-10, PN532 UART at `pn532_uart:/dev/ttyUSB0`, card
  UID `FA B9 CF F4`, block 4 Key A `FFFFFFFFFFFF`:
  - `nfctl recover hardnested --known-block 4 --target-block 20
    --target-type A` collected 1816 frames / 1795 distinct nonces, reached
    256/256 first bytes, estimated 2.394e9 states, tested 2152066416 state
    pairs through the all-bitflip path, recovered sector 5 Key A
    `FBBECDF6304E`, and verified it with a fresh authentication. The run took
    5m29s including table loading.

## Problems and resolutions

- The first acquisition attempt reselected the card with the ordinary driver
  path, which releases the target or cycles the RF field between nonces. The
  encrypted nonce repeated from a small set and could never cover 256 first
  bytes. The fix is a persistent recovery session with PM3's fast-select
  sequence: WUPA, SELECT, known-key authentication, nested nonce, dummy-byte
  abort, and card timeout.
- WUPA initially timed out because no dummy response was sent after the nested
  nonce. PM3 sends a one-byte incomplete answer and waits for the card's
  authentication timeout before fast select. Adding that abort made fast select
  reliable.
- The initial `all_bitflips` implementation intersected every first-byte state
  set exactly. That removes the true state for a group because different
  first-byte groups describe different post-first-byte states. PM3's
  `COUNT_BITARRAY_LOW20_AND` only clears a 16-bit chunk when the other set has
  no state in that chunk; using that operation restored the true state and the
  full fixture recovered the key.
- A SELECT response is three data bytes plus three parity bits (27 bits), not
  24. Passing 24 to `transceive_parity_expect` made the driver treat a partial
  parity frame as data and fail.
- The ACR122U `IN_RELEASE` / `IN_LIST_PASSIVE_TARGET` reset path works for
  isolated nested samples but resets the nonce progression enough that
  hardnested acquisition cannot reach all first-byte values. It remains the
  fallback after fast-select failures.
- Loading tables takes several seconds. Hardnested loads them before the
  known-key probe so the card is not left authenticated while the tables are
  decompressed, and `auto` only loads them when nested has failed.

## Deviations

- At the time of this phase, FM11RF08S key recovery was detection-only. The
  without-backdoor distance path needs deterministic reader timing that the
  PN532 UART and ACR122U do not expose. Update 2026-11-12: phase 25.3 added
  native backdoor acquisition and one-nonce recovery; see
  [25c-fm11rf08s.md](25c-fm11rf08s.md). The without-backdoor path remains
  unclaimed.
- The bit-sliced kernel is portable 64-lane Rust, not SIMD-dispatched. Release
  hardware runs use the release build; debug builds are intentionally slower.
- No GUI recovery screen is added in phase 07. The shared `Service::recover`
  and progress sink are ready for it.

## Remaining blockers

- A hard-PRNG MIFARE Classic card was not available. The solver and acquisition
  path are tested with simulated hard nonces and both weak-PRNG test cards.
- An FM11RF08S card was not available; only the normal-card negative case was
  checked.
- A 4K card was not available for the EV1 signature read path.
- The PN532 hardnested run exercised the all-bitflip branch and tested
  2.15e9 state pairs. The ACR run used the Sum(a8) branch. Both branches and
  both transports now have hardware records.
