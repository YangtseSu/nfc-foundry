# Phase 00: repository transition

## Goal

Turn the NFCX-derived tree into the NFC Foundry project without changing card
behavior yet.

## Work

- Archive old NFCX docs under `docs/archive/nfcx-upstream/`.
- Replace root `AGENTS.md` and `README.md`.
- Switch the application license to GPL-3.0-or-later while preserving the NFCX
  MIT notice.
- Add REUSE annotations and SPDX license texts; `reuse lint` must pass.
- Create the standalone `libnfc-ng` fork in `~/projects/libnfc-ng`.
- Record the library ABI decision: unique `libnfcng.so.6`, pkg-config
  `libnfc-ng`, headers under `/usr/include/libnfc-ng`.

## Acceptance

- Old docs are discoverable from the archive index.
- `reuse lint` passes.
- The fork exists, builds on Linux, and is documented as a separate project.
- No old README/AGENTS remains as active product documentation.

## Non-goals

- No GUI rewrite.
- No card-family changes.
- No packaging changes beyond documenting the target.
