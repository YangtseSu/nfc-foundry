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
| `--show-keys` | Allow key values, Type 2 PWD/PACK pages, and raw engine logs in output |
| `--timeout <duration>` | Bound the whole command |
| `--verbose` | Progress and diagnostics on stderr |
| `--version` | Print build metadata and exit |
| `--help` | Print usage and exit |

Options may appear before or after the subcommand. `--json` and `--jsonl` are
mutually exclusive. Human output is written to stdout; diagnostics, progress,
warnings, and raw engine logs are written to stderr.

`--dry-run` is supported by `classic write`, `classic restore`,
`classic value set`, `classic trailer set`, `classic clone`, `classic format`,
`uid set`, `type2 write`, `type2 restore`, and `ndef write`. Other commands
reject it instead of silently ignoring it.

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
for a card, and prints reader capabilities, UID, ATQA, SAK, and inferred type.
`card info` requires a card and prints the same facts.

Both `list` and `scan` expose the reader capability profile in JSON:
`driver`, `known`, `classicRead`, `classicWrite`, `type2Read`, `type2Write`,
`rawBytes`, `rawBits`, `apdu`, `nested`, `darkside`, `hardnested`, and
`validation`. Human output prints a compact capability line.

An operation the selected reader cannot perform fails before any card command
with `CodeUnsupported` (exit 4) and a message naming the missing capability.
Examples are Classic or Type 2 on the generic `pcsc` driver, and any raw-frame
workflow on a reader without raw support. The JSON error code remains
`hardware` because the reader cannot provide the operation, not because the
card failed.

An ACR122U may enumerate as direct USB and two PC/SC aliases. `list` keeps the
direct-USB entry and hides the matching PC/SC aliases. An explicitly passed
`--device` connstring is still honored, but the cross-process lock treats all
ACR122 connstrings as one physical reader.

`scan --watch` prints appearance/removal events until interrupted. It requires
human or `--jsonl` output; `--json` is rejected because a watch has no single
final document. Interrupting a watch exits 0.

### MIFARE Classic

