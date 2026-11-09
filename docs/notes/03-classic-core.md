# Phase 03 note: MIFARE Classic core

Date: 2026-11-09

## Implemented

- `Layout` covers Classic 1K and 4K, including the sixteen-block large 4K
  sectors. It maps blocks to sectors, first blocks, trailers, and access groups
  and exposes the stable `classic_1k` and `classic_4k` names used by dump files.
- `bcc` calculates the four-byte UID XOR check byte and validates manufacturer
  block 0. Results are `Valid`, `Invalid { expected, actual }`, or
  `NotApplicable` for unknown, seven-byte, and ten-byte UIDs.
- `access` decodes and encodes the redundant access-bit representation and
  implements data read/write/increment/decrement permissions, trailer write
  permissions, and Key B readability for all eight codes.
- `value` encodes the canonical value block and decodes only when the repeated
  value, inverted-value, address, and inverted-address copies all agree.
- `Dump` stores a per-byte known mask, block status, optional card identity, and
  an optional error string. It provides exact raw import/export, geometry
  validation, readability, access-byte decoding, and trailer validation.
- `diff` compares two images block-wise. A block with any unknown byte is
  `Unknown`; complete equal blocks are `Equal`; complete differing blocks are
  `Changed`. Byte differences, trailer Key A/Key B/access/GPB changes, and
  identity changes are reported separately.
- `exchange` detects and handles four formats:
  - project JSON preserves partial images, known masks, statuses, errors, and
    card identity;
  - raw images are exact 1024/4096-byte images;
  - MCT text is parsed by `+Sector:` headers and requires every sector and block;
  - Proxmark3 `mfc v2`/`mfc v3` JSON preserves Card identity and validates
    `SectorKeys` against trailer bytes.
- JSON parsing rejects duplicate object keys at every depth. External formats
  reject incomplete images and invalid trailers; BCC is checked when a
  four-byte card UID is known.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `nfc-classic` has 28 unit tests, 10 exchange tests, and 7 property tests:
  - access vectors: `FF 07 80` transport configuration and all 4096 access-bit
    combinations;
  - data and trailer permission tables for all eight codes;
  - value-block vectors `0/0`, `1/4`, `100/5`, and `-1/3`, plus every
    inconsistent-copy case;
  - BCC valid, invalid, incomplete, and not-applicable cases;
  - trailer validation for 1K and 4K, masked Key A accepted, unknown access
    bytes and broken complements rejected;
  - raw round trips for both layouts and rejection of every unsupported size;
  - readability with a partially known sector;
  - diff equal/changed/unknown blocks, byte offsets, trailer fields, and
    identity changes;
  - project partial-state round trip, duplicate-key rejection, and structural
    errors;
  - raw/MCT/Proxmark3 export and import round trips, format detection, MCT
    duplicate/missing/short-sector rejection, Proxmark3 BCC and SectorKey
    conflicts, and v3 ATS warnings;
  - property tests for access decode/encode, value round trips, raw image round
    trips, partial project state, external-format round trips, and an exhaustive
    geometry walk over every block in both layouts.

## Problems and resolutions

- The first project importer produced `Option<Option<CardInfo>>` because the
  project card field and the decoded card are both optional. The importer now
  flattens the nested option.
- JSON maps in `serde_json` silently keep the last duplicate key, which could
  drop a duplicate block entry. The exchange parser now deserializes through a
  recursive value visitor that rejects duplicate keys before typed decoding.
- Trailer validation initially required a complete dump. That would reject a
  partial dump whose trailer access bytes were read while Key A or Key B was
  masked. Validation now requires only bytes 6..8 to be known and ignores
  masked key fields.
- BCC validation initially took the selected card UID bytes. The check now uses
  the UID bytes in manufacturer block 0 and takes only the UID length, matching
  the previous implementation and keeping card identity separate from block
  contents.
- MCT layout detection must treat sector 16 as the 4K boundary, not sector 32;
  a 1K card has sectors 0-15. The parser selects 4K when any sector above 15
  appears and then requires the full 4K sector set.
- Content detection initially checked for an MCT header before checking the raw
  image size, so a 1024/4096-byte binary image containing the ASCII bytes
  `+Sector:` could be misclassified. Exact raw sizes now win, and MCT detection
  also requires the file to contain only ASCII graphic and whitespace
  characters.
- Project and Proxmark3 export now validate that a known card UID has 0, 4, 7,
  or 10 bytes, so the exporter cannot write an identity that its own importer
  rejects.
- Clippy rejected constant XOR expressions in BCC test fixtures and a complex
  function-pointer array type in the value tests. The fixtures now derive the
  check byte from the data, and the mutation type has an alias.

## Deviations

- The exchange layer operates on bytes. File paths, sidecar files, atomic
  writes, and backup policy belong to the phase 04 workflow layer.
- EML, MCT JSON, key-array, and sidecar formats are not part of this phase.
- Project import ignores unknown fields from the previous implementation, such
  as device, timestamps, and key provenance, so older project files remain
  readable; this phase exports only version, completeness, layout, card,
  blocks, known masks, statuses, and errors.
- Proxmark3 v3 `ATS` and `SIGNATURE` fields are not stored in the dump model and
  are reported as dropped fields on import.
- `DumpDiff` identity changes are reported only when both images have a known
  card identity. Missing identity is not treated as a change.

## Remaining blockers

- Real MCT Android and Proxmark3 client file interoperability has not been
  exercised; validation is file-level against their published field shapes.
- Hardware dump, restore, write, clone, and key workflows are phase 04 work.
- Cross-process reader locking and backup policy are not part of this phase.
