# Note: phase 19e bilingual UI, parity, and packaging

Plan: [19e-completion.md](../plan/19e-completion.md). Spec:
[19e-completion.md](../specs/19e-completion.md).

## Implemented

### Language catalog

- `crates/gui/src/i18n.rs` owns `Key`, `Language`, `Translator`, locale
  detection, preference codes, and the display-time translator. Every key has
  an English and Simplified Chinese value; `Key::ALL` drives the completeness,
  placeholder-count, and mapping tests.
- The catalog is used at the display edge. `Localized::set_localized` translates
  a label or button when a view sets it, and `localize_widget_tree` relabels the
  built widget tree (labels, buttons, check buttons, menu buttons, entry
  placeholders, stack page titles, column titles, dropdown items, tooltips,
  window and `AdwWindowTitle` text). Views keep English source strings, so the
  same code path fills both languages.
- Every translatable property records its English source in a thread-local
  weak registry (no `unsafe` and no GObject qdata). A later switch projects
  from that source, so ambiguous Chinese values such as `Card`/`CARD` or
  `Block`/`data block` restore the exact original copy. The language itself is
  a UI-thread local value, which also keeps parallel tests independent.
- Translation handles exact lines, formatted patterns with `{}` placeholders,
  captured tokens (for example `Sector 01 · data block · read`), and multi-line
  labels. Unknown strings pass through, which keeps protocol names, hex, UIDs,
  paths, counts, service diagnostics, and the activity log unlocalized.
- Settings gained a language selector. Selecting a language calls
  `switch_language`: it updates the UI-thread language, relabels the existing
  window, and writes `ui.json` atomically. Page, reader, card image, table
  selection, and open form values are widget state and are preserved. The saved
  choice wins; otherwise `LC_ALL`, `LC_MESSAGES`, and `LANG` are checked, then
  English. `nfctl` never consults the language.
- The page switcher is wrapped in a horizontal scroller with compact tab
  padding so all eleven localized tabs fit at the default width; it remains
  scrollable on narrower windows.

### Parity

- [docs/design/parity.md](../design/parity.md) lists every non-test `nfctl`
  operation with its CLI command, GUI control, confirmation/Advanced gate, and
  hardware claim.
- The audit found one missing file operation: exporting the raw NDEF message
  that `nfctl classic ndef read --output` and `nfctl ndef read --output` write.
  The NDEF screen now has `Export message…`, enabled only when a stored message
  exists, and it writes the same encoded bytes for Classic and Type 2.
- `nfctl scan --watch` remains a CLI presentation loop over the shared `scan`
  read; it is not a separate card or file operation.
- Type 4 still has no CLI command or GUI page.

### UI tests

The gated `gtk_workbench_scenarios` suite now also covers:

- every shell page through the stack, the empty table/inspector state, the
  busy spinner and disabled primary action, and the focus chain;
- a language switch to `zh-CN` that asserts immediate relabeling, `ui.json`
  persistence of `"zh-CN"`, and preservation of the current page, reader,
  card UID, and selected block;
- a Classic read and a Type 2 read executed in Chinese with translated status,
  a Chinese write dialog (heading, body, response labels, and `cancel` close
  response), and no English chrome left in the visible window or dialog;
- switching back to English.

Unit tests cover catalog completeness, placeholder parity, exact/pattern
round-trips, multi-line labels, locale detection, language persistence, and
rejection of an unknown language code.

### Packaging and identity

- `nfc-foundry` sets the default and window icon name to `nfc-foundry`.
- AUR and local-git packages install `nfc-foundry`, `nfctl`, the desktop entry,
  SVG and 128/256/512 PNG icons, the udev rule, hardnested tables, licenses,
  and notices. Runtime dependencies include `gtk4`, `libadwaita`, and
  `hicolor-icon-theme`.
- `.SRCINFO` was regenerated. `namcap` is clean.
- The release workflow stages both binaries, the desktop entry, all icon sizes,
  README, `Cargo.lock`, licenses, and notices in the tarball.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes; the GTK scenario takes its skip branch
  without the gate.
- `env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env
  GDK_BACKEND=x11 cargo test -p nfc-foundry gtk_workbench_scenarios
  -- --nocapture`: passes. The same command with `LANG=zh_CN.UTF-8
  LC_ALL=zh_CN.UTF-8` also passes; the test harness pins English and the
  scenario switches explicitly, so the host locale cannot change the result.
- `reuse lint`: clean.
- `desktop-file-validate resources/nfc-foundry.desktop`: clean.
- `namcap packaging/aur/PKGBUILD`: clean.
- `cargo build --release --locked --workspace`: passes. The staged release tree
  contains both binaries, the desktop file, all four icon files, the udev rule,
  hardnested tables, licenses, notices, README, and `Cargo.lock`; the
  `nfc-foundry-0.1.0-x86_64.tar.gz` layout and `SHA256SUMS` were produced with
  the workflow commands.
- The staged `gtk::IconTheme` resolves `nfc-foundry` to
  `share/icons/hicolor/128x128/apps/nfc-foundry.png` with
  `XDG_DATA_DIRS` pointing at the staged `share/`.
- The staged release `bin/nfc-foundry` launches under Xvfb and renders the
  Chinese window. `gtk-launch nfc-foundry` resolves and starts the same
  executable; the nested test D-Bus session cannot complete GtkApplication
  registration in this environment, so the launch proof uses the resolved
  executable directly.
- Chinese capture on a private Xvfb display with a `zh-CN` `ui.json`: the
  header subtitle, rail, tabs, inspector, empty state, and status bar render
  Chinese; CJK glyphs are present; all eleven tabs fit at the default width.

## Limits

- Keyboard actuation (Tab traversal activation) cannot be observed under a
  bare Xvfb server without a window manager; the test asserts focusability and
  that the window focus chain advances, and Escape is covered through the write
  dialog's `cancel` close response.
- Service diagnostics and the raw activity log stay English by design; they
  are technical records, not interface copy.
- No hardware was touched. Hardware claims remain the phase 02-10 records in
  `docs/hardware-matrix.md`; the GUI uses the same `nfc-app` paths.
- The AUR source package builds tag `v0.1.0`, which predates this phase, so it
  was not the build under test. `packaging/local-git` was built from a
  temporary git snapshot of the current working tree; the resulting package
  contains both binaries, the desktop entry, all four icon sizes, the udev
  rule, hardnested tables, and licenses/notices with the expected runtime and
  make dependencies. `namcap` is clean and `.SRCINFO` was regenerated.
  Publishing the AUR package from this change still requires a commit and tag.

Superseded after this phase: `recover auto` now scans every sector, seeds with
EV1 or host-driven darkside, and read-verifies the result. See
[20-autopwn.md](20-autopwn.md).
