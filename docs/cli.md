# `nfctl` CLI contract

`nfctl` is the scripting and automation surface for NFC Foundry. It shares the
same Go application services as the GUI.

**The CLI is English-only.** Help, errors, progress messages, engine output,
JSON keys, and documentation are not localized. Do not add locale-dependent
output or translated error strings.

## Global options

| Option | Meaning |
|---|---|
| `--device <connstring>` | Exact reader selection; otherwise `LIBNFC_DEVICE`, then autodetect |
| `--json` | One JSON document on stdout |
| `--jsonl` | Streaming JSON Lines events plus a final result |
| `--yes` | Confirm a destructive or active operation |
| `--dry-run` | Stop after preflight/planning where supported |
| `--show-keys` | Allow key values and raw engine logs in output |
| `--timeout <duration>` | Bound the whole command |
| `--verbose` | Progress and diagnostics on stderr |
| `--version` | Print build metadata and exit |
| `--help` | Print usage and exit |

Options may appear before or after the subcommand. `--json` and `--jsonl` are
mutually exclusive. Human output is written to stdout; diagnostics, progress,
warnings, and raw engine logs are written to stderr.

`--dry-run` is supported by `classic write`, `classic restore`, and `uid set`.
Other commands reject it instead of silently ignoring it.

## Device selection and ownership

Without `--device`, `nfctl` uses `LIBNFC_DEVICE` if set, then enumerates
readers. Enumeration must yield exactly one reader; with multiple readers the
command fails and prints the connstrings so the user can choose one.

`list` only enumerates. Every other reader command opens exactly one reader and
holds a cross-process lock file keyed by the exact connstring. The GUI and a
second CLI cannot open the same reader concurrently; the lock returns exit code
4 with the holder's program and PID when available.

A one-shot command waits up to 30 seconds for a card unless a shorter
`--timeout` deadline applies.

Key verification state is card-scoped and persisted with the key catalog, so a
keyscan or recovery result remains usable by a later `nfctl` invocation.

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

Never encode success only in prose. Scripts can rely on the exit code and, when
requested, the JSON fields.

## Commands

### Discovery and card state

```text
nfctl list
nfctl scan [--watch]
nfctl card info
```

`list` enumerates readers without opening one. `scan` selects one reader, waits
for a card, and prints reader, UID, ATQA, SAK, inferred type, and recovery
capabilities. `card info` requires a card and prints the same card facts.

`scan --watch` prints appearance/removal events until interrupted. It requires
human or `--jsonl` output; `--json` is rejected because a watch has no single
final document. Interrupting a watch exits 0.

### MIFARE Classic

```text
nfctl classic read --block <n> [--key A:HEX|B:HEX]
nfctl classic write --block <n> --data <HEX> [--key ...] --yes
nfctl classic dump <file.bin|file.mfd|file.json> [--keys <file>]
nfctl classic restore <file.bin|file.mfd|file.json> [--keys <file>] \
    [--yes] [--confirm-trailers] [--dry-run]
nfctl classic keyscan [--keys <file>]
```

`read` authenticates with the explicit key or a verified key for the target
sector. It prints the 16 bytes and ASCII. Reading a sector trailer is allowed;
MIFARE may mask Key A on trailer reads, which is a card property rather than a
dump error.

`write` accepts ordinary data blocks only. Block 0 is rejected (use
`nfctl uid set`) and sector trailers are rejected (use `nfctl classic
restore`). `Reader.WriteBlock` performs immediate read-back verification.
`--dry-run` authenticates and reads the current block but does not write.

`dump` reads with all verified keys for the card. A `.json` path always stores
an NFC Foundry project, complete or partial. Other paths store a complete raw
image as `.bin`/`.mfd` plus `<name>.nfcx.json`; an incomplete image is rejected
for raw paths because unknown bytes are never zero-filled. If the read task
fails, no dump is written.

`restore` loads a complete dump, runs the restore preflight, and writes ordinary
blocks before trailers, verifying every block. Manufacturer block 0 is always
skipped. An actual restore requires `--yes` **and** `--confirm-trailers`,
because a full restore replaces keys and access conditions. `--dry-run` prints
the preflight plan and does not require either confirmation.

`keyscan` authenticates every known candidate for every Key A/Key B slot,
persists verified matches, and reports missing slots. Values appear only with
`--show-keys`.

`--keys <file>` imports a dictionary. For `dump`, `restore`, and
`nested`/`hardnested` recovery, it also runs a keyscan first so the imported
candidates become verified keys for the current card. `classic keyscan --keys`
uses the same scan it already performs.

