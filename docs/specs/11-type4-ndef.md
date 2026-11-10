# Phase 11 spec: Type 4 NDEF

Date: 2026-11-10

## Goal

Implement the NFC Forum Type 4 Tag mapping over ISO-DEP / ISO 7816-4 APDUs:
read the capability container and NDEF file, write an NDEF message with NLEN
and read-back verification, and reuse `nfc-ndef` for message encoding. The
transport gains an APDU exchange on both supported readers. Hardware validation
is a separate, environment-gated step; this phase records no hardware result
yet.

## Mapping facts

- NDEF Tag Application DF name: `D2 76 00 00 85 01 01`.
- Capability Container file: ISO FID `E103`.
- Mandatory NDEF file: ISO FID from the first NDEF File Control TLV, usually
  `E104`.
- CC header: CCLEN (2), mapping version (1), MLe (2), MLc (2), then TLVs.
- NDEF File Control TLV: `04 06 <file id:2> <max size:2> <read access> <write
  access>`.
- The NDEF file starts with a two-byte big-endian NLEN followed by the NDEF
  message.
- Mapping version 2.0 (`0x20`) only. Access condition `00` is free; `FF` is
  no access; `80..=FE` is proprietary. The mandatory NDEF read access must be
  free for this adapter.
- Write procedure: update NLEN to `0000`, write the message in chunks no
  larger than MLc and the reader APDU limit, write the final NLEN, then read
  NLEN and the message back.

## Deliverables

- `nfc-transport`:
  - `ApduCapabilities { supported, max_command }` and
    `ApduResponse { data, sw }`.
  - `Reader::apdu_capabilities` and `Reader::transceive_apdu`, both defaulting
    to `Unsupported`.
  - A scripted Type 4 tag in `MockReader` that answers SELECT by DF name,
    SELECT FILE, READ BINARY, and UPDATE BINARY, including a writable/
    read-only gate and an ignored-write fault.
- `nfc-drivers`:
  - PN532 UART `InDataExchange` APDU path after the existing auto-RATS
    configuration. The APDU is not retried.
  - ACR122U pseudo-APDU `D4 40 01` APDU path with the same non-retry rule.
  - A shared `MAX_ISO_DEP_APDU` of 247 bytes, set by the ACR122U wrapper, so
    the Type 4 adapter uses one chunk limit on both readers.
  - Scripted tests for the command shape, status-word split, non-ISO rejection,
    raw-mode rejection, and oversized APDUs.
- `nfc-type4`:
  - APDU builders and status-word mapping.
  - CC and TLV parser with short and long TLV lengths, unknown/proprietary TLV
    preservation, mapping-version and size validation, and read/write access
    facts.
  - NDEF file state (empty or message) and exact capacity checks.
  - Read workflow: SELECT application, SELECT CC, read CCLEN and the remaining
    CC bytes in MLe-sized chunks, SELECT the NDEF file, read NLEN, read the
    message in MLe-sized chunks.
  - Write workflow: writability preflight, NLEN zero, chunked message write,
    final NLEN, read-back verification.
  - Clear workflow: NLEN zero with read-back verification.
  - Project JSON export containing the UID, raw CC, and current NDEF file
    bytes for pre-mutation backups.
  - Deterministic and property tests for malformed containers, access modes,
    chunking, empty state, ignored writes, and cancellation.
- `nfc-app`:
  - `type4_read`, `type4_ndef_write`, `type4_clear`, and `type4_dump` service
    methods.
  - A pre-mutation project backup, read-back verification, and a card identity
    check after every mutation.
  - Read-only files are refused before a backup is written.
  - Clearing requires an explicit confirmation at the service boundary.
- Hardware:
  - `nfctl/tests/hardware_type4.rs` with read-only and write/restore tests.
    Both are ignored by default and require `NFC_FOUNDRY_HARDWARE=1`; the write test
    additionally requires the `hardware-write` feature and
    `NFC_FOUNDRY_HARDWARE_WRITE=yes`.

## Safety

- The adapter never writes the capability container, proprietary files, or
  file settings. It only selects files and updates the mandatory NDEF file.
  An NDEF File Control TLV whose ID is outside `E100`, `E101`, or
  `E104..=E10F` is rejected, so a malformed CC cannot redirect a write into
  the CC or a proprietary file.
- A non-free read access condition, a read-only write condition, a
  proprietary write condition, or an invalid mapping version blocks the read
  or write with an explicit error.
- A write is preceded by a project backup and followed by NLEN and message
  read-back. A mismatch is `VerificationFailed`; a partial write is never
  reported as success.
- NLEN is written last, so a failure during the message write cannot make a
  truncated message visible as complete. The update is still not atomic; the
  error text and note say that NLEN may remain zero.
- Cancellation is checked before each APDU. A cancelled write may leave the
  file with NLEN zero.
- The reader lease and card identity check still come from `nfc-app`.

## Non-goals

- No mapping version 3.0 or Extended NDEF File Control TLV.
- No writing of non-mandatory NDEF files or proprietary files.
- No CC modification, read-only transition, or lock operation.
- No CLI or GUI command in this phase. The phase deliverable is the adapter
  and application service; the CLI/GUI exposure needs the same-rule parity
  work in a later phase.
- No hardware claim until a reader/card record is added to the phase note.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Deterministic tests cover NTAG 424 and DESFire CC layouts, read-only and
  proprietary access, malformed lengths, unknown TLV preservation, APDU
  offsets, empty and populated NDEF files, MLe and MLc chunk boundaries,
  ignored writes, read-only refusal, and cancellation.
- Property tests cover arbitrary CC bytes and arbitrary NDEF file bytes
  without panics or out-of-bounds slicing.
- Scripted application tests cover read, write with backup and read-back,
  clear confirmation, read-only refusal, ignored writes, dump, and
  cancellation.
- Hardware validation remains pending until a Type 4 card and a supported
  reader are available.
