# Phase 11 note: Type 4 NDEF

Date: 2026-11-10

## Implemented

### Transport

`nfc-transport` now exposes ISO-DEP capability and APDU exchange:

- `ApduCapabilities { supported, max_command }` and
  `ApduResponse { data, sw }`.
- `Reader::apdu_capabilities` and `Reader::transceive_apdu`; both default to
  `Unsupported`, so readers without ISO-DEP never pretend to support it.
- `MockReader` contains a stateful Type 4 tag. It answers SELECT by DF name,
  SELECT FILE, READ BINARY, and UPDATE BINARY with the expected status words.
  The tag has CC and NDEF byte arrays, a write-access gate, and an
  `ApduWrite` fault target so an ignored UPDATE BINARY can be tested without
  guessing the APDU sequence.

### Drivers

Both drivers add an APDU path:

- PN532 UART: `InDataExchange` with the APDU as the payload. The PN532 is
  already configured with `AUTO_ATR_RES_AND_RATS`, so the selected ISO-DEP
  card is activated before the first APDU. APDUs use `Retry::No`; a timeout
  is not replayed because UPDATE BINARY changes state.
- ACR122U: the APDU is carried as `FF 00 ... D4 40 01 <APDU>` through the
  pseudo-APDU layer. The same non-retry rule applies.
- Both paths split the trailing two status-word bytes into `ApduResponse`.
  A response shorter than two bytes is an I/O error.
- The ACR122U pseudo-APDU length limit is 255 bytes. `D4 40 01` plus
  `InDataExchange` headers leave 247 bytes for the complete command APDU, so
  both drivers advertise `max_command = 247`. The Type 4 adapter uses this
  value to cap UPDATE BINARY chunks.
- Scripted tests cover the command shape, response split, non-ISO card
  rejection, raw-mode rejection, oversized APDUs, and a missing status word.

### Type 4 adapter

`nfc-type4` is now a complete hardware-free adapter:

- `apdu.rs` builds SELECT by DF name, SELECT FILE, READ BINARY, and UPDATE
  BINARY. It validates 15-bit offsets, 1..=256 READ lengths, 1..=255 UPDATE
  lengths, and maps status words to typed errors.
- `cc.rs` parses the CC header and TLVs. It supports short and three-octet TLV
  lengths, preserves unknown and Proprietary File Control TLVs, requires the
  first TLV to be an NDEF File Control TLV, validates mapping version 2.0,
  MLe >= 15, NDEF file size >= 5, and exposes read/write access facts. Mapping
  version 3.0 and Extended NDEF File Control TLVs are rejected explicitly.
- `file.rs` decodes NLEN and the NDEF message, checks NLEN against the File
  Control TLV capacity, and keeps the canonical NLEN+message bytes for
  backups.
- `read.rs` selects the application and CC, reads the 15-byte header, reads
  any remaining CC bytes in MLe-sized chunks, parses the CC, selects the NDEF
  file, reads NLEN, and reads the message in MLe-sized chunks. An incomplete
  step is an error; there is no partial-success result.
- `write.rs` refuses read-only or proprietary write conditions before any
  I/O, encodes and capacity-checks the message, selects the application and
  NDEF file, writes NLEN zero, writes the message in MLc/transport-sized
  chunks, writes the final NLEN, and verifies both NLEN and the message by
  read-back. `clear_ndef` writes NLEN zero and verifies it.
- `exchange.rs` exports `{format, version, uid, cc, ndef}` project JSON. The
  NDEF field is the canonical NLEN+message file bytes, not the unused tail of
  the file.

The NTAG 424 DNA factory CC is 23 bytes and contains a Proprietary File
Control TLV after the NDEF TLV. The parser therefore iterates every TLV and
does not assume CCLEN 15 or a single TLV. The proprietary file is never read or
written by the adapter.

### Application

`nfc-app::type4` adds the service workflows:

- Read returns the parsed CC and NDEF state.
- Write reads the current tag, refuses a read-only or proprietary write
  condition, exports a project backup, writes with read-back verification,
  checks card identity, and returns the new NDEF state and backup record.
- Clear requires an explicit confirmation, performs the same backup and
  identity checks, and writes an empty NDEF file.
- Dump reads and saves a project backup without mutating the tag.
- Backups happen before the first mutation. A read-only refusal does not
  create a backup.

### Test coverage

- `cargo test -p nfc-type4`: 22 unit tests plus 2 property tests.
  - CC: NTAG 424 (23-byte, proprietary TLV), DESFire (15-byte), read-only,
    proprietary write, malformed length/version/size/first TLV, unknown TLV,
    long-form length.
  - APDU: frame bytes, offsets, invalid lengths, status-word mapping.
  - File: empty/populated, NLEN beyond capacity, truncated file.
  - Read: empty and populated NDEF, non-Type-4 refusal, read-only CC, MLe
    chunking, cancellation.
  - Write: successful write/read-back, clear, read-only refusal, ignored
    writes, oversized message, MLc chunking.
  - Property: arbitrary CC bytes and arbitrary NDEF file bytes never panic;
    successful parses never exceed the input.
- `cargo test -p nfc-app --test type4_workflow`: 7 scripted workflows.
  - read, write with backup, clear confirmation, read-only refusal without a
    backup, ignored-write detection with a backup, dump JSON, cancellation.
- `cargo test -p nfc-drivers`: driver unit tests include the new APDU paths.
- `cargo test -p nfctl --features hardware-write --test hardware_type4` only
  compiles the hardware test; it is ignored unless the hardware gates are set.

## Problems and decisions

- The ACR122U pseudo-APDU cannot carry an APDU larger than 247 bytes. Rather
  than let the Type 4 layer discover this at runtime, both drivers advertise
  the same conservative `max_command`. This keeps the chunk size deterministic
  across readers.
- The PN532 `InDataExchange` command supports a larger payload, but using 247
  everywhere avoids a second chunk-size path and a hardware-dependent NDEF
  writer.
- The Type 4 write is not atomic. NLEN is written last so a truncated message
  is not visible as complete, but a failure before the final NLEN leaves the
  file empty. This is documented in the write API and the spec.
- The capability container is never written. Formatting, read-only
  transitions, and DESFire application/file creation remain out of scope.
- Project backups contain the raw CC and canonical NLEN+message bytes. They
  are sufficient to restore the visible NDEF message, but they do not preserve
  unused tail bytes or proprietary files.
- No CLI or GUI command is added in this phase. The phase plan lists the
  transport and adapter as the deliverable; CLI/GUI exposure would require the
  same-parity rule and is deferred.

## Hardware

No Type 4 hardware record exists yet. The read and write tests are in
`crates/nfctl/tests/hardware_type4.rs` and are ignored by default.

Read-only (a Type 4 card on either supported reader):

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware --test hardware_type4 -- --ignored
```

Write and restore an original NDEF message:

```sh
NFCX_HARDWARE=1 \
NFCX_DEVICE=acr122_usb:001:017 \
NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD \
NFCX_HARDWARE_BACKUP_DIR=/tmp/nfc-type4-backups \
cargo test -p nfctl --features hardware-write --test hardware_type4 -- --ignored
```

Expected first cards: NTAG 424 DNA (factory 256-byte NDEF, free read/write) and
MIFARE DESFire EV3 8K (after external AN11004 formatting). Neither record is
claimed until the commands above run on the actual card and reader.
