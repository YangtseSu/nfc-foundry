# Phase 05 spec: ACR122U USB/CCID driver

Date: 2026-11-10

## Goal

Add the ACR122U as a second supported reader through direct USB CCID, without
PC/SC or a C shim. The reader exposes the same `Reader` boundary as the PN532
UART driver: ISO/IEC 14443A selection, MIFARE Classic authentication and block
access, raw byte and bit exchange, and a Gen1A manufacturer-block write
capability. Card-family workflows, NDEF, and recovery stay above this layer.

## Deliverables

- `nfc-drivers::acr122_usb`:
  - USB enumeration by `072F:2200` with exact
    `acr122_usb:<bus>:<address>` connstrings;
  - CCID interface claim, bulk endpoint discovery, command sequence numbers,
    bounded transfer timeouts, and abort through transfer cancellation;
  - ICC power-on, ACR122 PICC operating parameters, and PN532 firmware
    validation;
  - ACR122 pseudo-APDU direct transmit (`FF 00 00 00 Lc D4 ...`) for PN532
    commands over CCID `PC_to_RDR_XfrBlock`;
  - response status-word handling, `61xx` GetResponse continuation, and
    missing-card mapping;
  - `InListPassiveTarget` selection with UID, ATQA, SAK, and conservative card
    type inference;
  - `InDataExchange` MIFARE Classic authentication, read, and write;
  - `InCommunicateThru` raw bytes and raw bits with TX/RX trailing-bit
    handling; raw mode disables hardware CRC and rejects ordinary MIFARE
    operations;
  - key release, `InRelease`, and RF field cycling so repeated selection and
    raw frames start from IDLE;
  - Gen1A `0x40` / `0x43` unlock, CRC-A HALT and write frames, BCC validation,
    and restoration of ordinary mode after the sequence;
  - `ManufacturerWriter` for the Gen1A block 0 path.
- Shared PN53x command values, firmware parsing, selection parsing, response
  validation, and status mapping used by both drivers.
- Scripted tests for CCID framing, APDU status words, initialization, retries,
  disconnects, selection, authentication, block access, raw mode, trailing
  bits, and the Gen1A frame sequence.
- Hardware-gated ACR122U tests for firmware identification, Classic and Type 2
  reads, a raw 7-bit REQA, and an explicitly confirmed Gen1A manufacturer-block
  write with reselection and read-back.
- udev guidance for the ACR122U vendor/product pair.
- Phase 05 spec, note, plan status, README, architecture, hardware, testing, and
  udev updates.

## Non-goals

- No `acr122_pcsc` backend. The PC/SC alias is not used by the product.
- No external recovery engine or Classic workflow change.
- No service, CLI, or GUI command that writes manufacturer block 0. The driver
  capability is validated by a gated hardware test; the UID/clone workflow and
  its Advanced-mode confirmation rules belong to that later feature.
- No APDU/ISO-DEP or Type 4 support.
- No interrupt-endpoint event handling, reader LED/buzzer controls, or
  automatic polling.
- No cross-process reader lock or physical-reader identity.
- No Windows or macOS transport.

## Safety

- Enumeration never opens or claims the reader.
- `Reader::open` uses the exact bus/address connstring; no auto-selection.
- A busy interface is reported with the PC/SC alias hint instead of stealing
  the device.
- Gen1A writes require a four-byte UID MIFARE Classic 1K card, a valid block 0
  BCC, an explicit hardware-test confirmation, and a read-back after
  reselection. The hardware test writes the current block 0 unchanged unless
  `NFC_FOUNDRY_TEST_UID` is set.
- Raw mode rejects ordinary MIFARE operations; ordinary mode rejects raw
  exchange.
- No write is retried by the protocol layer; callers verify read-back.
- USB disconnects and transfer timeouts map to stable `NfcError` values.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p nfc-drivers --features hardware-write --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Scripted tests cover CCID message construction and parsing, sequence
  assignment, status-word handling, firmware checks, selection, retries,
  disconnects, authentication, block reads/writes, raw bytes, raw trailing
  bits, and Gen1A frame order.
- Hardware: with `NFC_FOUNDRY_HARDWARE=1` and an exact `NFC_FOUNDRY_DEVICE`, the ACR122U
  identifies its internal PN532, reads a Classic block and a Type 2 page, and
  returns a 16-bit ATQA from a raw 7-bit REQA. With `hardware-write`,
  `NFC_FOUNDRY_HARDWARE_WRITE=yes`, and
  `NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM=yes`, a Gen1A block 0 write is
  verified by reselection and read-back.
- The phase note records the PC/SC coexistence behavior and any hardware
  checks that could not be run.
