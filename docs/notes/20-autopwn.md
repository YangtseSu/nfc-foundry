# Note: all-sector autopwn and host-driven darkside

Plan: [19e-completion.md](../plan/19e-completion.md).
Supersedes the auto and darkside behavior in
[06-recovery-foundation.md](06-recovery-foundation.md).

## Problem

`RecoveryStrategy::Auto` was a single-target attack. It authenticated the
candidate list against the configured *known block* only, then ran nested or
hardnested for the configured *target block*. It never scanned the other
sectors, so a card with no key on block 4 could not be recovered even when
another sector used a common key. The GUI and `nfctl` therefore required the
user to supply a known block and target block, and the engine's documented
`auto::plan` dictionary step was dead code.

The darkside collector also did not follow a working host-driven sequence. It
sent one fabricated reader answer per re-authentication and stored zero for the
reader nonce and reader answer, so the solver could not derive a key even when
eight NACKs were observed. The earlier note concluded that host-driven
darkside was impossible on PN532 UART; the previous Go implementation and the
upstream NFCX project both run it through MFCUK, so that conclusion described
the old collector, not the transport.

## Implemented

### Auto recovery

`attack::auto` now runs the PM3/MFCUK autopwn sequence without block inputs:

1. Scans every sector of the Classic 1K/4K layout with the request candidates
   (key store, `--key`, `--key-file`, common keys) for Key A and Key B. A
   successful authentication records the key; the trailer read is an extra
   check and does not reject a key whose access bits forbid it.
2. On a 4K card with no dictionary hit, uses the EV1/QL88 originality
   signature key as a known key.
3. With no known key, runs darkside on sector 0 Key A then Key B and verifies
   every solved candidate.
4. Checks the seed key for a static encrypted nonce and reports the FM11RF08S
   limitation instead of running an attack that cannot work.
5. Recovers each missing sector key with nested, falling back to hardnested
   when nested returns `AuthenticationFailed`. Hardnested tables load once,
   lazily, and a missing table set becomes a note rather than aborting the
   keys already recovered.
6. Reads every recovered sector with its verified key and reports the read
   verification coverage.

`RecoveryResult` now carries `sector_keys` with sector, key type, key, and the
method that produced it. The GTK Recovery page shows one line per sector key
and saves them with their sector attribution; the known and target block
inputs are disabled in Auto. `nfctl recover auto` no longer takes block
arguments and prints `verified sector N key A|B: ...`. New progress events
cover the dictionary scan, darkside probes, per-sector nested/hardnested, and
read verification.

### Darkside

`darkside::collect_with_progress` is a port of MFCUK 0.3.8's host-driven loop:

- A fresh card selection per probe, then one partial authentication to read a
  tag nonce.
- One crafted 64-bit reader answer per appearance of that nonce. The first
  stage finds a NACK with a spoofed `0xDEADBEEF` nonce and `0xFACECAFE`
  response; the second stage varies the top three nonce bits and sweeps the
  low five parity bits for eight NACKs.
- A cache keyed by tag nonce (bounded at 65,536) collects the eight NACKs
  across nonce appearances.
- The eight keystream nibbles and parity rows go to the existing
  `lfsr_common_prefix` solver. The returned sample is only accepted when the
  solver produces at least one candidate; the caller verifies candidates with
  a fresh authentication.

The module header records the MFCUK attribution. The seed budget is 20,000
probes per key type and cancellation is checked on every probe.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes.
- New `nfc-recover` integration tests cover a 1K card whose 32 key slots are
  found by the all-sector dictionary and read back 16/16 sectors, the
  no-candidate path that reports the missing darkside seed, and a partial card
  that returns the known keys with `read verification 1/16` instead of failing.
- New `nfctl` test runs `recover auto` against the mock Classic 1K card and
  checks sector-attributed output and 32/32 + 16/16 coverage.
- Gated GTK scenarios:
  `env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env
  GDK_BACKEND=x11 cargo test -p nfc-foundry gtk_workbench_scenarios --
  --nocapture`: passes. The existing Type 2 i18n assertion (`untranslated
  Type 2`) is intermittently flaky under Xvfb and is unrelated to recovery.

## Hardware status

Hardware run on 2026-11-11, PN532 UART at `pn532_uart:/dev/ttyUSB0`, card
UID `FA B9 CF F4`:

- `nfctl recover auto` finished in 29.5 s with no block inputs.
- The all-sector dictionary found `FFFFFFFFFFFF` for 30 of the 32 slots.
- Sector 5 was not in the candidate dictionary; nested recovered Key A
  `FBBECDF6304E` and Key B `04413209CFB1` using the dictionary seed.
- Read verification covered 16/16 sectors and all 32 key slots were reported
  with sector attribution: `sector keys 32/32`.

The darkside seed path was not exercised by this card because the dictionary
found a seed. It remains covered by the `nonce2key` golden vector and the
collector logic, but it still needs a hardware run on a card whose keys are not
in the dictionary. To exercise it:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
  cargo test -p nfc-recover --test hardware_recovery -- --ignored --nocapture
```

Until that record exists, do not describe the darkside seed path as
hardware-validated in release notes.

## Remaining limitations

- The darkside collector needs the card to repeat a tag nonce across
  selections. A card with a long nonce cycle produces no sample.
- Auto does not yet classify static-nonce cards and route them to a dedicated
  static-nested acquisition. `recover_static_nested` still shares the nested
  acquisition and requires distinct encrypted nonces.
- FM11RF08S recovery was detection-only at the time of this phase. Phase 25.3
  added native backdoor acquisition and one-nonce recovery; the
  without-backdoor path remains unclaimed. See
  [25c-fm11rf08s.md](25c-fm11rf08s.md).
- Hardnested fallback still requires a known key; the seed comes from the
  dictionary, the EV1 signature key, or darkside.

## Recovery page consolidation

The recovery page no longer repeats one state in several places. The former
progress line, summary line, limitation line, and recovered-key label are
replaced by one read-only, scrollable log under the action bar. Dictionary
sectors, nested/hardnested targets, verified sector keys, read verification,
limitations, and the completion summary are written there; acquisition,
filtering, brute force, darkside probes, and verification counters replace a
single live row. The status bar keeps one compact start/finish line and the
activity list one start/finish record. Success toasts were removed; validation
errors still use a toast.

The known/target block inputs moved into a collapsed `Advanced strategy
options` expander. Auto is the default and hides the block rows; the explicit
strategies show them. The save action and the start/cancel row stay outside the
log scroll area, so a long result cannot push `Save verified keys` off the
page. The GTK scenario asserts the save action stays inside the window after a
32-key result.

Keyscan remains the standalone authentication scan and the source of
dictionary import/export; `classic keyscan` stays in `nfctl`. Auto no longer
requires the user to run it first.

The left rail's raw `TASK & SECURITY LOG` was removed. `append_log` used to
write the same event to the rail text buffer and the Activity table, and
`Clear activity` cleared both. Activity keeps the structured Time/Operation/
Result/Detail table; the rail duplicates no longer exist.
