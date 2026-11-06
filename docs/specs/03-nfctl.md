# Spec 03: nfctl CLI

## Goal

Expose the shared Go application services as a stable, English-only CLI that
scripts and CI can rely on. `nfctl` calls the same `app.Service` methods as the
MyGo GUI, so card protocols, dump validation, write safety, recovery handoff,
and key persistence cannot diverge between the two surfaces.

Phase 03 builds the command framework and every command backed by the current
MIFARE Classic layer. Type 2/NDEF commands arrive with Phase 05; the APDU
command and the raw PN532 console arrive with Phases 04 and 07, when their
transport primitives exist. The command tree is extended then instead of
shipping commands that cannot perform real work.

## Deliverables

- `cmd/nfctl` entry point and `internal/cli` command package.
- Global options: `--device`, `--json`, `--jsonl`, `--yes`, `--dry-run`,
  `--timeout`, `--verbose`, `--show-keys`, `--version`.
- Commands: `list`, `scan`, `card info`, `classic read`, `classic write`,
  `classic dump`, `classic restore`, `classic keyscan`,
  `recover nested|darkside|hardnested`, `uid set`, `diag`,
  `keys list|add|import|export`.
- Stable exit codes and one stable JSON envelope.
- A cross-process lock file keyed by the exact connstring, enforced by the
  device manager so the GUI, CLI, and a second CLI cannot open the same reader.
- Command-level tests driven by `internal/nfc/mock`, plus lock tests.
- `docs/cli.md` updated to the implemented contract.

### Deferred command groups

| Command | Lands in |
|---|---|
| `classic read/write/dump/restore/keyscan`, `recover`, `uid set` | Phase 03 |
| `pn532 shell` | Phase 04 (raw transceive shim) |
| `type2 ...`, `ndef ...` | Phase 05 (Type 2 card module) |
| `apdu --raw` | Phase 07 (ISO14443-4 transport) |

No placeholder command is registered. `nfctl` that does not know a command
exits with code 2 and prints usage.

## Package layout

### `internal/cli`

- `run.go`: argument parsing, global options, dispatch, signal handling.
- `output.go`: human/JSON/JSONL writers, envelope, error classification, exit
  codes.
- `events.go`: `app.EventEmitter` implementation with task, card, and device
  channels; task waiting and cancellation.
- `session.go`: reader selection, connect/disconnect, card wait, key-file
  import.
- `cmd_*.go`: one file per command group.
- Tests use the same `app.Service` constructed with `app.Options`, a mock
  enumerator, mock readers, a temporary key store, and a temporary lock
  directory.

### `app`

- `NewServiceWithOptions(Options)` becomes the single constructor; `NewService`
  supplies the libnfc backend and standard user paths.
- CLI-oriented methods reuse existing workflows:
  - `WaitForCard` waits for the device manager's polling result.
  - `ReadClassicBlock` / `WriteClassicBlock` use `workflow.ClassicIO` and the
    verified key catalog or an explicit key override.
  - `LoadDumpFile` / `SaveDumpFile` bypass native file dialogs but keep the
    same validation and workbench state transitions as the GUI.
  - `ImportKeyDictionaryFile` / `ExportKeyDictionaryFile` bypass dialogs.
  - `StartHardnested` exposes the existing Hardnested service.
- `NewService` behavior for the GUI is unchanged.

### `device`

- `Config.LockDir` selects the lock directory; empty uses the per-user runtime
  directory (`XDG_RUNTIME_DIR`, else the system temporary directory).
- `Connect` acquires an exclusive `flock` on
  `reader-<sha256(connstring)[:16]>.lock`; `Disconnect`, a failed connect, and
  shutdown release it. Recovery and external-engine handoff keep the lock.
- A held lock returns `nfc.CodeBusy` with the holder's PID when available.

## Command contract

### Global behavior

