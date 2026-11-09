# Phase 02 spec: PN532 UART driver and reader lifecycle

Date: 2026-11-09

## Goal

Implement the PN532 UART backend behind the existing transport boundary:
serial discovery with exact connstrings, PN532 framing, reader initialization,
ISO/IEC 14443A selection, MIFARE Classic authentication and block access, and
raw byte and bit exchange. No card-family workflow, dump, or recovery logic is
added in this phase.

## Deliverables

- `nfc-drivers::pn532_uart`:
  - serial port enumeration as `pn532_uart:<port>` candidates;
  - serial open at 115200 8N1 with configurable read timeouts;
  - PN532 normal and extended frame encoding/decoding, ACK, NACK, application
    error frame, checksum validation, and junk resynchronization;
  - wake-up preamble, `SAMConfiguration` normal mode, and
    `GetFirmwareVersion` validation of the PN532 IC and ISO/IEC 14443A support;
  - register configuration for CRC, parity, Crypto1, initiator mode, and
    trailing-bit control;
  - `InListPassiveTarget` selection with UID, ATQA, SAK, and conservative card
    type inference;
  - `InDataExchange` MIFARE Classic authentication, 16-byte block read, and a
    single block-write payload containing command, block, and data;
  - `InCommunicateThru` raw byte and bit exchange with hardware parity, TX/RX
    trailing-bit handling, and MI continuation frames;
  - bounded retries for idempotent commands and single-attempt writes and raw
    exchanges;
  - explicit raw mode: CRC handling disabled, ordinary MIFARE operations
    rejected, and raw transceive rejected outside raw mode.
- `nfc-core::CardType::infer` for the shared selection-data classification.
- `nfc-app`: `DeviceManager` now calls `Reader::open` when it creates a reader,
  and keeps the reader open across operations until release or refresh.
- Mock reader lifecycle counters used by application tests.
- Environment-gated hardware tests for the selected card: Classic block read
  with a configured key, or Type 2 page read via a raw `READ` command; a
  same-value Classic write is compiled only with the `hardware-write` feature
  and requires explicit environment confirmation.
- Phase 02 spec, note, plan status, README, architecture, hardware, and testing
  updates.

## Non-goals

- No Classic geometry, access-bit validation, dump, restore, key store, or
  write workflow.
- No Type 2 page model, NDEF, or MAD handling.
- No recovery attacks or nonce acquisition.
- No ACR122U implementation.
- No cross-process reader lock or physical-reader identity.
- No automatic baud probing or serial port identification by probing during
  enumeration.

## Safety

- Enumeration never opens a serial port.
- A reader is opened only through the exact selected connstring.
- Raw transceive is unavailable until raw mode is enabled, and ordinary
  authentication/read/write is unavailable while raw mode is enabled.
- Writes are never retried by the framing layer; callers must read back.
- Hardware write tests require the `hardware-write` feature and
  `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`, write the original value back, refuse
  block 0 and sector trailers, and read back.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p nfc-drivers --features hardware --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Scripted protocol tests cover frame parsing, ACK/NACK/error handling, split
  frames, checksums, firmware validation, card selection, authentication,
  block read/write, retries, timeouts, disconnects, raw bytes, raw bits, and
  raw-mode gating.
- Hardware: with `NFCX_HARDWARE=1` and an exact `NFCX_DEVICE`, a PN532 reads a
  MIFARE Classic block and a Type 2 page. A same-value Classic write runs only
  with `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`.

## Current validation status

The protocol and lifecycle are implemented and covered by scripted tests. The
attached CH340 serial adapter produced no bytes for the PN532 wake-up and
firmware commands across the standard HSU baud rates, so the hardware acceptance
items remain open. Hardware support is not claimed until a recorded PN532 run
succeeds.
