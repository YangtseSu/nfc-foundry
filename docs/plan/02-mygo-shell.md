# Phase 02: MyGo shell

## Goal

Replace Wails with the MyGo native UI without changing the application
services.

## Work

- Remove Wails runtime, dialogs, desktop shell, and build dependency.
- Keep the existing Go application services and DTOs behind a UI-agnostic
  boundary.
- Implement MyGo window, navigation, dialogs, events, and task log.
- Build the first screens: device list, card facts, Classic block view, and key
  catalog.
- Add English and Simplified Chinese catalogues; English is the base language.
- Update build, CI, and packaging scripts for MyGo.
- Remove the HTML/TypeScript frontend if native UI replaces it.

## Acceptance

- The GUI starts, enumerates a reader, shows card info, and reads a Classic
  block.
- Language switch works; both catalogues are complete.
- `go test ./...` passes without Wails.
- No Wails import remains.
- A MyGo UI smoke test runs in CI.

## Non-goals

- No new card families.
- No CLI work.
- No Wails feature parity beyond the initial workflow.
