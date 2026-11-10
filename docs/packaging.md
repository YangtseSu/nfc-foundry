# Packaging and CI

## Supported outputs

- Arch Linux AUR source package for `nfc-foundry`.
- GitHub CI binary tarball for Linux amd64 built on Ubuntu 26.04.
- GPL source archive from the release tag.

No Windows, macOS, Debian/RPM, AppImage, Flatpak, Snap, desktop entry, or
container image is part of the product.

## Version and tag

`[workspace.package].version` in `Cargo.toml` is the only version source. The
release tag is `v<version>`. `nfctl --version`, `nfctl diag`, and the GUI window
title report the same value.

## AUR source package

Files:

```text
packaging/aur/PKGBUILD
packaging/aur/.SRCINFO
```

The package is a source package. It builds the exact release tag:

```sh
cd packaging/aur
makepkg --printsrcinfo > .SRCINFO
makepkg -sri
```

`prepare()` runs `cargo fetch --locked`. `build()` runs
`cargo build --release --locked --offline --workspace`. `check()` runs
`cargo test --workspace --offline` and `cargo check -p nfc-foundry --offline`.
Hardware tests are never enabled by a package build.

Package contents:

```text
/usr/bin/nfc-foundry
/usr/bin/nfctl
/usr/lib/udev/rules.d/70-nfc-foundry.rules
/usr/share/nfc-foundry/hardnested_tables/*.lz4
/usr/share/licenses/nfc-foundry/LICENSE
/usr/share/licenses/nfc-foundry/LICENSES/
/usr/share/licenses/nfc-foundry/NOTICE
/usr/share/licenses/nfc-foundry/THIRD_PARTY_NOTICES.md
/usr/share/licenses/nfc-foundry/STANDARD_LICENSE_TEXTS.md
/usr/share/licenses/nfc-foundry/dependencies/
/usr/share/doc/nfc-foundry/README.md
/usr/share/doc/nfc-foundry/{architecture,hardware,hardware-matrix,testing,packaging,recovery,ndef}.md
/usr/share/doc/nfc-foundry/plan.md
/usr/share/doc/nfc-foundry/adr/
```

Do not install:

- `libnfc`, `pcsclite`, or a C shim;
- `mfoc`, `mfcuk`, `mfoc-hardnested`, or any external recovery engine;
- `nfc-*` utilities from libnfc;
- a broad `MODE="0666"` udev rule;
- files under `/usr/local` or `/opt`.

Runtime dependencies are the direct NEEDED libraries of the packaged ELF
files (`glibc`, `libgcc`, `libxcb`, `libxkbcommon`, `libxkbcommon-x11`,
`systemd-libs`) plus the GPUI runtime libraries it loads dynamically
(`libglvnd` for EGL/GL, `vulkan-icd-loader`, and `wayland`). Build-only
dependencies are `cargo`, `git`, `rust`, and `python` (the dependency license
collector).

## Coexistence

The package installs only `nfc-foundry` namespaced paths and declares no
`conflicts`, `provides`, or `replaces`. Another NFC reader application can stay
installed: the two stacks share no binary, configuration, data, udev rule, or
resource directory.

They cannot both own one physical reader at the same time. Stop the other
process before opening the reader. The ACR122U direct USB path also requires
`pcscd` to be stopped; another application that uses PC/SC needs it running, so
switch the reader between the stacks instead of running both.

## Binary tarball

```sh
cargo build --release --locked --workspace
packaging/release/package.sh 0.1.0 dist
```

or:

```sh
make dist
```

The result is `dist/nfc-foundry-0.1.0-x86_64.tar.gz`:

```text
nfc-foundry-0.1.0-x86_64/
  README.md
  Cargo.lock
  bin/nfc-foundry
  bin/nfctl
  lib/udev/rules.d/70-nfc-foundry.rules
  share/nfc-foundry/hardnested_tables/*.lz4
  share/licenses/nfc-foundry/
  share/doc/nfc-foundry/
```

