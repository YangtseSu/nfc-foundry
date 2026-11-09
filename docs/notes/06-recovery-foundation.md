# Phase 06 note: native recovery foundation

Date: 2026-11-10

## Implemented

- `nfc-recover` now contains:
  - `crypto1.rs`: Crypto1 state, keystream, rollback, filter,
    `prng_successor`, `nonce_distance`, `validate_prng_nonce`,
    `lfsr_recovery32`, `lfsr_recovery64`, `lfsr_prefix_ks`,
    `lfsr_common_prefix`, and `nonce2key`.
  - `mfkey.rs`: `mfkey32`, `mfkey32_moebius`, `mfkey32_nested`,
    `mfkey64`, nested candidate generation, and two-sample intersection.
  - `acquire.rs`: CRC-A, odd parity, UID-tail extraction, partial
    authentication, software Crypto1 authentication, and nested nonce
    acquisition. A `Session` keeps recovery mode active between the software
    authentication and the nested frame.
  - `nested.rs`: measured known-key nonce distance, parity filtering,
    two-sample nested recovery, static-nested entry point, and reselecting key
    verification.
  - `darkside.rs`: repeated-nonce collection, PM3-compatible parity/keystream
    packing, and `nonce2key`.
  - `classify.rs`: weak/hard/static nonce classification and known-key probe.
  - `attack.rs`: dictionary, darkside, nested, static-nested, and `auto`
    execution with cancellation and one reader reopen/retry.
- `nfc-transport` adds the opt-in `RecoveryCapabilities`, `ParityFrame`,
  `set_recovery_mode`, and `transceive_parity` boundary.
- PN532 UART and ACR122U implement parity-disabled framing:
  `CIU_ManualRCV` parity disable, caller-supplied parity, 9-bit frame
  wrapping/unwrapping, and trailing-bit handling. Recovery mode rejects
  ordinary and raw exchange; raw mode rejects recovery mode.
  `transceive_parity_expect` supplies a known response size and skips the
  post-response trailing-bits reset and `CIU_Control` read, removing two host
  round trips between the tag nonce and the reader response.
- `nfc-app::recovery` runs recovery under `OperationKind::Recovery`, re-checks
  card identity, and returns verified keys.
- `nfctl recover auto|darkside|nested|staticnested` (alias
  `static-nested`) uses `--key`, `--key-file`, `--key-store`, and
  `--no-common-keys` candidates. `hardnested` and `fm11rf08s` return
  `NotImplemented`.
- Golden tests:
  - `mfkey32`: `57DA41DA 01200145 7B70C62C 909121F2 F9206A8B 908B8981` →
    `ffffffffffff`.
  - `mfkey32v2`: `12345678 1AD8DF2B 1D316024 620EF048 30D6CB07 C52077E2
    837AC61A` → `A0A1A2A3A4A5`.
  - `mfkey32nested`: `5C467F63 4bbf8a12 abb30bd1 46033966 adc18162` →
    `059e2905bfcc`.
  - `mfkey64`: `9c599b32 82a4166c a1e458ce 6eea41e0 5cadf439` →
    `ffffffffffff`.
  - `nonce2key`: `e9cadd9c a8bf4a12 a020a8285858b090 050f010607060e07` →
    `fc00018778f7`.
- Scripted driver tests cover the PN532 and ACR122U recovery register
  sequences, parity frame round trips, trailing bits, and mode exclusivity.
- App tests cover the recovery lease, cancellation, dictionary recovery, and
  reader reopen after a transport timeout. The CLI test covers the honest
  no-parity failure on the mock transport.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- PM3 C tools were used as development oracles:
  - `tools/mfc/card_reader/mfkey32v2`
  - `tools/mfc/card_reader/mfkey32nested`
  - `tools/mfc/card_reader/mfkey64`
  - `tools/mfc/card_only/nonce2key`
  - a temporary C harness for `lfsr_common_prefix` and the nested
    `lfsr_recovery32` input. The harness confirmed the PM3 state counts
    (`nonce2key` produced one state, 25 odd and 19 even prefix candidates)
    and found the two porting mistakes described below.
