# Phase 08 note: NDEF core

Date: 2026-11-10

## Implemented

- `nfc-ndef::record`:
  - `Tnf` values 0x00 through 0x06; 0x07 is rejected with an explicit error.
  - `NdefRecord` validation for the Empty, Unknown, and Unchanged TNF field
    rules, 255-octet TYPE/ID limits, and the 32-bit payload limit.
- `nfc-ndef::message`:
  - Short and normal record layouts, TYPE, ID, and payload fields, and the
    MB/ME/CF/SR/IL flags.
  - Physical-record parsing with byte-offset error messages and bounds checks
    before slicing or allocation.
  - Chunk reassembly for initial, middle, and terminating chunks, including
    zero-length initial chunks, MB/ME/CF placement, continuation without an
    initial chunk, a new record inside a chunk, and unterminated chunks.
  - Canonical `encode`, explicit `encode_chunked(max_chunk)`, `encoded_len`,
    `fits_in`, `decode`, and `decode_prefix`.
- `nfc-ndef::tlv`:
  - NULL, Lock Control, Memory Control, NDEF Message, Proprietary, reserved
    unknown (0x04 through 0xFC), and Terminator TLVs.
  - Short length and `0xFF` + 16-bit length forms with the 65,535-octet limit.
  - Parse/encode preservation of unknown and proprietary values and of bytes
    after the Terminator.
  - Lock Control and Memory Control length checks, at most one NDEF TLV per
    stream, and NDEF payload validation on encode.
  - `ndef_capacity` for the shortest TLV form that fits.
- `nfc-ndef::template`:
  - Text RTD with mandatory 1..63 byte language tags, UTF-8 generation, and
    UTF-8/UTF-16 decoding including BOM handling and big-endian fallback.
  - URI RTD with all 36 identifier prefixes, longest-prefix selection, and
    rejection of reserved identifier codes.
  - Smart Poster with nested URI, multiple Text titles, Action, and preserved
    additional nested records.
  - vCard 3.0 `text/vcard` records with CRLF lines, escaped values, line
    unfolding, and required FN/VERSION checks.
  - Wi-Fi WSC `application/vnd.wfa.wsc` credentials with SSID,
    authentication, encryption, network key, MAC address, and preserved
    unknown WSC attributes.
  - Bluetooth LE OOB `application/vnd.bluetooth.le.oob` records with flags,
    appearance, LE address, LE role, complete local name, and preserved
    unknown AD structures.
  - DPP bootstrapping through the URI record and the
    `application/vnd.wfa.dpp` carrier record.
- Documentation, plan status, README, architecture, testing, and NDEF design
  notes.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- 54 deterministic NDEF tests:
  - the RTD Text example bytes, short and normal record layouts, IDs, long
    payloads, chunked messages, and zero-length initial chunks;
  - reserved TNF, missing MB/ME, MB on a later record, trailing bytes,
    unterminated and reordered chunks, chunk continuation with an ID, chunked
    Empty records carrying payload, an Empty record with a payload, and
    truncated payloads;
  - all 36 URI prefixes, reserved URI codes, empty URI payloads, Smart Poster
    round trips, and missing Smart Poster URIs;
  - UTF-16 text with BOM, empty language, and odd UTF-16 input;
  - vCard round trips, newline escaping, and version/FN failures;
  - Wi-Fi open and WPA2 credentials, preserved WSC attributes, an open network
    with a key, and a payload without a Credential;
  - Bluetooth LE round trips and bad address/role lengths;
  - both DPP carriers and malformed DPP URIs;
  - all TLV length forms through 65,535 octets, NULL/Terminator, unknown and
    Lock/Memory Control TLVs, padding preservation, malformed lengths, and
    capacity boundaries.
- 11 property tests:
  - message encode/decode round trips, chunked encode/decode round trips, and
    chunk sizes from 1 to 63;
  - arbitrary-byte message and TLV decode stability;
  - Text, URI, Wi-Fi, and Bluetooth LE template round trips;
  - TLV stream round trips, unknown TLV preservation, and NDEF TLV round trips.
- No hardware test applies: this phase is hardware-free and the crate has no
  reader, card, or transport dependency.

## Problems and resolutions

- The first decoder version passed the whole buffer to the physical-record
  parser instead of the slice at the current offset, so chunk continuation
  records were re-read from the start. Fixed by parsing `&input[offset..]` while
  keeping the absolute offset for error messages.
- The vCard parser treated the final CRLF as an empty property line. Trailing
  empty lines are now removed before BEGIN/END validation.
- vCard CRLF values produced two `\n` escapes. The escaper consumes CRLF as one
  logical newline.

## Deviations from the spec

- Reserved TNF 0x07 is rejected instead of being treated as Unknown. The NDEF
  specification recommends treating unsupported TNF values as Unknown, but the
  project rule is no silent repair; the raw value is required to round-trip.
- A decoded chunked payload is normalized to one logical record. `encode` does
  not reproduce the original chunk boundaries; `encode_chunked` lets a caller
  choose them explicitly.
- vCard, Wi-Fi, and Bluetooth LE templates cover the fields listed in the
  phase plan. They preserve unknown WSC attributes and BLE AD structures but
  are not general WSC/AD/BLE stack implementations.
- The DPP template validates the `DPP:` prefix, the trailing `;;`, printable
  ASCII, and the presence of the `;K:` public-key field. It does not validate
  the base64 key or the DPP field grammar beyond that.

## Remaining blockers

- Type 2 / NTAG page mapping, capacity against a real capability container,
  and lock/config handling belong to phase 09.
- Classic MAD/CC mapping and NDEF sector access checks belong to phase 10.
- No hardware round trip is possible until the family adapters exist.
