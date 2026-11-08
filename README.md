<p align="center"><strong>NFC Foundry</strong></p>

<p align="center">A Linux-only NFC workbench for readers driven by the <code>libnfc-ng</code> fork.</p>

NFC Foundry is a desktop application and command-line workbench for reading,
writing, dumping, and recovering MIFARE-class NFC cards. It is built for Arch
Linux first and distributed as an AUR source package or a GitHub CI binary
tarball.

This repository is the continuation of the NFCX codebase. The previous
documentation is archived in [`docs/archive/nfcx-upstream/`](docs/archive/nfcx-upstream/);
the current architecture and migration steps live in [`docs/`](docs/).

## Status

The native MyGo shell and the `nfctl` CLI are in place: reader discovery, card
facts, Classic block read/write, dump/restore, key scanning, key recovery, UID
writes with an in-process Gen1A path, Type 2 page access, NDEF read/write
from the CLI and GUI, Classic NDEF read/write/format through the MIFARE
Application Directory, an ISO/IEC 14443-4 APDU foundation with read-only
DESFire access and Type 4 NDEF, diagnostics, and the key catalog. Classic GUI writes
cover ordinary data blocks, changed-block writes, full restore with a mandatory
pre-write backup, and advanced UID writes. The reader layer compiles the five
drivers in the application profile and reports an explicit capability profile
for each; the remaining migration is tracked in
[`docs/plan/`](docs/plan/).

MIFARE Classic 1K/4K is hardware-tested on a PN532 UART reader and on the
ACR122U through `acr122_usb`. UID writes run natively in Go: the ordinary
DirectWrite path covers CUID/Gen2, UFUID, and FUID, and Gen1A cards use the
in-process backdoor. Gen1A is hardware-tested on the ACR122U; UFUID/FUID
hardware validation is pending. No `nfc-mfsetuid` executable is shipped or
required. Type 2 page access and NDEF read/write are hardware-tested on an
NTAG215 with the ACR122U. Classic NDEF read/write/format is hardware-tested on
a blank MIFARE Classic 1K with the ACR122U. The ISO-DEP, DESFire read, and
Type 4 NDEF paths are implemented but not hardware-validated yet. Do not assume
the planned features below are already available.

## Supported hardware

| Reader | Backend | Status |
|---|---|---|
| PN532 + FT232RL | `pn532_uart` via `libnfc-ng` | Tested baseline: Classic and card selection; ISO-DEP card workflows pending |
| Other PN532 UART adapters | `pn532_uart` | Expected compatible; not tested |
| PN53x USB | `pn53x_usb` | Compiled in the application profile; not hardware-validated |
| ACR122U | `acr122_usb` | Tested: Classic 1K keyscan, dump/restore, Gen1A UID write, Classic 1K MAD1 NDEF read/write/format, and NTAG215 page/NDEF I/O; ISO-DEP/Type 4/DESFire hardware pending |
| ACR122U | `acr122_pcsc` | Compiled but not validated; open failed or timed out on the tested firmware |
| ACR122U | `pcsc` | Selection only; PC/SC is an APDU/smart-card path, not a raw NFC transport |
| Other PC/SC readers | `pcsc` | Selection only; APDU transport not implemented; Classic/Type 2 are out of scope |

`pn53x_usb` is for NXP PN531/PN533 USB dongles, SCM SCL3711/SCL3712,
Sony PaSoRi RC-S360, and ASK LoGO. ACR122U is `acr122_usb`, and a PN532 UART
adapter is `pn532_uart`; neither appears as `pn53x_usb`.

One ACR122U can enumerate as direct USB and two PC/SC aliases. NFC Foundry
prefers direct USB, hides the matching PC/SC aliases, and uses one physical
lock for every ACR122 connstring so two processes cannot claim the same reader.

## Supported cards

| Card family | Status |
|---|---|
| MIFARE Classic 1K/4K | Read, write, dump, restore, keyscan, recovery, and MAD-based NDEF read/write/format from the CLI and GUI; Classic 1K NDEF hardware-tested on ACR122U, 4K/MAD2 pending |
| NTAG213/215/216 | Page and NDEF read/write from CLI and GUI; Text language, provider presets, Smart Poster metadata, Bluetooth LE, raw/multi-record editors, file import/export, copy/erase/lock/password lifecycle; decoded Text, URI, Smart Poster, vCard, Wi-Fi (including DPP bootstrapping), Bluetooth, and app records; NTAG215 hardware-tested |
| MIFARE Ultralight | Page read/write and dump; original-Ultralight fallback not hardware-tested |
| ISO/IEC 14443-4 / DESFire | APDU foundation, read-only DESFire enumeration/file read, and Type 4 NDEF read/write; no hardware validation yet |
| FeliCa / NFC Forum Type 3 | Later; NDEF scope to be defined |
| Jewel / Topaz | Optional, later |

