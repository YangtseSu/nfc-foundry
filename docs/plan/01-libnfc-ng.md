# Phase 01: libnfc-ng as a pure library

## Goal

Ship `libnfc-ng` as a standalone Linux library package with no command-line
utilities.

## Work

- Build `libnfcng.so.6`, public headers, and `libnfc-ng.pc`.
- Disable upstream `utils/` and `examples/` install targets.
- Keep upstream Windows/macOS source; build/test Linux only.
- Enable `pn532_uart`; add USB/PC-SC drivers only with validation.
- Keep patches minimal and documented in the fork.
- Install no `nfc-*` binary.
- Add an AUR `libnfc-ng` package.
- Record the fork commit/tag used by the application.

## Acceptance

- `pkg-config --cflags --libs libnfc-ng` works.
- A C smoke test can initialize, enumerate, open, select, and close.
- `pacman -Ql libnfc-ng` contains no `/usr/bin/nfc-*`.
- The library coexists with Arch's `libnfc`.
- `reuse lint` passes in both repositories.

## Non-goals

- No card protocols.
- No application integration.
- No untested drivers.
