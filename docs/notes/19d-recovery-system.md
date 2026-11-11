# Note: phase 19d recovery and system surfaces

Plan: [19d-recovery-system.md](../plan/19d-recovery-system.md). Spec:
[19d-recovery-system.md](../specs/19d-recovery-system.md).

## Implemented

### Service

- `ClassicKeyscan` has an optional `KeyscanSink`. One `KeyscanProgress` event
  is emitted after each completed sector with the sector index, sector count,
  attempts, and verified trailer reads. The callback runs on the scanning
  thread; the GUI forwards events through an unbounded channel.
- `KeyStore::recovery_candidates` expands every stored key into the key types
  its kind allows, appends common keys when requested, removes duplicates, and
  sorts by key type and key value. `nfctl` and the GUI use this one helper.
- `KeySource::Recovery` records keys saved from a recovery result. The JSON
  store round-trips the new provenance.

### Settings

- `crates/gui/src/settings.rs` owns `UiPreferences`: key store path, backup
  directory, hardnested table directory, common-key toggle, and default dump
  format. The language field from the removed GPUI preferences is not carried
  into 19d.
- `$XDG_CONFIG_HOME/nfc-foundry/ui.json` is written atomically with mode
  `0600`. A missing file yields defaults; malformed JSON is returned as an
  error and shown at startup instead of being discarded.
- Validation rejects an unreadable key store, a non-file key store, a non-
  directory backup path, a table directory without `bitflip_*_states.bin`
  files, and an unknown dump format. Missing output paths are allowed when
  their nearest existing parent is a directory.
- Loaded preferences are validated too: a semantically invalid file is kept
  but reported at startup and in the Settings status. Saving applies the
  preferences immediately: the key store is reloaded, the backup store and
  hardnested default are replaced, and the common-key default and dump format
  are used by the read, scan, recovery, and save paths. An empty key-store
  field resolves to the default store path and an empty backup field to the
  default backup directory. Advanced mode and recovery cancellation remain
  session-only.

### Workbench

- **Key scan** takes candidates from the key store, an optional text/JSON
  dictionary, and the built-in common keys. Key A, Key B, or both are
  selectable. The page shows per-sector progress, attempts, verified reads, and
  a sector table. Discovered keys merge into the store only through an explicit
  action; `.dic` export writes only the scanned set. Cancel uses the scan's
  `CancelToken`.
- **Recovery** exposes Auto, Darkside, Nested, Static nested, Hardnested, and
  FM11RF08S. Inputs cover known/target block and key type, tables directory,
  acquisition cap, and key-store/common candidates. `RecoveryProgress` events
  render acquisition frames/nonces/first bytes/states, filtering estimates,
  and brute-force percentage; a one-second timer shows elapsed time. Cancel
  sets the engine token. A result is displayed only after service
  verification; saving to the store is explicit and records the sector when
  the loaded image covers the target block. Unsupported engine results and
  missing hardnested tables are shown verbatim as limitations, not as a
  generic failure.
- **Diagnostics** shows the application/service version, supported driver
  prefixes, resolved hardnested table path, enumerated readers, and build
  profile/OS/architecture. It performs no card I/O and logs no secrets.
- **Activity** is a structured session list with timestamp, operation, result,
  and detail. The existing log writes feed it through `append_log`; warnings
  and backup paths stay in the detail. Key material is no longer written when
  a key is added, and NDEF payloads are not logged. Clear empties both the
  table and the rail log.
- **Settings** edits the persisted fields, validates before saving, reports the
  preference path, and reloads the key store after a save. Invalid paths leave
  the previous preferences and store in effect and show the error.

The new views are pages in the existing `gtk::Stack`; reader work still runs on
worker threads and returns through `async-channel` to the GTK main context.

### CLI

- `classic keyscan` now accepts `--key` and `--key-file` candidates in addition
  to the key store and common keys.
- Every recovery strategy that uses a known key accepts `--known-type A|B`.
- `diag` reports the supported driver prefixes, enumerated readers, the
  resolved (or explicitly configured) hardnested table directory, and the
  build profile/target. It still performs no card I/O.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes; the GTK scenario takes its skip branch
  without the gate.
- `env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env
  GDK_BACKEND=x11 cargo test -p nfc-foundry gtk_workbench_scenarios
  -- --nocapture`: passes.
- `cargo test -p nfctl --test cli`: passes.
- `reuse lint`: clean.

The gated scenario adds coverage for:

- keyscan progress, 16-sector result rows, merge, `.dic` export, and the
  cancelled render;
- darkside limitation rendering and explicit verified-key saving;
- diagnostics reader/version/build facts and activity redaction and clear;
- settings save with `0600`, reload of the key store and backup store, and
  rejection of a directory as the key-store path;
- the existing Classic, Type 2, NDEF, Diff, Restore, and Keys scenarios.

No card path or recovery algorithm changed in this phase. Hardware recovery
behavior remains the phase 06/07 record; darkside no-sample and FM11RF08S
detection-only are presented from the engine's existing results.

Superseded after this phase: `recover auto` is now an all-sector autopwn and
darkside uses the host-driven MFCUK sequence. See
[20-autopwn.md](20-autopwn.md).
