# Phase 02 implementation note: MyGo shell

Date: 2026-11-05

> UI status: the first shell implementation was superseded by the reviewed
> workbench mockup in [`docs/design/`](../design/README.md) and is being
> rebuilt to that design. The service, build, and packaging migration recorded
> below remains valid.

## Implemented

- Replaced Wails with MyGo v0.7.3 native UI. The root `main.go` starts the
  MyGo application directly; there is no embedded frontend, `wails.json`, or
  Wails import.
- Added `internal/desktop` with:
  - the MyGo application lifecycle and single-instance lock;
  - a `Backend` interface implemented by `app.Bindings`;
  - an event emitter that applies service events through `Window.Update`;
  - native file dialogs for key import/export;
  - English/Simplified Chinese application menu and runtime language switch.
- Added the initial screens: devices, card facts, Classic block table with
  block detail, key catalog, and activity/task log.
- Added `locales/en.json`, `locales/zh-Hans.json`, and the generated
  `locales/catalog_gen.go` (113 messages).
- Removed telemetry and the auto-updater services, application bindings, and
  the `nfcx-updater` command. User files now live under `NFC Foundry`.
- Replaced application-facing service messages and task-log strings in the
  `app` package with English; GUI labels remain catalogue-driven.
- Switched the CGO shim to `pkg-config libnfc-ng` and the `libnfcng.so.6`
  identity.
- Rewrote the libnfc toolchain script to build the pinned `libnfc-ng` fork
  checkout (or a cached clone), copy the runtime library, and verify the
  identity and driver set.
- Updated the external-engine toolchain scripts to link `libnfc-ng`, and the
  runtime locator to find `/usr/lib/nfc-foundry` for installed packages.
- Replaced the Wails release scripts with Linux-only packaging:
  `scripts/package-runtime.sh` and `scripts/release/package-linux.sh`.
- Added `.github/workflows/ci.yml` with tests, vet, catalogue check, MyGo view
  smoke test, REUSE lint, and a hardware-free build.
- Added `mygo.json` and `resources/icon.png`.

## Verification

- `make check`: `go test ./...`, `go vet ./...`, `go tool mygo i18n check .`,
  the MyGo tester smoke test, and `reuse lint` all pass.
- `go test -race ./internal/desktop ./app` passes.
- `make libnfc-binding-test` (all packages with `-tags libnfc`) passes,
  including a new environment-selected device enumeration test.
- `make libnfc-ng-build` / `verify` / `smoke` pass against tag
  `v1.8.0-ng.1`; the smoke program reports `v1.8.0-ng.1` and 0/1 devices as
  expected.
- `make build` produces `build/nfc-foundry-linux-amd64/nfc-foundry` with the
  `$ORIGIN/runtime/linux-amd64` rpath and bundled `libnfcng.so.6`.
- The packaged binary starts under a headless mutter 51.1 Wayland compositor
  (`mutter --headless --no-x11 --virtual-monitor 1280x800`) and exits cleanly
  (0) on SIGTERM. AT-SPI exposed the real window as `nfc-foundry` with the
  `NFC Foundry` frame, the five navigation items, the Language menu/segmented
  control, and the Devices page.
- With `LIBNFC_DEVICE=pn532_uart:/dev/nfc-foundry-ui-test` and autoscan off,
  the live Devices page rendered the exact connstring, transport, Nested /
  Darkside / Hardnested capabilities, and a Connect button. This is the
  hardware-free reader-enumeration gate.
- `--self-check --json` loads `libnfc-ng v1.8.0-ng.1`. It reports unhealthy
  only because the recovery engines and release manifest are not built in this
  workspace.

## Problems encountered

- `go tool mygo generate` could not run before `go.mod` had a `go.sum` entry
  for the tool and the `locales` package had a Go file. Generation was bootstrapped
  with `go run github.com/egoist/mygo/cmd/mygo@v0.7.3 generate .`, then
  `go mod tidy`.
- Integer placeholders in `keysImported`, `keysExportDone`, and
  `keysImportIssues` needed `@key` metadata; ICU does not infer `int` for a
  bare `{name}`.
- MyGo's build CLI defaults to `CGO_ENABLED=0`; the Makefile builds the GUI
  with `CGO_ENABLED=1` and `-tags libnfc` through plain `go build`, and packages
  the private runtime separately.
- Headless mutter initially reported `gdk_monitor_get_workarea` assertions
  without a monitor. Adding `--virtual-monitor 1280x800` removes them.

## Blockers and follow-up

- No PN532 UART reader or authorized MIFARE Classic card was available, so the
  hardware acceptance in `docs/plan/02-mygo-shell.md` (enumerate, show card,
  read a Classic block) was not executed. The GUI and service paths are ready
  for that manual gate.
- The recovery engines were not built in this workspace, so the packaged app
  warns that `mfoc`, `mfcuk`, and `mfoc-hardnested` are absent. `make package`
  builds them.
- `nfc-mfsetuid` remains referenced by the diagnostic and release metadata
  until Phase 04 replaces Gen1A with native Go. It is not bundled by the new
  runtime package.
- `nfctl` is still Phase 03.

## Deviations from the spec

- None in scope. `make build` uses `go build` rather than `mygo build` so the
  cgo tags, rpath, and private runtime can be controlled; MyGo still owns the
  application lifecycle, views, catalogues, and development command.
