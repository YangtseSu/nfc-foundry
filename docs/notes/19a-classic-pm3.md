# Note: phase 19a Classic completion and Proxmark3 interchange

Plan: [19a-classic-pm3.md](../plan/19a-classic-pm3.md). Spec:
[19a-classic-pm3.md](../specs/19a-classic-pm3.md).

## Implemented

### Dump contract

- `nfc-classic::DumpFormat` adds `Auto`. `resolve` selects raw for a complete
  image and project for a partial one; `can_represent` drives the GUI gates.
- Default names live on `DumpFormat`: `card-dump.bin`,
  `card-dump.mfc.json`, `card-dump.mct`, and `card-dump.nfc-foundry.json`.
- `nfctl classic dump --format` defaults to `auto` and prints the resolved
  format. Explicit `raw`, `proxmark3`, `mct`, and `project` still work.
- The GTK save action presents the five choices, preselects `Auto`, disables
  formats that cannot represent a partial image, and uses the resolved
  default name. Load keeps content detection and reports warnings.
- No generated name, field, or label uses `nfcx`.

### Key interchange

- `KeyStore::export_dic` writes unique uppercase keys in store order, one per
  line. `KeyStore::import_dic_str` accepts blank lines, `#` comments, inline
  comments, lowercase hex, and ignores duplicates; malformed lines fail with
  the line number. `import_text_str` still detects JSON first.
- `KeyStore::to_json` provides the internal store as bytes without a path.
- `nfctl classic keys export --format dic|json [--output PATH]` writes to a
  file or standard output.
- The GTK Keys screen lists sector/type/key/source, adds one key, imports
  `.dic`/JSON, saves the JSON store, and exports `.dic`. It states that
  `.dic` is lossy and that imported keys become wildcard `any` entries.
- Reads and writes take a clone of the same `KeyStore`; there is no second
  key source.

### Write identity

- `ClassicBlockWrite`, `ClassicRestore`, `ClassicNdefWrite`, and
  `ClassicNdefFormat` have an optional `expected_card`.
- The shared `check_expected_card` runs immediately after opening the
  operation and before any read used for a backup or write. A mismatch is
  `NfcError::CardChanged`.
- Restore also checks the source dump's identity and now returns
  `CardChanged` instead of `VerificationFailed`.
- GUI block writes and restores pass the loaded image's card. The CLI leaves
  the field unset so a write intentionally targets the current card.

### Screens

- Inspector decodes access groups, all four trailer conditions, trailer
  permissions, Key A/B and GPB, value blocks, manufacturer BCC, and warnings
  for unknown or invalid trailer state.
- Diff, Keys, and Restore are stack pages beside Card and NDEF.
- Diff loads two images without a reader, uses `nfc-classic::diff`, and lists
  changed/unknown blocks with byte and trailer details.
- Restore loads an image, confirms with the destructive dialog, calls
  `ClassicRestore` with a backup and the image identity, and lists every block
  as written, unchanged, unknown, or failed in a result table. It leaves the
  table stale until a fresh read.
- The block write confirmation and the restore confirmation are stored in
  `pending_write_dialog` / `pending_restore_dialog`, which lets the GTK test
  emit `response("cancel")` and verify that no card call happened.

### GTK test entry point

- `Workbench` carries the service, key store, key path, and backup store.
  `build_ui_with` builds the shell from that value; `build_ui` uses
  `Workbench::hardware()`.
- The single `gtk_workbench_scenarios` test drives a scripted Classic 1K
  through `Service::with_transport(MockTransport)`. It covers selection and
  inspector decoding, write gates, trailer confirmation cancel, reader-refresh
  invalidation, data write confirm/cancel, identity refusal, restore
  confirm/cancel and completion, diff classification, and key add/save.
- The test only runs with `NFC_FOUNDRY_GUI_TESTS=1`; normal `cargo test`
  skips it so headless CI without an X server still passes. CI runs it through
  `xvfb-run`.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes; the GTK scenario reports success but takes
  its skip branch unless the gate is set.
- `env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env
  GDK_BACKEND=x11 cargo test -p nfc-foundry gtk_workbench_scenarios`: passes
  (Mesa reports no DRI3; the test does not need rendering).
- `reuse lint`: clean.

## Limits

- Save and load file dialogs are native widgets; the tests cover the format
  option set and file-name helpers rather than opening a portal dialog.
- Diff file loading and key import/export dialogs are not driven by the test;
  their parsing and export helpers are unit-tested in `nfc-classic` and
  `nfc-app`.
- No hardware was touched. The write and restore paths were exercised only
  against the scripted transport.
