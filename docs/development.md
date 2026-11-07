# Development workflow

## Prerequisites

- Arch Linux (the only supported build host).
- Go 1.27.3 or newer.
- MyGo's native UI build/runtime requirements as reported by `mygo doctor`.
- A C toolchain, `pkgconf`, `make`, `autoconf`, `automake`, and `libtool`.
- `libusb`/`pcsclite` development files for the transports being built.
- `reuse` for license compliance.
- A PN532 UART reader and/or ACR122U, plus authorized test cards, for hardware
  work. PN53x USB is compiled but has no recorded validation hardware.

## Commands

```sh
make libnfc-ng-build
make libnfc-ng-check
make test
make build
make check
make libnfc-ng-hardware-smoke LIBNFC_DEVICE=pn532_uart:/dev/ttyUSB0
make libnfc-binding-hardware-smoke LIBNFC_DEVICE=acr122_usb:001:004
make libnfc-type2-hardware-test LIBNFC_DEVICE=acr122_usb:001:004
make libnfc-type2-protected-hardware-test LIBNFC_DEVICE=acr122_usb:001:005 \
  NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM=I_OWN_THIS_DISPOSABLE_TAG \
  NFCX_TEST_TYPE2_BACKUP_DIR=<directory> \
  NFCX_HARDWARE_TYPE2_PROTECT_AUTH0=4
```

- `libnfc-ng-build`: build/install the fork into the development prefix.
- `libnfc-ng-check`: toolchain, SDK, and runtime checks.
- `test`: hardware-free Go tests.
- `ui-test`: MyGo native view tests without a window.
- `build`: native application plus the private runtime directory.
- `check`: `go test`, `go vet`, catalogue validation, UI tests, and `reuse
  lint`.
- `libnfc-ng-hardware-smoke`: explicit raw-capable PN53x enumeration; never
  part of ordinary CI.
- `libnfc-binding-hardware-smoke`: open/close lifecycle and repeated card
  selection on the driver named by `LIBNFC_DEVICE`. The accepted drivers are
  `pn532_uart`, `pn53x_usb`, `acr122_usb`, and `acr122_pcsc`.
- `libnfc-type2-hardware-test`: read-only Type 2 validation by default; set
  `NFCX_HARDWARE_TYPE2_CONFIRM=I_OWN_THIS_TAG` and
  `NFCX_TEST_TYPE2_BACKUP_DIR=<directory>` to exercise NDEF write, restore,
  and read-back. `NFCX_TEST_TYPE2_PWD` provides an optional four-byte password.
- `libnfc-type2-protected-hardware-test`: destructive, one-way protection
  validation on a disposable tag. It requires
  `NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM=I_OWN_THIS_DISPOSABLE_TAG`,
  `NFCX_TEST_TYPE2_BACKUP_DIR=<directory>`, and at least one of
  `NFCX_HARDWARE_TYPE2_PROTECT_AUTH0`, `..._ACCESS`, `..._STATIC_LOCK`, or
  `..._DYNAMIC_LOCK`. It leaves the tag in the tested state.

## Current baseline and migration

The application now uses the MyGo native UI and the `libnfc-ng` fork. The
remaining migration is:

1. archive old docs (done);
2. make `libnfc-ng` a pure library (done);
3. replace Wails with MyGo (done);
4. expose `nfctl` (done);
5. port Gen1A to Go (done);
6. add Type 2/NDEF (done);
7. expand validated readers and add the APDU foundation (reader capability
   matrix and profile in place; PN53x USB hardware and the PC/SC-native APDU
   transport remain open).

## Testing

Required checks:

- `go test ./...` without the `libnfc` build tag;
- `make libnfc-binding-test`, which runs `go test -tags libnfc ./...` against
  the development SDK;
- `make libnfc-ng-verify`, which checks that the SDK was built with the
  application driver profile;
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
