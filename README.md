# NFC Foundry (Rust)

NFC Foundry is a Linux-only NFC reader workbench for MIFARE Classic and NFC
Forum Type 2 / NDEF cards, written in Rust.

This repository is the clean Rust rewrite. The previous Go/C implementation is
kept as a behavioral oracle and hardware-regression reference; it is not a
dependency of this project.

## Status

**Phase 04 complete.** The PN532 UART driver and the hardware-free Classic core
are implemented. The Classic workflow layer covers key stores, common-key
authentication, partial dumps, single-block reads and writes, restores, key
scans, and atomic project-format backups. `nfctl` and the GPUI Kit Classic
workspace call the same `nfc-app` methods. Every write checks access
conditions, saves a backup, writes the block, reads it back, and rechecks card
identity; restore requires confirmation, refuses to change manufacturer block
0, writes data blocks before sector trailers, and stops at the first failure by
default. PN532 hardware validation covered a 64/64-block dump, a 16-sector key
scan, a temporary data-block write with backups and read-back, and restore of
that block. Recovery algorithms and NDEF codecs arrive in later phases;
operations that are not ready return an explicit `NotImplemented` error.

## Product scope

Two first-class domains:

1. **MIFARE Classic workbench**: discover readers, select cards, authenticate,
   read/write/dump/restore, keys, diff, clone, factory format, UID safety, and
   native key recovery.
2. **NDEF**: read, write, format, and protect NDEF data on Type 2 / NTAG and
   MIFARE Classic; Type 4 NDEF follows the same NDEF core later.

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
  nfc-type4           # ISO-DEP/APDU NDEF adapter (later)
  nfc-app             # device manager, services, workflows, events
  nfctl               # CLI
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

## GUI

The GPUI Kit Classic workspace is the first feature screen:

```sh
cargo run -p nfc-foundry
```

It provides reader selection, card read, dump save/load, restore with
confirmation, block write with trailer confirmation, key scan, key store
management, a block inspector, and a two-dump diff. All operations use the same
`nfc-app` services and safety policy as `nfctl`.

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
