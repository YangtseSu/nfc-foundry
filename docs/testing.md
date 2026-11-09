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
- `NFCX_HARDWARE_KEY`: 12-hex MIFARE Classic key; defaults to
  `FFFFFFFFFFFF`.
- `NFCX_HARDWARE_KEY_TYPE`: `A` or `B`; defaults to `A`.
- `NFCX_HARDWARE_BLOCK`: Classic block for read/write tests; defaults to `4`.
- `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`: allow any write. The phase 02 test
  writes the original value back and refuses block 0 and sector trailers.
- `NFCX_HARDWARE_RECOVERY=I_OWN_THIS_CARD`: allow active recovery testing.
- `NFCX_HARDWARE_BACKUP_DIR`: preserve backups.

The PN532 read test is ignored unless selected:

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

Run it once with a MIFARE Classic card and once with a Type 2 tag; it reads the
selected card and reports the block or pages. The same-value write test is
compiled only with the additional `hardware-write` feature and requires
`NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`:

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD \
cargo test -p nfc-drivers --features hardware-write -- --ignored
```

A missing card fails with an instruction to place one; it is not skipped. Never
write a card by default. A test that cannot run must report the missing gate,
not skip silently in release validation.

The phase 02 hardware record covers firmware identification, Classic 1K
selection/authentication/read, Type 2 selection/read, and a raw 7-bit REQA with
a 16-bit ATQA response on a PN532 (CH340 UART, firmware 1.6). Wrong-reader and
missing-card errors were checked on hardware. The same-value write needs
explicit authorization and was not run. `cargo test --workspace` uses only the
scripted link and never touches hardware.

The phase 03 core tests are hardware-free. Deterministic vectors cover access
codes, value blocks, BCC, trailers, 1K/4K geometry, diff, and malformed dump
files. Property tests round-trip access and value codecs, walk every block in
both layouts, and round-trip raw, project, MCT, and Proxmark3 images. Real MCT
and Proxmark3 client file interoperability remains a release check.

## Hardware failure triage

A silent or malformed reader response is a driver bug until the reference path
proves otherwise. Before reporting a hardware blocker:

1. Verify the same exact port with the known-good stack:

   ```sh
   LIBNFC_DEVICE=pn532_uart:/dev/ttyUSB0 nfc-list -v
   ```

2. Capture the reference exchange:

   ```sh
   LIBNFC_DEVICE=pn532_uart:/dev/ttyUSB0 \
   LIBNFC_LOG_LEVEL=3 nfc-list
   ```

3. Compare the reference host frames with the Rust driver byte-for-byte. Check
   the wake-up preamble, command item values, authentication payload, and card
   state transitions.
4. Check the validated Python scripts and the previous Go shim for the
   workflow-level sequence before concluding the adapter is unsupported.

If the reference fails too, record the exact error and the gates used. If the
reference works, fix the Rust driver and add a regression test.

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

For changed driver hardware tests, also run:

```sh
cargo clippy -p nfc-drivers --features hardware --all-targets -- -D warnings
```

## Fixtures

- Classic dump and codec fixtures: access, value, BCC, trailer, geometry, diff,
  and malformed project/raw/MCT/Proxmark3 inputs.
- NDEF message/TLV vectors, including malformed and chunked records.
- Nonce/key fixtures from Proxmark3 tests and captured old-engine runs.
- Mock transport scripts for card selection, auth, read/write, timeout,
  disconnect, and card change.

## What not to test

Do not add tests that only prove a wiring call, a copy, a mock echo, a default
literal, a non-empty vector, or a bare `is_ok()`. Test observable behavior,
boundaries, invariants, transitions, precedence, and failures.
