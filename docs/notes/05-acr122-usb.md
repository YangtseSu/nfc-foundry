# Phase 05 note: ACR122U USB/CCID driver

Date: 2026-11-10

## Implemented

- `nfc-drivers::acr122_usb` is split into CCID transport, PN532 command layer,
  and reader modules:
  - enumeration by vendor/product `072F:2200` with exact
    `acr122_usb:<bus>:<address>` connstrings, normalizing the address to three
    decimal digits;
  - CCID interface claim through `nusb`, bulk endpoint discovery, CCID
    `PC_to_RDR_IccPowerOn` and `PC_to_RDR_XfrBlock`, bounded USB timeouts,
    zero-length-packet handling at the max packet boundary, and stale bulk-IN
    draining on open;
  - ACR122 pseudo-APDU direct transmit (`FF 00 00 00 Lc D4 ...`) for PN532
    commands; the ACR122 vendor `bError` value `0x81` is accepted because it
    accompanies otherwise successful data blocks, while `ICC_MUTE` maps to a
    timeout;
  - ICC power-on, PICC operating parameters, PN532 firmware identification,
    `SetParameters`, CRC/parity register setup, initiator mode,
    `RFConfiguration` field off/on and passive retry configuration;
  - `InListPassiveTarget` selection, `InRelease`, and RF field cycling;
  - `InDataExchange` MIFARE Classic authentication, 16-byte read, and write;
  - `InCommunicateThru` raw bytes and bits, TX/RX trailing-bit handling,
    `CIU_Control` RX trailing-bit read, and MI continuation;
  - APDU status word handling with `61xx` GetResponse continuation and `6300`
    missing-card mapping;
  - raw mode disables hardware CRC, clears the Crypto1 status bit, and rejects
    ordinary MIFARE operations; ordinary mode restores CRC and rejects raw
    exchange;
  - Gen1A `0x40` / `0x43` unlock with CRC-A HALT and write frames, BCC
    validation, a mandatory reselect before the raw sequence, and ordinary mode
    restoration on every exit path;
  - `ManufacturerWriter` implementation for the Gen1A block 0 capability.
- Shared `pn53x` module for command values, register addresses, firmware and
  selection parsing, response validation, and status mapping. The PN532 UART
  driver now uses the same definitions and keeps its behavior unchanged.
- Scripted tests cover CCID framing, little-endian lengths, APDU status words,
  chaining, unbounded/truncated responses, initialization order, RF field
  setup, retries, disconnects, card selection, authentication, block access,
  raw bytes, raw bits, and the Gen1A frame sequence.
- Hardware-gated tests:
  - firmware identification and Classic read with a known key;
  - Type 2 read through a raw `READ` command;
  - raw 7-bit REQA with a 16-bit ATQA response;
  - same-value Gen1A manufacturer-block write with reselection and read-back.
- ACR122U udev rule and PC/SC coexistence guidance.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p nfc-drivers --features hardware-write --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `cargo test -p nfc-drivers`: 39 scripted tests pass.
- ACR122U hardware suite on 2026-11-10 with `pcscd` stopped and the reader at
  `acr122_usb:001:017`:
  - internal PN532 firmware `1.6`, IC `0x32`, ISO/IEC 14443A support `0x07`;
  - MIFARE Classic 1K `FA B9 CF F4`, block 4 with Key A
    `D3F7D3F7D3F7`: `03 2B D1 01 27 54 02 65 6E 4E 46 43 20 46 6F 75`;
  - raw 7-bit REQA returned ATQA `04 00` in 16 bits;
  - Type 2 tag `04 34 4C C2 83 5C 80` pages 4..7:
    `03 4B D2 0A 3E 74 65 78 74 2F 76 63 61 72 64 42`;
  - Gen1A manufacturer block 0 write on the owned magic Classic 1K card:
    original and written block
    `FA B9 CF F4 78 08 04 00 01 08 AC 26 6A A4 45 1D`, verified by
    reselection and read-back; the test wrote the current value back, so the
    UID did not change.
  - The hardware suite performs several open/close cycles. A physical
    unplug/replug was not exercised.
