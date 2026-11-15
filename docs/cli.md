# `nfctl` CLI contract

`nfctl` is the scripting surface for NFC Foundry. It shares `nfc-app` services
with the GTK workbench; every card and file operation has the same safety
preconditions on both surfaces.

The CLI is English-only. Help, errors, JSON keys, and progress messages are not
localized.

## Global options

Options may appear before or after the subcommand.

| Option | Meaning |
|---|---|
| `--json` | one JSON document on stdout |
| `--jsonl` | compact JSON Lines events and one final result on stdout |
| `--timeout <duration>` | total operation deadline: `500ms`, `30s`, `5m`, `1h`, or a sum such as `1m30s` |
| `--verbose` | reader and driver detail on stderr |
| `--dry-run` | print the plan and stop; mutation commands only |
| `--device <connstring>` | exact reader selection; overrides `NFC_FOUNDRY_DEVICE` and auto-selection |
| `--mock` | scripted in-memory readers instead of hardware |
| `--yes` | confirm destructive or active operations |
| `--key`, `--key-file`, `--key-store`, `--backup-dir` | key candidates, key files, key store, and backup directory |
| `--no-common-keys`, `--hardnested-tables`, `--max-nonces` | recovery and read tuning |
| `--type2-password`, `--type2-pack` | protected Type 2 credentials |
| `--amiibo-keys <path>` | 160-byte combined Amiibo key file, kept in memory |
| `--amiibo-data-key <path>`, `--amiibo-tag-key <path>` | the two 80-byte Amiibo key files instead of the combined file |

`--json` and `--jsonl` are mutually exclusive; clap rejects the pair with exit
code 2.

Human-readable results are written to stdout. Warnings, progress, notes, and
reader diagnostics are written to stderr in human and `--json` modes. `--jsonl`
writes progress events on stdout and keeps stderr for diagnostics.

`--dry-run` is accepted by `classic write`, `classic uid`, `classic seal`,
`classic restore`, `type2 uid`, `amiibo write`, `desfire format-ndef`,
`desfire replace-ndef`, `desfire format-picc`, and `desfire file-settings`. Every other command rejects it with
a usage error instead of ignoring it.

`--timeout` cancels the shared `CancelToken` at the deadline. The cancellation
is observed before the next reader exchange; a multi-second exchange already in
flight finishes first. Deadline expiry is an operation failure, not a user
cancel (see the exit-code table). One Ctrl-C requests cancellation; a second
Ctrl-C terminates the process immediately.

## Reader selection

A reader command resolves its reader in this order:

1. `--device <connstring>`;
2. the `NFC_FOUNDRY_DEVICE` environment variable;
3. the only enumerated supported reader;
4. no readers: exit 4 with `code: "no_reader"`;
5. several readers: exit 2 with `code: "usage"` and the candidate connstrings.

Only the first present source participates. An invalid `--device` or
`NFC_FOUNDRY_DEVICE` value exits 2 instead of falling back to auto-selection;
an empty environment value counts as unset. `--device` is the exact connstring
reported by `list`:

```text
pn532_uart:/dev/ttyUSB0
acr122_usb:001:005
acr122_pcsc:ACS ACR122U 00 00
mock:classic-1k
```

`LIBNFC_DEVICE` is not read. The migration is a rename:

```sh
LIBNFC_DEVICE=pn532_uart:/dev/ttyUSB0 ...
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 ...
```

Reader enumeration failures, a missing card, and reader timeouts exit 4.

The GUI and CLI take one exclusive advisory lock per physical reader, under
`$XDG_RUNTIME_DIR/nfc-foundry` (or a per-user temporary directory when the
runtime directory is unset). The lock is acquired before the reader opens and
held until the connection closes. A second process on the same physical reader
exits 4 with `code: "reader_busy"`; the message names the holder when its
record is available. Two different connstrings for one reader, such as a PN532
`/dev/ttyUSB0` path and its `/dev/serial/by-id` alias, share the lock.

## Exit codes

