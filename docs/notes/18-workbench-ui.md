# Phase 18 note: workbench UI and bilingual UI

Status: implementation complete; hardware unchanged.

## What changed

- Added `crates/gui/ui` (`nfc-ui`) with `Language`, `Translator`, exhaustive
  English and `zh-CN` catalogs, `UiPreferences`, session state, shared
  presentation primitives, and private file/hex helpers.
- Replaced the two-workspace tab shell with the persistent workbench:
  sidebar navigation, reader card/menu, domain switch, card identity, language
  switch, page header, fixed inspector actions, and status bar.
- Rewrote the Classic and Type 2 crates as page-only screens. Added Classic
  Recovery, Diagnostics, Activity, and Settings screens.
- Brought GUI behavior to parity with every non-test `nfctl` operation:
  format selectors, key import, raw/URI/file NDEF writes, restore include
  options, presence watch, and all recovery strategies.
- Added the application icon (SVG plus 128/256/512 PNG), desktop entry, AUR
  install paths, release tarball paths, and a CI desktop-entry validation step.
- Removed the old per-workspace headers, toolbars, tab bars, status lines, and
  their UI tests.

## Localization decisions

- `Translator` is stored in the session entity. Workspaces observe the session
  and re-render on language changes; there is no process-global locale.
- The catalog is a pair of exhaustive matches, so a new key cannot ship
  untranslated. `Key::ALL` drives the parity and empty-value test.
- Status messages carry a localized summary key plus the unlocalized technical
  detail. Error summaries map `NfcError` variants to keys for every operation.
- `nfctl` has no dependency on `nfc-ui`; the CLI test runs help and an error
  under `LANG=LC_ALL=zh_CN.UTF-8` and asserts English output.

## Verification

Run on 2026-11-10:

- `cargo test -p nfc-ui` — 17 passed (catalog parity, real fallback path,
  locale detection, preference round trip and permissions, session state,
  hex and page parsing).
- `cargo test -p nfc-foundry` — 8 shell/UI tests and 3 unit tests passed.
  Covered: Classic read and screen navigation, diff after a second read, key
  add and settings persistence, language switch and `ui.json`, Type 2 read and
  page selection, Advanced page typed dialog and Escape dismissal, trailer
  typed dialog, block 0 refusal, and settings reset.
- `cargo test -p nfctl --test cli help_and_errors_stay_english_under_a_chinese_locale`
  passed.
- `desktop-file-validate resources/nfc-foundry.desktop` passed, including the
  staged path under a temporary prefix.
- A staged install was launched with `gio launch` using
  `XDG_DATA_DIRS=<stage>/usr/share` and `PATH=<stage>/usr/bin` on the headless
  mutter compositor; the GUI process stayed alive.
- GTK's `IconTheme.lookup_icon("nfc-foundry", 128)` resolved to
  `<stage>/usr/share/icons/hicolor/128x128/apps/nfc-foundry.png`.
- The actual binary launched under `LANG=zh_CN.UTF-8` on a private
  `mutter --headless --wayland-display=wayland-nfc` compositor and stayed alive
  without panicking. No window opened on the desktop session.
- `mutter --headless` exposes no screenshot protocol. `flameshot` cannot capture
  that display: its Wayland path uses the operator session portal, and its X11
  path reached the operator X display, so both attempts captured the real
  desktop. Those images were deleted; `flameshot` must not be used for a nested
  check.
- A real render was captured from an isolated `Xvfb :99` (`1440x900x24`) inside
  `dbus-run-session`, with private `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`,
  `XDG_STATE_HOME`, and `XDG_DATA_HOME`. The window was driven with `xdotool`
  and captured with ImageMagick `import -window root`; the image was inspected
  and deleted. `noto-fonts-cjk 20240730-1` supplies the `Noto Sans CJK` family
  GPUI resolves, and the Chinese navigation, headings, buttons, statuses, and
  table cells rendered without tofu. Install `noto-fonts-cjk` on Arch when the
  UI shows tofu boxes.
- The render found and fixed: explicit 1280x800 window bounds, a right shell
  column that did not stretch to the status bar, a duplicate Read action in
  page headers, English block status pills, untranslated block/sector units in
  the card identity, and table layouts that used fixed heights instead of
  filling the page. A mock Classic dump was loaded through the UI to inspect
  populated blocks, the inspector, the localized status bar, and the fixed
  action footer. The screenshot and dump were deleted after inspection.

## Known limits

- The workbench is a light content surface with a dark navigation rail set from
  the theme tokens; the sidebar colors are installed by `install_shell_theme`.
- No hardware operation, reader backend, recovery algorithm, or write path was
  added. The existing hardware records remain the only support claims.
- Screen-capture verification of the rendered window remains a manual desktop
  check because the available Wayland compositor does not expose the capture
  protocol.