### Key recovery

```text
nfctl recover nested --yes [--keys <file>]
nfctl recover darkside --yes
nfctl recover hardnested --yes [--keys <file>]
```

All modes require `--yes` as explicit authorization for active card testing.

- `nested` and `hardnested` need at least one verified seed key. `--keys`
  imports a dictionary and scans it first.
- `darkside` requires the Classic 1K / PN532 UART profile and no verified keys;
  it rejects `--keys`.

The reader is released to the engine, the exact connstring is passed through
`LIBNFC_DEVICE` with autoscan and intrusive scan disabled, artifacts are
validated, the reader is reopened, and recovered keys are re-verified
in-process. Output reports engine version, claims, verified/existing/rejected/
conflict counts, and merged dump bytes.

### UID

```text
nfctl uid set <4-byte-hex> --yes [--confirm-uid <4-byte-hex>] [--dry-run]
```

The card must be a 4-byte UID MIFARE Classic 1K special card. Preflight prints
the current/new UID, BCC, and old/new block 0. `--dry-run` stops after
preflight and does not require `--yes`.

An actual write requires `--yes` and an exact `--confirm-uid` value equal to
the target UID. The service backs up block 0, writes, reselects, and verifies
the complete block.

### Diagnostics and keys

```text
nfctl diag
nfctl keys list [--show-keys]
nfctl keys add <A:HEX|B:HEX|HEX>
nfctl keys import <file>
nfctl keys export <file>
```

`diag` runs the shared release self-check and prints each check. It exits 0
when the report was produced; scripts inspect the `healthy` field.

`keys list` omits values unless `--show-keys`. `keys add` strips an optional
`A:`/`B:` prefix, persists the catalog with mode 0600, and does not echo the
value. `keys import` reports valid/added/duplicate/ignored counts and per-line
issues. `keys export` writes a 0600 dictionary.

Keys are sensitive. Full values and raw engine logs appear only with
`--show-keys`; no other flag or log level reveals them.

## JSON output

Every `--json` run writes exactly one JSON document to stdout:

```json
{
  "ok": true,
  "command": "classic.dump",
  "device": "acr122_usb:001:005",
  "card": {"present": true, "uid": "04 11 22 33", "atqa": "00 04", "sak": "08", "type": "MIFARE Classic 1K", "typeCode": "mifare_classic_1k", "inferred": true},
  "warnings": [],
  "error": null,
  "result": {}
}
```

- `error` is `null` on success and otherwise
  `{"code": "...", "message": "...", "exitCode": 2..7}`.
- `warnings` is always an array.
- `result` is command-specific. Stable result keys include `devices`, `card`,
  `block`, `sector`, `kind`, `hex`, `ascii`, `path`, `sidecar`, `complete`,
  `verified`, `total`, `sectors`, `preflight`, `restore`, `mfoc`, `mfcuk`,
  `keyScan`, and `keys`.
- `command` is one of `list`, `scan`, `scan.watch`, `card.info`,
  `classic.read`, `classic.write`, `classic.dump`, `classic.restore`,
  `classic.keyscan`, `recover.nested`, `recover.darkside`, `recover.hardnested`,
  `uid.set`, `diag`, `keys.list`, `keys.add`, `keys.import`, `keys.export`.

`--jsonl` emits one compact object per line:

- `{"type":"progress", ...}` for task events, including `event`, `phase`,
  `message`, and optional progress/count fields;
- `{"type":"card", ...}` for `scan --watch` presence changes;
- one final `{"type":"result", ...}` line with the same envelope fields.

Raw engine `log` fields are omitted from progress lines unless `--show-keys` is
set.

## Planned command groups

These commands are not registered yet; they arrive with their phases:

| Command | Phase |
|---|---|
| `nfctl pn532 shell` | Phase 04 (raw transceive shim) |
| `nfctl type2 read|write|dump` | Phase 05 (Type 2 card module) |
| `nfctl ndef read|write` | Phase 05 |
| `nfctl apdu --raw <HEX>` | Phase 07 (ISO14443-4 transport) |

An unknown command exits with code 2 and prints usage. Card emulation is not
exposed by the initial CLI; Phase 09 defines its surface after target-mode
research.

## Explicitly out of scope

- relay/DEP/anticol/SAM demonstration commands;
- barcode experiments;
- generic "write everything writable";
- translated CLI output.
