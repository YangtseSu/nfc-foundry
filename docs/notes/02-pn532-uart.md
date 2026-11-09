# Phase 02 note: PN532 UART driver and reader lifecycle

Date: 2026-11-09

## Implemented

- `nfc_core::CardType::infer` classifies ISO/IEC 14443A selection data:
  Classic 1K (`SAK 08`), Classic 4K (`SAK 18`), Type 2 (`SAK 00` with ATQA
  `00 44`), ISO/IEC 14443-4 (`SAK` bit `20`), and Unknown otherwise. The
  cascade bit is ignored; MIFARE Mini stays Unknown because the product does
  not support it.
- `nfc-drivers::pn532_uart` is split into frame, link, protocol, and reader
  modules:
  - normal and extended information frames, ACK, NACK, application error
    frame, LEN/LCS and DCS validation, junk resynchronization;
  - `Pn532Link` byte transport with a `serialport` implementation at 115200
    8N1, and a scripted implementation for tests;
  - every host command is sent with the HSU wake-up preamble in the same
    write, which is required after power-down and works while the chip is
    already awake;
  - `SAMConfiguration` normal mode, `GetFirmwareVersion` PN532 and ISO/IEC
    14443A checks, register reset, initiator mode, and bounded passive
    activation retries;
  - `InListPassiveTarget` card selection and UID/ATQA/SAK parsing;
  - `InDataExchange` MIFARE authentication with the four-byte UID suffix,
    16-byte block read, and a single block-write payload containing command,
    block, and data;
  - `InCommunicateThru` raw byte and bit exchange, TX trailing-bit setup, RX
    trailing-bit read from `CIU_Control`, and MI continuation;
  - retries for idempotent commands, single attempts for writes and raw
    exchange;
  - raw mode disables hardware CRC and rejects ordinary MIFARE operations;
    ordinary mode rejects raw transceive.
- `nfc-app::DeviceManager` now calls `Reader::open` after the transport factory
  constructs a reader. The reader stays open across operations and is closed
  on release or when a selection disappears. The mock reader records open and
  close counts, and application tests cover open-once/reopen-after-release.
- `nfctl list` reports every serial port as a `pn532_uart:` candidate; exact
  selection and firmware validation happen only on open.
- Environment-gated hardware tests:
  - one read test selects the present card and reads a Classic block with
    `NFCX_HARDWARE_KEY`, `NFCX_HARDWARE_KEY_TYPE`, and `NFCX_HARDWARE_BLOCK`,
    or reads Type 2 pages with a raw `READ` command;
  - a same-value Classic write behind the `hardware-write` feature and
    `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD`, refusing block 0 and sector trailers
    and verifying read-back.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p nfc-drivers --features hardware-write --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Scripted tests cover:
  - documented firmware frame bytes and ACK/extended frames;
  - junk resynchronization, split frames, NACK, error frame, and checksums;
  - open command sequence, wake preamble on every command, and firmware
    rejection;
  - Classic 1K, Classic 4K-compatible, and Type 2 selection, missing card, and
    malformed selection data;
  - authentication command bytes with a seven-byte UID suffix, auth failure,
    short reads, card timeout;
  - write command sequence and no-retry behavior;
  - ACK timeout retry for selection but not for writes;
  - disconnect mapping;
  - raw-mode gating, CRC register writes, raw bytes, raw bits, and RX bit
    counts.
- CLI smoke:
  - `nfctl --mock list`, `scan`, and `card info` still pass;
  - `nfctl list` reports `pn532_uart:/dev/ttyUSB0`;
  - `nfctl --device pn532_uart:/dev/ttyUSB0 scan` and `card info` report the
    MIFARE Classic 1K with UID `FA B9 CF F4`, ATQA `00 04`, SAK `08`.
