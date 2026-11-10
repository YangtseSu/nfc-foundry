# Testing strategy

## Layers

1. **Pure unit tests**: card geometry, BCC, access bits, value blocks, dump,
   diff, NDEF/TLV, templates, validation.
2. **Property tests**: codec round trips, arbitrary malformed bytes, length
   boundaries, unknown TLV preservation.
3. **Transport tests**: mock reader with scripted cards, authentication
   failures, timeouts, disconnects, cancellation, card changes.
4. **Application tests**: workflows, backups, preflight, read-back verification,
   error mapping, CLI contracts.
5. **UI integration tests**: GTK4 views, focus, keyboard, overlays,
   destructive confirmations, empty/loading/error states.
6. **Hardware tests**: explicit, environment-gated, never run by `cargo test`.

## Hardware gates

Hardware tests are compiled only with the `hardware` feature and run only when
the environment selects them:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

Required gates:

- `NFC_FOUNDRY_HARDWARE=1`: enable hardware tests.
- `NFC_FOUNDRY_DEVICE`: exact connstring.
- `NFC_FOUNDRY_HARDWARE_KEY`: 12-hex MIFARE Classic key; defaults to
  `FFFFFFFFFFFF`.
- `NFC_FOUNDRY_HARDWARE_KEY_TYPE`: `A` or `B`; defaults to `A`.
- `NFC_FOUNDRY_HARDWARE_BLOCK`: Classic block for read/write tests; defaults to `4`.
- `NFC_FOUNDRY_HARDWARE_SECTOR0_KEY`: optional 12-hex Key A for manufacturer block 0
  when it differs from `NFC_FOUNDRY_HARDWARE_KEY`.
- `NFC_FOUNDRY_HARDWARE_WRITE=yes`: allow any write. The phase 02 test
  writes the original value back and refuses block 0 and sector trailers. The
  phase 04 workflow test changes one ordinary data block temporarily and writes
  the original back.
- `NFC_FOUNDRY_HARDWARE_RECOVERY=yes`: allow active recovery testing.
- `NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=yes`: allow the Classic NDEF
  format/write/restore hardware round trip. It also requires
  `NFC_FOUNDRY_HARDWARE_WRITE=yes`.
- `NFC_FOUNDRY_HARDWARE_BACKUP_DIR`: preserve backups.

The PN532 read test is ignored unless selected:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

Run it once with a MIFARE Classic card and once with a Type 2 tag; it reads the
selected card and reports the block or pages. The same-value write test is
compiled only with the additional `hardware-write` feature and requires
`NFC_FOUNDRY_HARDWARE_WRITE=yes`:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
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
mock Classic image.

The phase 08 NDEF tests are hardware-free. `nfc-ndef` has deterministic tests
for the Text RTD example, short/normal/chunked record layouts, all 36 URI
prefixes, TLV length forms through 65,535 octets, unknown TLV and padding
preservation, capacity boundaries, vCard/WSC/BLE/DPP payloads, and malformed
records and TLVs. Property tests cover message and chunked encode/decode round
trips, arbitrary-byte decode stability, TLV stream round trips, and unknown TLV
preservation. The crate depends only on `nfc-core`; it never touches hardware.

The phase 09 Type 2 tests are hardware-free. `nfc-type2` covers CC parsing,
product layouts, static/dynamic lock positions, configuration, NDEF state and
TLV preservation, project dumps, partial reads, verified writes, ignored
writes, locks, PWD_AUTH verification, and restore validation. `nfc-app` adds
scripted workflows for backups, Advanced confirmations, NDEF write/read/format,
lock denial, password configuration, and CC mismatch. `nfctl` exercises
`type2 info|write|dump|restore|lock` and `ndef write|read|format` against the
mock NTAG215. Driver unit tests assert the Type 2 READ CRC (`30 04 26 EE`) and
raw-mode restoration for both transports.

The phase 10 Classic NDEF tests are hardware-free. `nfc-classic-ndef` covers
MAD1/MAD2 and CRC vectors, NFC AIDs, contiguity, GPB mapping version and
read-only state, TLV framing/capacity, long messages, leading proprietary TLV
preservation, and partial data areas. `nfc-app` adds scripted 1K and 4K format,
NDEF write/read, confirmation, complete-read enforcement, Key B preservation,
read-only refusal, unformatted refusal, and malformed MAD tests. `nfctl`
exercises `classic ndef info|read|write|format`.

