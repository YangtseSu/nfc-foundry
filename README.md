# NFC Foundry (Rust)

NFC Foundry is a Linux-only NFC reader workbench for MIFARE Classic and NFC
Forum Type 2 / NDEF cards, written in Rust.

This repository is the clean Rust rewrite. The previous Go/C implementation is
kept as a behavioral oracle and hardware-regression reference; it is not a
dependency of this project.

## Status

**Phase 10 complete: the Classic 1K NDEF format/write/read/restore round trip
passed on PN532 UART. The ACR122U record and the ACR122U Type 2 record remain
pending.**
Both first-release readers are implemented: PN532 UART
and ACR122U direct USB/CCID. The ACR122U path claims the CCID interface with
`nusb`, carries PN532 commands in the ACR122 pseudo-APDU, and supports
selection, Classic authentication and block access, raw bytes/bits, and a
Gen1A manufacturer-block write capability. Hardware validation covered
Classic and Type 2 reads, a raw 7-bit REQA, and a same-value Gen1A block 0
write verified by reselection and read-back. The Classic workflow layer covers
key stores, common-key authentication, partial dumps, single-block reads and
writes, restores, key scans, and atomic project-format backups. `nfctl` and the
GPUI Kit Classic workspace call the same `nfc-app` methods. Every write checks
access conditions, saves a backup, writes the block, reads it back, and
rechecks card identity; restore requires confirmation, refuses to change
manufacturer block 0, writes data blocks before sector trailers, and stops at
the first failure by default. Native recovery now covers Crypto1, the `mfkey`
solvers, parity-based nonce acquisition, nested and static-nested recovery, and
darkside collection, and hardnested with the PM3 state tables and 64-lane
bit-sliced solver, exposed as
`nfctl recover auto|darkside|nested|staticnested|hardnested`. Both readers
complete software Crypto1 authentication, collect hardnested encrypted nonces
with fast select, and verify recovered keys. Darkside requires
reader-scheduled frames and reports no sample on these host-driven readers.
Static encrypted nonce cards are detected and report the FM11RF08S profile and
the transport limitation instead of a key. EV1/QL88 signature detection uses
the originality-signature block as a known key when no supplied key works.
The hardware-free NDEF core is complete: a strict message/record codec with
short, normal, and chunked layouts; Type 2 TLV framing with unknown and
proprietary preservation; Text, URI, Smart Poster, vCard, Wi-Fi WSC, Bluetooth
LE OOB, and DPP templates; and message/TLV capacity checks. The Type 2 / NTAG
adapter is implemented: CC and product layouts, static/dynamic locks, NTAG
configuration, project dumps, NDEF TLV preservation, and verified page and
NDEF writes over both readers. `nfctl type2|ndef` and the GPUI Kit Type 2
workspace share the `nfc-app` services, with backups, read-back verification,
and typed confirmations for one-way locks and configuration. A PN532 UART
NTAG215 read plus authorized NDEF write/read/restore round trip is recorded for
2026-11-10; the ACR122U record and lock/password paths remain pending. The read
path was validated earlier on both readers. Classic NDEF maps NDEF onto MAD1,
MAD2, and NFC sectors: the adapter parses the directory and per-sector GPB,
reads and writes the mandatory NDEF Message TLV with access checks, and formats
a blank card after a typed confirmation and backup. `nfctl classic ndef` and
the Classic workspace NDEF tab share the same services. The Classic mapping
has no capability-container record; the GPB carries the mapping version and
access state.

## Product scope

Two first-class domains:

1. **MIFARE Classic workbench**: discover readers, select cards, authenticate,
   read/write/dump/restore, keys, diff, clone, factory format, UID safety, and
   native key recovery.
2. **NDEF**: read, write, format, and protect NDEF data on Type 2 / NTAG and
   MIFARE Classic; a Type 4 / ISO-DEP adapter and application workflows are
   implemented, with hardware validation still pending.

Supported hardware for the first release:

- PN532 UART (`pn532_uart`)
- ACR122U direct USB/CCID (`acr122_usb`)

Out of scope for the first release: PN53x USB, PC/SC-only readers, Proxmark3
device control, Windows, macOS, containers, AppImage/Flatpak/Snap, telemetry,
and a browser-first UI.

## Technical stack