- Hardware run on 2026-11-09:
  - adapter `/dev/ttyUSB0` is a CH340 (`1a86:7523`);
  - the PN532 reports firmware `ic=32 version=1.6 support=07`;
  - `card_info` selected a MIFARE Classic 1K (`FA B9 CF F4`, ATQA `00 04`,
    SAK `08`);
  - `authenticate` with Key A `FFFFFFFFFFFF` and `read_block(4)` succeeded in
    the gated hardware test, returning 16 zero bytes;
  - with a Type 2 tag present, `card_info` reported UID
    `04 34 4C C2 83 5C 80`, ATQA `00 44`, SAK `00`, and the raw `READ` of pages
    4..7 returned the NDEF TLV (`03 4B D2 0A ...`);
  - a 7-bit raw REQA after an RF field cycle returned ATQA `44 00` and a
    16-bit response;
  - a missing card returned `no card is present` with exit code 2;
  - a wrong serial port returned an `I/O error` with exit code 2;
  - the same-value write has not been run because no explicit write
    confirmation was given.

## Problems and resolutions

- The first implementation sent the wake-up preamble as a separate write,
  waited 100 ms, and then sent `SAMConfiguration` without a preamble. The
  PN532 did not answer. Every command now carries the HSU preamble in the same
  write, matching the known-good UART sequences; open succeeds.
- `RFConfiguration` uses command item `0x05` (`RFCI_RETRY_SELECT`) for maximum
  retries, not `0x00`. Item `0x00` produced a PN532 application error frame
  during open. The open sequence now sends
  `32 05 00 01 00` for bounded retries.
- MIFARE authentication through `InDataExchange` requires the last four UID
  bytes after the six-byte key. Without the suffix the PN532 returned an
  application error frame. The protocol now stores the selected `CardInfo` and
  appends `uid[len-4..]` to every authentication payload.
- The ACK frame `00 00 FF 00 FF 00` does not satisfy the normal
  `LEN + LCS = 0` check. The parser recognizes the ACK marker before the
  generic length check.
- The first scripted test used 300 bytes of command data, above the parser's
  extended-frame sanity limit. It now uses 280 bytes.
- The phase 01 device manager constructed a reader but never called
  `Reader::open`, which left the trait lifecycle unused. The manager now opens
  the reader exactly once per open cycle; mock counters and a service test pin
  that behavior.
- `Pn532Protocol::firmware` was initially only used by tests and triggered a
  dead-code warning. It is exposed through `Pn532Uart::firmware` for
  diagnostics and hardware-test output.
- Raw REQA initially timed out even though the card was present. After
  `InListPassiveTarget` the card is ACTIVE, and a HALTed card does not answer
  REQA; REQA only works from IDLE. `cycle_rf_field` now powers the RF field off
  and on and clears the selected-card record, returning the card to IDLE before
  raw frames. The hardware test uses it before the 7-bit REQA.
- The initial hardware run was misdiagnosed as an adapter limitation after a
  minimal probe returned no bytes. Running `nfc-list` on the same port proved
  the PN532 path worked; the validated scripts and the previous shim then
  supplied the missing wake-prefix, `RFConfiguration` item, and UID-suffix
  details. The rule to verify the reference path before reporting a hardware
  blocker is now in `AGENTS.md` and `docs/testing.md`.
- `serialport::available_ports` cannot distinguish a PN532 from another UART
  device without opening it. The driver lists every serial port as a candidate
  and validates the firmware only after exact selection.

## Deviations

- Raw bit exchange lets the PN532 generate and check parity; the transport
  trait does not carry caller-supplied parity bits. The bit count is returned
  from `CIU_Control`.
- `Pn532Uart::cycle_rf_field` is a PN532-specific utility outside the `Reader`
  trait. It is used to return a selected card to IDLE before raw frames.
- Enumeration does not filter platform `ttyS*` ports. Filtering by port type
  would hide GPIO/UART adapters that can legitimately carry a PN532.
- Reconnection is manual: a failed open or I/O error is reported, and the next
  operation retries only after the manager releases the reader.
- No cross-process reader lock is added in this phase.

## Remaining blockers

- The physical disconnect path is covered by scripted tests only; a hardware
  unplug/replug run remains for the release hardware matrix.
- The same-value write test needs explicit
  `NFCX_HARDWARE_WRITE=I_OWN_THIS_CARD` authorization.
- ACR122U USB/CCID remains a stub.
- Classic geometry, workflows, recovery, and NDEF build on this driver in later
  phases.
