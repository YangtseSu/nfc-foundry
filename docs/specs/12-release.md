# Phase 12 spec: Release

Date: 2026-11-10

## Goal

Make the first Rust release installable and auditable:

- one Arch Linux AUR source package named `nfc-foundry`;
- one GitHub Actions binary tarball built on Arch Linux;
- one source archive for GPL compliance;
- udev rules, license texts, notices, and dependency license bundle;
- a hardware matrix that states exactly which operations each reader has
  passed;
- no package or file conflict with another installed NFC application.

The release does not add a card operation. It only packages what the completed
phases already implement and records what has been validated.

## Version contract

- `[workspace.package].version` in `Cargo.toml` is the only version source.
- A release tag is `v<version>`.
- `nfctl --version` prints `nfctl <version>`.
- `nfctl diag` prints `NFC Foundry <version>`, `nfctl <version>`, the supported
  driver prefixes, and the resolved hardnested table directory. It performs no
  hardware I/O and succeeds without a reader.
- The GUI window title is `NFC Foundry <version>`.

## AUR source package

Files live in `packaging/aur/`:

- `PKGBUILD`
- `.SRCINFO`
- `nfc-foundry.install` only if an install script is justified; it is not used
  for udev reloads.

Rules:

- `pkgname=nfc-foundry`, `license=('GPL-3.0-or-later')`, `arch=('x86_64')`.
- Source is the release tag with a VCS source and `SKIP` checksum, so the
  package always builds the exact tag instead of a mutable branch.
- `prepare()` runs `cargo fetch --locked`; `build()` runs
  `cargo build --release --locked --workspace`.
- `check()` runs `cargo test --workspace --offline` and
  `cargo check -p nfc-foundry --offline`. Hardware tests stay gated and never
  run in a package build.
- `package()` installs:
  - `/usr/bin/nfc-foundry`
  - `/usr/bin/nfctl`
  - `/usr/lib/udev/rules.d/70-nfc-foundry.rules`
  - `/usr/share/nfc-foundry/hardnested_tables/*.lz4`
  - `/usr/share/licenses/nfc-foundry/` with `LICENSE`, `LICENSES/`, `NOTICE`,
    `THIRD_PARTY_NOTICES.md`, and the dependency license bundle
  - `/usr/share/doc/nfc-foundry/` with the product, hardware, testing,
    packaging and plan documents.
- No `libnfc`, `pcsclite`, external recovery engine, `nfc-*` utility, desktop
  file, or icon is installed. The first release ships two binaries and the
  data they need.
- Runtime dependencies are the direct NEEDED libraries of the packaged ELF
  files plus the GPUI runtime libraries loaded dynamically. Build-only
  dependencies are `cargo`, `git`, `python`, and `rust`.
- The package must not contain `/usr/local`, `/opt`, or a broad
  `MODE="0666"` udev rule.

## Binary tarball

`packaging/release/package.sh <version> <out-dir>` builds a reproducible
`nfc-foundry-<version>-x86_64.tar.gz` from an already built
`target/release`:

```text
nfc-foundry-<version>-x86_64/
  README.md
  bin/nfc-foundry
  bin/nfctl
  lib/udev/rules.d/70-nfc-foundry.rules
  share/nfc-foundry/hardnested_tables/*.lz4
  share/licenses/nfc-foundry/
  share/doc/nfc-foundry/
```

The tree is relocatable. `nfctl` resolves the tables at
`/usr/share/nfc-foundry/hardnested_tables`, through
`NFC_FOUNDRY_HARDNESTED_TABLES`, or through `../share/nfc-foundry/hardnested_tables`
relative to the executable. The tarball ships a `SHA256SUMS` file at the
release root, generated separately for each uploaded asset.

A source archive is produced with `git archive` from the tagged commit, with
the `nfc-foundry-<version>/` prefix. It includes `Cargo.lock`,
`resources/hardnested_tables`, and all license files.

## CI

`.github/workflows/release.yml` runs on `v*` tags and on manual dispatch:

1. install Arch build dependencies in an `archlinux` container;
2. resolve the version and require `v<version>` to equal the tag;
3. run `make check` (`fmt`, `clippy`, `test`, GUI check, `reuse lint`);
4. build the release workspace;
5. create the binary tarball, dependency license bundle, and source archive;
6. upload the assets; a tag build publishes them to the GitHub release.

CI never sets a hardware gate and never writes a card. It does not claim a
hardware result that is not in `docs/hardware-matrix.md`.

The existing `check.yml` remains the pull-request gate. The release workflow is
the only place that produces release assets.

## Hardware matrix

`docs/hardware-matrix.md` is the release certificate:

- reader identity, firmware, connection string, and date for every record;
- one row per operation with `validated`, `not recorded`, or `not applicable`;
- an explicit "not advertised" list for implemented but unrecorded operations;
- the exact gate commands used.

A row is `validated` only when the phase note contains the corresponding run.
The release is not allowed to promote an operation from `not recorded` to
`validated` without a new note entry.

## Safety

- The release does not enable Advanced actions by default.
- CI never runs hardware tests and never writes a card.
- No keys, dumps, card data, or credentials are added to the repository or the
  release assets.
- udev rules use `uaccess` for `072F:2200` only.
- The package does not install external recovery engines.

## Non-goals

- Windows, macOS, Debian/RPM, AppImage, Flatpak, Snap, or container images.
- AUR upload automation or repository push from this phase.
- A desktop entry or icon.
- Advertising Type 4, ACR122U Type 2 writes, one-way locks, password
  configuration, trailer writes, 4K Classic, or Type 4 NDEF without a recorded
  hardware run.
- A replacement for the old `nfctl` JSON/exit-code contract.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `namcap PKGBUILD` reports no error.
- `makepkg --printsrcinfo` matches the committed `.SRCINFO`.
- The package builds and its file list contains only the documented paths.
- The package declares no `conflicts`, `provides`, or `replaces`; it uses only
  `nfc-foundry` namespaced binaries, paths, and resources, so another NFC
  application can remain installed.
- The binary tarball contains both binaries, the udev rule, the tables, the
  license bundle, and the documentation.
- `bin/nfctl --version` and `nfctl diag` report `0.1.0`; the hardware
  matrix lists the PN532 UART and ACR122U records and their gaps.