| Code | Meaning |
|---:|---|
| 0 | success, including `--help` and `--version` |
| 1 | operation failed: I/O, authentication, verification, or deadline expiry |
| 2 | usage or argument error: bad flags, malformed hexadecimal, missing `--yes`, unknown reader |
| 3 | validation failure: dump layout, BCC, access bits, write plan, card mismatch |
| 4 | permission, reader busy, or hardware unavailable: no reader, no card, reader timeout, unsupported reader capability |
| 5 | reserved for external-engine validation; never emitted while recovery is native-only |
| 6 | cancelled by the user or a signal |
| 7 | internal error: broken invariant, serialization failure, not-implemented path |

Clap help and version print their own text and exit 0. Other parse failures
exit 2 and, when `--json` or `--jsonl` is present before the failing argument,
still produce a machine envelope in that mode.

## Error codes

The error envelope carries one stable `code` and the numeric `exitCode`.

| `code` | Exit | Source |
|---|---:|---|
| `usage` | 2 | CLI argument or confirmation precondition |
| `validation` | 3 | `NfcError::InvalidArgument` from a service, or a CLI card/dump validation finding |
| `card_changed` | 3 | `NfcError::CardChanged` |
| `io` | 1 | `NfcError::Io` |
| `authentication` | 1 | `NfcError::AuthenticationFailed` |
| `authentication_required` | 1 | `NfcError::AuthenticationRequired` |
| `verification` | 1 | `NfcError::VerificationFailed` |
| `cancelled` | 6 | `NfcError::Cancelled` from a user interrupt |
| `deadline_exceeded` | 1 | `NfcError::Cancelled` after `--timeout` expiry |
| `no_card` | 4 | `NfcError::NoCard` |
| `no_reader` | 4 | `NfcError::NoReader`, or no enumerated reader during selection |
| `reader_busy` | 4 | `NfcError::ReaderBusy` from the cross-process reader lock |
| `timeout` | 4 | `NfcError::Timeout` from a reader exchange |
| `unsupported` | 4 | `NfcError::Unsupported` |
| `not_implemented` | 7 | `NfcError::NotImplemented` |
| `internal` | 7 | CLI invariant or serialization failure |

Classification is typed. Text is never inspected to choose an exit code.

## JSON envelope

`--json` writes exactly one document on stdout:

```json
{
  "ok": true,
  "command": "classic.read",
  "device": "pn532_uart:/dev/ttyUSB0",
  "card": {"uid": "04AABBCC", "atqa": "0004", "sak": "08", "type": "mifare_classic_1k"},
  "warnings": [],
  "error": null,
  "result": {"block": 4, "hex": "00 01 02 ...", "ascii": "................"}
}
```

