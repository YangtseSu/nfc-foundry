# Spec: phase 19e completion, bilingual UI, parity, packaging

Status: pending. Plan: [19e-completion.md](../plan/19e-completion.md).

## Language

- English base; complete `zh-CN`.
- Exhaustive Rust catalog: every user-visible key has both values; empty
  values and missing keys are test failures.
- Language switch is immediate, persists in `ui.json`, and preserves page,
  reader, card image, and selection. System locale is used on first run with
  English fallback.
- Protocol names, hex, UIDs, paths, and counts are not translated.
- `nfctl` stays English-only.

## Parity

- `docs/design/parity.md` lists every non-test `nfctl` operation with its GUI
  path, safety gates, and hardware status.
- A CLI-only card/file operation in shipped scope is a defect.
- Type 4 remains unadvertised until a hardware record; a read surface requires
  a matching CLI read path in the same change.
- Shared `nfc-app` semantics and confirmation rules on both surfaces.

## UI tests

- Gated `NFC_FOUNDRY_GUI_TESTS=1` scenario suite under `xvfb-run`.
- Cover every screen, navigation, dialogs, keyboard/focus, Escape, language
  switch, and empty/busy/partial/error/locked/read-only states.
- Normal workspace tests remain hardware-free and headless-safe.

## Packaging

- Binaries: `nfc-foundry`, `nfctl`.
- Desktop file: `share/applications/nfc-foundry.desktop`; validate with
  `desktop-file-validate`.
- Icons: SVG and PNG sizes under `share/icons/hicolor/.../apps/`; GTK window
  icon name `nfc-foundry`; icon lookup resolves.
- Runtime dependencies: gtk4, libadwaita, existing reader/runtime libraries.
- Release tarball and AUR package install the same file set; the GUI launches
  from the installed desktop entry.
- Licenses, notices, and hardnested tables remain installed.

## Final checks

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `NFC_FOUNDRY_GUI_TESTS=1 xvfb-run ...`
- `reuse lint`
- `desktop-file-validate`, icon lookup, packaged file list, AUR/tarball build
- No Windows/macOS artifact.
