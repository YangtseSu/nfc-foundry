# `nfctl` CLI contract

`nfctl` is the scripting and automation surface for NFC Foundry. It shares the
same Go services as the GUI.

**The CLI is English-only.** Help, errors, progress messages, log output, JSON
keys, and documentation are not localized. Do not add locale-dependent output
or translated error strings.

## Global options

| Option | Meaning |
|---|---|
| `--device <connstring>` | Exact reader selection; otherwise autodetect or `LIBNFC_DEVICE` |
| `--json` | Machine-readable JSON output |
| `--jsonl` | Streaming JSON Lines for scans/tasks |
| `--yes` | Confirm a destructive operation |
| `--dry-run` | Run preflight/planning without writing or starting a destructive task |
| `--timeout <duration>` | Operation timeout |
| `--verbose` | Human-readable diagnostics on stderr |
| `--version` | Print the version and exit |

Global options may appear before or after the subcommand. `--json` and
`--jsonl` are mutually exclusive. Human-readable output is written to stdout;
diagnostics and raw engine logs are written to stderr.

## Exit codes

| Code | Meaning |
|---:|---|
| 0 | Success |
| 1 | Operation failed (I/O, engine failure, write rejected) |
| 2 | Usage or argument error |
| 3 | Validation failure (dump layout, BCC, access bits, card mismatch) |
| 4 | Permission or hardware unavailable |
| 5 | External engine unavailable or failed validation |
| 6 | Cancelled by the user or a signal |
| 7 | Internal error |

Never encode success only in prose. Scripts must be able to rely on the exit
code and, when requested, JSON fields.

## Commands

### Discovery and card state

```text
nfctl list
nfctl scan [--watch] [--jsonl]
nfctl card info [--json]
```

`list` enumerates readers. `scan` selects one card and prints UID, ATQA, SAK,
inferred type, and available capabilities. `--watch` keeps polling and emits
presence/removal events.

### MIFARE Classic

```text
nfctl classic read --block <n> [--key A:HEX|B:HEX]
nfctl classic write --block <n> --data <HEX> [--key ...] --yes
nfctl classic dump <file.mfd|file.bin> [--keys <file>]
nfctl classic restore <file.mfd|file.bin> [--yes] [--dry-run]
nfctl classic keyscan [--keys <file>]
nfctl classic recover --mode nested|darkside|hardnested [--keys <file>]
```

`dump` writes raw data plus a sidecar metadata file. `restore` validates
capacity, BCC, trailers, and access bits before writing; it writes trailers
last and verifies every block.

### Type 2 / NTAG

```text
nfctl type2 read --page <n> [--count <n>]
nfctl type2 write --page <n> --data <HEX> --yes
nfctl type2 dump <file> [--all-pages]
nfctl type2 restore <file> --yes
```

`type2` covers MIFARE Ultralight and NTAG213/215/216. User-memory pages are the
normal target. Configuration pages, lock bytes, and password/PACK changes
require a separate guarded command once that workflow is implemented.

### NDEF

```text
nfctl ndef read [--file <out>] [--family type2|auto]
nfctl ndef write --file <message.ndef> [--family type2|auto] --yes
nfctl ndef inspect --file <message.ndef>
```

NDEF is committed for Type 2 first. FeliCa/Type 3 NDEF is not part of the
initial contract; do not advertise `--family felica` until its scope is
accepted.

### UID / block 0

```text
nfctl uid set <4-byte-hex> [--block0 <32-byte-hex>] --yes
```

This command targets supported Gen1A and Gen2/CUID cards only. It requires a
card-type check, a block-0 backup, explicit confirmation, and post-write
re-selection. Native Go Gen1A replaces the upstream `nfc-mfsetuid` executable.

### Diagnostics and advanced access

```text
nfctl diag
nfctl pn532 shell
nfctl apdu --raw <HEX> [--expect <HEX>]
```

`diag` reports reader, firmware, transport, and capability checks. `pn532 shell`
is an interactive raw-command console. `apdu` is a diagnostic APDU exchange;
it must not be presented as a safe write path.

### Keys

```text
nfctl keys list
nfctl keys add <A:HEX|B:HEX>
nfctl keys import <file>
nfctl keys export <file>
```

Keys are sensitive. Do not print full keys unless the user explicitly requests
it. JSON output must follow the same rule.

## JSON shape

Common fields:

```json
{
  "ok": true,
  "command": "classic.dump",
  "device": "pn532_uart:/dev/ttyUSB0",
  "card": {"uid": "04AABBCC", "atqa": "0004", "sak": "08", "type": "mifare_classic_1k"},
  "warnings": [],
  "error": null
}
```

Errors use a stable `code`, an English `message`, and optional `details`.
Progress events use `type`, `phase`, `message`, and optional numeric progress.
Do not include card keys or full dumps in logs unless explicitly requested.

## Device ownership and concurrency

Only one process may own a reader. A CLI command and the GUI must not open the
same connstring concurrently. Use a lock file keyed by the connstring; fail with
exit code 4 and an actionable message when the lock is held.

When an external engine runs, the same handoff rules as the GUI apply: release
the reader, run the engine with `LIBNFC_DEVICE`, validate the artifact, reopen,
and verify in-process.

## Explicitly out of scope

- card emulation commands (future mobile-Linux direction, not initial);
- relay/DEP/anticol/SAM demonstration commands;
- barcode experiments;
- generic "write everything writable";
- translated CLI output.