- Hardware on 2026-11-10. The two test cards are both MIFARE Classic 1K with
  UID `FA B9 CF F4` but different sector keys:
  - ACR122U at `acr122_usb:001:017`, block 4 Key A `D3F7D3F7D3F7`.
  - Partial authentication with parity-disabled framing returned
    `07 04 A6 75`, 32 bits, parity `0 0 1 0`.
  - Software Crypto1 authentication completed with a known key; one observed
    session was `nt=F7159B6A`, state
    `2BC73E15/D12FD652`.
  - Nested calibration measured a fixed tag-nonce distance of `160` for
    authenticated-to-nested sequences on the test card.
  - `nfctl recover nested --known-block 4 --target-block 0` with known
    block 4 Key A `D3F7D3F7D3F7` recovered sector 0 Key A
    `A0A1A2A3A4A5`, distance window `156..164`, and verified it with a fresh
    authentication.
  - `nfctl recover staticnested` produced the same verified result and
    reported `StaticNested`.
  - `nfctl recover auto` classified the card as `PrngProfile::Weak`, found the
    supplied known key, and recovered and verified the same target key through
    nested authentication.
  - The complete `hardware_recovery` suite passed: software authentication,
    PRNG classification, nested calibration, nested target distance, nested
    recovery, and darkside collection (6 passed, 33.86 s).
  - Darkside collection ran 200 attempts in the tight-selected loop and found
    no group of eight observations with one tag nonce. An earlier 1500-attempt
    run with a full reselect per attempt also found none.
  - PN532 UART at `pn532_uart:/dev/ttyUSB0` with the other card, which shares
    UID `FA B9 CF F4` but uses canonical `FFFFFFFFFFFF` keys:
    parity-disabled partial authentication returned a 32-bit nonce, full
    software authentication completed, nested calibration measured distance
    `128` (window `124..132`), and nested, static-nested, and auto recovered
    and verified sector 5 Key A `FBBECDF6304E` from block 4 Key A
    `FFFFFFFFFFFF`. The full `hardware_recovery` suite passed on the PN532
    (6 passed, 48.54 s).
  - Both readers classified their card as `PrngProfile::Weak`,
    `darkside_vulnerable=true`, with no known key supplied to the classifier.
- `cargo test --workspace` never opens a reader; all hardware runs used
  `NFCX_HARDWARE=1` and explicit `NFCX_DEVICE`.

## Problems and resolutions

- PM3 `lfsr_common_prefix` uses `fastfwd[isodd]`; row 0 is the even prefix and
  row 1 is the odd prefix. The first port had the rows swapped, producing the
  wrong prefix candidate counts. The `nonce2key` golden vector now locks the
  correct order.
- The nested solver's `lfsr_recovery32` input is the **plaintext** tag nonce
  XOR UID, while the keystream is ciphertext XOR plaintext. Using the
  ciphertext in the input position produced 78k candidates without the true
  key; the C oracle and the golden fixture now lock the correct input.
- A second nested authentication in one selected session timed out over the
  ACR122U. Acquisition therefore uses one nested frame per selected session
  and collects two sessions with distinct nonces, rather than two nested frames
  in one session.
- ACR122U recovery mode is toggled off for reselection and back on for the raw
  frame. Keeping it active across the software authentication and nested frame
  avoids resetting the authenticated card state; the `Session` abstraction now
  does that.
- Darkside requires FPGA-scheduled authentication frames to hold the PRNG at
  one tag nonce for eight parity probes. Host-driven PN532/ACR122U commands
  cannot reproduce that scheduling. The collector tries a tight selected-card
  loop, then returns no sample; `nfctl recover darkside` reports the limitation
  instead of claiming a key.
- The first PN532 full-authentication attempt used the ACR card's
  `D3F7D3F7D3F7` key on the PN532 card. Both cards share UID
  `FA B9 CF F4`, so the mistake was invisible in selection, but the PN532 card
  uses `FFFFFFFFFFFF`. The wrong key produced a card timeout and was initially
  misread as a UART timing limit. With the correct key, software
  authentication and nested recovery pass on the PN532 UART.
- The expected-response-size path was added while investigating that timing:
  it removes the trailing-bits reset and `CIU_Control` read after each
  authentication frame. Both readers still pass the scripted and hardware
  suites with it.

## Deviations

- Hardnested acquisition/solver, PM3 tables, FM11RF08S, static encrypted nonce,
  EV1 signature, and autopwn sequencing stay in phase 07.
- No GUI recovery screen in phase 06. The phase deliverable is the shared
  service and the `nfctl recover` skeleton; the GUI can call the same
  `Service::recover` entry point when phase 07 adds hardnested and autopwn.
- Static-nested shares the nested two-sample acquisition and measured-distance
  solver. On the test card the measured distance was fixed, so the same path
  works for both profiles; a card whose nested plaintext behaves differently
  from the measured known-key sequence will fail parity filtering and report
  `AuthenticationFailed`.

## Remaining blockers

- Darkside over host-driven PN532/ACR122U is not possible without
  reader-scheduled frame timing. Golden solver coverage and the honest failure
  are the phase 06 result.
- Physical unplug/replug during recovery was not tested; the engine's reopen
  retry is covered by a scripted transport error.
- Hardnested and static encrypted nonce recovery remain phase 07.