- `command` is the stable leaf name from [Command names](#command-names).
- `device` is the exact connstring when a reader was selected, otherwise
  `null`.
- `card` is the selected card identity, otherwise `null`. UID, ATQA, and SAK
  are uppercase hexadecimal without separators; `type` is one of `unknown`,
  `mifare_classic_1k`, `mifare_classic_4k`, `mifare_mini`, `type2`, or
  `iso14443a`.
- `warnings` is always an array.
- `error` is `null` on success, otherwise `{"code", "message", "exitCode"}`.
- `result` is `null` on failure and a command-specific object on success.

JSON keys are unordered; consumers must not depend on document order. JSON data
uses uppercase hexadecimal. Block and page payloads use space-separated hex;
UIDs use compact hex.

### Errors

```json
{
  "ok": false,
  "command": "classic.read",
  "device": "mock:classic-1k",
  "card": null,
  "warnings": [],
  "error": {
    "code": "usage",
    "message": "block 0 writes are not enabled",
    "exitCode": 2
  },
  "result": null
}
```

### JSONL

Each line is one compact object.

```json
{"type":"progress","phase":"keyscan","message":"sector 1/16, 2 attempts, 2 verified","sector":1,"sectorCount":16,"attempts":2,"verified":2,"progress":6.25,"elapsed_ms":0}
{"type":"card","event":"appeared","card":{"uid":"04A2248A","atqa":"0004","sak":"08","type":"mifare_classic_1k"}}
{"type":"result","ok":true,"command":"classic.keyscan","device":"mock:classic-1k","card":{...},"warnings":[],"error":null,"result":{...}}
```

- `progress` carries `phase`, `message`, `elapsed_ms`, and optional numeric
  fields and `progress` (percent).
- `card` is emitted by `scan --watch` on appearance and removal.
- `result` is the last line and repeats the JSON envelope fields.

Recovery progress is rate-limited: phase changes and at most one update every
two seconds (acquire adds a 64-frame threshold). Key scan emits one event per
scanned sector. Progress lines are diagnostic; the terminal `result` line is
authoritative.

## Secrets

Machine output never contains:

- MIFARE keys or verified key values (`classic keyscan`, `classic keys list`,
  recovery);
- Type 2 PWD/PACK values (`type2 password`) and Amiibo key bytes;
- DESFire key material (`desfire change-key`, `desfire verify-key`,
  `desfire uid`);
- the request frame of `pn532 shell` and the request APDU of `apdu --raw`,
  because a diagnostic exchange can carry key material;
- raw dump or file payloads from `classic dump`, `type2 dump`, `amiibo
  read`, or DESFire backups. `amiibo decrypt` and `amiibo encrypt` write the
  requested file and report only its path; `desfire read` returns the data it
  was asked to read.

Explicit exports remain explicit:

- `classic keys export` writes key material only to `--output`. Without
  `--output` it refuses in `--json`/`--jsonl` mode and exits 2.
- `ndef read --output`, `classic ndef read --output`, and `desfire ndef-read
  --output` write the encoded message to the named file and report its path.
- `desfire change-key --export PATH` writes the new key to that file before the
  change; the envelope reports the path only.

`--verbose` adds reader, driver, and retry detail. It never adds card payloads
or key values.

## Command names

```text
list
scan
scan.watch
card.info
card.health
classic.read
classic.dump
classic.write
classic.uid
classic.restore
classic.keyscan
classic.keys.list
classic.keys.import
classic.keys.add
classic.keys.delete
classic.keys.export
classic.diff
classic.ndef.info
classic.ndef.read
classic.ndef.write
classic.ndef.format
type2.info
type2.read
type2.write
type2.dump
type2.restore
type2.lock
type2.password
type2.uid
tunion.read
emv.read
amiibo.read
amiibo.decrypt
amiibo.encrypt
amiibo.write
desfire.info
desfire.format_ndef
desfire.delete_ndef
desfire.replace_ndef
desfire.format_picc
desfire.ndef_read
desfire.verify_key
desfire.uid
desfire.read
desfire.write
desfire.value
desfire.record
desfire.memory
desfire.app_create
desfire.app_delete
desfire.file_create
desfire.file_delete
desfire.file_settings
desfire.key_settings
desfire.change_key
ndef.read
ndef.write
ndef.format
recover.auto
recover.darkside
recover.nested
recover.static_nested
recover.hardnested
recover.fm11rf08s
diag
# developer-tools builds only:
pn532.shell
apdu.raw
```

## Developer commands

`pn532 shell` and `apdu --raw` exist only in a build with the non-default
`developer-tools` Cargo feature. Release packages and AUR builds do not enable
it, so the commands are absent from their help, parser, and command names.
They are diagnostics, not supported card paths, and the GUI has no entry for
either.

```sh
cargo build -p nfctl --features developer-tools
```

### `pn532 shell`

Interactive native command console for a PN532 UART reader. It resolves the
reader like every other command and holds one exclusive diagnostic lease for
the session; no card is required.

- One input line is one native command frame: hexadecimal command code and
  parameters, with spaces or `:` allowed.
- `help` or `?` prints the one-line usage; `quit`, `exit`, and EOF end the
  session.
- Replies print as space-separated hexadecimal, `(empty)` for an empty body,
  and a failed command prints `error: …` without ending the session.
- The session prints
  `warning: developer console: raw PN532 commands bypass card workflows and can leave the reader in an unknown state`.
- Only human output is supported. `--json` and `--jsonl` exit 2; `--dry-run`
  is rejected.

The console keeps the command timeout, ACK, framing, and response validation of
the driver. It never prints key material, Type 2 credentials, or the request
frame. ACR122U readers have no native command console.

### `apdu --raw <HEX>`

Selects the card and sends one complete ISO 7816-4 command APDU, with no
retry. The card must be ISO/IEC 14443-4.

- Warning in human mode and in the envelope `warnings`:
  `raw APDU is a developer diagnostic, not a safe write path`.
- Human result: `data: <hex>` or `data: (none)`, then `sw: XXXX`.
- JSON result: `data`, `sw`, and `statusOk`; the envelope `card` carries the
  selected card identity.
- The request APDU is never echoed; an experimental APDU can carry key
  material.

`data` is the comma-free compact hex of the response data. A malformed APDU is
a usage error (2); a non-ISO-DEP card or a reader without APDU support is an
unsupported/hardware error (4).

## Result shapes

Fields are additive. A command may gain fields; existing names and types stay
stable. Fields marked `null` may be absent in a human run but are always present
in JSON.

### Discovery and self-check

| Command | `result` fields |
|---|---|
| `list` | `devices`: `[{name, connString}]` |
| `scan`, `card.info` | `card`: the envelope card identity |
| `card.health` | `path`, `format`, `layout`, `identity`, `readableBlocks`, `totalBlocks`, `readableSectors`, `totalSectors`, `complete`, `skippedSectors`, `keyCoverage`, `device`, `startedAt`, `finishedAt`, `dirtyBlocks`, `warnings`, `errors`, `valid` |
| `diag` | the self-check document: `app`, `platform`, `drivers`, `readers`, `pcsc`, `hardnested_tables` |

### MIFARE Classic

| Command | `result` fields |
|---|---|
| `classic.read` | `block`, `hex`, `ascii` |
| `classic.dump` | `path`, `format`, `bytes`, `identityOnly`, `layout`, `blocksReadable`, `blocksTotal`, `sectorsReadable`, `sectorsTotal`, `skippedSectors`, `sidecarPath`, `keysPath` |
| `classic.write` | `block`, `changed`, `verified`, `backup`; dry run: `dryRun: true`, `plan` |
| `classic.uid` | `changed`, `uidBefore`, `uidAfter`, `block0`, `backend`, `backup`; dry run: `dryRun: true`, `plan` |
| `classic.restore` | `dryRun`, `written`, `unchanged`, `unknownSkipped`, `notSelected`, `planned`, `trailers`, `backup`, `failures`, `blocks`, `uid`; dry run: `dryRun: true`, `plan` |
| `classic.keyscan` | `attempts`, `verified`, `cancelled`, `sectors`: `[{sector, keyA, keyB}]`, `saved`, `captured` |
| `classic.keys.list` | `entries`: `[{sector, keyType, source}]`, `commonKeys`, `builtinListed` |
| `classic.keys.import` | `path`, `added`, `duplicates`, `ignored`, `issues` |
| `classic.keys.add` | `path`, `added` |
| `classic.keys.delete` | `path`, `deleted` |
| `classic.keys.export` | `path`, `format`, `bytes`, `keys`, `scope` (`stored` or `all_candidates`); requires `--output` in machine modes |
| `classic.diff` | `layout`, `changed`, `equal`, `unknown`, `trailerChanges`, `identityChanged`, `blocks` |
| `classic.ndef.info`, `classic.ndef.read` | `ndef`: `{mad, state, capacity, messageBytes, records}`; read adds `output` |
| `classic.ndef.write` | `writtenBlocks`, `changedBlocks`, `backup`, `ndef` |
| `classic.ndef.format` | `writtenBlocks`, `trailers`, `backup`, `ndef` |

`classic.write` and `classic.restore` plans contain `layout`, `uid`,
`selection`, `blocks`: `[{block, sector, role, trailer, old, new}]`,
`unchanged`, `unknownSkipped`, `trailers`, `expiresAt`, and the optional `uid`
plan.

### Type 2 and NDEF

| Command | `result` fields |
|---|---|
| `type2.info` | `product`, `uid`, `pagesRead`, `pageCount`, `userFirst`, `userLast`, `userBytes`, `cc`, `ndef`, `locks`, `configuration` |
| `type2.read` | one page: `page`, `role`, `hex`; all pages: `pages`: `[{page, role, hex}]` |
| `type2.write` | `page`, `changed`, `verified`, `hex`, `previous`, `backup` |
| `type2.dump` | `path`, `product`, `uid`, `pagesRead`, `pageCount`, `bytes` |
| `type2.restore` | `written`, `unchanged`, `backup` |
| `type2.lock` | `pages`, `alreadyLocked`, `staticChanged`, `dynamicChanged`, `backup` |
| `type2.password` | `auth0`, `access`, `verified`, `backup` |
| `type2.uid` | `changed`, `uidBefore`, `uidAfter`, `pages`, `backup`; dry run: `dryRun`, `currentUid`, `targetUid`, `pages` |
| `ndef.read` | `ndef`, `output` |
| `ndef.write` | `written`, `unchanged`, `backup` |
| `ndef.format` | `written`, `unchanged`, `otp`, `backup` |

NDEF objects use `state` (`unformatted`, `empty`, `message`, or `invalid`),
`capacity`, `messageBytes`, `readOnly`, and `records`. A record is
`{index, tnf, type, payload}` plus decoded `kind` (`text`, `uri`, `wifi`) and
its fields when the payload matches a standard record.

### China T-Union

`nfctl tunion read` selects the PPSE, selects `A000000632010105`, reads the
electronic purse balance, and reads up to ten transaction records (SFI `0x18`)
and thirty travel records (SFI `0x1E`). The command is read-only: it has no
credit, purchase, top-up, PIN, key, or write path, and `--dry-run` is
rejected.

Human output prints the card facts, AID, label, application version, card
type, city, masked card number, issue/expiry dates, balance, and one line per
record. `--json` result keys:

| Key | Meaning |
|---|---|
| `aid` | Selected application AID, uppercase hex |
| `label` | Application label from the FCI or PPSE |
| `appVersion` | `9F08` value, uppercase hex, or `null` |
| `cardType` | `normal`, `student`, `elder`, `test`, `soldier`, or `unknown` |
| `cityId` | City ID, uppercase hex |
| `cardNumber` | Card number masked to the last four digits |
| `issued`, `expires` | `YYYY-MM-DD` or `null` |
| `balanceCents` | Electronic purse balance in CNY cents |
| `currency` | `CNY` |
| `complete` | `false` when a record file stopped early or a record failed to decode |
| `transactions[]` | `sequence`, `amountCents`, `kind`, `kindCode`, `terminalId`, `timestamp` |
| `travels[]` | `kind`, `kindCode`, `terminalId`, `subType`, `stationId`, `amountCents`, `balanceCents`, `timestamp`, `cityId`, `institutionId` |

Timestamps are `YYYY-MM-DD HH:MM:SS` or `null` when the BCD field is invalid.
A non-ISO-DEP card exits 4 with `unsupported`; a non-T-Union ISO-DEP card
exits 4 with a message that the T-Union application is absent. The full card
number, balance, terminal IDs, and records are never written to logs or
project files.

### EMV payment card

`nfctl emv read` selects `2PAY.SYS.DDF01`, selects the highest-priority
application, sends Get Processing Options with a zero amount, and reads the
records named by the AFL. It prints the card facts, application list, selected
AID/label, masked PAN, expiry, cardholder name when present, AIP/ATC facts, and
record count. The command is read-only: it has no GENERATE AC, PIN, balance,
statement, top-up, purchase, or write path, and `--dry-run` is rejected.

The card may include an application cryptogram in its GPO response and may
advance its ATC; the cryptogram is discarded and never sent to an issuer. The
full PAN and Track 2 bytes are never printed or persisted.

`--json` result keys:

| Key | Meaning |
|---|---|
| `aid` | Selected application AID, uppercase hex |
| `label` | Application label from the FCI or PPSE |
| `priority` | PPSE priority byte, or `null` |
| `appVersion` | `9F08` value, uppercase hex, or `null` |
| `applications[]` | Every PPSE entry: `aid`, `label`, `priority` |
| `pan` | Primary account number masked to the last four digits, or `null` |
| `cardholderName` | `5F20` printable name, or `null` |
| `expires`, `effective` | `YYYY-MM-DD` or `YYYY-MM` for month-only expiry, or `null` |
| `panSequence` | `5F34` value, or `null` |
| `serviceCode` | Three-digit Track 2 service code, or `null` |
| `aip` | AIP uppercase hex |
| `aipCapabilities` | Decoded AIP labels |
| `atc` | Application transaction counter, or `null` |
| `issuerCountry` | `5F28` uppercase hex, or `null` |
| `applicationUsage` | `9F07` uppercase hex, or `null` |
| `recordsRead` | Number of AFL records read |
| `complete` | `false` when a record read or the PAN is missing |

A non-ISO-DEP card exits 4 with `unsupported`; a card without a PPSE or a
payment application also exits 4. Warnings stay in the envelope and on stderr.
The masked PAN and cardholder name are cardholder data; do not add them to
logs or project files.

### Amiibo

```sh
nfctl amiibo read --output tag.bin
nfctl --amiibo-keys key_retail.bin amiibo read
nfctl --amiibo-keys key_retail.bin amiibo decrypt --input tag.bin --output tag.plain.bin
nfctl --amiibo-keys key_retail.bin amiibo encrypt --input tag.plain.bin --output rebuilt.bin
nfctl --amiibo-keys key_retail.bin amiibo write --input tag.bin --advanced --dry-run
nfctl --amiibo-keys key_retail.bin amiibo write --input tag.bin --advanced --yes
```

`read` requires a live NTAG215. `decrypt` and `encrypt` are file-only.
`write` requires `--advanced` in every mode, re-signs the source for the
selected tag's UID, and writes pages 4..129, the CC, PWD/PACK, both lock pages,
and the configuration. It refuses a locked target and writes a backup before
the first mutation. A dry run builds the same plan and writes nothing.

| Command | `result` fields |
|---|---|
| `amiibo.read` | `uid`, `product`, `pagesRead`, `pageCount`, `signature` (`valid` or `not_checked`), `identity`, `warnings`, `output` |
| `amiibo.decrypt` | `sourceUid`, `identity`, `input`, `output`, `bytes`, `warnings` |
| `amiibo.encrypt` | `sourceUid`, `identity`, `input`, `output`, `bytes` |
| `amiibo.write` | dry run: `dryRun`, `targetUid`, `sourceUid`, `identity`, `writePages`, `credentials`, `backup`, `warnings`; execution adds `verified`, `written`, `backup` |

`identity` is the numeric tag identity: `gameCharacterId`, `gameId`,
`characterId`, `variant`, `figureType`, `figureTypeLabel`, `modelNumber`,
`series`, `formatVersion`, `writeCount`, `appId`, and `initialized`. The
catalog names are never shipped; only the values from the user's dump are
reported. PWD and PACK are derived from the target UID and are not printed.

### DESFire

| Command | `result` fields |
|---|---|
| `desfire.info` | `version`, `applications`: `[{aid, files: [{fileId, settings}]}]` |
| `desfire.format_ndef` | `aid`, `cc`, `ndef`; dry run: `dryRun`, `aid`, `keyNumber` |
| `desfire.delete_ndef` | `deleted`, `applications` |
| `desfire.replace_ndef` | `aid`, `previous`, `cc`, `ndef`; dry run adds `current` |
| `desfire.format_picc` | `card`, `applicationsErased`, `applicationsAfter`, `freeMemory`; dry run: `dryRun`, `applications` |
| `desfire.ndef_read` | `ndef`, `output` |
| `desfire.verify_key` | `aid`, `keyNumber`, `keyType`, `secureMessaging`, `keyVersion` |
| `desfire.uid` | `aid`, `keyNumber`, `keyType`, `uid`, `integrity` |
| `desfire.read` | `aid`, `fileId`, `kind`, `settings`, `protection`, and `data` or `value` or `records` |
| `desfire.write` | `aid`, `fileId`, `written`, `offset`, `communication`, `integrity`, `committed`, `backup` |
| `desfire.value` | `aid`, `fileId`, `operation`, `amount`, `oldValue`, `newValue`, `committed`, `transactionMac` |
| `desfire.record` | `aid`, `fileId`, `operation`, `written`, `recordsBefore`, `recordsAfter`, `recordSize`, `committed`, `transactionMac` |
| `desfire.memory` | `aid`, `freeMemory` |
| `desfire.app_create` | `aid`, `keys`, `keyType`, `freeMemoryBefore`, `freeMemoryAfter`, `applications` |
| `desfire.app_delete` | `aid`, `manifest`, `freeMemoryBefore`, `freeMemoryAfter`, `applications` |
| `desfire.file_create` | `aid`, `fileId`, `settings`, `freeMemoryBefore`, `freeMemoryAfter`, `required` |
| `desfire.file_delete` | `aid`, `fileId`, `manifest`, `freeMemoryBefore`, `freeMemoryAfter`, `files` |
| `desfire.file_settings` | `aid`, `fileId`, `changed`, `before`, `after`; dry run: `dryRun`, `aid`, `fileId`, `current` |
| `desfire.key_settings` | `aid`, `settings`, `keyVersions`; a change adds `changed`, `before`, `after` |
| `desfire.change_key` | `aid`, `keyNumber`, `keyType`, `version`, `versionBefore`, `versionVerified`, `card`, `export` |

A file settings object is `{type, communication, access, contents}`.
`access` is `{raw, read, write, readWrite, change}`; `contents` is tagged by
`kind`: `data`, `value`, `record`, or `transaction_mac`.

### Recovery

`recover.*` returns `strategy`, `attempts`, `profile`, `verifiedKeys`,
`note`, and `keyStore`. `verifiedKeys` carries only `sector` and `keyType`
(or only `keyType` for a single-target attack). Key values are never in the
envelope.

### Developer diagnostics

| Command | `result` fields |
|---|---|
| `apdu.raw` | `data`, `sw`, `statusOk` |

`pn532.shell` is interactive and produces no machine result.

## Dump sidecars

Raw, MCT, and Proxmark3 images carry bytes only. `classic dump` also writes
`<image>.nfcx.json` beside the image with `version`, `complete`, `layout`,
`card`, `device`, `startedAt`, `finishedAt`, and sector `keys`. The sidecar is
metadata: image bytes remain authoritative, and a sidecar never changes a
block.

- Written by default for image-only formats, atomically with mode `0600`.
  `--no-sidecar` disables it; project exports keep metadata inline and never
  get a sidecar.
- A sidecar write failure is a warning; the image write still succeeds.
- `classic restore` and `card health` apply a matching sidecar to a raw, MCT,
  or Proxmark3 load. A missing sidecar is normal. A malformed sidecar, an
  unknown version, or a layout/completeness/card mismatch warns and loads the
  image without metadata.
- Project JSON never consults a sidecar. `classic diff` compares images only.
- `classic.keys export --all-candidates` writes the unique union of stored
  keys and the built-in dictionary, sorted by uppercase hexadecimal. The
  default remains stored-only. `--all-candidates` never mutates the store.
  JSON output has `{"version":1,"scope":"all_candidates","keys":[{"key","source"}]}`;
  `.dic` output is one key per line.

## Device and file side effects

- `classic dump` writes the image, an image-only `.nfcx.json` sidecar unless
  `--no-sidecar` is set, and, when keys were verified, a paired `.dic` key
  file. The envelope reports the image, sidecar, and key-file paths.
- `classic restore`, `classic write`, `classic uid`, `classic seal`, Type 2 writes and formats,
  NDEF writes and formats, and DESFire backups write a pre-change backup under
  the backup directory, `--backup-dir`, or `$XDG_DATA_HOME/nfc-foundry`.
- DESFire app/file deletion writes a manifest before the delete and reports its
  path.
- A write is reported as `verified: true` only after read-back.

## Compatibility

- `nfctl pn532 shell` and `nfctl apdu --raw` exist only in a `developer-tools`
  build. Release builds and packages do not compile or advertise them.
- The GUI and CLI share one cross-process lock per physical reader. A second
  process on the same reader exits 4 with `reader_busy`; the lock is released
  when the holder closes the reader or exits.