Card emulation is a future direction for mobile Linux devices. It is not part
of the current milestones and is not advertised yet.

Use NFC Foundry only with cards, readers, and systems you own or are explicitly
authorized to test.

## Architecture

```text
MyGo native GUI            nfctl CLI
        \                    /
         Go application services
         |-- device manager and lock
         |-- card-family workflows
         |-- dump/restore and write safety
         `-- external engine adapters
                    |
          CGO + narrow C shim
                    |
              libnfc-ng (LGPL)
                    |
        Linux readers (pn532_uart, USB, PC/SC)
```

`libnfc-ng` is a standalone fork of libnfc. It is a pure library package:
shared library, headers, and pkg-config only. It does not install upstream
`nfc-*` utilities. The application links it through a narrow C shim; card
protocols and safety rules stay in Go.

Key recovery continues to use `mfoc`, `mfcuk`, and `mfoc-hardnested` as
separate processes. The application releases the reader, runs the engine with
an exact connstring, validates its output dump, reopens the reader, and
verifies recovered keys itself.

## Build

Requirements on Arch Linux:

- the latest stable Go release (MyGo requires 1.27.3 or newer);
- MyGo's native UI dependencies (install what `mygo doctor` reports);
- a C compiler, `pkgconf`, `make`, `autoconf`, `automake`, and `libtool`;
- `libusb-compat` and `pcsclite` development files if those transports are enabled.

```sh
make libnfc-ng-build
make test
make build
make check
```

`make build` writes `nfc-foundry`, `nfctl`, and the private runtime to
`build/nfc-foundry-linux-amd64/`, ready to run without installation.

## Languages

The GUI ships in English and Simplified Chinese. English is the base language;
Chinese is a complete first-party translation.

The `nfctl` CLI is English-only: help, errors, logs, and JSON keys are not
localized. Automation should not depend on translated output.

## CLI

`nfctl` is the scripting interface. It shares the same services as the GUI:

```text
nfctl list
nfctl scan [--watch]
nfctl card info
nfctl classic read|write|dump|restore|keyscan
nfctl recover nested|darkside|hardnested
nfctl uid set
nfctl diag
nfctl keys list|add|import|export
nfctl type2 info|read|write|dump|restore|copy|erase|lock|password
nfctl ndef read|write
nfctl apdu [--raw HEX]
nfctl desfire info|apps|files|read
```

Destructive commands require `--yes`; trailer and UID writes also require their
specific confirmation flags. `--json` and `--jsonl` output is intended for
scripts and CI. See [`docs/cli.md`](docs/cli.md) for the stable contract.

## Distribution

Supported channels:

- AUR: `libnfc-ng` and `nfc-foundry`;
- GitHub Releases: Arch Linux binary tarball and corresponding source.

No Windows, macOS, Debian/RPM, AppImage, Flatpak, or Snap packages are planned.

## Documentation

- [Documentation index](docs/README.md)
- [Architecture](docs/architecture.md)
- [Card support and safety](docs/cards.md)
- [CLI contract](docs/cli.md)
- [Packaging](docs/packaging.md)
- [Development workflow](docs/development.md)
- [Phased plan](docs/plan/README.md)

## License

NFC Foundry is licensed under **GPL-3.0-or-later**. See [`LICENSE`](LICENSE).

Third-party components:

- `libnfc-ng`: LGPL-3.0-or-later, forked from libnfc;
- `mfoc`, `mfcuk`, `mfoc-hardnested`: GPL-2.0-or-later, separate executables;
- NFCX reference code: MIT;
- MyGo: MIT.

The repository is [REUSE](https://reuse.software/) compliant: path-based
annotations are in [`REUSE.toml`](REUSE.toml), SPDX license texts are in
[`LICENSES/`](LICENSES/), and `reuse lint` is part of the checks.

Redistributions must include the corresponding source, license texts, and any
NFC Foundry patches. See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

## Responsible use

NFC Foundry is intended for interoperability, backup, research, development,
and authorized testing. You are responsible for complying with applicable law
and for using it only on cards and systems you own or are permitted to test.
