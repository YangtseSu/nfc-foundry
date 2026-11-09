# ADR 0005: GPUI Kit is the GUI stack

- Status: accepted
- Date: 2026-11-09

## Context

The rewrite is all Rust and needs a native desktop shell with keyboard
navigation, focus management, overlays, tables, and a component system.

## Decision

Use GPUI Kit as the only GUI dependency:

- `gpui_kit::*` for GPUI.
- `gpui_kit::component` for styled components.
- `gpui_kit::base` for behavior primitives.
- `gpui_kit::assets` for default icons.
- `gpui_kit::init` and `open_window` for bootstrap.

## Consequences

- The old MyGo UI and its component conventions are not migrated.
- Feature crates own model, views, commands, dialogs, and workflow together.
- `RenderOnce`/`Entity<T>` boundaries follow the GPUI Kit Coding Guides.
- UI tests use the GPUI Kit test facility, including real focus, keyboard, and
  overlay behavior.
- Theme tokens and interface-language rules follow the GPUI Kit Design Guides.
