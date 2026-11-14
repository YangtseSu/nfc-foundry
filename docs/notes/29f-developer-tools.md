# Note: 29.6 developer-only raw diagnostics

Plan: [29-cli-reader-ownership.md](../plan/29-cli-reader-ownership.md). Spec:
[29f-developer-tools.md](../specs/29f-developer-tools.md).

## Implemented

- `nfc-transport`: non-default `developer-tools` feature, the `RawConsole`
  trait, and `Reader::raw_console`. The mock reader implements the trait and
  `MockHandle` scripts replies and records requests.
- `nfc-drivers`: `Pn532Protocol::raw_command` sends one native command through
  the existing framing, ACK, and response validation. `Pn532Uart` implements
  `RawConsole`; ACR122U USB and PC/SC keep the default `None`.
- `nfc-app`: `developer::DiagnosticLease` and `Service::begin_diagnostics`,
  feature gated. The lease opens the selected reader, does not select a card,
  holds the busy state for the session, and clears it on drop. `raw_apdu`
  selects the card, requires ISO/IEC 14443-4, and returns the card identity
  with the response.
- `nfctl`: `pn532 shell` and `apdu --raw <HEX>`, feature gated with command
  names `pn532.shell` and `apdu.raw`. Without the feature the variants,
  handlers, and help entries do not exist.
- `docs/cli.md`: command list, developer section, result shape, and secret
  rules. Plan and plan index updated.

## Behavior

- `developer-tools` is not in any default feature set. The release workflow and
  both PKGBUILDs use `cargo build --release --workspace`, so release and AUR
  binaries exclude the commands.
- `pn532 shell` requires human output. `--json`/`--jsonl` exit 2 and
  `--dry-run` is rejected like any other non-planning command.
- The console prints its warning on stderr, then reads one native command per
  line. `help`/`?`, `quit`/`exit`, blank lines, and EOF are handled locally;
  command errors print `error: …` and keep the session. The reply body prints
  as space-separated hex or `(empty)`.
- No card is required for the console. It holds one diagnostic lease, so the
  reader and its cross-process lock stay claimed until the process exits.
- `apdu --raw` prints the response data and status word, and the JSON envelope
  includes the selected card and `apdu.raw` result fields. The request APDU is
  never echoed.
- Neither command prints keys, the Type 2 password, or the request frame.

## Limitations

- The console talks to the reader, not the card. A command such as
  `InListPassiveTarget` updates the PN532 target state without updating the
  driver's cached selection, so the next `nfctl` invocation starts clean while
  a later command in the same session may see the reader's real state.
- The PN532 console owns no card workflow: there is no automatic field reset,
  target reselection, or recovery-mode transition. That is the diagnostic
  contract.
- ACR122U has no native command console. Its card-level APDU path is available
  through `apdu --raw` like any other ISO-DEP reader.
- Ctrl-C cancels the shared token at the next loop check. A blocking
  `read_line` on a terminal may need a second Ctrl-C to leave the console, the
  same process-wide signal behavior as the rest of `nfctl`.
- Phase 13's read-only APDU diagnostic is still pending and remains a separate
  normal command; this slice adds none to release builds.

## Tested

2026-11-15:

- `cargo fmt --all --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo clippy -p nfctl -p nfc-app --all-targets --features developer-tools --
  -D warnings`.
- `cargo test --workspace`: 790 passed, 0 failed, 9 ignored (hardware-gated).
- `cargo test -p nfctl -p nfc-app --features developer-tools`: 324 passed, 0
  failed (nfctl 87, nfc-app 237).
- `reuse lint`: 698/698 files compliant.
- Manual smoke against the mock readers:
  `printf '02\nhelp\nquit\n' | nfctl --mock --device mock:empty pn532 shell`
  printed the warning, the usage line, and `response: (empty)`;
  `nfctl --mock --device mock:type4-desfire apdu --raw
  00A4040007D276000085010100 --json` returned `sw: "9000"`, `statusOk: true`,
  the selected card, and the diagnostic warning.

New tests:

- `nfc-app`: `developer::tests::a_raw_console_runs_before_a_card_is_present`,
  `developer::tests::a_raw_apdu_requires_an_iso_dep_card`.
- `nfctl` default build:
  `without_feature::the_developer_commands_are_absent_from_help_and_parsing`.
- `nfctl` developer build: help listing, console warning with secret-free
  output, APDU result, and the machine-output rejection.
