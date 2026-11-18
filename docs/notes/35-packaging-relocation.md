# Note: packaging relocation to the AUR and arch-pkgs

Date: 2026-11-18.

## Implemented

- Published `nfc-foundry` 0.3.0-1 to the AUR:
  <https://aur.archlinux.org/packages/nfc-foundry>.
- Added `nfc-foundry-git` to
  [arch-pkgs](https://github.com/YangtseSu/arch-pkgs). It builds the tip of
  `main`, uses the same build, check, and install functions as the AUR package,
  and provides/conflicts with `nfc-foundry`.
- Removed `packaging/aur/` and `packaging/local-git/` from this repository.
  The repository no longer carries a PKGBUILD; the AUR repository owns the
  release package and arch-pkgs owns the development package.
- Updated `README.md`, `docs/packaging.md`, the phase 12 plan status, and the
  phase 12 spec with the new locations. The phase 12 spec keeps its original
  implementation record and points to the AUR repository.
- Added `glib2` as a direct runtime dependency. `nfc-foundry` links
  `libglib-2.0.so.0`, `libgio-2.0.so.0`, and `libgobject-2.0.so.0` directly;
  `gtk4` pulled it in transitively before.

## Verification

- `makepkg -f` in the AUR checkout built `nfc-foundry 0.3.0-1`; the workspace
  tests in `check()` passed.
- `namcap nfc-foundry-0.3.0-1-x86_64.pkg.tar.zst` reports only
  `Unused shared library '/usr/lib64/ld-linux-x86-64.so.2'` for both binaries.
  The loader entry is normal for a Rust ELF. The `glib2` dependency is
  declared, so the implicit-satisfaction warning is gone.
- `pacman -Qlp` lists only the documented paths: both binaries, the udev rule,
  the desktop entry, four icon sizes, the hardnested tables, and the project's
  license notices. No `/usr/local`, `/opt`, `/bin`, or `/usr/libexec` path is
  present.
- Extracted `nfctl --version` reports `nfctl 0.3.0`; `nfctl diag` reports the
  release commit, the three driver prefixes, and the packaged hardnested table
  directory.
- The rebuilt binaries contain no `$srcdir` or build-directory path.
- `makepkg --printsrcinfo` matches the committed `.SRCINFO`.
- The AUR push created `refs/heads/master`; the package page returns HTTP 200.
- `makepkg -f` in arch-pkgs built `nfc-foundry-git 0.3.0.r0.gbf570b0-1`; the
  same workspace tests passed. `namcap` reports only the two loader warnings.
  The package declares `provides=nfc-foundry` and `conflicts=nfc-foundry`.
- The `nfc-foundry-git` file list matches the AUR package except for the
  license directory, which uses `/usr/share/licenses/nfc-foundry-git/`.
- Extracted `nfctl --version` from both packages reports `nfctl 0.3.0`.

## Limits

- No hardware test ran. The package installs the same binaries as the release
  tarball, and the hardware matrix is unchanged.
- The package was not installed system-wide during this check; the extracted
  package was exercised directly.
