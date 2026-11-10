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
- `NFCX_HARDWARE_SECTOR0_KEY`: optional 12-hex Key A for manufacturer block 0
  when it differs from `NFCX_HARDWARE_KEY`.
- `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`: allow any write. The phase 02 test
  writes the original value back and refuses block 0 and sector trailers. The
  phase 04 workflow test changes one ordinary data block temporarily and writes
  the original back.
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

The phase 04 workflow tests are hardware-free. `nfc-app` tests use a scripted
transport with per-sector key rules, ignored writes, failing writes, and
cancellation hooks. `nfctl` integration tests run the built binary against the
mock Classic image. The GPUI Classic workspace has UI integration tests for
read, block selection, diff, key add/remove, write, key scan, and restore
confirmation.

The phase 08 NDEF tests are hardware-free. `nfc-ndef` has deterministic tests
for the Text RTD example, short/normal/chunked record layouts, all 36 URI
prefixes, TLV length forms through 65,535 octets, unknown TLV and padding
preservation, capacity boundaries, vCard/WSC/BLE/DPP payloads, and malformed
records and TLVs. Property tests cover message and chunked encode/decode round
trips, arbitrary-byte decode stability, TLV stream round trips, and unknown TLV
preservation. The crate depends only on `nfc-core`; it never touches hardware.

Hardware workflow checks:

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware -- --ignored
```

The read-only test dumps the card and scans Key A per sector. The temporary
write test requires the additional feature and gate:

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFCX_HARDWARE_KEY=FFFFFFFFFFFF \
NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD \
NFCX_HARDWARE_BLOCK=4 \
cargo test -p nfctl --features hardware-write -- --ignored
```

It changes one ordinary data block, writes the original value back, and
verifies the final read-back. It refuses block 0 and sector trailers.

ACR122U hardware checks use the same gates with an `acr122_usb:` connstring.
Stop `pcscd` first because it holds the same CCID interface:

```sh
sudo systemctl stop pcscd
NFCX_HARDWARE=1 \
NFCX_DEVICE=acr122_usb:001:017 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

The Gen1A write additionally requires `hardware-write` and two confirmations:

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=acr122_usb:001:017 \
NFCX_HARDWARE_KEY=<sector-key> \
NFCX_HARDWARE_SECTOR0_KEY=<sector-0-key> \
NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD \
NFCX_HARDWARE_GEN1A_CONFIRM=I_OWN_THIS_CARD \
cargo test -p nfc-drivers --features hardware-write -- --ignored
```

The test writes the current manufacturer block back unchanged unless
`NFCX_TEST_UID` is set, then reselects and verifies block 0. It refuses a card
that is not a four-byte UID MIFARE Classic 1K and a block with an invalid BCC.
Restart `pcscd` afterwards when the PC/SC alias is needed.

The phase 05 hardware record on 2026-11-10 used an ACR122U at
`acr122_usb:001:017` with internal PN532 firmware 1.6:

- Classic 1K `FA B9 CF F4`, block 4 with Key A `D3F7D3F7D3F7`:
  `03 2B D1 01 27 54 02 65 6E 4E 46 43 20 46 6F 75`;
- Type 2 tag `04 34 4C C2 83 5C 80`, pages 4..7:
  `03 4B D2 0A 3E 74 65 78 74 2F 76 63 61 72 64 42`;
- raw 7-bit REQA: ATQA `04 00` in 16 bits;
- Gen1A manufacturer block 0 same-value write on the owned magic Classic 1K
  card, verified by reselection and read-back; the UID stayed
  `FA B9 CF F4`;
- several open/close cycles; a physical unplug/replug was not exercised.

The phase 04 hardware record on 2026-11-10 used a PN532 over
`/dev/ttyUSB0` and a MIFARE Classic 1K (`FA B9 CF F4`):

- full dump with common candidates: 64/64 blocks and 16/16 sectors readable;
- key scan: 16/16 sectors, 37 attempts, 32 verified keys;
- temporary write on block 4 with Key A `D3F7D3F7D3F7`: written, read back,
  original restored, read back; two `0600` project backups;
- restore of the full dump after the temporary change: 1 written, 62
  unchanged, one `0600` pre-write backup;
- restore of the unchanged dump: 0 written, 63 unchanged.

