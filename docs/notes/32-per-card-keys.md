# Per-card key files and XDG data directories

Date: 2026-11-12

## Data directories

User-facing files live under `$XDG_DATA_HOME/nfc-foundry` (default
`~/.local/share/nfc-foundry`):

| Directory | Contents |
|---|---|
| `dumps/` | Classic and Type 2 project images, raw/MCT/PM3 dumps |
| `keys/` | Per-card Proxmark3 `.dic` key files |
| `ndef/` | NDEF message files |
| `backups/` | Pre-write backups |

Configuration and the legacy JSON key store stay under
`$XDG_CONFIG_HOME/nfc-foundry`. File dialogs open the matching data directory;
the directory is created on first use.

## Built-in key groups

The old 57-key dictionary is split into curated groups:

- **Default** (checked on the first scan): `FFFFFFFFFFFF`,
  `A0A1A2A3A4A5`, `D3F7D3F7D3F7`. These match MCT's `std.keys`: NXP factory
  default, MAD public Key A, and NFC Forum NDEF public Key A.
- **Default extended** (unchecked): `000000000000`, `B0B1B2B3B4B5`,
  `89ECA97F8C2A`, `A5A4A3A2A1A0`. The blank key and the PM3-labeled NFC Forum
  MAD Key B values. NXP leaves MAD Key B issuer-defined, so these are not
  normative defaults.
- **EV1 signature** (unchecked): the four NXP MIFARE Classic EV1 hidden
  signature-block keys. They are not general sector keys.
- **Common** (unchecked): the rest of the built-in dictionary.

Reads use the three default keys when no stored key is available. The full
dictionary is reserved for an explicit key scan; a read is not a key search.

## Scan selection

The Key scan page opens a multi-select list grouped by the sets above, plus
loaded dictionary keys and any custom keys. The first run checks the three
default keys. The exact checked set is stored in `ui.json` and restored on the
next run. **Use defaults** resets to the three default keys; **Select keys…**
edits the set. The scan uses only checked keys.

## Dump/key pairing

A dump and its key file share a base name across the two directories:

```
dumps/home.bin
keys/home.dic
```

A complete image defaults to raw (`auto` resolves to raw). The project JSON
format (`home.nfc-foundry.json`) is used only when the image is partial or
identity-only: raw, MCT, and Proxmark3 formats cannot represent unknown bytes,
per-block status, or key/provenance metadata. The project JSON is therefore
not the default for a complete dump; it is the lossless format for an
incomplete one. The key pairing uses the dump stem regardless of the dump
extension.

Saving a dump also merges the keys used by the read into the paired `.dic`.
Loading a dump loads that file (falling back to a sibling `.dic` for older
layouts). The same rule applies to `nfctl classic dump` and
`nfctl classic restore --input`. A key file stores only keys verified for that
card; it is not a global inventory.

## Deduplication

`KeyStore::add` deduplicates by sector, key type, and key value, ignoring
provenance. Loading an old JSON store with repeated keys from different
sources collapses them to one entry. The `.dic` format is a flat unique key
list.

## CLI

- `nfctl classic dump --output <path>` writes `<path>` and
  `$XDG_DATA_HOME/nfc-foundry/keys/<stem>.dic`.
- `nfctl classic keyscan --save <name>.dic --name "..."` writes a named
  per-card `.dic`. A `.json` path keeps the legacy store format.
- `nfctl classic restore --input <dump>` loads the paired key file
  automatically.
- `--key` and `--key-file` remain explicit overrides.

## Legacy JSON

The global JSON key store is still readable and remains the backing store for
the Keys management page and `nfctl classic keys`. It is no longer the primary
per-card format. New scans default to `.dic`; JSON is accepted for migration
and for callers that explicitly ask for it.

## Testing

- `KeyFile` round-trips the PM3 text format with `# name:` and `# card:`
  comments.
- `KeyStore` deduplicates by key identity rather than source.
- CLI tests cover `.dic` save, sibling/paired dump key files, and legacy JSON.
- GTK tests cover the selected-key scan and the per-card `.dic` header.

The layout was exercised on 2026-11-12 with a PN532 UART: the source card
"饭卡" and its Xiaomi phone emulation used `dumps/饭卡.json` +
`keys/饭卡.dic` and `dumps/phone.json` + `keys/phone.dic`; the phone read
64/64 blocks, a data block was written and restored, and a full diff reported
0 changed / 64 equal. See `26-phone-emulation.md` for the hardware record.
