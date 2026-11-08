# Phase 01 implementation note: libnfc-ng pure library

Date: 2026-11-05

## Implemented

- Applied the `LIBNFC_DEVICE` fix directly in `libnfc/nfc.c`: the
  `user_defined_devices` loop in `nfc_list_devices` is no longer guarded by
  `CONFFILES`, so a build with `--disable-conffiles` still lists devices
  selected through environment variables.
- Renamed the Autotools and CMake library target to `libnfcng`, keeping
  `-version-info 6:0:0` and therefore `libnfcng.so.6`.
- Installed public headers under `/usr/include/libnfc-ng/nfc/`.
- Replaced `libnfc.pc` with `libnfc-ng.pc` (`Libs: -lnfcng`,
  `Cflags: -I${includedir}/libnfc-ng`).
- Kept `m4/libnfc_drivers.m4` untouched. The default driver set and
  `--with-drivers=all` still work.
- Made upstream utilities and examples diagnostic-only: disabled by default,
  built as `noinst_PROGRAMS`, man pages moved to `EXTRA_DIST`, and CMake
  install rules removed. `--enable-example` implicitly enables the utility
  helpers because the examples link `libnfcutils.la`.
- Added `REUSE.toml`, `LICENSES/LGPL-3.0-or-later.txt`,
  `LICENSES/BSD-2-Clause.txt`, `LICENSES/BSD-3-Clause.txt` and
  `LICENSES/BSD-4-Clause.txt`.
- Added the Arch package `libnfc-ng-git` under `~/projects/arch-pkgs`
  (package version `1.8.0.ng.1.r0.gc1eb612-1`).
- Recorded the fork baseline as tag `v1.8.0-ng.1`, commit `c1eb612`.

## Verification

- `./configure --prefix=/usr --sysconfdir=/etc --disable-conffiles
  --enable-envvars` reported the upstream default driver set
  (`acr122_usb`, `acr122s`, `arygon`, `pn53x_usb`, `pn532_uart`,
  `pn532_spi`, `pn532_i2c`; `pcsc` and `acr122_pcsc` off, `pn71xx`
  unavailable). `make` produced `libnfcng.so.6.0.0` with SONAME
  `libnfcng.so.6`; no utility or example binary was built.
- `--with-drivers=all` enabled every available driver. `pn71xx` stayed off
  because `linux_nfc_api.h` is not installed on this host, matching upstream
  availability detection.
- `--with-drivers=pn532_uart` enabled exactly `DRIVER_PN532_UART_ENABLED`.
- `--enable-example` built `utils/nfc-list`, `examples/nfc-poll` and
  `examples/pn53x-tamashell`; `make install DESTDIR=...` installed none of
  them.
- Staged install contained `libnfcng.so.6*`, the three headers under
  `usr/include/libnfc-ng/nfc`, and `usr/lib/pkgconfig/libnfc-ng.pc`.
  `pkg-config --cflags --libs libnfc-ng` returned
  `-I.../include/libnfc-ng -L.../lib -lnfcng` plus the libusb require.
- The C smoke test reported `devices=0` with autoscan disabled and no device,
  and `devices=1` with a synthetic `LIBNFC_DEVICE` connstring. The second
  result proves the conffiles fix.
- CMake configured and built the same identity; its install staged
  `libnfcng.so.6`, the headers under `libnfc-ng/nfc`, `libnfc-ng.pc` and the
  CMake package files.
- `makepkg` built `libnfc-ng-git` from a local clone and ran `check()`
  successfully. `pacman -Qlp` showed no `/usr/bin/nfc-*`, no `libnfc.so.6`,
  no static archive and no `.la` file. The package links
  `libpcsclite.so.1`, `libusb-0.1.so.4` and `libc.so.6`.
- `namcap PKGBUILD` and `namcap` on the package reported no error or warning.
- `reuse lint` passes in `libnfc-ng` and `arch-pkgs`.
- Rebuilt `libnfc-ng-git` after the hardware tests from a temporary local `git+file://` source override, because the
  canonical private GitHub URL cannot be fetched without credentials. The
  package is
  `libnfc-ng-git-1.8.0.ng.1.r0.gc1eb612-1-x86_64.pkg.tar.zst` under
  `~/projects/arch-pkgs/libnfc-ng-git/`. `.SRCINFO` was generated locally but
  is ignored by that repository's whitelist `.gitignore`; it is not an AUR
  publication.

## Problems encountered

- Out-of-tree configure refused to run until the in-tree build was
  `make distclean`ed. The profile matrix was then built in three independent
  build directories.
- The examples link `utils/libnfcutils.la`; enabling examples alone would
  have failed. Configure now enables the utility helpers implicitly and
  reports it.
- The default libtool install includes a static archive and a `.la` file. The
  package uses `--disable-static` and `options=('!libtool')`; the staged
  library install is still tested with `--disable-static`.

## Blockers and follow-up

- The fork branch and tag `v1.8.0-ng.1` are pushed to GitHub. The repository
  is currently private, so the canonical PKGBUILD source
  (`git+https://github.com/YangtseSu/libnfc-ng.git`) cannot be cloned
  anonymously. The package was verified from `git+file://` against the same
  commit; a public build and clean-chroot `pkgctl build` remain blocked until
  the repository is made public or a public source mirror is provided.
- Decision on 2026-11-05: keep `libnfc-ng` private and do not publish the
  package to the AUR yet. The PKGBUILD remains local-only until a public
  source location is chosen.
- Resolved on 2026-11-08: `libnfc-ng` and `nfc-foundry` are public, so the
  canonical PKGBUILD source is anonymously cloneable. AUR publication remains
  a separate release step.
- No hardware test was performed; Phase 01 has no reader/card acceptance
  requirement. The first hardware gate is the application integration in
  Phase 02.
- The plan names an AUR `libnfc-ng` source package; this change provides the
  requested `libnfc-ng-git` package. The stable package remains for the
  release phase.
- The application lock still points at upstream libnfc 1.8.0; Phase 02
  switches the toolchain and C shim to the fork.