```text
nfctl classic read --block <n> [--key A:HEX|B:HEX]
nfctl classic write --block <n> --data <HEX> [--key ...] --yes
nfctl classic dump <file.bin|file.mfd|file.json|file.mct|file.eml> [--keys <file>] [--scan]
nfctl classic export <file> --format raw|project|proxmark3|keyarray|mct|mct-json|eml|sidecar \
    [--keys <file>] [--scan]
nfctl classic restore <file.bin|file.mfd|file.json|file.mct|file.eml> [--keys <file>] [--scan] \
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

`dump` reads with all verified keys for the card and infers the output format
from an unambiguous extension. A `.json` path always stores an NFC Foundry
project, complete or partial. `.bin`/`.mfd` stores a complete raw image plus
`<name>.nfcx.json`; `.mct` and `.eml` store complete MCT text and EML text.
Incomplete images are rejected by every external format because unknown bytes
are never zero-filled. If the read task fails, no dump is written.

`export` performs the same card read and then writes an explicit format,
resolving the `.json` ambiguity. `raw` writes the image and sidecar, `project`
writes the NFC Foundry JSON, `sidecar` writes only `<file>.nfcx.json`,
`keyarray` writes the Proxmark3 raw key array, and
`proxmark3`/`mct-json`/`mct`/`eml` write the corresponding external format.
External formats are complete-image only. The `classic.dump` and
`classic.export` JSON result includes the selected `format`; `classic.restore`
includes import `warnings`.

| Format | Extension | Complete only | Preserves partial state |
|---|---|---|---|
| NFC Foundry project | `.json` | no | yes |
| Raw image + sidecar | `.bin`, `.mfd` | yes | no |
| Metadata sidecar | `.nfcx.json` | yes | no |
| Proxmark3 `mfc v2` JSON | `.json` | yes | no |
| Proxmark3 raw key array | `.bin` | yes | no |
| MCT `mfcard` JSON | `.json` | yes | no |
| MCT text | `.mct` | yes | no |
| EML text | `.eml` | yes | no |

`restore` auto-detects a raw image with or without its sidecar, an NFC Foundry
project, MCT text, EML text, MCT JSON, or Proxmark3 `mfc v2`/`mfc v3` JSON,
then runs the restore preflight and writes ordinary blocks before trailers,
verifying every block. The dump must carry metadata whose card UID/ATQA/SAK
matches the current card. MCT text, EML text, and MCT JSON without a `Card`
object carry no identity, so they load for inspection and export but cannot be
restored; import warnings are printed before the preflight. Manufacturer block
0 is always skipped. An actual restore requires `--yes` **and**
`--confirm-trailers`, because a full restore replaces keys and access
conditions. Before writing, it reads the current card into a complete JSON
backup under `$XDG_CONFIG_HOME/NFC Foundry/classic-backups/`; a failed backup
aborts the restore. The preflight output includes the backup path. `--dry-run`
prints the plan and does not create the backup or require either confirmation.
Loading a dump does not mark its embedded sector keys as verified for the
current card; run `classic keyscan` or pass `--keys` when the catalog does not
already hold a key for the target sector.

`keyscan` authenticates every known candidate for every Key A/Key B slot,
persists verified matches, and reports missing slots. Values appear only with
`--show-keys`.

`--keys <file>` imports a dictionary. For `dump`, `restore`, and
`nested`/`hardnested` recovery, it also runs a keyscan first so the imported
candidates become verified keys for the current card. `classic keyscan --keys`
uses the same scan it already performs. `--scan` runs the same keyscan without
importing a dictionary; it is off by default and cannot be combined with
`--keys`, which already scans.

A read or block operation that receives an authentication failure clears the
card-scoped verification for that slot while keeping the key record. The
verification is keyed by `UID/ATQA/SAK`; two physical cards with the same
selection identity cannot be distinguished, so the stale claim is removed
rather than hidden.

### Analysis, editing, and clone

```text
nfctl classic diff <before> <after> [--json]
nfctl classic value set <file> --block N --value V [--address N] \
    [--keys <file>] [--scan] [--yes] [--dry-run]
nfctl classic trailer set <file> --block N [--key-a HEX] [--key-b HEX] \
    [--access G0,G1,G2,G3] [--gpb HEX] --yes --confirm N \
    [--keys <file>] [--scan] [--dry-run]
nfctl classic clone <file> --yes --confirm <source-uid> \
    [--keys <file>] [--scan] [--dry-run]
nfctl classic format [--keep-data] --yes --confirm FORMAT \
    [--keys <file>] [--scan] [--dry-run]
