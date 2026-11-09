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

The `auto` strategy mirrors PM3 `autopwn` rather than a fixed linear list:

1. Read card identity and capabilities.
2. Run dictionary/known-key authentication.
3. If no known key and the card is darkside-vulnerable, run darkside.
4. With a known key:
   - weak PRNG -> nested;
   - static nonce -> staticnested;
   - hard PRNG -> hardnested;
   - static encrypted/FM11RF08S -> specialized path.
5. Verify every recovered key with a fresh authentication.
6. Re-read the card; validate dump geometry, BCC, and trailers.
7. Return keys, dump, attempt log, and failure stage.

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

- Nested/staticnested: `rayon` worker threads for state recovery.
- Hardnested: bit-sliced kernels, `std::arch` SIMD, AVX2/AVX-512/NEON only
  behind runtime feature detection.
- Tables: LZ4-compressed PM3 tables, loaded on demand; verify integrity and
  report missing data.
- Cancellation: atomic token checked between nonce batches and solver chunks.
- Memory: bounded nonce storage; avoid retaining unbounded per-attempt state.

## Licensing

PM3-derived code and tables are GPL-3.0-or-later. Preserve upstream copyright
and license notices. The old mfoc family is GPL-2.0-or-later; if any test
oracle code is copied, the combined work remains GPL-3.0-or-later.

## Test oracles

- PM3 test vectors and `tools/pm3_tests.sh` fixtures.
- Golden nonce/key sets captured from the old C engines.
- Differential runs on the same card and nonce data during development.
- Hardware tests on PN532 UART and ACR122U, gated by environment variables.
