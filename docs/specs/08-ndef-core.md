# Phase 08 spec: NDEF core

Date: 2026-11-10

## Goal

Implement the hardware-free NDEF core in `nfc-ndef`: the message/record codec,
the Type 2 tag TLV framing, the standard record templates, and validation and
capacity calculations. Family adapters (`nfc-type2`, `nfc-classic-ndef`,
`nfc-type4`) consume this model in later phases. The crate has no reader, card,
or transport dependency.

## Deliverables

- Record codec:
  - TNF values 0x00 through 0x06, with the reserved value 0x07 rejected.
  - MB, ME, CF, SR, and IL flags, short and normal payload lengths, TYPE and
    ID fields, and the IL/ID absence rules.
  - Empty, unknown, and unchanged TNF field constraints.
  - Chunked payload decoding that reassembles initial, middle, and terminating
    record chunks and verifies MB/ME/CF placement, chunk termination, and the
    absence of TYPE/ID on continuation chunks.
  - Canonical single-record encoding and explicit chunked encoding with a
    caller-supplied maximum chunk size.
  - Strict bounds checks with byte offsets; truncated headers, TYPE, ID, and
    payload fields are errors. A prefix decoder reports the consumed byte
    count, and `decode` rejects trailing bytes.
- TLV framing:
  - NULL, Lock Control, Memory Control, NDEF Message, Proprietary, reserved
    unknown tags, and Terminator.
  - Short length and three-byte `0xFF` length forms, with 0xFF as the long-form
    marker and a 16-bit value limit.
  - Exact parse/encode preservation of unknown and proprietary TLVs, including
    bytes after the Terminator.
  - Lock Control and Memory Control value-length validation, at most one NDEF
    TLV per stream, and NDEF payload validation on encode.
  - NDEF TLV capacity calculation from available user memory, including the TLV
    header and Terminator.
- Templates:
  - Text RTD (`T`): UTF-8 generation, UTF-8 and UTF-16 decoding with BOM
    handling, mandatory IANA language code.
  - URI RTD (`U`): the 36 NFC Forum URI identifier prefixes, longest-prefix
    encoding, and strict decoding of reserved identifier codes.
  - Smart Poster RTD (`Sp`): nested URI, Text titles, Action (`act`), and
    preservation of additional nested records.
  - vCard: `text/vcard` MIME record using vCard 3.0 with CRLF lines and escaped
    values.
  - Wi-Fi: `application/vnd.wfa.wsc` MIME record with a WSC Credential
    attribute carrying SSID, authentication type, encryption type, network
    key, and optional MAC address; unknown WSC attributes are preserved.
  - Bluetooth LE: `application/vnd.bluetooth.le.oob` MIME record with
    advertising-data structures for flags, appearance, LE device address, LE
    role, and complete local name; unknown AD structures are preserved.
  - Wi-Fi DPP: bootstrapping URI record (`U`) and the
    `application/vnd.wfa.dpp` carrier record.
- Validation and capacity:
  - `NdefMessage` must contain at least one record; every record is validated
    before encoding.
  - Exact encoded length for messages, records, TLVs, and TLV streams.
  - `NdefMessage::fits_in` and `nfc_ndef::ndef_capacity`.
- Documentation: phase spec and note, plan status, README, architecture,
  testing, and NDEF design notes.

## Non-goals

- No Type 2 / NTAG page model, capability container, lock/config handling, or
  tag read/write. Phase 09.
- No Classic MAD/CC/NDEF-sector mapping or format. Phase 10.
- No Type 4 APDU adapter. Phase 11.
- No CLI or GUI NDEF editor. Phase 09/10.
- No signature RTD, handover records, or device-information records.
- No silent repair: malformed lengths, reserved TNF values, bad chunk order,
  and reserved URI identifier codes are errors.

## Safety

- This crate is pure data handling and performs no card I/O.
- Decoding is bounded by the input buffer; a claimed length larger than the
  remaining bytes is rejected before allocation or slicing.
- Chunk reassembly is bounded by the input and rejects an unterminated or
  reordered chunk sequence.
- Encoding validates lengths and TNF constraints, so a malformed message cannot
  be silently serialized for a later card write.
- Unknown and proprietary TLVs survive a parse/encode round trip unless the
  caller explicitly constructs a different stream.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Deterministic vectors cover the Text RTD example, short and normal record
  layouts, chunk reassembly, URI prefixes, WSC attributes, BLE advertising
  structures, DPP carrier records, and every TLV length form.
- Malformed-input tests cover truncated records and TLVs, reserved TNF and URI
  codes, bad chunk order, missing MB/ME, trailing bytes, and bad Lock Control
  lengths.
- Property tests cover message encode/decode round trips (including chunked
  encoding), arbitrary-byte decode stability, TLV encode/parse round trips,
  and unknown TLV preservation.
- The crate dependency graph contains no `nfc-transport`, driver, or card
  crate.
