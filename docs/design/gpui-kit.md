# GPUI Kit notes

The GUI depends on `gpui-kit` only. Read the GPUI Kit Coding Guides and Design
Guides before changing a screen.

Bootstrap:

```rust
gpui_kit::application()
    .with_assets(gpui_kit::assets::Assets)
    .run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| FoundryApp::default())
        })
        .expect("failed to open window");
    });
```

Rules:

- `Root` is mounted by `open_window`; overlays use `WindowExt`.
- Feature crates own model, views, commands, dialogs, and workflow.
- `RenderOnce` for value-like UI, `Entity<T>` for retained state.
- No business logic or hardware I/O in `render`.
- Theme tokens only; no raw colors in application code.
- Stable domain IDs for repeated elements and tables.
- Test keyboard, focus, overlays, empty/loading/error states.
