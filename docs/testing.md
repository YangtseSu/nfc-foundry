# Testing strategy

## Layers

1. **Pure unit tests**: card geometry, BCC, access bits, value blocks, dump,
   diff, NDEF/TLV, templates, validation.
2. **Property tests**: codec round trips, arbitrary malformed bytes, length
   boundaries, unknown TLV preservation.
3. **Transport tests**: mock reader with scripted cards, authentication
   failures, timeouts, disconnects, cancellation, card changes.
4. **Application tests**: workflows, backups, preflight, read-back verification,
   error mapping, GUI/CLI parity.
5. **UI integration tests**: GPUI Kit views, focus, keyboard, overlays,
   destructive confirmations, empty/loading/error states.
6. **Hardware tests**: explicit, environment-gated, never run by `cargo test`.

## Hardware gates

Hardware tests are compiled only with the `hardware` feature and run only when
the environment selects them:

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

Required gates:

- `NFCX_HARDWARE=1`: enable hardware tests.
- `NFCX_DEVICE`: exact connstring.
- `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`: allow any write.
- `NFCX_HARDWARE_RECOVERY=I_OWN_THIS_CARD`: allow active recovery testing.
- `NFCX_HARDWARE_BACKUP_DIR`: preserve backups.

Never write a card by default. A test that cannot run must report the missing
gate, not skip silently in release validation.

## Required checks

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p nfc-foundry
reuse lint
```

For changed GUI screens, add GPUI Kit UI integration tests and a manual window
check for keyboard, focus, theme, zoom, and Chinese input.

## Fixtures

- Classic dump fixtures from the old implementation and independent images.
- NDEF message/TLV vectors, including malformed and chunked records.
- Nonce/key fixtures from Proxmark3 tests and captured old-engine runs.
- Mock transport scripts for card selection, auth, read/write, timeout,
  disconnect, and card change.

## What not to test

Do not add tests that only prove a wiring call, a copy, a mock echo, a default
literal, a non-empty vector, or a bare `is_ok()`. Test observable behavior,
boundaries, invariants, transitions, precedence, and failures.
