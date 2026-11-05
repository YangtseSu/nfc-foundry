# Development workflow

## Prerequisites

- Arch Linux (the only supported build host).
- Go 1.27.3 or newer.
- MyGo's native UI build/runtime requirements as reported by `mygo doctor`.
- A C toolchain, `pkgconf`, `make`, `autoconf`, `automake`, and `libtool`.
- `libusb`/`pcsclite` development files for the transports being built.
- `reuse` for license compliance.
- A PN532 UART reader and authorized test cards for hardware work.

The repository is still the NFCX-derived baseline while the migration in
`docs/plan/` is implemented. Until the new build targets land, the archived
NFCX commands are the only working build reference.

## Target commands

The intended developer interface is:

```sh
make libnfc-ng-build
make libnfc-ng-check
make test
make build
make check
make hardware-smoke
```

- `libnfc-ng-build`: build/install the fork into the development prefix.
- `libnfc-ng-check`: ABI, pkg-config, and symbol checks.
- `test`: hardware-free Go tests.
- `build`: MyGo application and `nfctl`.
- `check`: `go vet`/static checks, `reuse lint`, and packaging smoke tests.
- `hardware-smoke`: explicit PN532 UART test; never part of ordinary CI.

## Current baseline and migration

The checked-in application still contains Wails, the HTML/TypeScript frontend,
the old private runtime layout, and the `nfc-mfsetuid` adapter. The migration
steps replace those pieces:

1. archive old docs (done);
2. make `libnfc-ng` a pure library;
3. replace Wails with MyGo;
4. expose `nfctl`;
5. port Gen1A to Go;
6. add Type 2/NDEF;
7. expand validated readers and card families.

Do not add new features to the Wails frontend. Treat it as a temporary baseline
until the MyGo shell lands.

## Testing

Required checks:

- `go test ./...` without the `libnfc` build tag;
- `go test -tags libnfc ./...` with the development SDK;
- `reuse lint`;
- `go vet ./...`;
- CLI command tests with a mock transport;
- replay tests for dump validation and external engine output;
- MyGo screen smoke tests;
- explicit hardware tests.

Use fixed fixtures for dumps and engine output. Never make ordinary tests
depend on a serial port, a real card, or a recovery engine being installed.

## Hardware tests

Hardware tests must:

- be gated by an explicit environment variable or build tag;
- use `LIBNFC_DEVICE` or `--device`, never a hard-coded port;
- refuse to run when the selected card is absent or of the wrong type;
- never write unless a separate write-test flag is set;
- verify UID/ATQA/SAK before and after every write;
- leave the reader and card in a known state.

## i18n

- GUI base language: English.
- GUI translation: Simplified Chinese, kept complete.
- All user-visible GUI strings go through the catalogue.
- CLI output, errors, logs, JSON keys, and docs are English-only.
- Do not put translated strings in card, device, or transport logic.

## REUSE

- `REUSE.toml` is the source of path-based annotations.
- `LICENSES/` contains SPDX-named license texts.
- `reuse lint` must pass before committing.
- New files are covered by `REUSE.toml` or carry SPDX headers.
- Keep `NOTICE` and `THIRD_PARTY_NOTICES.md` current.

## Documentation

- `docs/plan/`: phases, status, acceptance criteria.
- `docs/specs/`: one spec per implementation change.
- `docs/notes/`: implementation records, problems, blockers, and hardware
  findings.
- `docs/archive/`: historical NFCX material; do not update it as product docs.

When a C shim, ABI, packaging, or external-engine interface changes, update the
spec, notes, README, and plan in the same change.

## Commits

- Keep changes scoped to one step or one behavior.
- Review the full staged diff before committing.
- Do not commit secrets, serial ports, card dumps, or user-specific paths.
- Do not push without explicit permission.
