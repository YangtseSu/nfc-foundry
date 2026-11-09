# Packaging and CI

## Supported outputs

- Arch Linux AUR source package for `nfc-foundry`.
- GitHub CI binary tarball built on Arch Linux.

No Windows, macOS, Debian/RPM, AppImage, Flatpak, Snap, or container packaging.

## Dependencies

Expected system dependencies:

- Rust stable toolchain for building from source.
- `udev` rules for PN532 UART and ACR122U USB permissions.
- `gpui-kit` Linux runtime dependencies as documented by the crate (Wayland/X11,
  GPU, fonts, accessibility).
- No `libnfc`, no `pcsclite`, no external recovery engines in the first
  release.

`nusb` is used for ACR122U USB and does not require libusb. `serialport` uses
the OS serial API.

## AUR

Source package builds with:

```sh
cargo build --release --locked --workspace
cargo install --path crates/nfctl --locked
cargo install --path crates/gui/nfc-foundry --locked
```

- Use vendored dependencies or an offline `Cargo.lock` strategy consistent with
  AUR `cargo` packaging.
- Install `nfc-foundry`, `nfctl`, udev rules, license, and notices.
- Do not install `mfoc`, `mfcuk`, or `mfoc-hardnested`.
- Install no `nfc-*` utilities from libnfc.

## CI

Required jobs:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `cargo deny check` for licenses/advisories once the dependency set is stable
- package build and binary tarball

Hardware jobs are separate, manual, and environment-gated. CI must never write
a card.

## Release contents

- `nfc-foundry` GUI binary
- `nfctl`
- udev rules
- `LICENSE`, `LICENSES/`, `NOTICE`
- `THIRD_PARTY_NOTICES.md`
- documentation and plan status
- `Cargo.lock`
- source archive for GPL compliance

## Versioning

- Rust workspace version in `Cargo.toml`.
- `Cargo.lock` committed.
- Release tags correspond to the packaged version.
- GUI and CLI report the same build version.