The same run also exposed two hardware-only issues: repeated PN532 selection
needed an RF field cycle, and block 0 needs sector 0 authentication. Both are
now covered by tests and documented in
`docs/notes/04-classic-workflows.md`. Trailer writes and a 4K card run remain
future hardware checks.


Recovery hardware checks are read-only and require an owned card:

```sh
NFCX_HARDWARE=1 NFCX_DEVICE=acr122_usb:001:017 NFCX_HARDWARE_KEY=D3F7D3F7D3F7 cargo test -p nfc-recover --test hardware_recovery -- --ignored --nocapture
```

The suite checks parity-disabled partial authentication, software Crypto1
authentication, nested nonce distance, nested recovery of a known target key,
static-nested recovery, hardnested acquisition and key recovery, and darkside
repeated-nonce collection. The hardnested case is the long one: it loads the
PM3 tables, collects until all 256 first nonce bytes are present, and runs the
bit-sliced solver. `NFCX_HARDNESTED_MAX_NONCES` caps the acquisition (default
4000) and `NFCX_HARDNESTED_TABLES` selects a table directory. Place a
MIFARE Classic 1K card on the reader. The current test fixture uses block 4 Key
A `D3F7D3F7D3F7` and target block 0 Key A `A0A1A2A3A4A5`.

Both transports complete full software authentication. The tests need the key
that actually belongs to the card in the reader: the two test cards share UID
`FA B9 CF F4` but use different sector keys. Set `NFCX_HARDWARE_KEY` and
`NFCX_HARDWARE_TARGET_KEY` for the card under test. Darkside collection over
either host-driven reader normally finds no eight-observation repeated-nonce
group and must report that result; it does not fabricate a key.

The phase 06 hardware record on 2026-11-10 used two MIFARE Classic 1K cards
that share UID `FA B9 CF F4` but have different keys.

ACR122U at `acr122_usb:001:017`:

- parity probe `07 04 A6 75`, 32 bits, parity `0 0 1 0`;
- software authentication `nt=F7159B6A`;
- nested measured distance `160`, window `156..164`;
- nested, static-nested, and auto recovered sector 0 Key A
  `A0A1A2A3A4A5` from block 4 Key A `D3F7D3F7D3F7` and verified it;
- hardnested collected 1840 frames / 1805 distinct nonces, reached 256/256
  first bytes, estimated 2.309e8 states, tested 729232 state pairs, recovered
  the sector 0 Key A target, and verified it with a fresh authentication;
- darkside found no repeated-nonce sample in 200 tight-loop attempts.

PN532 UART at `pn532_uart:/dev/ttyUSB0` with canonical keys
`FFFFFFFFFFFF`:

- parity probe returned a 32-bit nonce;
- software authentication completed;
- nested measured distance `128`, window `124..132`;
- nested, static-nested, and auto recovered sector 5 Key A
  `FBBECDF6304E` from block 4 Key A `FFFFFFFFFFFF` and verified it;
- hardnested collected 1816 frames / 1795 distinct nonces, reached 256/256
  first bytes, tested 2152066416 state pairs through the all-bitflip path,
  recovered the same target key, and verified it with a fresh authentication;
- darkside found no repeated-nonce sample.

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
check for keyboard, focus, theme, zoom, and Chinese input. The Classic
workspace tests run headlessly:

```sh
cargo test -p nfc-classic-gui --test ui
```

For hardware features:

```sh
cargo clippy -p nfctl --features hardware-write --all-targets -- -D warnings
```

For changed driver hardware tests, also run:

```sh
cargo clippy -p nfc-drivers --features hardware --all-targets -- -D warnings
```

## Fixtures

- Classic dump and codec fixtures: access, value, BCC, trailer, geometry, diff,
  and malformed project/raw/MCT/Proxmark3 inputs.
- NDEF message/TLV vectors, including malformed and chunked records.
- PM3 `mfkey32v2`, `mfkey32nested`, `mfkey64`, and `nonce2key` golden
  vectors, plus captured hardware nonce/key samples from both readers.
- Mock transport scripts for card selection, auth, read/write, timeout,
  disconnect, and card change.
- Classic workflow scripts with per-sector key rules, denied sector keys,
  ignored writes, failing writes, and cancellation after a chosen attempt.

## What not to test

Do not add tests that only prove a wiring call, a copy, a mock echo, a default
literal, a non-empty vector, or a bare `is_ok()`. Test observable behavior,
boundaries, invariants, transitions, precedence, and failures.
