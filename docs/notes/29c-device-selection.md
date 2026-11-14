# Note: 29.3 device selection

Plan: [29-cli-reader-ownership.md](../plan/29-cli-reader-ownership.md). Spec:
[29c-device-selection.md](../specs/29c-device-selection.md). Contract:
[cli.md](../cli.md).

## Implemented

- `crates/nfctl/src/select.rs`: pure `resolve_device(explicit, environment,
  devices)` with the flag, environment, single-reader, and failure order.
  `SelectionError` distinguishes no readers, ambiguity, and an unknown target
  from each source. Candidate connstrings are carried into the message.
- `crates/nfctl/src/main.rs`: `prepare` reads `NFC_FOUNDRY_DEVICE` through
  `environment_device`, resolves the connstring, and only then calls
  `select_device`. `--device` uses clap's non-empty parser. `--help` states
  the precedence.
- `crates/nfctl/src/error.rs`: `CliError::NoReader` carries the actionable
  no-reader message and classifies as `no_reader`/4. Ambiguity and unknown
  targets stay `usage`/2.
- `crates/gui/src/app.rs`:
  - refresh explicitly selects the first enumerated reader instead of relying
    on the combo notification, which is silent when the index stays 0;
  - a guard suppresses the user handler during the programmatic combo update;
  - the reader panel renders `{transport name}\n{exact connstring}` from the
    monitor snapshot, with a transport-label fallback for a connstring that
    left the device list;
  - the user combo change and refresh share `select_reader_device`.
- Documentation: selection order, environment variable, `LIBNFC_DEVICE`
  migration, exit codes, GUI reader label, plan index, and testing entry.

## Behavior

- Explicit `--device` suppresses the environment, including an invalid value.
- An empty environment value is treated as unset.
- A non-Unicode environment value is a usage error.
- Several readers without a choice exit 2 with the candidate connstrings; an
  arbitrary first reader is never opened.
- Zero readers exit 4 with `no_reader` and an actionable message.
- The JSON `device` field carries the resolved connstring, whether it came
  from the flag, the environment, or auto-selection.
- The GUI still auto-selects the first reader. It now re-resolves that
  selection on every refresh, so a changed enumeration cannot leave the
  service on a reader that is no longer first.
- The GUI ignores `NFC_FOUNDRY_DEVICE`; it keeps its own selection state.

## Limitations

- Zero and single-reader behavior is covered by unit tests because the
  scripted `--mock` transport intentionally exposes several readers. The
  multi-reader integration path and environment precedence run the real
  binary.
- Hardware enumeration on the development host currently reports a PN532 UART
  and an ACR122U USB plus its PC/SC alias, so `nfctl` without a selection
  exits 2 there; that is the intended ambiguity path, not a hardware failure.

## Tested

2026-11-14:

- `cargo fmt --all --check`.
- `cargo clippy -p nfctl -p nfc-foundry --all-targets -- -D warnings`.
- `cargo test -p nfctl --bin nfctl`: 15 tests pass, including the selection
  and no-reader classification tests.
- `cargo test -p nfctl --test cli_contract`: 20 tests pass, including
  environment selection, flag precedence, invalid environment target, and
  multi-reader ambiguity.
- `NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry --bin
  nfc-foundry app::tests::gtk_workbench_scenarios -- --exact`: the scenario
  passes, including the reader refresh label check.
- `cargo test --workspace`: 783 passed, 0 failed.
- `reuse lint`: 691/691 files compliant.
