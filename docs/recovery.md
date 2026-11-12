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
Darkside acquisition follows MFCUK 0.3.8's host-driven sequence; its solver is
the PM3/Crapto1 `nonce2key` path.

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

The `auto` strategy is a full autopwn and needs no block or key input beyond
the configured candidate store:

1. Scans every sector with the key store, `--key`, `--key-file`, and common
   candidates. A successful authentication records the key; the trailer read
   is attempted as an extra check and is not required when access bits forbid
   it.
2. When nothing authenticates on a 4K card, checks for the EV1/QL88
   originality signature and uses the signature key as a known key.
3. When there is still no known key, runs host-driven darkside on sector 0
   Key A then Key B and verifies every candidate before using it as a seed.
4. With a seed key, checks for a static encrypted nonce. A static encrypted
   card is routed to the FM11RF08S backdoor recovery, which acquires clear
   nonces and needs no target key.
5. Otherwise recovers every missing sector key with nested first and hardnested
   as the fallback. Each recovered key is verified with a fresh
   authentication.
6. Reads every recovered sector with its verified key and reports the
   read-verification coverage. A partial card is returned with explicit notes
   instead of being reported as complete.

`recover hardnested` loads the PM3 tables, collects encrypted nonces with fast
select, filters the state space, and runs the bit-sliced solver. It needs a
candidate key that authenticates the known block. `recover fm11rf08s` uses the
published FM11RF08S backdoor (`0x64`/`0x65` and the PM3 backdoor keys) to
acquire clear static nonces, generates candidates with a Rust port of
`staticnested_1nt`, verifies them, and read-verifies every recovered sector.
The without-backdoor distance path depends on deterministic reader timing and
is not claimed for the PN532 UART or ACR122U.

## Workbench and CLI

The GTK Recovery page runs the same `nfc-app` service and `nfc-recover` engine
as `nfctl`. It exposes Auto, Darkside, Nested, Static nested, Hardnested, and
FM11RF08S. Auto scans all sectors and disables the known/target block inputs;
the single-target strategies use them. The hardnested table directory, an
acquisition cap, and the key-store/common candidate controls stay shared.
Scan, darkside probe, nested/hardnested sector, acquisition, filtering,
brute-force, and read-verification events update one progress line and an
elapsed timer. Cancel sets the same `CancelToken` used by the engine, so it
stops before the next reader exchange.

A recovered key is shown only after the service authenticates it again. Auto
reports each key with its sector and saves them per sector; single-target
results keep the old target-block attribution. Saving to the key store is a
separate action and records the `recovery` provenance. Engine limitations are
shown verbatim: darkside no-sample, FM11RF08S transport limits, and missing or
empty hardnested tables are not collapsed into a generic failure.

`nfctl recover auto` takes no block arguments. The other strategies accept
`--known-type A|B` and their block inputs. The GUI and CLI use the same settings
for the default hardnested table directory; `nfctl diag` reports the resolved
table path and enumerated readers.

## Reader behavior

Nonce acquisition uses caller-supplied parity over the PN53x
parity-disabled frame path. Both the PN532 UART and ACR122U complete software
Crypto1 authentication and nested recovery. A known response size skips the
post-response trailing-bits reset and status read, removing two round trips
between the tag nonce and the reader response.

Darkside uses the MFCUK host-driven sequence: each probe reselects the card,
reads one tag nonce, sends one crafted 64-bit reader answer with spoofed nonce
and parity bits, and records the encrypted four-bit NACK. Eight NACKs that
share one tag nonce are solved with `lfsr_common_prefix`. The collector caches
up to 65,536 tag nonces and sweeps the low five parity bits per slot, matching
MFCUK. A card whose PRNG never repeats a tag nonce produces no sample; the
engine reports that limitation instead of guessing.

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
FM11RF08S backdoor acquisition and the one-nonce candidate generator live in
`staticnested.rs`; `fm11rf08s_attack` owns key reuse, candidate verification,
progress, cancellation, and read verification.

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
- Nonce storage is bounded per attack. Darkside caches at most 65,536 tag
  nonces with eight observations each, matching MFCUK, and the seed attempt
  budget is 20,000 probes; hardnested stops at the PM3 reduction criterion or
  the configured `--max-nonces` cap.
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
- Hardware tests on PN532 UART and ACR122U, gated by `NFC_FOUNDRY_HARDWARE=1` and
  `NFC_FOUNDRY_DEVICE`. `cargo test --workspace` never touches hardware.
