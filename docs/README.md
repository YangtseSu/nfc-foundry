# NFC Foundry documentation

This directory describes the current product. The old NFCX documentation is
preserved under `archive/nfcx-upstream/` and is not authoritative for the new
implementation.

## Current documents

- [Architecture](architecture.md): component boundaries, transport, device
  ownership, and external engines.
- [Cards](cards.md): card-family support matrix, memory models, NDEF, and
  write-safety rules.
- [CLI contract](cli.md): `nfctl` commands, flags, output, and exit codes.
- [Packaging](packaging.md): Arch/AUR layout, dependencies, and release
  obligations.
- [Development workflow](development.md): build, tests, hardware gates, i18n,
  REUSE, and documentation rules.
- [Phased plan](plan/README.md): implementation order and acceptance criteria.

## Language rules

- Repository documentation is English.
- The `nfctl` CLI is English-only.
- The GUI base language is English; Simplified Chinese is a complete
  first-party translation.

## Historical material

- [Archived NFCX documentation](archive/nfcx-upstream/README.md)
- [Archived implementation specs](archive/nfcx-upstream/specs/)
- [Archived implementation notes](archive/nfcx-upstream/notes/)
- [Archived toolchain notes](archive/nfcx-upstream/toolchain/)

Use the archive for hardware findings and protocol history, not for current
product or packaging decisions.
