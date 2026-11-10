# Phase 19e plan: completion, bilingual UI, parity, packaging

Status: complete 2026-11-11. Final phase 19 work package after 19b–19d.

Goal: make the GTK workbench complete enough to ship on Linux: English and
Simplified Chinese UI, full CLI/GUI parity audit, broader UI tests, and
packaging with the application identity.

## Bilingual UI

- Port the exhaustive Rust `Key`/`Translator` catalog from the removed GPUI
  `nfc-ui` module into `crates/gui/src/i18n.rs`. English is the base and
  fallback; `zh-CN` is complete.
- Keep protocol names, hex, UIDs, paths, and counts unlocalized.
- Language switch in Settings/header; applies immediately without restart.
  Rebuild labels within the existing window while preserving the current page,
  reader, card image, and selection.
- Persist the choice in `ui.json`; detect the system locale on first run and
  fall back to English.
- `nfctl` remains English-only under any locale.
- Add catalog tests: every key has both languages, no empty values, no missing
  key used by the UI (key enum exhaustiveness), and language persistence.
- Render a Chinese screenshot on a private Xvfb display to check fallback
  glyphs and truncation.

## CLI/GUI parity audit

- Build an explicit matrix in `docs/design/parity.md` for every non-test
  `nfctl` operation:
  - reader list/scan/card info;
  - Classic read/dump/write/restore/keyscan/keys/diff/NDEF/recovery;
  - Type 2 info/read/write/dump/restore/lock/password/NDEF;
  - diagnostics.
- Mark each row GUI path, CLI path, safety gate, and hardware status.
- Add any missing GUI path or remove the CLI-only operation from the shipped
  scope. Type 4 stays gated until a hardware record exists; if a read surface
  is added, add its CLI read counterpart in the same change.
- No card operation may have different semantics or relaxed gates between
  surfaces.

## UI tests

- Extend the gated GTK scenario suite to cover every screen and navigation
  path: Card/NDEF/Keys/Diff/Restore/Recovery/Protection/Diagnostics/Activity/
  Settings, language switch, empty/busy/partial/error/locked/read-only states,
  keyboard focus, Escape dismissal, and confirmation dialogs.
- Run with `NFC_FOUNDRY_GUI_TESTS=1` under `xvfb-run` in CI; normal
  `cargo test --workspace` stays headless-safe.
- Keep pure unit tests for formatting, catalogs, and state helpers.

## Packaging and application identity

- Install both binaries: `nfc-foundry` and `nfctl`.
- Install `resources/icons/nfc-foundry.svg` and PNG sizes under
  `share/icons/hicolor/.../apps/`; set the GTK window icon name to
  `nfc-foundry`.
- Install `resources/nfc-foundry.desktop` under `share/applications/`.
  `desktop-file-validate` and icon lookup must pass.
- Update `packaging/aur/PKGBUILD`:
  - runtime `depends`: gtk4, libadwaita, and existing reader/runtime libs;
  - `makedepends` already includes gtk4/libadwaita;
  - package both binaries, icons, and the desktop file;
  - keep licenses/NOTICE/hardnested tables.
- Update the release workflow to stage the GUI binary, icons, desktop file,
  README, Cargo.lock, licenses, and notices. The tarball must launch the GUI on
  a clean Arch/Ubuntu runtime with GTK/libadwaita installed.
- Update README, packaging docs, and hardware matrix for the GUI.

## Order

1. Port the catalog and language switch; persist and test.
2. Parity matrix; close gaps.
3. Expand UI tests.
4. Packaging, icons/desktop, release staging.
5. Final checks and hardware-matrix note.

## Acceptance

- English and complete `zh-CN` copy; switch is immediate and persisted;
  missing keys fall back to English; CLI stays English.
- Parity matrix has no unexplained CLI-only card/file operation.
- Gated GTK tests pass under Xvfb; workspace tests/clippy/fmt/REUSE pass.
- `desktop-file-validate` passes; icon lookup resolves; `Exec` launches the
  GUI; packaged binary list contains both binaries, icons, and desktop file.
- No Windows/macOS artifact is added.
- Release tarball and AUR package build locally.

## Non-goals

- Windows/macOS.
- New card families, protocols, recovery algorithms, or write paths.
- A second frontend or web UI.
- Phase 13–17 RF/read-only adapters.