- Reference comparison used the installed `libnfc-ng` 1.8.0 `nfc-list -v`
  against the same `acr122_usb:001:017` connstring and the same card.

## Problems and resolutions

- The ACR122U reports CCID `bError = 0x81` on successful data blocks (the
  same status is visible in libnfc traces). Treating it as a failure broke
  every exchange. Only `ICC_MUTE` with an empty payload is now a timeout.
- The first hardware run could not claim the interface because `pcscd` held
  the PC/SC alias. The driver now reports the busy interface with an
  instruction to stop `pcscd`; the reference and direct USB paths both work
  once it is stopped.
- Selection timed out after a close because the driver explicitly turns the RF
  field off on shutdown and never turned it back on during the next open.
  Initialization now cycles the field on and applies the passive retry
  configuration.
- Selection returned zero targets even with the field on because the internal
  PN532 was not put in initiator mode. Initialization now writes
  `CIU_Control = 0x10`.
- The first Gen1A attempt failed at the 7-bit `0x40` unlock. The card had just
  been authenticated, so Crypto1 encryption was still active in the PN532.
  The manufacturer write now releases and reselects the same card before
  entering raw mode, which clears authentication state.
- A raw REQA immediately after `InRelease` did not answer on the ACR122U. The
  hardware test cycles the RF field before raw framing; this is the required
  transition for a card to return to IDLE on this reader.
- Strict CCID sequence checking rejected a stale response after an
  initialization retry. The ACR122U does not need the host sequence value, so
  the transport drains pending bulk-IN data on open and no longer rejects a
  mismatched sequence.
- The ACR122U intermittently reports `ICC_MUTE` during PICC setup. PICC
  parameter setup tolerates that transient, firmware validation remains the
  authoritative check, and reader open retries a timed-out initialization.
- The first ACR card used for the test was a CUID card, not Gen1A. The
  reference `nfc-mfsetuid` also failed its `0x40` unlock on that card. A
  read-only raw probe identified the Gen1A card on the PN532 UART; after
  swapping the cards, the ACR path passed.

## Deviations

- No `acr122_pcsc` backend and no PC/SC fallback.
- No service, CLI, or GUI command existed at this phase. Phase 24c added the
  guarded UID service, CLI, and GUI path and a PN532 Gen1A writer. The ACR122U
  service path remains unrecorded; the PN532 service path is validated.
- The hardware run used the user-approved same-value write. A UID-changing
  write was not performed; `NFC_FOUNDRY_TEST_UID` supports it, and the test restores
  the original block when the UID changes.
- Type 2 is exercised through raw `READ` only; the Type 2 page model and
  workflow arrive in phase 09.
- No LED/buzzer control, interrupt-endpoint event handling, or automatic
  polling.

## 2026-11-11 service revisit

`pcscd` was stopped and the direct USB path ran with the application services
at `acr122_usb:001:002`. Classic full dump and key scan reached 64/64 blocks
and 16/16 sectors; data-block and sector-trailer writes backed up and read
back; the guarded UID service selected Gen1A and changed and restored the UID;
restore with UID restored a full dump with a changed data block and UID; the
Classic NDEF format/write/read/restore round trip left the original dump
byte-for-byte. NTAG215 NDEF format/write/read/restore restored the original
75-byte vCard. See `09-type2-ndef.md`, `10-classic-ndef.md`,
`24c-uid-write.md`, and `24d-restore-with-uid.md`.

## Remaining blockers

- Physical unplug/replug recovery was not tested; disconnect mapping is covered
  by scripted transport tests.
- Cross-process locking and physical-reader identity are still future work.
- The PC/SC alias and the direct USB path cannot be used at the same time;
  users must stop `pcscd` for `acr122_usb`.
