# Spec 01: libnfc-ng as a pure library

## Goal

Turn the `libnfc-ng` fork into a standalone, Linux-only NFC library package
with a unique ABI identity:

```text
/usr/lib/libnfcng.so.6
/usr/include/libnfc-ng/nfc/
/usr/lib/pkgconfig/libnfc-ng.pc
```

Upstream utilities and examples stay buildable for diagnostics but are not
installed and are disabled by default. The library keeps the upstream default
driver set; the application package selects its validated profile explicitly.

## Non-goals

- Card protocols, Go bindings, or GUI work.
- Changing the upstream default driver set or removing driver sources.
- Windows/macOS release support.
- Installing any `nfc-*` executable.

## Affected packages and interfaces

- `libnfc-ng` source tree: Autotools and CMake identities, header install path,
  pkg-config metadata, utility/example install rules.
- `nfc-foundry`: packaging documentation and the pinned fork revision.
- `arch-pkgs`: new `libnfc-ng-git` package.

The C API and exported symbols do not change. Only the library filename,
pkg-config name, and header prefix change. Consumers must build with
`pkg-config --cflags libnfc-ng`; direct `-I/usr/include` use is unsupported.

## Capabilities and safety

No card or reader capability is added by this phase. The Arch profile compiles
`pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc`; Phase 06 owns the
per-driver capability matrix and the hardware-validation claims. The build
disables system configuration files and keeps environment-variable device
selection, so `LIBNFC_DEVICE` must work without `/etc/nfc`.

The required `LIBNFC_DEVICE` fix removes the `CONFFILES` guard around
`user_defined_devices` in `nfc_list_devices`.

## Test plan

Hardware-free:

- `./configure` with no driver option reports the upstream default set.
- `./configure --with-drivers=all` reports every available driver.
- `./configure --with-drivers=pn532_uart` is accepted.
- `make install DESTDIR=...` with `--disable-static` installs only
  `libnfcng.so.6*`, the headers under `usr/include/libnfc-ng/nfc`, and
  `libnfc-ng.pc`; the Arch package drops the libtool `.la` file.
- `pkg-config --cflags --libs libnfc-ng` resolves against the staged tree.
- A C smoke test runs `nfc_init`, `nfc_list_devices`, and `nfc_exit`; with
  `LIBNFC_DEVICE` set to a synthetic connstring it reports exactly one device
  even with `--disable-conffiles`.
- The fork and the Arch package pass `reuse lint`.

Hardware: none in this phase. The first real reader test belongs to the
application integration phase.

## Packaging and license impact

`libnfc-ng` remains LGPL-3.0-or-later; the utilities/examples remain
BSD-2-Clause. The fork adds REUSE metadata and license texts. The Arch package
is `libnfc-ng-git` and must not install `libnfc.so.6` or any `nfc-*` binary.

## Rollback

The fork identity changes are ordinary commits; reverting them restores the
upstream `libnfc` target. No data migration or hardware state is involved.
