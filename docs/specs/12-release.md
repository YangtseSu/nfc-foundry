# Phase 12 spec: Release

Date: 2026-11-10

## Goal

Make the first Rust release installable and auditable:

- one Arch Linux AUR source package named `nfc-foundry`;
- one GitHub Actions binary tarball for Linux amd64 built on Ubuntu 26.04;
- GitHub-generated source archives for GPL compliance;
- udev rules, the project's license texts, and notices;
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
  - `/usr/share/licenses/nfc-foundry/` with `LICENSE`, the GPL text,
    `NOTICE`, and `THIRD_PARTY_NOTICES.md`
- No `libnfc`, `pcsclite`, external recovery engine, `nfc-*` utility, desktop
  file, or icon is installed. The first release ships two binaries and the
  data they need.
- Runtime dependencies are the direct NEEDED libraries of the packaged ELF
  files plus the GPUI runtime libraries loaded dynamically. Build-only
  dependencies are `cargo`, `git`, and `rust`.
- The package must not contain `/usr/local`, `/opt`, or a broad
  `MODE="0666"` udev rule.

## Binary tarball

The release workflow builds `nfc-foundry-<version>-x86_64.tar.gz` inline from
the `x86_64-unknown-linux-gnu` build:

```text
nfc-foundry-<version>-x86_64/
  README.md
  bin/nfc-foundry
  bin/nfctl
  lib/udev/rules.d/70-nfc-foundry.rules
  share/nfc-foundry/hardnested_tables/*.lz4
  share/licenses/nfc-foundry/
```

The tree is relocatable. `nfctl` resolves the tables at
`/usr/share/nfc-foundry/hardnested_tables`, through
`NFC_FOUNDRY_HARDNESTED_TABLES`, or through
`../share/nfc-foundry/hardnested_tables` relative to the executable.
The release includes a `SHA256SUMS` file. GitHub attaches its generated source
archives to the release; no custom source archive is built.

## CI

`.github/workflows/release.yml` runs on `v*` tags and on manual dispatch:

1. install the Linux build dependencies on Ubuntu 26.04;
2. resolve the version and require `v<version>` to equal the tag;
3. build only the `x86_64-unknown-linux-gnu` target;
4. create the binary tarball and license notices;
5. upload the binary tarball and `SHA256SUMS`; a tag build publishes them.

The release workflow does not re-run formatters, clippy, tests, or REUSE.
`check.yml` runs that suite on branches and pull requests and does not run on
tag pushes.

The workflow contains no Arch Linux, `makepkg`, or AUR step. The AUR package is
maintained for the AUR release path outside GitHub CI.

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
  license notices.
- `bin/nfctl --version` and `nfctl diag` report `0.1.0`; the hardware
  matrix lists the PN532 UART and ACR122U records and their gaps.
