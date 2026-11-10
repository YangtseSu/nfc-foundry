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

## Reader and layout feedback round

Follow-up feedback after the first workbench run:

- Reader discovery no longer floods the menu with legacy `/dev/ttyS*`
  placeholder nodes. `Pn532Uart::enumerate` skips `ttyS*` entries whose sysfs
  `type` is 0, sorts USB serial adapters first, and the shell keeps the
  auto-selection deterministic. `nfctl list` shares the same order and filter.
- The reader menu is painted with `deferred` so it stays above page content,
  and its list is capped at 360 px with vertical scrolling.
- `callout` now lets its text shrink and wrap (`flex_1` + `min_w_0` on the text,
  non-shrinking icon). The recovery scope note and the Activity safety notes
  wrap inside their 360 px and 320 px panels instead of overflowing.
- The window keeps the compositor's default decoration request and renders the
  GPUI Kit `TitleBar` only when the compositor falls back to client
  decoration. That gives drag, double-click maximize, and
  minimize/maximize/close controls on CSD compositors and avoids a duplicate
  bar under server decorations. The minimum window size is 960x640.
- The shell root is a column with a full-height content row; the main column
  keeps `h_full` inside the `items_center` `h_flex`, so the top bar, page, and
  status bar stay anchored at the top and bottom on resize.

Headless verification on 2026-11-10:

- `cargo test -p nfc-foundry` — 12 UI integration tests passed at 1280x800,
  including reader-menu click-through above page content, wrapped recovery
  scope note, wrapped Activity safety notes, topbar/statusbar anchoring, and
  the existing navigation, dialog, and write-gate flows.
- `./target/debug/nfctl list` on the development host returns only
  `pn532_uart:/dev/ttyUSB0`; before the filter it returned 32 `ttyS*`
  placeholders plus the USB adapter.
- The app was rendered on a private `Xvfb :99` display with
  `WAYLAND_DISPLAY` unset. Captures of Blocks, Keys, Recovery, Activity, and
  the open reader menu confirmed the wrapped callouts, the reader menu above
  page content, and the anchored shell; no desktop window was opened.
- The same binary also ran against a private `gnome-shell --headless` Wayland
  display. That compositor has no server-side decoration, so it exercised the
  client-decoration branch and the in-app title bar without a crash. GNOME's
  headless screenshot API denies capture in GDM mode, so the title bar chrome
  itself was not captured.

## Card-centered shell follow-up (2026-11-10)

Feedback after using the workbench with real cards: the per-domain switch made
the normal flow fail before the user found the right workspace, actions moved
between screens, and the dark rail with a white top bar read as two products.

Changes:

- The shell scans card presence in the background and records the detected
  family. A Classic or Type 2 card replaces the task group in the rail and the
  Read action routes to the matching workspace. The top-bar domain switch and
  the duplicate `Read` button are gone.
- The bottom bar is now a fixed action bar: status, detail, last backup,
  Cancel during recovery, Scan keys (Classic), and the family-aware Read
  action. Errors and progress stay in one place on every screen.
- The rail shows reader state, the card (type, family, UID, blocks/pages), the
  family tasks, and System. The reader menu is the only reader selector and
  closes on outside clicks.
- The workbench palette is the light rail and teal accent from the Go design:
  soft gray canvas, white panels, teal primary actions. All Lucide icons are
  bundled instead of only the default subset, so nav icons render.
- Type 2 empty states no longer reuse MIFARE Classic copy; the NDEF message
  title is family-neutral.
- NDEF format is the first right-column card when the card or tag is not
  NDEF-formatted, so the next action is above the fold.
- Restore controls appear only on Blocks, and Type 2 restore include options
  only after a dump is loaded. Keys and Classic NDEF no longer show a second
  Restore row.
- Duplicate actions are removed: one Compare action on Diff, one Import keys
  path, and Scan keys lives in the action bar instead of the Keys toolbar.
- Block and page writes are disabled while the editor still matches the
  current bytes, and the write button names the target block or page.
- Recovery strategies are a radio list instead of five full-width buttons.

Verification on 2026-11-10:

- `cargo test -p nfc-foundry --test ui` — 12 tests pass, including automatic
  Type 2 routing without a domain click, the trailer typed dialog after an
  edited trailer, the unchanged-write refusal, block 0 refusal, settings,
  language, and reader-menu click-through.
- The GUI was rendered on a private `Xvfb :99` display with the real GNOME
  session untouched. Captures verified the light rail, all nav icons, the
  bottom action bar, the teal primary action, the reader menu anchored under
  its trigger and dismissed by an outside click, the NDEF format card above
  the fold, and the Chinese catalog without missing glyphs.
