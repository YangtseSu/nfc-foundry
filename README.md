# NFC Foundry (Rust)

NFC Foundry is a Linux-only NFC reader workbench for MIFARE Classic and NFC
Forum Type 2 / NDEF cards, written in Rust.

This repository is the clean Rust rewrite. The previous Go/C implementation is
kept as a behavioral oracle and hardware-regression reference; it is not a
dependency of this project.

## Status

**Phase 02 complete.** The PN532 UART framing, initialization, ISO/IEC 14443A
selection, MIFARE Classic authentication and block read/write, raw byte/bit
exchange, and reader lifecycle are implemented. Firmware identification,
Classic 1K selection/authentication/read, Type 2 selection/read, raw REQA, and
missing-card/wrong-reader errors are validated on a PN532 over UART; the
same-value write test remains behind explicit authorization and was not run.
Classic workflows, recovery algorithms, and NDEF codecs arrive in later phases;
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
  nfc-classic         # geometry, BCC, access, value, dump, diff
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

## Hardware reader

Device selection is always an exact connstring. `nfctl list` reports every
serial port as a `pn532_uart:` candidate because a UART adapter cannot be
identified without opening it. Opening validates the PN532 firmware:

```sh
cargo run -p nfctl -- list
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 scan
```

PN532 UART firmware identification, Classic 1K selection/authentication/read,
Type 2 selection/read, and raw 7-bit REQA are hardware-validated. The
same-value write remains gated by `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`; see
[Testing](docs/testing.md) for the gated checks and
[the phase 02 note](docs/notes/02-pn532-uart.md) for the validation record.

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
