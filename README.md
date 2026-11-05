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

The native MyGo shell is in place: reader discovery, card facts, Classic block
viewing, the key catalog, and the task log. The remaining migration is tracked
in [`docs/plan/`](docs/plan/):

- expose the shared service layer as `nfctl`;
- replace `nfc-mfsetuid` with native Go Gen1A code;
- add NFC Forum Type 2 and NDEF support;
- validate additional readers and card families one at a time.

Only MIFARE Classic 1K/4K on a PN532 UART reader is currently hardware-tested.
Do not assume the planned features below are already available.

## Supported hardware

| Reader | Backend | Status |
|---|---|---|
| PN532 + FT232RL | `pn532_uart` via `libnfc-ng` | Tested baseline |
| Other PN532 UART adapters | `pn532_uart` | Expected compatible; not tested |
| PN53x USB | `pn53x_usb` | Planned; not validated |
| ACR122U | `acr122_usb` / `acr122_pcsc` | Planned; not validated |
| Other PC/SC readers | `pcsc` | Planned; APDU-oriented |

## Supported cards

| Card family | Status |
|---|---|
| MIFARE Classic 1K/4K | Read, write, dump, restore, keyscan, recovery |
| MIFARE Ultralight / NTAG213/215/216 | Planned; Type 2 page access and NDEF |
| ISO/IEC 14443-4 / DESFire | Later; APDU foundation first |
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

- Go 1.27.3 or newer;
- MyGo's native UI dependencies (install what `mygo doctor` reports);
- a C compiler, `pkgconf`, `make`, `autoconf`, `automake`, and `libtool`;
- `libusb` and `pcsclite` development files if those transports are enabled.

```sh
make libnfc-ng-build
make test
make build
make check
```

`make build` writes the executable and its private runtime to
`build/nfc-foundry-linux-amd64/`, ready to run without installation.

## Languages

The GUI ships in English and Simplified Chinese. English is the base language;
Chinese is a complete first-party translation.

The `nfctl` CLI is English-only: help, errors, logs, and JSON keys are not
localized. Automation should not depend on translated output.

## CLI

`nfctl` is the planned scripting interface. It shares the same services as the
GUI:

```text
nfctl list
nfctl scan [--watch]
nfctl card info
nfctl classic read|write|dump|restore|keyscan
nfctl type2 read|write|dump
nfctl uid set
nfctl recover nested|darkside|hardnested
nfctl diag
nfctl pn532 shell
nfctl apdu
nfctl ndef read|write
```

Destructive commands require `--yes`. `--json` output is intended for scripts
and CI. See [`docs/cli.md`](docs/cli.md) for the stable contract.

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
