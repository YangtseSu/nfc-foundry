# Native MIFARE Classic recovery

## Source policy

The recovery engine is a Rust port of the current
`RfidResearchGroup/proxmark3` behavior. It does not use
`mfoc`/`mfcuk`/`mfoc-hardnested` as a source. The old C tools may be used only
to compare outputs during development.

Required PM3 areas:

- `common/crapto1/`: Crypto1/LFSR functions.
- `client/src/mifare/mifarehost.c`: darkside, nested, staticnested, nonce/card
  classification.
- `client/src/cmdhfmfhard.c`: hardnested acquisition and progress.
- `client/deps/hardnested/`: bit-sliced solver.
- `client/resources/hardnested_tables/`: tables.
- `client/src/mifare/mfkey.c`: mfkey32/mfkey64 variants.
- `client/src/cmdhfmf.c`, `client/src/cmdhfmfsen.c`, and
  `tools/mfc/card_only/`: EV1, static encrypted nonce, FM11RF08S, autopwn.
- `doc/mfc_notes.md`: card categories and attack applicability.

The PM3 firmware protocol (`CMD_HF_*`) is not ported. The Rust transport
implements the equivalent partial authentication and nonce acquisition.

## Card profiles

Classify before choosing an attack:

| Profile | Evidence | Primary attack |
|---|---|---|
| Weak PRNG, no known key | predictable first nonce | darkside |
| Weak PRNG, known key | predictable nonces | nested |
| Hard PRNG, known key | non-predictable nonces | hardnested |
| Static nonce | repeated plain/encrypted nonce | staticnested |
| Static encrypted nonce | static encrypted nested nonce | FM11RF08S / specialized |
| EV1 signature | signature sector/key B | hardnested with signature key |
| Unsupported | no auth, no nonce, no backdoor | fail honestly |

## Attack sequence

Phase 06 implements the read-only foundation. The `auto` strategy currently:

1. Classifies the card with three partial authentications.
2. Tries the supplied key store, `--key`, `--key-file`, and common candidates
   against the known block.
3. With a known key, runs nested or static-nested recovery depending on the
   classifier.
4. Without a known key, runs darkside only when the card reports usable
   authentication and nonce behaviour.
5. Verifies every recovered key with a fresh authentication before returning
   it.

Hardnested, static encrypted nonce, FM11RF08S, EV1 signature, and full autopwn
sequencing are phase 07. The engine returns `NotImplemented` for those
strategies.

## Reader behavior

Nonce acquisition uses caller-supplied parity over the PN53x
parity-disabled frame path. Both the PN532 UART and ACR122U complete software
Crypto1 authentication and nested recovery. A known response size skips the
post-response trailing-bits reset and status read, removing two round trips
between the tag nonce and the reader response.

Darkside needs eight parity probes against one tag nonce. PM3 produces them
with FPGA-scheduled authentication frames. PN532/ACR122U host commands cannot
schedule frames at the PRNG boundary, so the collector returns no sample and
`nfctl recover darkside` reports the limitation. This is a transport limit, not
a missing solver; the PM3 `nonce2key` vector is covered by a golden test.

## Implementation boundaries

```text
nfc-recover/
  crypto1.rs       # Crypto1/LFSR primitives
  prng.rs          # nonce progression/distance
  darkside.rs
  nested.rs
  staticnested.rs
  hardnested/
    acquire.rs
    filter.rs
    solver.rs
    tables.rs
  fm11rf08s.rs
  mfkey.rs
  auto.rs
  attack.rs        # Attack trait, request/result, progress
```

`nfc-recover` receives an exclusive `Reader` lease from `nfc-app`. It never
opens a device itself and never spawns an external attack process.

## Performance

- Nested/static-nested recovery is single-process and single-threaded in phase
  06. A two-sample run takes a few seconds on the validated test card.
- Cancellation is an atomic token checked before every nonce attempt, sample,
  and key verification.
- Nonce storage is bounded per attack. Darkside keeps at most 64 candidate
  nonce groups and eight observations per group.
- Hardnested bit-sliced kernels, PM3 tables, and SIMD runtimes remain phase 07.

## Licensing

PM3-derived code and tables are GPL-3.0-or-later. Preserve upstream copyright
and license notices. The old mfoc family is GPL-2.0-or-later; if any test
oracle code is copied, the combined work remains GPL-3.0-or-later.

## Test oracles

- PM3 `tools/pm3_tests.sh` vectors for `mfkey32v2`, `mfkey32nested`,
  `mfkey64`, and `nonce2key`; each is a golden Rust test.
- The PM3 C tools were run as development oracles for common-prefix state
  counts and the nested state input.
- Hardware tests on PN532 UART and ACR122U, gated by `NFCX_HARDWARE=1` and
  `NFCX_DEVICE`. `cargo test --workspace` never touches hardware.