- Options may appear before or after the subcommand.
- `--json` and `--jsonl` are mutually exclusive.
- `--device` is an exact connstring. Without it, `LIBNFC_DEVICE` is used, then
  enumeration; enumeration must yield exactly one reader.
- `--timeout` bounds the whole command after flag parsing. Zero means no
  deadline; the device manager still applies its own open/poll timeouts.
- `--verbose` writes task progress and phase messages to stderr. Raw engine
  output is printed only with `--show-keys`, because engine logs may contain
  recovered key material.
- `--show-keys` is the only switch that may print key values or raw engine
  logs. It never prints a dump unless the command writes one.

### Reader and card

```text
nfctl list
nfctl scan [--watch]
nfctl card info
```

- `list` enumerates readers and does not open or lock one.
- `scan` selects one reader, waits for a card, and prints reader plus card
  facts and attack capabilities. `--watch` prints appearance/removal events
  until interrupted.
- `card info` requires a card and prints UID, ATQA, SAK, inferred type, and
  reader.

### MIFARE Classic

```text
nfctl classic read --block <n> [--key A:HEX|B:HEX]
nfctl classic write --block <n> --data <HEX> [--key ...] --yes
nfctl classic dump <file.bin|file.mfd|file.json> [--keys <file>]
nfctl classic restore <file.bin|file.mfd|file.json> [--keys <file>] \
    [--yes] [--confirm-trailers] [--dry-run]
nfctl classic keyscan [--keys <file>]
```

- `read` authenticates with the explicit key or a verified key for the target
  sector and prints the 16 bytes plus ASCII. Reading a trailer is allowed and
  may show masked Key A, which is a card property, not a dump error.
- `write` accepts ordinary data blocks only. Block 0 is rejected (`uid set`),
  and sector trailers are rejected (`classic restore`). `Reader.WriteBlock`
  performs immediate read-back verification.
- `dump` reads with all verified keys. A complete image is written as raw
  `.bin`/`.mfd` plus `<name>.nfcx.json`. An incomplete image must use `.json`,
  because unknown bytes are never zero-filled. `--keys` imports a dictionary
  into the catalog first; run `classic keyscan` to verify candidates.
- `restore` loads a complete dump, runs the restore preflight, and writes
  ordinary blocks before trailers, verifying each block. It never writes block
  0. `--dry-run` prints the preflight plan only. An actual restore requires
  `--yes` and `--confirm-trailers`, because a full restore replaces keys and
  access conditions.
- `keyscan` verifies known candidates for every Key A/Key B slot, persists new
  candidates, and reports missing slots. Values appear only with
  `--show-keys`.

### Key recovery

```text
nfctl recover nested --yes [--keys <file>]
nfctl recover darkside --yes
nfctl recover hardnested --yes [--keys <file>]
```

- All modes require `--yes` as explicit authorization for active testing.
- `nested` and `hardnested` need at least one verified seed. `--keys` imports
  a dictionary and runs a keyscan first to verify seeds.
- `darkside` requires the Classic 1K / PN532 UART profile and no verified keys.
- The reader is released to the engine, the exact connstring is passed through
  `LIBNFC_DEVICE`, artifacts are validated, the reader is reopened, and
  recovered keys are re-verified in-process. Output reports engine version,
  claims, verified/existing/rejected/conflict counts, and merged dump bytes.

### UID

```text
nfctl uid set <4-byte-hex> --yes [--confirm-uid <4-byte-hex>] [--dry-run]
```

- The card must be a 4-byte UID MIFARE Classic 1K special card.
- Preflight prints the current/new UID, BCC, and old/new block 0.
- `--dry-run` stops after preflight.
- The write requires `--yes` and an exact `--confirm-uid` value equal to the
  target UID. The service backs up block 0, writes, reselects, and verifies the
  complete block.

### Diagnostics and keys

```text
nfctl diag
nfctl keys list [--show-keys]
nfctl keys add <A:HEX|B:HEX|HEX>
nfctl keys import <file>
nfctl keys export <file>
```

