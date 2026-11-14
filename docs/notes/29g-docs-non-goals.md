# Note: 29.7 docs and non-goals

Plan: [29-cli-reader-ownership.md](../plan/29-cli-reader-ownership.md). Spec:
[29g-docs-non-goals.md](../specs/29g-docs-non-goals.md).

## Implemented

- `README.md`:
  - the CLI machine-output example block now includes a `--timeout` dump run;
  - device selection records that one process owns a reader at a time: a second
    `nfctl` exits 4 with `reader_busy` and the workbench shows the same busy
    error;
  - the GUI section now covers Connect/Disconnect, the inspection-only image,
    the Activity task card, and honest cancellation of a write that may have
    committed;
  - the product scope records that a GUI command terminal or shell launch is
    out of scope.
- `docs/design/workbench.md`: the main-menu contract states there is no
  command terminal entry and that the GUI never launches a shell, bundles a
  private runtime or external engine, or exposes raw reader/APDU commands.
- `docs/testing.md`: added the 29.4, 29.5, and 29.6 test records and the
  commands that exercise both sides of a feature gate.
- `docs/plan/29-cli-reader-ownership.md` and `docs/plan/README.md`: phase 29
  and all slices are marked done; the index records the dropped
  `OpenCommandTerminal`.
- `docs/cli.md` already carried the complete contract from 29.1 and 29.6; this
  slice verifies it and adds no competing CLI document.

## Behavior

No runtime behavior changed. The slice is documentation and non-goal
recording after 29.1-29.6.

- `OpenCommandTerminal` is dropped in four places: README product scope,
  workbench main-menu contract, phase non-goals, and the plan index rules.
- README keeps developer diagnostics out of the release-facing command list;
  `docs/cli.md` documents their non-default feature and absence from release
  builds.
- Device selection now names both halves of the lock contract: the resolution
  order and the busy outcome for the losing process.

## Verified

2026-11-15:

- README CLI examples against the mock readers with `target/debug/nfctl`:
  `--mock list --json`, `--mock --device mock:classic-1k scan --json`,
  `classic read --block 4 --json`, `classic keyscan --jsonl`, and
  `--timeout 30s --mock --device mock:classic-1k classic dump --output …`
  all exited 0 with `ok: true`. The dump run's temporary output and the
  auto-created `card.dic` were removed afterwards.
- `cargo fmt --all --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 790 passed, 0 failed, 9 ignored (hardware-gated).
- `cargo test -p nfctl -p nfc-app --features developer-tools`: 324 passed,
  0 failed.
- `NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry --bin
  nfc-foundry app::tests::gtk_workbench_scenarios -- --exact`: pass (147.05s).
- `reuse lint`: 701/701 files compliant.

## Limitations

- Hardware README examples were not executed because no reader was attached;
  their connstrings and flags are unchanged from the validated records.
- The documentation states the current behavior; the phase acceptance items
  for JSON stability, reader contention, GUI disconnect, and cancellation were
  closed by their own slices and tests.