The phase 11 Type 4 tests are hardware-free. `nfc-type4` covers NTAG 424 and
DESFire capability containers, read-only and proprietary access, malformed
lengths and versions, unknown TLV preservation, APDU offsets, empty and
populated NDEF files, MLe and MLc chunk boundaries, ignored writes, and
cancellation. Property tests feed arbitrary CC and NDEF file bytes and require
typed results instead of panics. `nfc-app` adds scripted read, write with a
pre-mutation backup, clear confirmation, read-only refusal without a backup,
ignored-write detection with a backup, dump, and cancellation. The driver
tests cover the PN532 UART and ACR122U APDU command shape, status-word split,
non-ISO rejection, raw-mode rejection, and the shared 247-byte command limit.
`MockReader` contains a stateful Type 4 tag so the adapter exercises the real
SELECT/READ/UPDATE sequence.

The gated Type 4 hardware test lives in `nfctl/tests/hardware_type4.rs`. The
read-only check needs a Type 4 / ISO-DEP tag:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware --test hardware_type4 -- --ignored
```

The write/read/restore round trip needs the write gate and restores the
original NDEF message or empty state afterwards:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-type4-backups \
cargo test -p nfctl --features hardware-write --test hardware_type4 -- --ignored
```

No Type 4 hardware record exists yet; the gate is defined but has not run.

The gated Classic NDEF hardware test lives in
`nfctl/tests/hardware_classic_ndef.rs`. The read-only check needs a Classic
card:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware -- --ignored
```

The format/write/read/restore round trip requires both write tokens and restores
the original dump afterwards. It passed on PN532 UART on 2026-11-10 with a
blank Classic 1K, UID `6A 31 5E 28`; the restored dump matched byte-for-byte:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=yes \
cargo test -p nfctl --features hardware-write -- --ignored
```

The gated Type 2 hardware tests live in `nfctl/tests/hardware_type2.rs`. The
read-only check needs a Type 2 tag:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
cargo test -p nfctl --features hardware -- --ignored
```

The NDEF round trip additionally needs the write gate. It reads the original
image, formats, writes a Text record, verifies it, and restores the original
user memory from a backup:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-type2-backups \
cargo test -p nfctl --features hardware-write -- --ignored
```

The password test additionally requires `NFC_FOUNDRY_TYPE2_PASSWORD` (8 hex digits)
and writes AUTH0/ACCESS/PWD/PACK on an owned tag. The read and NDEF round-trip
tests passed on a PN532 UART with an NTAG215 on 2026-11-10; the ACR122U and
password records remain pending.

Hardware workflow checks:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware -- --ignored
```

The read-only test dumps the card and scans Key A per sector. The temporary
write test requires the additional feature and gate:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BLOCK=4 \
cargo test -p nfctl --features hardware-write -- --ignored
```

It changes one ordinary data block, writes the original value back, and
verifies the final read-back. It refuses block 0 and sector trailers.

ACR122U hardware checks use the same gates with an `acr122_usb:` connstring.
Stop `pcscd` first because it holds the same CCID interface:

```sh
sudo systemctl stop pcscd
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

The Gen1A write additionally requires `hardware-write` and two confirmations:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
NFC_FOUNDRY_HARDWARE_KEY=<sector-key> \
NFC_FOUNDRY_HARDWARE_SECTOR0_KEY=<sector-0-key> \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM=yes \
cargo test -p nfc-drivers --features hardware-write -- --ignored
```

