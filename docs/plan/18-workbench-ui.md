# Phase 18 plan: Workbench UI redesign and bilingual UI

Status: planning complete, implementation pending.

Design sample: [`docs/design/03-workbench.html`](../design/03-workbench.html).
The sample is English-only and defines layout and interaction; the
implementation adds the language switch and the complete `zh-CN` catalog.

## Goal

- Replace the current per-workspace shells with the persistent workbench:
  sidebar navigation, reader/card header, page content, inspector, status bar.
- GUI uses English as the base language with a complete Simplified Chinese
  (`zh-CN`) translation and an in-app language switch.
- `nfctl` remains English-only: help, errors, progress, and output never depend
  on the system locale.
- The GUI exposes every non-test `nfctl` operation with the same semantics and
  safety gates.
- Ship an application icon and a desktop entry.
- Add no card operation, reader backend, recovery algorithm, or write path.

## Structure

```text
sidebar
  reader card + change-reader action
  Classic: Blocks · Classic NDEF · Diff · Keys · Recovery
  Type 2: Memory · NDEF · Protection
  System: Diagnostics · Activity · Settings
top bar
  domain switch · card identity · reader menu · Read · language switch
page
  header · screen actions · table/forms · fixed-action inspector
status bar
  operation · detail · cancel · last backup · version
```

- The shell owns navigation, reader selection, card identity, language, and
  activity. Workspace crates render screens only.
- Inspector actions stay fixed at the bottom; they never scroll out of view.
- Empty, busy, partial-read, locked, read-only, error, and disconnected states
  are first-class.
- Settings is a small dialog/drawer, not a card workspace: key-store path,
  backup directory, hardnested tables, default dump format, common keys,
  language.

## CLI parity

Every non-test `nfctl` operation must have a GUI path. A new CLI command needs
the matching GUI row in the same change.

| CLI | GUI |
|---|---|
| `list`, `scan`, `scan --watch`, `card info` | reader menu, card header, status, presence watch |
| `classic read/dump/write/restore` | Blocks table, inspector, format selector, restore options |
| `classic keyscan/keys list|import|add` | Keys screen, A/B/AB scan, import, add/remove |
| `classic diff` | Diff |
| `classic ndef info|read|write|format` | Classic NDEF: mapping, records, raw export, Text/URI/file, read-only format |
| `type2 info|read|write|dump|restore` | Memory/NDEF, page inspector, restore include options |
| `type2 lock|password` | Protection |
| `ndef read|write|format` | Type 2 NDEF: raw export, Text/URI/file, format |
| `recover auto|darkside|nested|static-nested|hard-nested|fm11rf08s` | Recovery: strategy, target, progress, cancel, verified key |
| `diag` | Diagnostics |
| `--key`, `--key-file`, `--key-store`, `--no-common-keys`, `--backup-dir`, `--hardnested-tables`, `--max-nonces`, `--device` | Keys, Settings, Recovery, reader menu |
| `--mock` | test harness only, excluded |

The GUI may present options differently, but it must not hide a capability or
relax a confirmation or backup rule.

## Localization

- `Language::En` is the base and fallback; `Language::ZhCn` is the complete
  second locale.
- New shared presentation crate `crates/gui/ui` owns:
  - `Language`, a context-owned `Translator`, embedded `en`/`zh-CN` catalogs;
  - shared page header, table, inspector, status, dialog, toast, empty state.
- No process-global locale; UI tests select a language per context.
- Key resolution: saved preference, `LC_ALL`, `LC_MESSAGES`, `LANG`, then
  English. A `zh` value selects `zh-CN`.
- Preference file: `$XDG_CONFIG_HOME/nfc-foundry/ui.json` (fallback
  `~/.config`), atomic `0600`. The CLI never reads it.
- Catalog parity test: same keys in both locales, no empty values; missing
  `zh-CN` falls back to `en`.
- Localize UI labels, buttons, dialogs, statuses, and error summaries. Keep
  UIDs, hex, paths, counts, and protocol names unlocalized. Map `NfcError`
  variants to keys; keep raw English detail for Diagnostics.
- Verify CJK font fallback with the actual GPUI window; document the Arch font
  package.
- `nfctl` has no dependency on `crates/gui/ui`; test help and errors under
  `LANG=zh_CN.UTF-8` remain English.

## Icon and desktop entry

Assets:

- `resources/icons/nfc-foundry.svg`: square, no text, readable at 16 px.
- PNG exports: 128, 256, 512 px.
- `resources/nfc-foundry.desktop`: English base plus `zh_CN`
  `GenericName`/`Comment`.

Install:

```text
/usr/share/applications/nfc-foundry.desktop
/usr/share/icons/hicolor/scalable/apps/nfc-foundry.svg
/usr/share/icons/hicolor/{128x128,256x256,512x512}/apps/nfc-foundry.png
```

`Exec=nfc-foundry`, `Icon=nfc-foundry`, `Terminal=false`. Verify
`desktop-file-validate`, icon resolution, `StartupWMClass` against the GPUI
window, and launch from the menu. The AUR package and release workflow install
the same paths.

## Milestones

1. **M0** — `crates/gui/ui` localization foundation, preference, catalog tests,
   CLI English-only test.
2. **M1** — shell, Classic Blocks, Type 2 Memory; remove old per-workspace
   headers/toolbars.
3. **M2** — Classic/Type 2 NDEF, Diff, Keys; dump/restore formats, key import,
   raw NDEF/URI writes.
4. **M3** — Recovery, Diagnostics, Activity; recovery progress and cancel.
5. **M4** — Protection, Settings, typed confirmation/action plan, icon/desktop,
   complete `zh-CN` coverage, keyboard/accessibility.
6. **M5** — remove old UI code/tests/contracts, update docs, run checks and
   package validation.

## Tests and acceptance

- Catalog parity, fallback, locale detection, persistence, plural helper.
- CLI English under Chinese locale; no GUI-crate dependency.
- UI tests for every screen, navigation, language switch, dialogs, empty/busy/
  partial/locked/read-only/error states, keyboard and Escape.
- Safety: block 0 disabled, locked pages disabled, read-only CC blocks writes,
  cancel leaves the card unchanged, backup path shown after mutation.
- `cargo fmt --all --check`, clippy with `-D warnings`, `cargo test --workspace`,
  `cargo check -p nfc-foundry`, `reuse lint`.
- `desktop-file-validate`, icon lookup, `Exec` launch, and package file list.
- No hardware tests in CI; no service/protocol/write-safety changes.

## Non-goals

- HTML/WebView at runtime; GPUI Kit is the only GUI dependency.
- Type 4 advertising before its hardware record.
- CLI localization.
- Mobile/web layout.
- New card family, transport, recovery algorithm, or write operation.
