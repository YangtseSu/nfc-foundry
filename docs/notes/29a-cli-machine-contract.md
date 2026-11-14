# Note: 29.1 CLI machine contract

Plan: [29-cli-reader-ownership.md](../plan/29-cli-reader-ownership.md). Spec:
[29a-cli-machine-contract.md](../specs/29a-cli-machine-contract.md). Contract:
[cli.md](../cli.md).

## Source

The machine contract follows the previous `../nfc-foundry` implementation:
`internal/cli/flags.go`, `errors.go`, `output.go`, `events.go`, `run.go`, and
`docs/cli.md`. That CLI had `--json`, `--jsonl`, `--timeout`, `--verbose`,
`--dry-run`, exit codes 0-7, a single envelope, and JSONL progress events. The
Rust rewrite keeps the shape but replaces string-matched error classification
with the typed `NfcError` table.

The previous Go CLI had an external-engine exit code 5. Native-only recovery
keeps the code reserved and never emits it.

## Implemented

### CLI runtime

- `crates/nfctl/src/error.rs`:
  - `CliError::{Usage, Service, Validation, Internal}`;
  - exhaustive `NfcError` classification with stable `code`/`exitCode`;
  - `Cancelled` maps to `deadline_exceeded`/1 when the `--timeout` watchdog set
    its flag, otherwise `cancelled`/6;
  - unit tests cover every `NfcError` variant.
- `crates/nfctl/src/output.rs`:
  - `Mode::{Human, Json, JsonLines}`;
  - `Output` owns the command name, device, card facts, warnings, and mode;
  - `line`/`inline` are muted outside human mode; `warn` prints on stderr or
    collects into the envelope; `note` and `verbose` always use stderr;
  - pretty envelope for `--json`, compact `type: result` line for `--jsonl`;
  - stable card facts and compact UID formatting.
- `crates/nfctl/src/runtime.rs`:
  - pre-parse `--json`/`--jsonl` detection so clap errors honor the mode;
  - duration parser (`ms`, `s`, `m`, `h`, sums);
  - detached deadline watchdog that cancels the shared token;
  - SIGINT/SIGTERM handler: first signal cancels, second exits 130.
- `Cargo.toml`: `serde_json` is now a runtime dependency; `signal-hook` is a
  workspace dependency.

### Global flags

- `--json`, `--jsonl` (conflicting), `--timeout`, `--verbose`, `--dry-run` on
  `Cli` as `global = true`.
- Per-command `--dry-run` flags removed; `diag`'s private `--json` removed.
- `Command::name()` returns a leaf name for every command; it is used for
  usage errors (including unsupported `--dry-run`) and as the envelope default.
- `--verbose` reports the selected connstring, reader status, capability
  profile, and firmware when available.

### Cancellation

- One token per invocation; every service request receives it, including
  scan/card-info via new `Service::scan_with`/`card_info_with`.
- `scan --watch` checks the token each poll.
- Write paths pass the token and keep their existing read-back and
  verification, so a deadline never turns an unverified write into success.

### JSON results

Every command family now returns a typed result object. Highlights:

- discovery: `devices`, card identity, health coverage, self-check document;
- Classic: read, dump metadata, write/UID/restore plans and results, keyscan
  sector booleans, key-management counts, diff bytes, Classic NDEF state;
- Type 2: product/CC/locks/configuration/NDEF, pages, mutations, UID plans;
- DESFire: inventory, file settings, reads/writes/values/records, memory,
  app/file/key management, NDEF provisioning;
- recovery: strategy, attempts, profile, verified `sector`/`keyType` only;
- JSONL progress for recovery (rate-limited), keyscan (per sector), and watch
  card events.

Secret handling: JSON never contains MIFARE keys, Type 2 PWD/PACK, DESFire key
material, or raw dump payloads. `classic keys export` refuses machine mode
without `--output`. Human output keeps the existing key display for explicit
key commands.

### Correctness details

- Unknown `--device` values are a usage error (2), not a validation error (3),
  because the service's `InvalidArgument` for a missing connstring is really an
  argument mistake.
- Missing `--device` is a usage error.
- Declining an interactive confirmation returns `Cancelled` (6); a
  non-interactive or machine-mode confirmation without `--yes` is usage (2).
- `--json` + `--jsonl` fails in clap and still emits a machine envelope.
- `--dry-run` on a command without a preflight exits 2 and names the leaf
  command.
- `mock:empty` was added to the scripted reader set so the no-card path is
  testable end to end.

## Tested

2026-11-14:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`: 61 test binaries, all `ok`, 0 failed.
- `cargo test -p nfctl --test cli`: 48 tests, all pass.
- `cargo test -p nfctl --test cli_contract`: 15 tests, all pass; they cover:
  - envelope schema and golden `list`/`scan` facts;
  - `classic.read`/`classic.dump`/`classic.diff`/`classic.ndef.read`,
    `type2.info`/`ndef.read`, `desfire.info`, `card.health` typed results;
  - recovery and `diag` documents;
  - keyscan and key-list redaction, export guard;
  - dry-run rejection and write plan;
  - usage, validation, hardware, deadline exit codes;
  - JSONL progress plus one terminal result, and SIGINT cancellation with exit 6;
  - human stdout/stderr separation;
  - sandboxed backup paths.
- `cargo test -p nfctl --bins`: 8 unit tests for classification, durations,
  mode pre-scan, and existing parsers.
- Manual smoke runs against the mock readers for `--json` and `--jsonl`,
  including a corrupted BCC that exits 3 with `code: validation`.
- `reuse lint`: 685/685 files with copyright and license information.

## Limitations

- `--timeout`/Ctrl-C are observed at token checkpoints. A single blocking
  exchange can outlast the deadline; a second Ctrl-C exits immediately.
- Recovery progress is throttled, not a complete event log.
- `--json` for `classic keys export` without `--output` is refused rather than
  interleaving key bytes with the envelope.
- Reader auto-selection, `NFC_FOUNDRY_DEVICE`, and the cross-process lock stay
  in 29.3 and 29.2. Until then two processes can still contend for one reader
  and the OS error is reported as `io`/1.
- Developer raw commands are not present (29.6).
- Envelope keys are alphabetically ordered by `serde_json`; the contract does
  not promise document order.