- Rust latest stable, edition 2024.
- GUI: [GPUI Kit](https://gpui-kit.com/) only.
- Transport: pure Rust drivers over `serialport` (PN532 UART) and `nusb`
  (ACR122U USB/CCID). No C shim, no CGO, no `libnfc`.
- Recovery: native Rust, ported from the current
  [RfidResearchGroup/proxmark3](https://github.com/RfidResearchGroup/proxmark3)
  implementation. The old `mfoc`/`mfcuk`/`mfoc-hardnested` binaries are
  differential test oracles only.
- CLI: `nfctl`, English-only, shares the application services with the GUI.
- License: GPL-3.0-or-later.

## Workspace

```text
crates/
  nfc-core            # card identity, errors, shared primitives
  nfc-transport       # Reader/Transport traits
  nfc-drivers         # pn532_uart, acr122_usb
  nfc-classic         # geometry, BCC, access, value, dump, diff, exchange
  nfc-recover         # Crypto1 and Classic key-recovery attacks
  nfc-ndef            # NDEF/TLV/message/record core
  nfc-type2           # Type 2 / NTAG page and NDEF adapter
  nfc-classic-ndef    # MAD / Classic NDEF adapter
  nfc-type4           # ISO-DEP/APDU CC and NDEF file read/write
  nfc-app             # device manager, services, workflows, events
  nfctl               # CLI
  gui/classic         # GPUI Kit Classic workspace
  gui/type2           # GPUI Kit Type 2 / NDEF workspace
  gui/nfc-foundry     # GPUI Kit desktop shell
```

Dependency direction:

```text
gui/* -> nfc-app -> nfc-recover / nfc-classic / nfc-type2 / nfc-classic-ndef
      -> nfc-ndef -> nfc-core
nfc-drivers -> nfc-transport
```

## Build

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The GUI is excluded from default members so core/CLI checks stay fast. Build it
explicitly:

```sh
cargo run -p nfc-foundry
```

Hardware tests never run by default. They require an explicit feature and
environment variable; see `docs/testing.md`.

## Mock reader

The CLI can run without hardware against two scripted readers:

```sh
cargo run -p nfctl -- --mock list
cargo run -p nfctl -- --mock --device mock:classic-1k scan
cargo run -p nfctl -- --mock --device mock:ntag215 card info
```

The mock Classic 1K carries a complete BCC-valid image, so the workflow
commands can be exercised without a card:

```sh
cargo run -p nfctl -- --mock --device mock:classic-1k classic dump \
  --output /tmp/card.json
cargo run -p nfctl -- --mock --device mock:classic-1k classic read --block 4
cargo run -p nfctl -- --mock --device mock:classic-1k \
  --backup-dir /tmp/nfc-backups classic write --block 4 \
  --data 000102030405060708090A0B0C0D0E0F
cargo run -p nfctl -- --mock --device mock:classic-1k classic keyscan
```

The mock Classic NDEF 1K carries a valid MAD1 image and a Text record:

```sh
cargo run -p nfctl -- --mock --device mock:classic-ndef classic ndef info
cargo run -p nfctl -- --mock --device mock:classic-ndef classic ndef read
cargo run -p nfctl -- --mock --device mock:classic-ndef classic ndef write --text hello
```

The mock NTAG215 carries a complete 135-page NTAG215 image with an empty NDEF
TLV:

```sh
cargo run -p nfctl -- --mock --device mock:ntag215 type2 info
cargo run -p nfctl -- --mock --device mock:ntag215 type2 read --page 4
cargo run -p nfctl -- --mock --device mock:ntag215 ndef write --text hello
cargo run -p nfctl -- --mock --device mock:ntag215 ndef read
cargo run -p nfctl -- --mock --device mock:ntag215 type2 dump --output ntag.json
```

`nfctl classic keys list|import|add` manages the JSON key store. Pass
`--key-store <path>` to choose it explicitly; otherwise the default is
`$XDG_CONFIG_HOME/nfc-foundry/classic-keys.json`.

## Hardware reader

Device selection is always an exact connstring. `nfctl list` reports every
serial port as a `pn532_uart:` candidate because a UART adapter cannot be
identified without opening it. Opening validates the PN532 firmware:

```sh
cargo run -p nfctl -- list
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 scan
```

Card workflow example:

```sh
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 \
  --key FFFFFFFFFFFF classic dump --output card.json
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 \
  --key FFFFFFFFFFFF classic restore --input card.json --confirm
```

PN532 UART firmware identification, Classic 1K selection/authentication/read,
Type 2 selection/read, raw 7-bit REQA, Classic dump, key scan, temporary block
write, and restore are hardware-validated. Writes remain gated by
`NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`; see [Testing](docs/testing.md), the
[phase 02 note](docs/notes/02-pn532-uart.md), and the
[phase 04 note](docs/notes/04-classic-workflows.md) for the records.

ACR122U selection is an exact bus/address connstring:

```sh
cargo run -p nfctl -- list
cargo run -p nfctl -- --device acr122_usb:001:017 scan
```

Recovery is read-only and verifies every returned key:

```sh
cargo run -p nfctl -- \
  --device acr122_usb:001:017 \
  --key D3F7D3F7D3F7 \
  recover nested --known-block 4 --target-block 0 --target-type A
```

`recover auto` classifies the card first, `recover staticnested` uses the
static-nonce path, `recover hardnested` loads the PM3 tables and runs the
bit-sliced solver with `--max-nonces` and throttled progress, and `recover
darkside` reports a no-sample failure when the card does not expose eight
probes against one tag nonce. Use only cards you own.

Hardnested tables are read from `/usr/share/nfc-foundry/hardnested_tables`, the
checkout's `resources/hardnested_tables`, or `NFCX_HARDNESTED_TABLES` (also
`--hardnested-tables`).

`pcscd` holds the same CCID interface, so stop it before using the direct USB
path and restart it afterwards. The ACR122U was validated for firmware
identification, Classic 1K read, Type 2 read, raw 7-bit REQA, and a same-value
Gen1A manufacturer-block write; see the
[phase 05 note](docs/notes/05-acr122-usb.md).

Type 2 and NDEF commands share the same device selection:

```sh
cargo run -p nfctl -- --device acr122_usb:001:017 type2 info
cargo run -p nfctl -- --device acr122_usb:001:017 type2 read --page 4
cargo run -p nfctl -- --device acr122_usb:001:017 ndef write --text "hello"
cargo run -p nfctl -- --device acr122_usb:001:017 ndef read
cargo run -p nfctl -- --device acr122_usb:001:017 \
  type2 lock --pages 4 --confirm I_OWN_THIS_CARD
```

Type 2 reads are hardware-validated on both readers. The PN532 UART path also
passed an authorized NTAG215 NDEF write/read/restore round trip with backups
and read-back verification. The ACR122U write record, one-way locks, and
password configuration still need the hardware record described in
[Testing](docs/testing.md).

Classic NDEF commands use the same readers:

```sh
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 classic ndef info
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 classic ndef write --text hello
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 \
  classic ndef format --confirm I_OWN_THIS_CARD
```

Formatting replaces sector keys and access bits on every NFC sector. On
2026-11-10 the PN532 UART path formatted a blank Classic 1K (UID `6A 31 5E 28`),
wrote and read back a Text record, restored the original dump with zero
failures, and matched the pre-test dump byte-for-byte. The ACR122U record is
still pending because no ACR122U was connected.

## GUI

The GPUI Kit shell switches between the Classic and Type 2 workspaces:

```sh
cargo run -p nfc-foundry
```

The Classic workspace provides reader selection, card read, dump save/load,
restore with confirmation, block write with trailer confirmation, key scan, key
store management, a block inspector, a two-dump diff, and a Classic NDEF tab
with mapping facts, record display, Text writes, and a confirmed format action.
The Type 2 workspace
provides a page table and inspector, verified user-page writes, dump
save/load/restore, Text NDEF write and format, and protection actions with
typed confirmation. All operations use the same `nfc-app` services and safety
policy as `nfctl`.

## Documentation

- [Architecture](docs/architecture.md)
- [Plan](docs/plan/README.md)
- [Hardware](docs/hardware.md)
- [Recovery](docs/recovery.md)
- [NDEF](docs/ndef.md)
- [Testing](docs/testing.md)
- [Packaging](docs/packaging.md)
- [ADRs](docs/adr/)

## Safety

Use only cards, readers, and systems you own or have explicit permission to
test. Destructive operations require a backup, an explicit confirmation, and
read-back verification. NDEF lock, configuration, password, and capability
writes are Advanced operations.