- `diag` runs the shared release self-check and prints each check. The command
  exits 0 when the report was produced; scripts inspect `healthy`.
- `keys list` omits values unless `--show-keys`.
- `keys add` strips an optional `A:`/`B:` prefix, writes the catalog with mode
  0600, and does not echo the value.
- `keys import` reports valid/added/duplicate/ignored counts and per-line
  issues.
- `keys export` writes a 0600 dictionary.

## JSON envelope

Every `--json` run writes exactly one JSON document to stdout:

```json
{
  "ok": true,
  "command": "classic.dump",
  "device": "pn532_uart:/dev/ttyUSB0",
  "card": {"present": true, "uid": "04AABBCC", "atqa": "00 04", "sak": "08", "type": "MIFARE Classic 1K", "typeCode": "mifare_classic_1k", "inferred": true},
  "warnings": [],
  "error": null,
  "result": {}
}
```

- `error` is `null` on success and otherwise
  `{"code": "...", "message": "...", "exitCode": 2..7}`.
- `result` is command-specific and documented in `docs/cli.md`.
- JSON keys are English, stable, and never localized.
- Key values are absent unless `--show-keys` or the command explicitly writes a
  key file.
- `--jsonl` streams progress events with `"type":"progress"` and ends with one
  `"type":"result"` line carrying the same envelope fields.

## Exit codes

| Code | Meaning |
|---:|---|
| 0 | Success |
| 1 | Operation failed (authentication, write rejection, engine exit) |
| 2 | Usage or argument error |
| 3 | Validation failure (dump layout, BCC, access bits, card mismatch) |
| 4 | Permission or hardware unavailable (no reader/card, busy, lock, timeout, unsupported capability) |
| 5 | External engine unavailable or failed validation |
| 6 | Cancelled by the user or a signal |
| 7 | Internal error |

## Safety

- Destructive commands refuse to start without the required flags.
- `classic write` refuses block 0 and trailers; trailers go through the
  validated restore workflow.
- A restore plan with trailers requires a separate trailer confirmation.
- UID writes require the exact new UID as confirmation and always create a
  block-0 backup before writing.
- Every write path keeps immediate read-back verification and aborts on card
  identity changes.
- The CLI never bypasses the device manager, the key catalog, or the write
  safety layer.

## Test plan

Hardware-free:

- `app.NewServiceWithOptions` with mock enumerator/reader, temporary key store,
  and temporary lock directory.
- Command tests: list JSON, scan/card info, Classic read/write, dump/load
  round-trip, restore dry-run and `--yes`/trailer refusal, UID refusal and
  preflight, keys add/list/import/export (values hidden by default), JSON
  envelope and exit-code mapping, cancelled task.
- Cross-process lock: a second manager cannot open the same connstring; a
  different connstring is independent; disconnect releases the lock.
- `go test ./...`, `go vet ./...`, `go tool mygo i18n check .`, desktop smoke
  test, and `reuse lint`.

Hardware gate (manual, not CI):

- `nfctl list`, `nfctl scan`, and `nfctl card info` against the validated
  ACR122U/PN532 setup.
- `nfctl classic keyscan` then `nfctl classic dump`.
- `nfctl classic restore --dry-run` on the produced dump; an actual restore
  only on an explicitly authorized test card.
- A second process is refused while the GUI or another CLI holds the reader.

## Packaging

- `make build` produces `nfc-foundry` and `nfctl` in the same application
  directory; both get the private runtime rpath.
- The release tarball installs both executables and the recovery engines.
- CI compiles `./cmd/nfctl` without the libnfc tag.

## Non-goals

- No relay, DEP, anticol, SAM, or barcode commands.
- No Type 2/NDEF/APDU implementation in this phase.
- No translated CLI output.
- No generic write-everything command.

Card emulation remains a conditional future phase (Phase 09).
