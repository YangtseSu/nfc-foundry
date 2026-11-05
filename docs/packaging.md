# Packaging

Supported distribution is intentionally narrow:

- Arch Linux AUR source package `libnfc-ng`;
- Arch Linux AUR source package `nfc-foundry`;
- GitHub CI binary tarball built in an Arch Linux container;
- corresponding source archive on the same GitHub release.

No Windows, macOS, Debian/RPM, AppImage, Flatpak, Snap, or container images are
part of the product.

## `libnfc-ng` package

Pure library package. Contents:

```text
/usr/lib/libnfcng.so.6
/usr/include/libnfc-ng/nfc/
/usr/lib/pkgconfig/libnfc-ng.pc
/usr/share/licenses/libnfc-ng/
/usr/share/doc/libnfc-ng/
```

It must not install any upstream `nfc-*` utility or example. Build with the
upstream utilities/examples disabled, or remove their install targets in the
fork; do not rely on deleting them after `make install`.

Dependencies follow the enabled driver set:

- `pn532_uart`: no extra runtime dependency beyond glibc;
- `pn53x_usb` / `acr122_usb`: `libusb`;
- `acr122_pcsc` / `pcsc`: `pcsclite`.

The initial package builds the explicit profile
`pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc`. Only `pn532_uart` is
advertised as validated; the others are compiled but remain experimental in
the capability matrix.

This profile is a package/build choice. The library's own default driver set
must remain the upstream default, and `--with-drivers=all` must continue to
work.

Keep the unique library identity (`libnfcng.so.6`, pkg-config `libnfc-ng`) so
the package can coexist with Arch's `libnfc`. Do not install
`/usr/lib/libnfc.so.6` from this package.

## `nfc-foundry` package

Contents:

```text
/usr/bin/nfc-foundry
/usr/bin/nfctl
/usr/lib/nfc-foundry/mfoc
/usr/lib/nfc-foundry/mfcuk
/usr/lib/nfc-foundry/mfoc-hardnested
/usr/lib/udev/rules.d/70-nfc-foundry.rules
/usr/share/applications/nfc-foundry.desktop
/usr/share/icons/hicolor/.../apps/nfc-foundry.png
/usr/share/licenses/nfc-foundry/
```

The external engines live in the application's private directory so they do
not collide with system packages. The Go binary locates them relative to its
own executable; do not depend on `PATH` or the current working directory.

The Gen1A helper is native Go. Do not install `nfc-mfsetuid`,
`nfc-list`, `nfc-scan-device`, `nfc-mfclassic`, or `nfc-mfultralight`.

Dependencies:

- `glibc`;
- `libnfc-ng`;
- `libusb` when USB transports are enabled;
- `pcsclite` when PC/SC is enabled; `pcscd` is optional at runtime;
- MyGo native runtime libraries as reported by `mygo doctor` on Arch; the
  native UI does not use WebKitGTK;
- fonts/icon theme consistent with the desktop file.

`makedepends` include Go, a C toolchain, `pkgconf`, `make`, autotools/libtool,
and the build-time libraries needed by `libnfc-ng`.

## Permissions

- PN532 UART: add the user to `dialout`; do not run the app as root.
- PN53x/ACR122 USB: ship a udev rule using `TAG+="uaccess"` or a documented
  group; do not use world-writable `MODE="0666"`.
- PC/SC: `pcscd` must be running. Report an actionable error if it is absent or
  the reader is claimed by another daemon.
- Direct USB and `pcscd` can claim the same reader. Do not run both paths
  against one device simultaneously.

## GitHub CI

Use an `archlinux:latest` container for build and test:

1. install `base-devel`, Go, `pkgconf`, `reuse`, and the native UI libraries
   that `mygo doctor` reports;
2. run `reuse lint`;
3. run `go test ./...` in the hardware-free mode, including the MyGo view
   smoke test;
4. run `go vet ./...` and `go tool mygo i18n check .`;
5. build the application with `go build`;
6. for a release, build `libnfc-ng`, the external engines, and both packages,
   then upload the `.pkg.tar.zst` files, the binary tarball, and the source
   archive.

Hardware tests are never part of ordinary CI. Keep them behind an explicit
environment variable or build tag.

## Source obligations

The release must provide:

- the complete NFC Foundry source and build scripts;
- the `libnfc-ng` source at the pinned commit and its patches;
- the patched `mfoc`, `mfcuk`, and `mfoc-hardnested` sources;
- SPDX license texts, `REUSE.toml`, `NOTICE`, and
  `THIRD_PARTY_NOTICES.md`.

A stable GitHub source tarball attached to the same release is the simplest
way to satisfy GPL/LGPL source obligations. Do not publish a binary release
without it.

## Versioning

- `nfc-foundry`: semantic version; the AUR `pkgver` matches the release tag.
- `libnfc-ng`: upstream base version plus the fork revision/tag; bump the
  soname only for an actual ABI break.
- external engines: pinned upstream tag/commit; rebuild them whenever the
  `libnfc-ng` ABI changes.

## Binary tarball

```text
nfc-foundry-<version>-linux-amd64/
  nfc-foundry
  runtime/linux-amd64/
    libnfcng.so.6
    mfoc
    mfcuk
    mfoc-hardnested
    LICENSES/
    source.lock
  LICENSES/
  NOTICE
  THIRD_PARTY_NOTICES.md
  README.md
```

The executable links `libnfcng.so.6` from `runtime/linux-amd64` through an
`$ORIGIN` rpath. `nfctl` joins the tarball when Phase 03 lands.

The binary tarball is for Arch Linux/current glibc only. State that explicitly;
do not present it as a generic Linux build.