The tree is relocatable. `nfctl` resolves hardnested tables from
`/usr/share/nfc-foundry/hardnested_tables`, `NFC_FOUNDRY_HARDNESTED_TABLES`, or
`../share/nfc-foundry/hardnested_tables` relative to the executable.

Extract and run without installing:

```sh
tar -xzf nfc-foundry-0.1.0-x86_64.tar.gz
./nfc-foundry-0.1.0-x86_64/bin/nfctl diag
```

For a system-wide unpack, copy `bin/` into `/usr/bin`, the udev rule into
`/usr/lib/udev/rules.d/`, and the two `share/` trees under `/usr/share/`.

## Source archive

```sh
packaging/release/source-archive.sh 0.1.0 dist v0.1.0
```

The archive is created with `git archive` from the tagged commit and uses the
`nfc-foundry-<version>/` prefix. It contains the complete source,
`Cargo.lock`, `resources/hardnested_tables/`, and every license file.

## License bundle

The release carries:

- `LICENSE`, `LICENSES/`, `NOTICE`, and `THIRD_PARTY_NOTICES.md`;
- one directory per dependency with its license files;
- `dependencies/manifest.tsv` with the package, version, SPDX expression, and
  bundled file names.

The collector reads `cargo metadata --locked` and the registry sources that
`cargo fetch` already downloaded:

```sh
python3 packaging/release/collect-licenses.py --root . --output /tmp/licenses
```

Packages that publish only an SPDX expression appear in `manifest.tsv` without
a bundled file; the expression names the license text to resolve.

## CI

`.github/workflows/check.yml` is the pull-request gate on Ubuntu 26.04:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`

`.github/workflows/release.yml` runs on `v*` tags and on manual dispatch. It
uses Ubuntu 26.04, installs the native build dependencies, verifies that the
tag matches `Cargo.toml`, runs the checks, builds only
`x86_64-unknown-linux-gnu`, creates the binary tarball, license bundle, and
source archive, writes `SHA256SUMS`, and uploads the assets. A tag build
publishes them to the GitHub release.

The workflow contains no Arch Linux, `makepkg`, or AUR step. The AUR package in
`packaging/aur` is maintained for the AUR release path and is validated there,
not in GitHub CI.

`.github/dependabot.yml` checks Cargo and GitHub Actions daily.

CI never sets a hardware gate and never writes a card. Hardware results live in
`docs/hardware-matrix.md` and are referenced by the release note, not produced
by CI.

## Release checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p nfc-foundry
reuse lint
```

AUR-side checks, run outside GitHub CI:

```sh
namcap packaging/aur/PKGBUILD
(cd packaging/aur && makepkg --printsrcinfo > .SRCINFO)
```

After a package build:

```sh
namcap nfc-foundry-0.1.0-1-x86_64.pkg.tar.zst
pacman -Qlp nfc-foundry-0.1.0-1-x86_64.pkg.tar.zst
```

The file list must contain only the documented paths. Start the installed
`nfctl` and run:

```sh
nfctl --version
nfctl diag
```

## udev and permissions

- ACR122U: `resources/udev/70-nfc-foundry.rules` matches `072F:2200` with
  `TAG+="uaccess"`. Replug the reader after installation.
- PN532 UART: add the user to `uucp` on Arch Linux (or `dialout` elsewhere) when
  the port is not accessible. Never grant access to every serial port.
- `pcscd` claims the ACR122U CCID interface. Stop it before a direct USB
  session and restart it afterwards if other smart-card applications need the
  reader.

## Release checklist

1. Update the workspace version and the phase notes.
2. Update `docs/hardware-matrix.md` with only recorded hardware runs.
3. Run the release checks above.
4. Build and inspect the AUR package and binary tarball.
5. Tag `v<version>` and push the tag only with explicit approval.
6. Let `.github/workflows/release.yml` publish the tarball and source archive.
7. Copy `PKGBUILD` and `.SRCINFO` into the `nfc-foundry` AUR repository and
   push that repository separately.