```

`diff` loads both files without opening a reader. It reports changed, equal,
and unknown blocks, trailer Key A/Key B/access/GPB changes, and identity
changes. A block with any unknown byte is `unknown`, never equal to a known
zero block. The command never writes a card.

`value set` loads a dump, encodes a signed 32-bit value plus address byte,
enables the ordinary single-data-block write path, and verifies the read-back.
The address defaults to the block number. Inconsistent value-block copies are
rejected, and the editor never bypasses authentication or access conditions.

`trailer set` assembles the full trailer from Key A, the four access groups,
the GPB byte, and Key B. `--access` takes four 3-bit group codes such as
`000,000,000,001`; when omitted, the current decoded groups are used. The
command requires `--yes` and the trailer block number as `--confirm`. It saves
a complete pre-write backup, writes one protected trailer, and verifies the
replacement keys and access bytes.

`clone` requires a complete image with card identity and a target card on the
reader. It preflights capacity, BCC, trailers, and target keys, saves a
complete target backup, writes data blocks, then trailers, then block 0 only
when the source manufacturer block differs. Block 0 uses the existing
DirectWrite/Gen1A path; a genuine read-only block 0 fails and is never reported
as a successful clone. The target is read back, diffed against the source, and
verified. `--confirm` must repeat the source UID.

`format` restores factory transport configuration: Key A/B
`FFFFFFFFFFFF`, access bytes `FF 07 80`, GPB `69`, and user data zeroed unless
`--keep-data` is set. It requires a fully readable target, saves a complete
backup, never writes block 0, and verifies the read-back. `--confirm FORMAT`
is required.

`classic dump` reports readable blocks and skipped sectors instead of calling
a partial image complete. A partial read keeps unknown bytes unknown and only
supports the NFC Foundry project format.

### Key recovery

```text
nfctl recover nested --yes [--keys <file>] [--name NAME] [--key-set NAME] [--no-save]
nfctl recover darkside --yes [--name NAME] [--key-set NAME] [--no-save]
nfctl recover hardnested --yes [--keys <file>] [--name NAME] [--key-set NAME] [--no-save]
```

All modes require `--yes` as explicit authorization for active card testing.
Recovery persists its verified keys by default. `--key-set NAME` creates or
updates that named set for the current card. `--name NAME` names the recovered
key only when exactly one key slot was recovered; otherwise it is ignored with
a warning. `--no-save` keeps recovered keys in memory for this invocation only,
prints values only with `--show-keys`, and states that nothing was persisted.
It cannot be combined with `--name`, `--key-set`, or `--keys`.

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
the target UID. Before writing, the service reads the current card into a
complete backup under `$XDG_CONFIG_HOME/NFC Foundry/classic-backups/`; a failed
backup aborts the write. It also backs up block 0 under
`$XDG_CONFIG_HOME/NFC Foundry/uid-backups/`, tries the ordinary DirectWrite
manufacturer write, then falls back to the in-process Gen1A unlock on the same
open reader. The ordinary path covers CUID/Gen2, UFUID, and FUID cards. It
reselects the card and verifies the complete block 0 before reporting success.
The UFUID permanent-lock command is never sent; a locked or already-written FUID card
fails after reselection and keeps its original block-0 backup. The reader is
not released to an external process; `nfc-mfsetuid` is not required.

### Type 2 and NDEF

```text
nfctl type2 info
nfctl type2 read [--page N] [--pwd HEX] [--all]
nfctl type2 write --page N --data HEX [--pwd HEX] [--yes] [--dry-run]
nfctl type2 dump <file.json> [--pwd HEX]
nfctl type2 restore <file.json> [--pwd HEX] [--protected] [--yes] \
    [--confirm-protected] [--dry-run]
nfctl ndef read [--keys FILE] [--raw] [--reveal] [--pwd HEX]
nfctl ndef write (--file <path>|--hex <hex>|--template ID [--field K=V]...) \
    [--keys FILE] [--pwd HEX] [--yes] [--dry-run]
nfctl ndef format --sectors N-M [--keys FILE] [--yes] [--confirm-format] \
    [--dry-run]
nfctl apdu --raw HEX
nfctl desfire info
nfctl desfire apps
nfctl desfire files AID
nfctl desfire read AID FILE
nfctl type2 copy SOURCE [--include-protected] [--same-uid-only] [--repeat N] \
    [--continue-on-error] [--pwd HEX] [--yes] [--confirm-copy] \
    [--confirm-protected] [--dry-run]
nfctl type2 erase [--pwd HEX] [--yes] [--confirm-erase] [--dry-run]
nfctl type2 lock [--static-lock HEX4] [--dynamic-lock HEX6] [--cc-readonly] \
    [--cfglock] [--pwd HEX] [--yes] [--confirm-lock] [--dry-run]
nfctl type2 password set --new-pwd HEX --pack HEX [--auth0 N] [--no-protect] \
    [--current-pwd HEX] [--yes] [--confirm-password] [--dry-run]
