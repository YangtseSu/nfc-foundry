# Phase 03 spec: MIFARE Classic core

Date: 2026-11-09

## Goal

Implement the hardware-free MIFARE Classic model in `nfc-classic`: 1K/4K
geometry, manufacturer-block BCC, access bits and trailer permissions, value
blocks, partial-dump certainty, readability, block diff, and dump interchange
for the project, raw, MCT, and Proxmark3 formats. No reader, write workflow,
key store, or CLI operation is added in this phase.

## Deliverables

- `Layout` for Classic 1K and 4K:
  - sector, block, trailer, and access-group mapping for small and large
    sectors;
  - stable `classic_1k` and `classic_4k` names used by dump files;
  - rejection of blocks and sectors outside the selected geometry.
- `bcc`: four-byte UID BCC calculation and manufacturer-block validation with
  explicit `Valid`, `Invalid { expected, actual }`, and `NotApplicable` results.
  Seven- and ten-byte UIDs are not guessed into a four-byte check.
- `access`:
  - access-bit complement decoding and encoding;
  - data read, write, increment, and decrement permissions;
  - full-trailer write permissions and Key B readability;
  - trailing permissions for all eight C1/C2/C3 codes.
- `value`: canonical value-block encoding and strict decoding of every value
  and address copy. Malformed blocks are rejected, never repaired.
- `Dump`:
  - per-block known mask and status so unknown bytes are never zero-filled;
  - exact raw import and export for 1024- and 4096-byte images;
  - optional card identity;
  - completeness and readability of blocks and sectors;
  - trailer access-byte validation that ignores masked key fields but rejects
    unknown access bytes.
- `diff`: block state `Equal`, `Changed`, or `Unknown`; byte-level differences;
  trailer Key A/Key B/access/GPB comparison; layout errors; and identity changes
  reported only when both images carry a known identity.
- `exchange`:
  - content-based detection of project JSON, raw images, MCT text, and
    Proxmark3 `mfc v2`/`mfc v3` JSON;
  - project import/export preserving partial data, known masks, statuses, and
    errors;
  - raw and MCT import/export with complete-image and trailer validation;
  - Proxmark3 import/export with Card identity, blocks, SectorKeys, access text,
    trailer-key agreement checks, and warnings for dropped v3 ATS/signature
    fields and missing UID;
  - duplicate JSON object keys rejected at every depth.
- Property tests for access codecs, value codecs, geometry walking, raw
  round trips, project state round trips, and external-format round trips.
- Phase 03 spec, note, plan status, README, architecture, testing, and
  third-party notices updates.

## Non-goals

- No reader I/O, dump read workflow, write, restore, clone, key scan, or key
  store. Those arrive with the Classic workflow phase.
- No file-path policy, sidecar format, EML, MCT JSON, or key-array interchange.
  The exchange layer operates on bytes; the workflow layer owns paths.
- No NDEF/MAD interpretation of Classic data.
- No recovery, hardnested tables, or FM11RF08S handling.
- No GUI or CLI surface.

## Safety

- Unknown bytes remain unknown through the dump model and project format.
- External formats require a complete image with structurally valid trailers.
- Raw and MCT imports have no card identity and therefore cannot validate BCC.
- Proxmark3 imports reject SectorKeys that disagree with trailer bytes rather
  than silently trusting either source.
- BCC validation uses the UID bytes stored in manufacturer block 0 and runs only
  for a known four-byte UID.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Known access, value, geometry, BCC, and trailer vectors from the previous
  implementation match.
- Together with the property tests, every 1K and 4K block maps to exactly one
  sector, trailer, and access group.
- Diff never reports a block with an unknown byte as equal.
- Project export/import round-trips partial dumps and unknown-byte masks
  exactly.
- Raw, MCT, and Proxmark3 exports reject incomplete images; Proxmark3 imports
  reject conflicting SectorKeys.

## Current validation status

Implemented and covered by deterministic tests plus property tests. No hardware
or real MCT/Proxmark3 client interoperability test is part of this phase; format
compatibility is checked against synthetic documents using the published
fields. Hardware dump and restore workflows remain in phase 04.
