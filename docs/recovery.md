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

The `auto` strategy follows PM3 autopwn ordering:

1. Classifies the card with three partial authentications.
2. Checks for the EV1/QL88 originality signature and uses the signature key as
   a known key when no supplied candidate works.
3. Tries the supplied key store, `--key`, `--key-file`, and common candidates
   against the known block.
4. With a known key, checks for a static encrypted nonce. A static encrypted
   card reports the FM11RF08S profile and the transport limitation instead of
   running an attack that cannot work.
5. With a known key and a normal or hard PRNG, runs nested first. A nested
   failure falls back to hardnested.
6. Without a known key, runs darkside only when the card reports weak PRNG and
   usable authentication. If darkside finds no repeatable sample, the error
   says that hardnested needs one known key.
7. Verifies every recovered key with a fresh authentication before returning
   it.

`recover hardnested` loads the PM3 tables, collects encrypted nonces with fast
select, filters the state space, and runs the bit-sliced solver. It needs a
candidate key that authenticates the known block. `recover fm11rf08s` reports
whether the static encrypted nonce profile is present; the FM11RF08S recovery
sequence itself needs reader-scheduled acquisition that the PN532 UART and
ACR122U transports do not expose.

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
  acquire.rs       # Crypto1 session, parity auth, fast select
  darkside.rs
  nested.rs        # nested and static-nested
  hardnested/
    acquire.rs     # fast-selected nonce acquisition
    bitset.rs      # PM3 state bitsets
    bitslice.rs    # 64-lane brute-force kernel
    filter.rs      # sum/bitflip filtering, candidate lists
    nonces.rs      # nonce storage and sum-probability model
    solver.rs      # Sum(a8) guesses and key selection
    tables.rs      # PM3 table loading
  mfkey.rs
  auto.rs
  attack.rs        # strategy execution, progress, verification
```

Static encrypted detection and EV1 signature detection live in `classify.rs`.
FM11RF08S recovery beyond detection is not implemented; the strategy returns an
actionable transport limitation.

`nfc-recover` receives an exclusive `Reader` lease from `nfc-app`. It never
opens a device itself and never spawns an external attack process.

## Performance

- Nested/static-nested recovery is single-process and single-threaded. A
  two-sample run takes a few seconds on the validated test card.
- Hardnested acquisition needs all 256 encrypted first-byte values. On the
  ACR122U this is roughly 1500-2500 frames at host round-trip speed; the
  collector reports frame, nonce, first-byte, and estimated-state progress.
- Hardnested filtering keeps 24-bit state bitsets and the PM3 effective tables
  in memory. Peak memory is bounded by the table set, per-first-byte bitsets,
  and candidate state lists; the solver reports tested and total state pairs.
- Cancellation is an atomic token checked during acquisition, filtering,
  candidate generation, and brute force.
- Nonce storage is bounded per attack. Darkside keeps at most 64 candidate
  nonce groups and eight observations per group; hardnested stops at the PM3
  reduction criterion or the configured `--max-nonces` cap.
- The 64-lane kernel is portable Rust. No SIMD runtime dispatch is required;
  release builds are used for real acquisitions.

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
