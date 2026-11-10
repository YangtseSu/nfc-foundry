# Phase 18 spec: workbench UI redesign and bilingual UI

Status: superseded. The GPUI Kit shell was removed on `feat/gtk4-ui`; GTK4
screen contracts will replace it.

## Goal

Replace the per-workspace shells with one persistent workbench described by
`docs/design/03-workbench.html`, provide English-base and complete Simplified
Chinese UI text, and ship an application icon and desktop entry. `nfctl` stays
English-only. The phase adds no card operation, reader backend, recovery
algorithm, or write path.

## Architecture

```text
crates/gui/ui              nfc-ui
  i18n.rs                  Language, Key, Translator, catalogs
  preferences.rs           ui.json, locale detection, atomic 0600 write
  session.rs               status, busy, cards, readers, activity, recovery
  components.rs            header, card, inspector, metric, pill, callout, bytes
  util.rs                  hex/page parsing and private file writes

crates/gui/nfc-foundry     shell and system screens
  AppShell                 navigation, reader menu, language, settings, activity
  Diagnostics              static facts and session paths
  Activity                 session-only operation log

crates/gui/classic         Classic screens only
  Blocks / Classic NDEF / Diff / Keys / Recovery

crates/gui/type2           Type 2 screens only
  Memory / NDEF / Protection
```

The shell owns reader selection, card identity, language, status, and activity.
The workspace crates render page content and call `nfc-app` services. No
workspace performs reader I/O in `render`.

## Localization

- `Language::En` is the base and fallback; `Language::ZhCn` is the second
  locale.
- `Translator` is a value owned by the session entity, not a process global.
  Tests select a language per GPUI context.
- Resolution order: saved preference, `LC_ALL`, `LC_MESSAGES`, `LANG`, English.
  A `zh` primary tag selects `zh-CN`.
- The preference file is `$XDG_CONFIG_HOME/nfc-foundry/ui.json`, falling back
  to `~/.config/nfc-foundry/ui.json`, written atomically with `0600`.
- UIDs, hex, paths, counts, and protocol names stay unlocalized. `NfcError`
  maps to a localized summary and keeps its English detail.
- The catalogs are exhaustive Rust matches; the parity test rejects empty
  values and the fallback path is exercised for an empty translation.

## Language and persistence

`UiPreferences` stores language, key-store path, backup directory, hardnested
table directory, default dump format, and the common-key choice. The CLI never
reads this file. Switching language updates the session immediately and writes
the preference when a path exists.

## GUI/CLI parity

| CLI | GUI |
|---|---|
| `list`, `scan`, `scan --watch`, `card info` | reader menu, card header, presence watch |
| `classic read/dump/write/restore` | Blocks table, inspector, file row, restore |
| `classic keyscan/keys list|import|add` | Keys screen, scan, import, add, remove |
| `classic diff` | Diff screen (current image or two files) |
| `classic ndef info|read|write|format` | Classic NDEF screen |
| `type2 info|read|write|dump|restore` | Memory screen, page inspector |
| `type2 lock|password` | Protection screen |
| `ndef read|write|format` | Type 2 NDEF screen |
| `recover auto|darkside|nested|static-nested|hard-nested|fm11rf08s` | Recovery screen |
| `diag` | Diagnostics |
| `--key`, `--key-file`, `--key-store`, `--no-common-keys`, `--backup-dir`, `--hardnested-tables`, `--max-nonces`, `--device` | Keys, Settings, Recovery, reader menu |
| `--mock` | test harness only |

## Safety gates

- Block 0 writes stay disabled.
- Locked pages and manufacturer pages stay disabled.
- A read-only capability container blocks NDEF writes and formats.
- Trailer writes, Classic NDEF format, Type 2 Advanced pages, locks, password
  configuration, restores, and Type 2 format require the typed dialog token
  `I_OWN_THIS_CARD`.
- Every mutation keeps the service-level backup, read-back, and card-identity
  checks unchanged.
- Recovery authenticates only and exposes cancel plus the verified result.

## Application identity

- `resources/icons/nfc-foundry.svg` with 128, 256, and 512 px PNG exports.
- `resources/nfc-foundry.desktop` with English and `zh_CN` metadata,
  `Exec=nfc-foundry`, `Icon=nfc-foundry`, and `Terminal=false`.
- AUR and release tarball install:
  `/usr/share/applications/nfc-foundry.desktop`,
  `/usr/share/icons/hicolor/scalable/apps/nfc-foundry.svg`, and
  `128x128`, `256x256`, `512x512` PNGs.

## Verification

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `desktop-file-validate resources/nfc-foundry.desktop`
- No hardware tests are added or rerun by this phase.
