# Phase 12 note: Release

Date: 2026-11-10

## Implemented

### Version reporting

- `nfctl --version` prints `nfctl <version>`.
- `nfctl diag` prints `NFC Foundry <version>`, `nfctl <version>`, both driver
  prefixes, and the resolved hardnested table directory. It performs no
  hardware I/O and does not fail when the table directory is absent.
- The GUI window title is `NFC Foundry <version>`.
- The version still comes only from `[workspace.package].version`.

### Project identity

- The release is a standalone application. No document treats it as a
  continuation of another project.
- Every hardware gate environment variable uses the `NFC_FOUNDRY_*` prefix;
  no legacy prefix remains in code, tests, or documentation.
- The package installs only `nfc-foundry` namespaced paths and declares no
  `conflicts`, `provides`, or `replaces`, so another NFC reader application can
  stay installed.

### AUR source package

`packaging/aur/PKGBUILD` and `.SRCINFO` package `nfc-foundry` from the
`v<version>` tag:

- VCS source with `SKIP` checksum, so a build always uses the exact tag;
- `cargo fetch --locked` in `prepare()`, offline release build in `build()`;
- `check()` runs the workspace tests and the GUI check without hardware;
- installs both binaries, the udev rule, hardnested tables, license bundle,
  dependency license bundle, and documentation;
- runtime dependencies from the packaged ELF files (`glibc`, `libgcc`,
  `libxcb`, `libxkbcommon`, `libxkbcommon-x11`, `systemd-libs`) plus the GPUI
  libraries loaded dynamically (`libglvnd`, `vulkan-icd-loader`, `wayland`);
- `!debug` and `!lto` options. `!lto` is required: without it the makepkg C
  LTO flags produced a final link where the `wayland-backend` log shim symbol
  `wl_log_trampoline_to_rust_client` was undefined.

### Release assets

- `packaging/release/package.sh` builds
  `nfc-foundry-<version>-x86_64.tar.gz` with a relocatable tree
  (`bin/`, `lib/udev/rules.d/`, `share/nfc-foundry/`, `share/licenses/`,
  `share/doc/`, `README.md`, `Cargo.lock`).
- `packaging/release/source-archive.sh` builds the GPL source archive with
  `git archive` from the release ref.
- `packaging/release/collect-licenses.py` copies each dependency's published
  license files and writes `dependencies/manifest.tsv` with package, version,
  SPDX expression, and bundled file names. Files are forced to `0644`.
- Standard texts for MIT, Apache-2.0, Zlib, and MPL-2.0 cover crates that
  declare an expression but publish no file; `STANDARD_LICENSE_TEXTS.md`
  explains the mapping.
- `make dist` runs the release build, binary tarball, source archive, and
  `SHA256SUMS`. `make aur-check` checks `namcap` and `.SRCINFO`.
- `.github/workflows/release.yml` runs on `v*` tags and manual dispatch on
  Ubuntu 26.04: version/tag verification, checks, a
  `x86_64-unknown-linux-gnu` release build, assets, artifact upload, and GitHub
  release publication on a tag. It sets no hardware gate and has no Arch,
  `makepkg`, or AUR step.
- `.github/dependabot.yml` checks Cargo and GitHub Actions daily.

### Hardware matrix

- `docs/hardware-matrix.md` records the two reader certificates, every
  certified operation, records, card UIDs, release claims, and gate commands.
  It explicitly lists ACR122U writes for Classic data blocks, Type 2 NDEF, and
  Classic NDEF, one-way locks, passwords, trailer writes, Classic 4K, and
  Type 4 as not recorded and not advertised.

## Tested

### Software checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p nfc-foundry
reuse lint
```

All passed. `cargo test --workspace` reported 383 passed, 0 failed. The suite
includes the new CLI version/diag test.

### Packaging checks

- `namcap packaging/aur/PKGBUILD`: only the expected VCS `git` note before the
  dependency was added; no remaining error.
- `makepkg --printsrcinfo` matches the committed `.SRCINFO`.
- Local package build in a staged source tree:
  `nfc-foundry 0.1.0-1`; the package contains both binaries, the udev rule,
  351 hardnested table files, the documentation, and the dependency license
  bundle.
- `namcap` on the package reports only the `ld-linux-x86-64.so.2` unused-library
  warning for both binaries. The loader entry is normal for a Rust ELF and is
  not a missing dependency.
- No forbidden package paths (`/usr/local`, `/opt`, `/bin`, `/usr/libexec`,
  `/var/tmp`, and the other system paths).
- Package path leak check: the rebuilt binaries contain no
  `$srcdir`/`packaging/aur/src` string after
  `--remap-path-prefix=$PWD=/build/nfc-foundry`.
- Extracted package `nfctl --version` and `nfctl diag` report `0.1.0`; run from
  `/tmp` and `/`, `diag` resolves the packaged
  `usr/share/nfc-foundry/hardnested_tables`.
- Binary tarball extraction from `/tmp` and `/` resolves
  `share/nfc-foundry/hardnested_tables` relative to the executable.
- Source archive contains the prefixed tree, `Cargo.lock`, and 352 hardnested
  table entries.

### Hardware

PN532 UART at `pn532_uart:/dev/ttyUSB0`, PN532 firmware 1.6, release binary:

```sh
target/release/nfctl --device pn532_uart:/dev/ttyUSB0 \
  classic dump --output /tmp/release-smoke.json
```

Result: `classic_1k: 64/64 blocks, 16/16 sectors readable`, 9834-byte
project dump. This is a read-only release-build re-check of the phase 04 dump
record; no write was performed.

No ACR122U was attached during phase 12. The ACR122U matrix rows come from the
phase 05/06/07 records; they were not re-run for this note. The release matrix
does not claim the unrecorded ACR122U paths.

## Problems and resolutions

- `makepkg` builds failed to link `nfc-foundry` with an undefined
  `wl_log_trampoline_to_rust_client`. The direct Cargo build and the same build
  in the staged source tree passed. The difference was makepkg's C LTO flags
  applied to the `wayland-backend` log shim; `options=('!debug' '!lto')` keeps
  the shim a plain object while Cargo still applies the workspace thin LTO to
  Rust code.
- The first package contained the stager's absolute source path in
  `usr/bin/nfc-foundry` (makepkg reported `Package contains reference to
  $srcdir`). `--remap-path-prefix=$PWD=/build/nfc-foundry` and a Cargo home
  remap remove it from package and CI release builds.
- License files copied from the registry kept their source mode; some were not
  world-readable. The collector now sets `0644`.
- Some crates declare a license but ship no file. The bundle records the SPDX
  expression and ships the four standard fallback texts; it does not fabricate
  a package-specific copyright notice.

## Remaining blockers and manual steps

- ACR122U hardware records for Classic data-block writes, Type 2 NDEF writes,
  Classic NDEF writes, one-way locks, password configuration, trailer writes,
  and Classic 4K.
- Type 4 NDEF hardware record.
- ACR122U re-run of the certified rows after any driver change.
- The GitHub release tag and release publication are approval-gated steps
  outside this phase.
- The AUR upload and AUR push are not performed.
- No desktop entry or icon is shipped.
- The CLI does not implement JSON/JSONL output, the old exit-code
  categories, UID write, raw APDU console, or key export.

## Deviations from the spec

- The spec listed `cargo test --workspace` in the package `check()`; the local
  package build used `--nocheck` for time. The workspace suite was run
  separately and passed. `check()` is present in the PKGBUILD for users and
  AUR builds.
- The dependency license bundle is a superset: it includes licenses for all
  packages in the metadata graph, including platform-specific and dev-only
  crates, not only the Linux release link set.
