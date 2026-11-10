# Phase 10 spec: Classic NDEF

Date: 2026-11-10

## Goal

Implement the NFC Forum Type MIFARE Classic NDEF mapping end to end: MAD1 and
MAD2 detection, the GPB mapping version and access conditions, NFC sector
mapping, NDEF TLV read/write/format with access-bit checks and backups, and a
CLI plus GPUI Kit view. The GUI and CLI call the same `nfc-app` services with
the same safety rules.

## Mapping facts

The Classic mapping (NXP AN1304) has no Type 2-style capability-container
record. Capability facts live in two places:

- the MAD sector trailer GPB: DA bit, MAD version, and the card-publisher
  pointer;
- each NFC sector trailer GPB: mapping major/minor version in bits 4..7 and
  read/write access conditions in bits 0..3.

This phase parses and reports those facts as the Classic capability. It does
not invent an `E1 10` capability container on a Classic card.

## Deliverables

- `nfc-classic-ndef`:
  - MAD1 parsing at sector 0 blocks 1..2 and MAD2 parsing at sector 0 plus
    sector 16 blocks 0..2.
  - CRC-8/MIFARE-MAD (poly `0x1D`, init `0xC7`) validation and encoding; stored
    AID bytes are application code then function cluster.
  - NFC AID `03 E1` detection, free/other AID preservation in the parsed
    directory, and MAD2 contiguity across sector 16.
  - GPB mapping-version and read/write-state parsing; read-only detection from
    access condition `11b` and the read-only access-bit set.
  - Logical data area built from every data block of the NFC sectors.
  - Classic TLV framing: NULL, NDEF Message, Proprietary, Terminator, and
    reserved tags skipped by their length. Three-byte length fields are
    supported and `0xFFFF` is rejected.
  - Mandatory NDEF Message TLV detection, offset, capacity, and state
    (unformatted, empty, message).
  - NDEF area encoding that preserves leading proprietary TLVs, enforces
    capacity, writes the Terminator, and zero-fills the rest.
  - Blank data area `03 00 FE`.
- `nfc-app`:
  - Classic NDEF read, inspect, write, and format services.
  - Format requires confirmation, a complete card read, a pre-write
    project backup, and read-back verification.
  - Format writes all NFC data blocks first, then sector trailers; 1K uses
    MAD1 and 4K uses MAD2. An existing Key B is preserved; a masked Key B
    falls back to the factory value.
  - NDEF write requires a formatted, writable mapping; it writes the length
    field as zero, writes the value and Terminator, then writes the final
    length field.
  - Every mutation checks writability and key availability before the backup,
    writes changed blocks only, verifies each block by read-back, checks card
    identity, and re-reads the mapping.
  - Read-only GPB, malformed CRC, non-contiguous sectors, proprietary sectors
    after usable sectors, partial data areas, and unavailable keys are
    explicit errors.
- `nfctl`:
  - `classic ndef info|read|write|format`.
  - Raw-message output and the shared Text/URI message sources.
  - `--yes`, or the interactive prompt, for format.
  - `--read-only` for a read-only format.
- GPUI Kit:
  - `gui/classic` NDEF tab with the mapping, capacity, records, Text write,
    format action, and read-only format option.
  - Format uses an explicit danger confirmation; writes are disabled for
    read-only or unformatted mappings.
  - UI integration tests cover format, NDEF write, and read-only format.
- Documentation: phase spec, note, plan status, README, NDEF domain, and
  architecture text updated.

## Non-goals

- No Type 2-style capability container on Classic cards.
- No MAD registration or card-publisher management beyond preserving the info
  byte on rewritten directories.
- No card-to-card copy, whole-card erase, or key-B discovery.
- No partial-sector format; format allocates every addressable NFC sector.
- No Type 4 ISO-DEP work.

## Safety

- Formatting changes sector keys and access bits. It requires the typed
  confirmation token, a complete read, and an atomic project backup.
- Manufacturer block 0 is never written. The UID/BCC/ATQA/SAK are rechecked
  after every mutation.
- NDEF writes never replace a sector trailer.
- A read-only mapping, a proprietary access condition, a malformed MAD CRC, or
  a non-contiguous sector list blocks every write.
- A write whose required key type is not available fails before the backup.
- Every written block is read back; a mismatch stops the workflow.
- A partial read is never treated as a complete image.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Deterministic and property tests cover MAD1/MAD2 round trips, CRC vectors,
  NFC AID detection, bad CRC, arbitrary contiguous sector subsets, mapping
  version, read-only GPB, TLV framing, capacity, long messages across sectors,
  leading proprietary preservation, and partial data areas.
- Scripted application tests cover format on 1K and 4K, confirmation, complete
  read enforcement, read-only refusal, unformatted refusal, backups, NDEF
  read/write, long messages, and malformed MAD.
- CLI tests cover `classic ndef info|read|write|format`, the confirmation
  token, and read-only format.
- GPUI UI tests cover format, NDEF write, and read-only format.
- Hardware: blank Classic 1K format, NDEF write, and read-back. Passed on
  PN532 UART on 2026-11-10 with UID `6A 31 5E 28`; the original dump was
  restored and compared byte-for-byte. The ACR122U record remains pending
  because no ACR122U was connected.
