# Phase 05f spec: NDEF UX and format completion

Date: 2026-11-07

## Goal

Finish the NDEF workflows deferred by Phase 05e: password-protected GUI
access, secret reveal, raw and multi-record composition, file import/export,
template parity between the GUI and CLI, language selection, provider presets,
Bluetooth LE OOB, Smart Poster metadata, binary Type 2 dumps, and per-record
raw/copy actions.

## Application model

### Sensitive fields

`type2.Field` gains `Sensitive bool`. Decoders keep the real value and mark
fields whose display must be masked: Wi-Fi `network_key`, and any future
credential field. CLI human output prints `••••••` unless `--reveal` is passed.
JSON output keeps the value because the tag's raw payload is already present in
the same DTO; the GUI masks it until the user reveals it.

### Message composition

`type2.RecordInput` describes one record for the multi-record builder:

```go
type RecordInput struct {
    Kind   string
    Fields map[string]string
}
```

`EncodeInputs([]RecordInput)` builds a validated message with correct MB/ME
flags. Supported kinds are the single-record template kinds plus `external` and
`mime`; Smart Poster keeps its nested URI/title/action/type/size records.

Raw editing accepts a complete NDEF message as hexadecimal. Both raw and
multi-record messages go through `ParseMessage` before they reach the write
path, so they share the template validation, backup, and read-back rules.

### Templates

- `text` gains an optional `language` select, default `en`.
- `smartposter` gains `language`, `action` (`do`, `save`, `edit`),
  `resource_type`, and `resource_size`.
- `bluetooth-le` adds a Bluetooth LE OOB record with address, address type
  (`public`, `random`), optional name, and optional role.
- `search` gains `search_provider`; when `search_prefix` is empty the provider
  preset supplies it. The prefix remains editable.
- `social` and `payment` gain provider presets and a handle. When `uri` is
  empty, `provider + handle` produces the URI. A non-empty `uri` always wins.

Provider preset IDs and prefixes live in `type2` so the GUI and CLI cannot
diverge. Localized labels live in the i18n catalogue.

### Bluetooth LE OOB

MIME `application/vnd.bluetooth.le.oob`. The payload is an AD structure list:

- `0x1B` LE Bluetooth Device Address: six address octets in wire order
  (little-endian text address) plus address type (`0x00` public, `0x01`
  random);
- `0x09` complete local name when supplied;
- `0x1C` LE Role when supplied (`0x00` peripheral only, `0x01` central only,
  `0x02` both).

The decoder reverses the address into the usual `AA:BB:CC:DD:EE:FF` display.

### Smart Poster metadata

Nested records are emitted in this order: URI, Action (`act`), Type (`t`),
Size (`s`), Title (`T`). Action carries one byte (`0` do, `1` save, `2` open).
Size carries four big-endian bytes. Type carries a MIME type string. The
decoder adds `action`, `resource_type`, and `resource_size` fields and keeps
backward compatibility with URI/title-only posters.

## Binary dump

`type2.Document` gains `MarshalBinary`/`UnmarshalBinary` with a versioned
header:

```text
"NFCT2BIN"   8 bytes
version      1 byte (1)
product      1 byte (0 ultralight, 1 NTAG213, 2 NTAG215, 3 NTAG216)
total pages  2 bytes big-endian
UID length   1 byte
UID          N bytes
ATQA         2 bytes
SAK          1 byte
pages        totalPages * 4 bytes
```

The loader validates product geometry, page count, UID length, BCC0/BCC1, and
CC capacity with the same rules as JSON. `type2 dump FILE.bin` and
`type2 restore FILE.bin` select the binary format by content or `.bin`
extension. JSON remains the default for `.json` and unknown extensions.

## GUI

- A masked password field supplies PWD_AUTH to reads and writes. It is cleared
  on card removal, device change, and Advanced-mode reset.
- The NDEF editor has three modes: Template, Raw message, and Multi-record.
  Raw input is validated on preview; the multi-record builder supports adding,
  removing, and reordering Text, URI, and Smart Poster records.
- Import/Export handles raw NDEF message files. Import validates before loading
  the editor.
- A reveal control unmasks sensitive decoded fields. The existing global raw
  toggle remains, and each record gains its own raw reveal plus copy actions
  for decoded fields and raw payloads.
- Search, social, and payment templates show provider presets; selecting one
  fills the editable URL. The text and Smart Poster templates show a language
  selector.
- Type 2 dump export/import chooses JSON or binary from the file extension.

## CLI

```text
nfctl ndef read [--raw] [--reveal] [--pwd HEX]
nfctl ndef write (--file F|--hex H|--template ID [--field K=V]...) \
    [--pwd HEX] [--yes] [--dry-run]
nfctl type2 dump FILE.json|FILE.bin [--pwd HEX]
nfctl type2 restore FILE.json|FILE.bin [--pwd HEX] [--protected] \
    [--yes] [--confirm-protected] [--dry-run]
```

`--field` is repeatable. `--template` and `--field` use the same
`type2.EncodeTemplate` catalogue as the GUI. Supplying another message source
with `--template` is a usage error.

## Tests

- Text language, provider preset, Bluetooth LE, and Smart Poster metadata
  round-trips.
- Multi-record encoding flags and validation.
- Raw message validation and sensitive-field masking/reveal.
- Binary dump round-trip, truncation, bad magic, and product/UID validation.
- CLI `--template` parity for each preset family.
- GUI password propagation, editor modes, import/export, reveal, and copy
  actions.

## Non-goals

- Chunked records and NDEF signatures.
- WSC Password Token and WSC 3.0.
- Arbitrary MIME editor beyond the existing external/MIME inputs.
- DPP protocol handling; only URI records.
