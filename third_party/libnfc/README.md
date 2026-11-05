# Pinned libnfc-ng source

NFC Foundry builds the standalone `libnfc-ng` fork at tag `v1.8.0-ng.1`,
commit `c1eb612d8f6032179e6994439802db1fdca1221a`. The git URL, driver
profile, pkg-config identity, and licenses are recorded in `libnfc.lock`.

The fork installs a distinct library identity so it can coexist with Arch's
upstream `libnfc` package:

```text
/usr/lib/libnfcng.so.6
/usr/include/libnfc-ng/nfc/
/usr/lib/pkgconfig/libnfc-ng.pc
```

The C shim and the external engines build with `pkg-config libnfc-ng`; they
never link `libnfc.so.6`. The fork keeps the upstream C API and exported
symbol names, and the library stays LGPL-3.0-or-later.

The `LIBNFC_DEVICE`/conffiles fix and the platform build fixes are ordinary
commits in the fork, not a patch series applied here. The fork's `COPYING`
is copied into the generated runtime, and its commit is recorded in
`source.lock`.

`nfc-mfsetuid-BSD-2-Clause.txt` preserves the notice for the Gen1A sequence
that is being ported to Go. No upstream `nfc-*` executable is shipped.
