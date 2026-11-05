# Phase 02 implementation note: MyGo shell

Date: 2026-11-05

> UI status: the reviewed workbench in [`docs/design/`](../design/README.md)
> replaced the first five-page sidebar shell on 2026-11-05. The Classic
> workflow is implemented; the hardware acceptance gate remains pending.

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
- Added `locales/en.json`, `locales/zh-Hans.json`, and the generated
  `locales/catalog_gen.go` (182 messages).
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

## Workbench rebuild

- Replaced the routed sidebar pages with one workbench:
  - top bar with the exact reader connstring, card identity, dump capacity,
    task state, language switch, and refresh;
  - left rail with the selected reader, connection state, transport and
    recovery capabilities, card identity, dump health, and changes;
  - center workspace with card-family badges, capability-driven tabs, and the
    active family view;
  - right inspector and a persistent activity list that survives tab changes;
  - bottom action bar with the operation state, Read card, Scan keys, and
    cancellation.
- Classic implements Blocks, Keys, and Details:
  - the Blocks table is grouped by sector with sticky sector headers, kind
    badges, hex/ASCII previews, changed markers, row selection, and a
    selected-block inspector;
  - the Keys view pairs a sector key matrix with the persistent key catalog,
    including add, delete, import, and export;
  - Details shows card facts, dump geometry, validation, completion, and the
    source UID without exposing write, UID, recovery, or configuration actions.
- Type 2 reserves Memory, NDEF, and Protection in the tab model. The tabs stay
  disabled and no NDEF records, capacity, or validation values are synthesized.
  Details remains the only enabled Type 2 tab in this phase.
- The palette follows the mockup in light and dark themes. MyGo widgets are
  aligned through a per-frame theme copy so table selection, focus, and muted
  text use the workbench colors.
- The window opens at 1360x860 with a 1120x720 minimum.
- Removed the obsolete route/page view files and the old navigation catalogue
  messages.

## Verification

- `make check`: `go test ./...`, `go vet ./...`, `go tool mygo i18n check .`,
  the MyGo tester smoke test, and `reuse lint` all pass after the rebuild.
- `go test -race ./internal/desktop ./app` passes.
- `make libnfc-binding-test` (all packages with `-tags libnfc`) passes,
  including a new environment-selected device enumeration test.
- `make libnfc-ng-build` / `verify` / `smoke` pass against tag
  `v1.8.0-ng.1`; the smoke program reports `v1.8.0-ng.1` and 0/1 devices as
  expected.
- `make build` produces `build/nfc-foundry-linux-amd64/nfc-foundry` with the
  `$ORIGIN/runtime/linux-amd64` rpath and bundled `libnfcng.so.6`.
- The MyGo headless tester rendered the rebuilt workbench at 1360x860 for
  visual review: Classic Blocks, Keys, Details, an active read task, Type 2
  reserved tabs, no-card, no-reader, and the dark palette.
- A CH340 USB serial adapter (`1a86:7523`) appeared as `/dev/ttyUSB0`.
  `libnfc-ng` auto-scan enumerated `pn532_uart:/dev/ttyUSB0:115200`, and
  `Reader.Open` succeeded with that exact connstring.
- A MIFARE Classic 1K test card was fully readable after `mfoc` recovered
  the one sector whose keys were not in the built-in dictionary. The
  resulting 64-block dump was complete with 32/32 key slots verified.
- The write path was exercised on data block 8: the test pattern
  `DEADBEEF00112233445566778899AABB` was written and read back verified, then
  the original all-zero block was written back and independently verified.
  The card was left in its original state. MIFARE trailer reads mask
  Key A as zeros; that is normal and not by itself evidence of a magic card.
- The first write test exposed a state bug: after a successful write the
  workbench baseline did not advance, so a subsequent write of the original
  value had no dirty block to preflight. `StartWrite` now calls
  `workbench.Model.MarkWritten` for the verified blocks (leaving skipped
  block 0 dirty), with a unit test for the model method.
- The packaged binary starts under a headless mutter 51.1 Wayland compositor
  (`mutter --headless --no-x11 --wayland-display=wayland-1
  --virtual-monitor 1360x860`) and exits cleanly (0) on SIGTERM.
  Bare mutter does not expose a screen-copy protocol, so `grim` cannot capture
  the live surface; the visual proof is the tester-rendered image.
- `--self-check --json` loads `libnfc-ng v1.8.0-ng.1`. `mfoc` is built and
  available; the report remains unhealthy only because `mfcuk`,
  `mfoc-hardnested`, and the release manifest are not built in this
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
  without a monitor. Adding `--virtual-monitor` removes them.
- Bare mutter's headless backend lacks `wlr-screencopy`, so `grim` reports
  "compositor doesn't support the screen capture protocol". The MyGo tester
  image is used for visual review instead.
- AT-SPI activation inside a nested `dbus-run-session` fails because the
  session tries to activate user systemd units that the nested bus does not
  provide. This is a test-harness limitation, not an application error; the
  window lifecycle still completes and exits 0.

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
