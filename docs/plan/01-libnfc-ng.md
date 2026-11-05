# Phase 01: libnfc-ng as a pure library

## Goal

Ship `libnfc-ng` as a standalone Linux library package with no command-line
utilities, while keeping the full upstream driver set available.

This phase has two separate concerns:

1. **The library** must not lose hardware support and must not narrow its
   default driver set.
2. **The NFC Foundry release profile** may select a smaller, validated driver
   set explicitly at configure time. That selection belongs to the application
   package/build profile, not to libnfc-ng's default.

## Work

- Build `libnfcng.so.6`, public headers, and `libnfc-ng.pc`.
- Keep upstream `utils/` and `examples/` buildable for diagnostics but do not
  install them. They default to `no` in the library build.
- Keep upstream Windows/macOS source; build/test Linux only.
- Keep the upstream default driver set in `m4/libnfc_drivers.m4`. Do **not**
  change the default to `pn532_uart`.
- Keep all upstream drivers available. The library must still support
  `--with-drivers=all`.
- Define the initial NFC Foundry driver profile explicitly in the AUR/CI build:
  `pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc`.
  SPI/I2C/pn71xx/legacy serial drivers stay available in the library but are
  not part of the initial package profile unless their dependencies and
  hardware are validated.
- Mark only `pn532_uart` as validated in the application capability matrix.
  Compiling a driver does not mean advertising full support for it.
- Apply the required NFCX patches as commits on `libnfc-ng`, using upstream PR
  versions when they exist.
- Install no `nfc-*` binary.
- Add an AUR `libnfc-ng` package.
- Record the fork commit/tag used by the application.

## Patch policy

Original NFCX patches and how to carry them:

| NFCX patch | Action | Upstream source |
|---|---|---|
| `0001-list-env-devices-without-conffiles.patch` | Apply as a fork commit | No upstream PR recorded |
| `0002-macos-uart-synchronous-send.patch` | Apply the final upstream PR version | PR #633 (open, unmerged); pin the adopted commit |
| `0003-modern-mingw-environment-compat.patch` | Apply as a fork commit | No upstream PR recorded |
| `0004-modern-mingw-native-bitness.patch` | Use the upstream merged fix instead of the local patch if it covers the same change | Upstream PR #661 / merged master commit |
| `0005-windows-uart-query-dos-device.patch` | Apply as a fork commit | No upstream PR recorded |

Rules:

- If an upstream PR exists, use the upstream patch/commit and record its URL
  and commit. Do not maintain a divergent local copy.
- If no upstream PR exists, apply the local patch as a reviewed fork commit and
  document why it is needed.
- Do not apply both a local patch and the upstream equivalent for the same
  change.
- **Never apply a patch that narrows the default driver set, removes driver
  sources, or disables hardware by default.** The `m4/libnfc_drivers.m4`
  default must remain the upstream default.
- A patch that only selects drivers for a specific package build belongs in
  `PKGBUILD`/CI, not in the library fork.

## Acceptance

- `pkg-config --cflags --libs libnfc-ng` works.
- A C smoke test can initialize, enumerate, open, select, and close.
- `./configure` without `--with-drivers` keeps the upstream default set.
- `./configure --with-drivers=all` includes every available driver.
- `./configure --with-drivers=pn532_uart` still works as an explicit profile.
- The AUR/CI build uses the documented NFC Foundry profile explicitly.
- `pacman -Ql libnfc-ng` contains no `/usr/bin/nfc-*`.
- The library coexists with Arch's `libnfc`.
- `reuse lint` passes in both repositories.

## Non-goals

- No card protocols.
- No application integration.
- No advertising an untested driver as fully supported.
- No narrowing of the library's default driver set.