The test writes the current manufacturer block back unchanged unless
`NFC_FOUNDRY_TEST_UID` is set, then reselects and verifies block 0. It refuses a card
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
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_DEVICE=acr122_usb:001:017 NFC_FOUNDRY_HARDWARE_KEY=D3F7D3F7D3F7 cargo test -p nfc-recover --test hardware_recovery -- --ignored --nocapture
```

The suite checks parity-disabled partial authentication, software Crypto1
authentication, nested nonce distance, nested recovery of a known target key,
static-nested recovery, hardnested acquisition and key recovery, and darkside
repeated-nonce collection. The hardnested case is the long one: it loads the
PM3 tables, collects until all 256 first nonce bytes are present, and runs the
bit-sliced solver. `NFC_FOUNDRY_HARDNESTED_MAX_NONCES` caps the acquisition (default
4000) and `NFC_FOUNDRY_HARDNESTED_TABLES` selects a table directory. Place a
MIFARE Classic 1K card on the reader. The current test fixture uses block 4 Key
A `D3F7D3F7D3F7` and target block 0 Key A `A0A1A2A3A4A5`.

Both transports complete full software authentication. The tests need the key
that actually belongs to the card in the reader: the two test cards share UID
`FA B9 CF F4` but use different sector keys. Set `NFC_FOUNDRY_HARDWARE_KEY` and
`NFC_FOUNDRY_HARDWARE_TARGET_KEY` for the card under test. Darkside collection over
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
reuse lint
```

For changed GTK screens, add GTK4 UI integration tests and a manual window
check for keyboard, focus, theme, zoom, and CJK input. The GTK app needs
`gtk4`/`libadwaita` development libraries to build; on a headless host, use a
private Xvfb display (for example `Xvfb :99 -screen 0 1400x920x24` with
`GDK_BACKEND=x11`). The phase 19a–19d workbench scenarios use a scripted
transport (Classic and NTAG213) and are gated:

```sh
env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 \
  xvfb-run -a env GDK_BACKEND=x11 \
  cargo test -p nfc-foundry gtk_workbench_scenarios
```

Without `NFC_FOUNDRY_GUI_TESTS=1` the test takes its skip branch so
`cargo test --workspace` still passes on a machine without a display. The same
gate covers the phase 19e language scenario: it switches to `zh-CN`, asserts
the immediate relabeling, persistence, and preservation of page/reader/card/
selection, runs a Classic read and a Type 2 read in Chinese, opens a Chinese
write dialog (heading, body, responses, close response), and fails if any
English chrome remains in the visible window or dialog. It then switches back
to English.

Phase 19e also adds an `i18n` unit test module: every catalog key has a
non-empty English and Chinese value and the same placeholder count, exact and
pattern lines translate both ways, multi-line labels keep their breaks, locale
detection prefers the saved choice, and preference codes round-trip. Settings
tests cover language persistence and rejection of an unknown code. The parity
matrix in `docs/design/parity.md` is checked against the `nfctl` command tree
and the GUI page/control tree; the audit added the GUI NDEF message export.

The navigation scenario visits every shell tab, checks the empty and busy
states, and verifies that the primary action is focusable and that the focus
chain advances. Escape/close behavior is asserted through the write dialog's
`cancel` close response.

For a visual Chinese check, run the built application on a private Xvfb
display with a temporary config and capture the window:

```sh
Xvfb :97 -screen 0 1500x1000x24 &
DISPLAY=:97 XDG_CONFIG_HOME=/tmp/nfc-zh GDK_BACKEND=x11 \
  target/debug/nfc-foundry &
DISPLAY=:97 import -window root /tmp/nfc-zh.png
```

Use `{"language":"zh-CN"}` in `/tmp/nfc-zh/nfc-foundry/ui.json`. Check CJK
glyphs, tab scrolling, and truncation; the 19e record used this capture.

The 19d scenarios cover keyscan progress, merge, and `.dic` export; the
recovery limitation surface and verified-key saving; diagnostics without card
I/O; activity timestamps, redaction, and clear; and settings persistence with
`0600` permissions plus invalid-path rejection. `nfc-app` adds a per-sector
keyscan progress assertion and recovery candidate expansion. `nfctl` covers
explicit `--key-file` keyscan candidates with common keys disabled and the
reader/build facts in `diag`. Settings unit tests cover missing and malformed
files, permissions, and path validation.

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
- Proxmark3 `mfc v2` golden JSON (`crates/nfc-classic/tests/fixtures/`) and
  PM3 `.dic` vectors for comments, inline comments, duplicates, lowercase, and
  malformed lines.

## What not to test

Do not add tests that only prove a wiring call, a copy, a mock echo, a default
literal, a non-empty vector, or a bare `is_ok()`. Test observable behavior,
boundaries, invariants, transitions, precedence, and failures.
