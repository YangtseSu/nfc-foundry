# Package slimming note

Date: 2026-11-10

## Problem

The first `nfc-foundry 0.1.0-1` Arch package contained 2590 entries and 55.03
MiB installed:

- 1406 dependency license files, 11 MiB;
- 351 hardnested table files, 9 MiB;
- development docs under `/usr/share/doc`;
- two binaries, 40.3 MiB.

The license tree duplicated every crate's license text, and development
documentation was installed as runtime payload.

## Change

- The package installs only NFC Foundry's own `LICENSE`, GPL text, `NOTICE`, and
  `THIRD_PARTY_NOTICES.md`. Dependency license files and the dependency
  manifest were removed.
- Development docs and ADRs are no longer installed. They remain in the source
  tree and source archive.
- Hardnested data keeps the Proxmark3 directory layout: 351
  `bitflip_*_states.bin.lz4` files under
  `/usr/share/nfc-foundry/hardnested_tables/`. Proxmark3 itself installs the
  same table set as individual files under
  `/usr/share/proxmark3/resources/hardnested_tables/`; no pack or archive
  format is introduced. (mfoc-hardnested instead embeds the tables as C arrays
  in its binary, but that is a different design.)
- An earlier custom pack format and loader were removed. The loader is the
  upstream-style directory loader.

## Measurements

| Item | Before | After |
|---|---:|---:|
| Package entries | 2590 | 369 |
| Installed size | 55.03 MiB | 47.97 MiB |
| Package size | 14.64 MiB | 14.48 MiB |
| Dependency license files | 1406 | 0 |
| Dependency manifest | — | 0 |
| Installed development docs | 16 | 0 |
| Hardnested data files | 351 | 351 (upstream layout) |
| Binaries | 40.3 MiB | 40.3 MiB |

The binaries dominate the installed size. The packaging payload is the
hardnested table set plus four license/notice files. This matches common Arch
Rust packages such as `ripgrep` and `bat`, which install only their own license
files, and Proxmark3's table layout.

## Verification

- `cargo test --workspace`: 383 passed, 0 failed.
- `cargo fmt --all --check`, clippy with `-D warnings`, and `reuse lint` pass.
- `makepkg --printsrcinfo` still matches `.SRCINFO`.
- The rebuilt package is extracted; `nfctl diag` resolves the installed table
  directory.
- No dependency license files, dependency manifest, or development docs are
  present in the package.

The local package was built with `makepkg --nocheck`; the workspace suite was
run separately. `check()` remains in the PKGBUILD.

## Notes

- The source archive (GitHub-generated) still contains
  `resources/hardnested_tables/`.
- The 0.1.0 release tag and GitHub assets predate this packaging change. They
  must be rebuilt and re-tagged before they are represented as the slimmed
  package.