nfctl type2 password remove [--current-pwd HEX] [--yes] \
    [--confirm-password] [--dry-run]
```

Type 2 commands use the standard selection signature (ATQA `00 44`, SAK `00`);
`info` and every read/recovery path refine the product with `GET_VERSION`.
NTAG213/215/216 are distinguished by the storage-size byte. The original
MIFARE Ultralight fallback is used only when `GET_VERSION` receives a protocol
NAK and page 0 has a valid BCC0.

`type2 info` prints the detected product, page count, user range, and version.
`type2 read` loads the full image and prints user pages by default; `--all`
also prints UID, lock, capability, configuration, password, and PACK pages.
`--page N` prints one page. PWD and PACK pages are masked as `--------` unless
`--show-keys` is set, and the optional `--pwd` value is never echoed. Type 2
dumps contain the PWD and PACK pages and are written with mode 0600; treat
them as sensitive.

`type2 dump` writes JSON by default and the self-describing `NFCT2BIN` binary
format when the path ends in `.bin`. `type2 restore` accepts either format.
`type2 write` accepts user-memory pages only. Protected pages are rejected;
they require a confirmed restore. `type2 restore` restores user memory by
default. `--protected` adds PWD/PACK, configuration, and dynamic-lock pages,
requires `--yes` and `--confirm-protected`, and writes dynamic lock bytes last.
A normal restore does not write pages 0-3. Lifecycle commands may write static
lock bits page 2 and the capability-container access byte page 3, with the
rules below. Every write creates a full-image backup under
`$XDG_CONFIG_HOME/NFC Foundry/type2-backups/` and verifies each page by
read-back. A restore is accepted only for the same product and UID.

`type2 copy` copies user memory and NDEF from a JSON or binary dump onto the
current tag. Source and target must be the same product; different UIDs are
the normal case. `--same-uid-only` rejects a different target UID.
`--include-protected` additionally copies PWD, PACK, configuration, and the
writable capability-container bits when the target allows it; it requires
`--confirm-protected`. UID pages are never written. `--repeat N` waits for a
new tag after each target and verifies every target independently. The default
stops at the first failure; `--continue-on-error` waits for the next tag and
reports both failures and successes.

`type2 erase` zeroes unlocked user-memory pages. It does not change lock,
configuration, PWD, or PACK pages and reports that those bits remain.

`type2 lock` applies explicit one-way lock targets. `--static-lock` accepts the
full 16-bit desired lock value (bit 0 is page 3, bit 1 is page 4, and so on);
`--dynamic-lock` accepts the three dynamic lock bytes. Existing set bits cannot
be cleared. `--cc-readonly` sets the capability-container access byte to
read-only, and `--cfglock` sets CFGLCK. Static lock is written last because it
can block later writes.

`type2 password set` writes PWD, PACK, AUTH0, and PROT. It protects user memory
by default; pass `--no-protect` to install the password without enabling PROT.
`--auth0` selects the first protected page and defaults to the first user page.
A protected target requires `--current-pwd`. `type2 password remove` restores
PWD `FF FF FF FF`, PACK `00 00`, AUTH0 `FF`, and clears PROT; it refuses when
CFGLCK makes the configuration pages read-only.

`ndef read` decodes and prints the record kind, summary, and fields. It covers
Text, URI, Smart Poster, vCard, MIME text/JSON, Wi-Fi WSC, Bluetooth BR/EDR
easy pairing, Bluetooth LE OOB, and Android Application Records. Sensitive
fields such as the Wi-Fi network key are masked unless `--reveal` is passed.
`--raw` prints only the message hex. It exits 3 when the tag has no valid
capability container or contains malformed NDEF. `ndef write` accepts a raw NDEF
message from `--file` or `--hex`, or a common template through `--template` and
repeatable `--field key=value` options. The template catalogue, provider
presets, language selector, Smart Poster metadata, and Bluetooth LE record are
shared with the GUI. It validates the record layout and TLV capacity, refuses
to overwrite a preceding lock/memory-control TLV, and verifies the complete
NDEF area by read-back.

`nfctl ndef` selects the implementation from the selected card. Type 4 tags
(SAK bit 5) use the NFC Forum NDEF application and capability container.
Type 2 tags use the capability container and user-memory TLV area. MIFARE Classic 1K/4K cards
use the MIFARE Application Directory: `03 E1` marks NDEF sectors, the TLV area
is the concatenation of their data blocks without trailers, and MAD2 may skip
sector 16. `--keys` imports a dictionary and runs the Classic key scan before
the operation; it is rejected for Type 2. Classic reads try verified catalog
keys first, then the standardized public MAD/NDEF Key A and the transport
default keys.

Classic `ndef write` changes only the NDEF data blocks, never sector trailers,
and saves a project dump under `$XDG_CONFIG_HOME/NFC Foundry/classic-backups/`
before the first write. Every block is verified by read-back. A read-only GPB
or absent NDEF application is refused without modifying the card.

Classic `ndef format` is the Advanced/confirmed path that creates or extends an
NDEF application. `--sectors` is one contiguous range on one side of sector 16,
for example `1-3` or `17-19`. A blank transport-state card receives a MAD1;
an existing MAD may only select free (`0000`) or already-NDEF sectors. The
formatter writes data blocks, then NDEF sector trailers, then MAD blocks, and
the MAD sector trailer last. It preserves the current Key B and refuses when a
trailer must change but Key B is secret and not verified. A project dump of
the whole card is read and saved before the first write, and the final MAD/TLV
area is re-read and compared. The GUI requires the card UID to be typed; the CLI requires both
`--yes` and `--confirm-format`.

### Diagnostics and keys

```text
nfctl diag
nfctl keys list [--show-keys]
nfctl keys add <A:HEX|B:HEX|HEX>
nfctl keys import <file> [--set NAME] [--format dic|keys|keyarray]
nfctl keys export <file> [--set NAME] [--format dic|keys]
nfctl keys rename <key-id> <name>
nfctl keys sets
nfctl keys set create <name> --key <id>... [--card <cardId>]
nfctl keys set rename <name> <new-name>
nfctl keys set delete <name> --yes
nfctl keys set export <name> [--format dic|keys|json] [--output <file>]
nfctl keys reload
```

`diag` runs the shared release self-check and prints each check. It exits 0
when the report was produced; scripts inspect the `healthy` field.

`keys list` omits values unless `--show-keys`. `keys add` strips an optional
`A:`/`B:` prefix, persists the catalog with mode 0600, and does not echo the
value. `keys import` reports valid/added/duplicate/ignored counts and per-line
issues. `--format dic` and `--format keys` read text dictionaries.
`--format keyarray` reads a Proxmark3 raw key array (`hf-mf-<uid>-key.bin`):
192 bytes for a 1K card or 480 bytes for a 4K card, with every sector's Key A
in the first half and Key B in the second half. Set `--format keyarray` and
`--set NAME` together to put all array keys into a named key set; existing
duplicates are reported. `--set NAME` also adds every valid unique key from a
text file to a global named key set; an existing set with the same name is
extended. `keys export` writes a 0600 value-only dictionary. `--set` exports
that key set instead of the complete catalog. `--format dic` and
`--format keys` produce the same text line content; the extension selects the
Proxmark3 or MCT vocabulary. Export a raw key array from a complete card dump
with `classic export FILE --format keyarray`; a key set is not a sector map
and cannot be rendered as one.

`keys rename` sets or clears a key's display name. `keys sets` lists named key
sets, their scope, member count, and source. `keys set create` references keys
by ID from `keys list`; `--card` takes the store's `UID/ATQA/SAK` identity and
without it the set is global. Recovery-created sets are scoped to that identity. `keys set export --format dic` and `--format keys` write one
12-digit key per line. `--format json` writes the explicit key-set exchange
object with values and names. Writing key values to stdout requires `--output`
or `--show-keys`. `keys reload` rebuilds the in-memory store from `keys.json`
and reports pruned members.

The internal store is authoritative and now writes version 2 with key names and
key sets. Version 1 files load without migration. Dictionary files and key-set
exports are exchange artifacts, not the source of truth. Key values remain
sensitive: full values appear only with `--show-keys`, and exported files are
mode 0600.

Keys and Type 2 PWD/PACK pages are sensitive. Human output reveals full key
values and raw engine logs only with `--show-keys`. Explicit export commands
write key material to mode 0600 files; `keys set export` to stdout also
requires `--show-keys`. No other flag or log level reveals key material.

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
  `keyScan`, `keys`, `sets`, `keySets`, `recovered`, `persisted`, `keyName`,
  `keySetName`, `prunedKeyIds`, `droppedKeySets`, `product`, `productName`,
  `pages`, `page`, `pageCount`,
  `userPages`, `cc`, `mad`, `application`, `family`, `type4cc`, `ats`, `sw`,
  `messageLength`, `ndef`, `protection`, `backup`, `planned`, `written`, and
  `skipped`. `product`, `productName`, and
  `userPages` are top-level for `type2 info` and nested under `info` in a full
  dump.
- `command` is one of `list`, `scan`, `scan.watch`, `card.info`,
  `classic.read`, `classic.write`, `classic.dump`, `classic.restore`,
  `classic.keyscan`, `recover.nested`, `recover.darkside`, `recover.hardnested`,
  `type2.info`, `type2.read`, `type2.write`, `type2.dump`, `type2.restore`,
  `ndef.read`, `ndef.write`, `ndef.format`, `apdu.raw`, `desfire.info`,
  `desfire.apps`, `desfire.files`, `desfire.read`, `uid.set`, `diag`,
  `keys.list`, `keys.add`, `keys.import`, `keys.export`, `keys.rename`,
  `keys.sets`, `keys.set.create`, `keys.set.rename`, `keys.set.delete`,
  `keys.set.export`, `keys.reload`.

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
| `nfctl apdu --raw <HEX>` | Phase 07 (ISO14443-4 transport) |

An unknown command exits with code 2 and prints usage. Card emulation is not
exposed by the initial CLI; Phase 09 defines its surface after target-mode
research.

## Explicitly out of scope

- relay/DEP/anticol/SAM demonstration commands;
- barcode experiments;
- generic "write everything writable";
- translated CLI output.


### ISO/IEC 14443-4, DESFire, and Type 4 NDEF

```text
nfctl apdu --raw HEX [--json]
nfctl desfire info|apps|files AID|read AID FILE [--json]
nfctl ndef read [--raw] [--reveal] [--json]
nfctl ndef write (...) [--yes] [--dry-run] [--json]
```

`apdu` activates ISO-DEP with RATS, sends exactly one short command APDU, and
prints the ATS, response data, and status word. It is a diagnostic path, not a
write interface.

`desfire` is read-only. `info` prints `GetVersion`; `apps` prints native
application IDs; `files AID` prints file types, communication modes, access
rights, and sizes; `read AID FILE` reads a standard or backup file. AID is six
hexadecimal digits. MAC-protected and enciphered files are refused because no
authentication is implemented.

Type 4 `ndef read` validates the capability container and decodes the NDEF
file. Type 4 `ndef write` requires free write access, saves the previous raw
message under `$XDG_CONFIG_HOME/NFC Foundry/type4-backups/`, writes NLEN and the
message in chunks, and verifies the complete file. `--dry-run` validates the
message and capacity without writing.

No PPS, CID/NAD, Type B, extended APDU, DESFire authentication, DESFire write,
or protected-file read is implemented. Hardware validation on Type 4/DESFire
cards is pending.
