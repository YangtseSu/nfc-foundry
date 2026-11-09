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
  - wake-up preamble, `SAMConfiguration` normal mode, `GetFirmwareVersion`
    PN532 and ISO/IEC 14443A checks, register reset, and initiator mode;
  - `InListPassiveTarget` card selection and UID/ATQA/SAK parsing;
  - `InDataExchange` MIFARE authentication, block read, and a single
    block-write payload containing command, block, and data;
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
- `nfctl list` now reports every serial port as a `pn532_uart:` candidate;
  exact selection and firmware validation still happen only on open.
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
- `cargo clippy -p nfc-drivers --features hardware --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Scripted tests cover:
  - documented firmware frame bytes and ACK/extended frames;
  - junk resynchronization, split frames, NACK, error frame, and checksums;
  - open command sequence and firmware rejection;
  - Classic 1K, Classic 4K-compatible, and Type 2 selection, missing card, and
    malformed selection data;
  - authentication command bytes, auth failure, short reads, card timeout;
  - write command sequence and no-retry behavior;
  - ACK timeout retry for selection but not for writes;
  - disconnect mapping;
  - raw-mode gating, CRC register writes, raw bytes, raw bits, and RX bit
    counts.
- CLI smoke:
  - `nfctl list` reports `pn532_uart:/dev/ttyUSB0` and the platform serial
    ports;
  - `nfctl --device pn532_uart:/dev/ttyUSB0 scan` returns
    `Unsupported("the serial port did not respond as a PN532")`.
- Hardware attempt on 2026-11-09:
  - `/dev/ttyUSB0` is a CH340 (`1a86:7523`);
  - the PN532 wake-up preamble followed by `SAMConfiguration` and
    `GetFirmwareVersion` produced no bytes at 115200, 9600, 19200, 38400,
    57600, 230400, 460800, or 921600 baud, with and without DTR/RTS toggling;
  - therefore no card was selected and no Classic or Type 2 read was executed.

## Problems and resolutions

- The ACK frame `00 00 FF 00 FF 00` does not satisfy the normal
  `LEN + LCS = 0` check. The parser now recognizes the ACK marker before the
  generic length check; without this, every response was read as the ACK and
  commands failed with "sent a response before acknowledging".
- The first scripted test used 300 bytes of command data, which produced an
  extended-frame payload above the parser's sanity limit. The test uses 280
  bytes, inside both the parser limit and the PN532 extended-frame range.
- The phase 01 device manager constructed a reader but never called
  `Reader::open`, which left the trait lifecycle unused. The manager now opens
  the reader exactly once per open cycle; mock counters and a service test pin
  that behavior.
- `Pn532Protocol::firmware` was initially only used by tests and triggered a
  dead-code warning. It is exposed through `Pn532Uart::firmware` for
  diagnostics and hardware-test output.
- `serialport::available_ports` cannot distinguish a PN532 from another UART
  device without opening it. The driver lists every serial port as a candidate
  and validates the firmware only after exact selection.

## Deviations

- Raw bit exchange lets the PN532 generate and check parity; the transport
  trait does not carry caller-supplied parity bits. The bit count is returned
  from `CIU_Control`.
- Enumeration does not filter platform `ttyS*` ports. Filtering by port type
  would hide GPIO/UART adapters that can legitimately carry a PN532.
- Reconnection is manual: a failed open or I/O error is reported, and the next
  operation retries only after the manager releases the reader.
- No cross-process reader lock is added in this phase.

## Remaining blockers

- Hardware acceptance is open. The attached serial adapter never responded as
  a PN532, so Classic and Type 2 hardware reads, the same-value write, and the
  hardware error matrix (wrong reader, missing card, timeout, disconnect) still
  need a recorded run with a known PN532.
- ACR122U USB/CCID remains a stub.
- Classic geometry, workflows, recovery, and NDEF build on this driver in later
  phases.
